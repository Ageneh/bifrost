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

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
FIXTURES="$REPO_ROOT/tests/fixtures/w02"

BIFROST_PIDS=()

log() {
    printf '%s: %s\n' "$(basename "$0")" "$*" >&2
}

die() {
    log "FAIL: $*"
    exit 1
}

# Build the binary from the working tree; prints its path.
build_candidate() {
    (cd "$REPO_ROOT" && cargo build --locked --quiet --bin bifrost) >&2 \
        || die "cargo build failed"
    printf '%s\n' "$REPO_ROOT/target/debug/bifrost"
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

# Stop everything this script started, newest first.
stop_all() {
    local i
    for ((i = ${#BIFROST_PIDS[@]} - 1; i >= 0; i--)); do
        kill "${BIFROST_PIDS[$i]}" 2>/dev/null || true
        wait "${BIFROST_PIDS[$i]}" 2>/dev/null || true
    done
    BIFROST_PIDS=()
}
