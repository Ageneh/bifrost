# shellcheck shell=bash
# Shared helpers for the gate scripts: build a binary, boot it on a fixture
# directory, wait for it to answer, and stop it again. Sourced, not run.
#
# Ports default to the wave-2 allocation and can be overridden, so two gate
# runs on one machine do not collide.

: "${BIFROST_HTTP_PORT:=8302}"
: "${BIFROST_HTTPS_PORT:=8402}"
: "${BIFROST_ENTM_PORT:=8502}"
: "${BIFROST_MOCK_Z2M_PORT:=9302}"
: "${BIFROST_BOOT_TIMEOUT:=10}"

# The master commit every golden file and the upgrade baseline come from.
# Changing it regenerates history, not a fixture: see tests/golden/README.md.
BIFROST_MASTER_COMMIT=12d9e37

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
# shellcheck disable=SC2034 # read by the scripts that source this file
FIXTURES="$REPO_ROOT/tests/fixtures/w02"

BIFROST_PIDS=()

log() {
    printf '%s: %s\n' "$(basename "$0")" "$*" >&2
}

die() {
    log "FAIL: $*"
    exit 1
}

# SHA-256 of a file, in the one spelling both Linux and macOS have.
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# Run a command, showing its output only when it fails.
quietly() {
    local output
    if ! output=$("$@" 2>&1); then
        printf '%s\n' "$output" >&2
        return 1
    fi
}

# Build the binary from the working tree, or take BIFROST_BIN when set (a
# prebuilt binary under test); prints its path.
build_candidate() {
    if [ -n "${BIFROST_BIN:-}" ]; then
        [ -x "$BIFROST_BIN" ] || die "BIFROST_BIN=$BIFROST_BIN is not executable"
        printf '%s\n' "$BIFROST_BIN"
        return
    fi
    (cd "$REPO_ROOT" && quietly cargo build --locked --bin bifrost) || die "cargo build failed"
    printf '%s\n' "$REPO_ROOT/target/debug/bifrost"
}

# Build the pinned master binary from `git archive`, not a worktree, so no
# checkout is created or touched; cached under target/. Prints its path.
build_master() {
    local root="$REPO_ROOT/target/regress/master-$BIFROST_MASTER_COMMIT"
    if [ ! -f "$root/src/Cargo.toml" ]; then
        mkdir -p "$root/src"
        git -C "$REPO_ROOT" archive "$BIFROST_MASTER_COMMIT" | tar -x -C "$root/src" \
            || die "cannot export $BIFROST_MASTER_COMMIT"
    fi
    (cd "$root/src" && quietly cargo build --locked --bin bifrost --target-dir "$root/target") \
        || die "cargo build of $BIFROST_MASTER_COMMIT failed"
    printf '%s\n' "$root/target/debug/bifrost"
}

# Build the mock z2m server; prints its path.
build_mock_z2m() {
    (cd "$REPO_ROOT" && quietly cargo build --locked --example mock-z2m) \
        || die "cargo build of the mock z2m server failed"
    printf '%s\n' "$REPO_ROOT/target/debug/examples/mock-z2m"
}

# Write the gate configuration into a fresh fixture directory. Loopback only:
# nothing a gate starts is reachable from the network.
write_config() {
    local dir="$1"
    cat >"$dir/config.yaml" <<EOF
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: 127.0.0.1
  netmask: 255.0.0.0
  gateway: 127.0.0.1
  timezone: UTC
  http_port: $BIFROST_HTTP_PORT
  https_port: $BIFROST_HTTPS_PORT
  entm_port: $BIFROST_ENTM_PORT

z2m:
  server1:
    url: ws://127.0.0.1:$BIFROST_MOCK_Z2M_PORT
EOF
}

# Fail early when another process holds a gate port; a stale server would
# answer in place of the binary under test.
require_free_port() {
    local port="$1"
    if curl -s -o /dev/null --max-time 1 "http://127.0.0.1:$port/"; then
        die "port $port is already serving; stop that process or set its BIFROST_*_PORT"
    fi
}

# Wait until GET <url> answers 200, or give up after BIFROST_BOOT_TIMEOUT.
wait_for() {
    local url="$1" deadline=$((SECONDS + BIFROST_BOOT_TIMEOUT))
    while [ "$SECONDS" -lt "$deadline" ]; do
        if [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 1 "$url")" = 200 ]; then
            return 0
        fi
        sleep 0.2
    done
    return 1
}

# Start `bin` with `dir` as its working directory, the layout both the master
# and the wave binary read config.yaml from. Extra arguments go to the binary.
start_bifrost() {
    local bin="$1" dir="$2"
    shift 2
    require_free_port "$BIFROST_HTTP_PORT"
    (cd "$dir" && exec env RUST_LOG="${RUST_LOG:-info}" "$bin" "$@") >>"$dir/bifrost.log" 2>&1 &
    BIFROST_PIDS+=("$!")
    wait_for "http://127.0.0.1:$BIFROST_HTTP_PORT/api/config" \
        || die "bifrost did not answer within ${BIFROST_BOOT_TIMEOUT}s; log: $dir/bifrost.log"
}

# Start the mock z2m server and wait until it accepts connections, so Bifrost
# does not spend its first connect attempt and a 3 s retry on an empty port.
start_mock_z2m() {
    local bin="$1" devices="$2" log="$3" deadline=$((SECONDS + BIFROST_BOOT_TIMEOUT))
    "$bin" --port "$BIFROST_MOCK_Z2M_PORT" --devices "$devices" --log "$log" >>"$log.stderr" 2>&1 &
    BIFROST_PIDS+=("$!")
    # curl exits 7 only while nothing listens
    while curl -s -o /dev/null --max-time 1 "http://127.0.0.1:$BIFROST_MOCK_Z2M_PORT/"; [ $? -eq 7 ]; do
        [ "$SECONDS" -lt "$deadline" ] || die "mock z2m did not listen; log: $log.stderr"
        sleep 0.1
    done
}

# Wait until the v2 API lists `count` lights, i.e. the mock's devices landed.
wait_for_lights() {
    local count="$1" deadline=$((SECONDS + BIFROST_BOOT_TIMEOUT))
    while [ "$SECONDS" -lt "$deadline" ]; do
        if [ "$(curl -s "http://127.0.0.1:$BIFROST_HTTP_PORT/clip/v2/resource/light" | jq '.data | length')" = "$count" ]; then
            return 0
        fi
        sleep 0.2
    done
    die "bifrost did not list $count lights within ${BIFROST_BOOT_TIMEOUT}s"
}

# Stop everything this script started, newest first.
stop_all() {
    local i
    for ((i = ${#BIFROST_PIDS[@]} - 1; i >= 0; i--)); do
        kill "${BIFROST_PIDS[$i]}" 2>/dev/null || true
        wait "${BIFROST_PIDS[$i]}" 2>/dev/null || true
    done
    BIFROST_PIDS=()
}
