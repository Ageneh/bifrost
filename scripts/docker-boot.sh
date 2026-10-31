#!/usr/bin/env bash
#
# G11, the container gate: build the image, start it the way
# docker-compose.yaml does (one data directory, BIFROST_CONFIG pointing into
# it) and require it to answer within BIFROST_BOOT_TIMEOUT seconds, write its
# certificate and state into the data directory, and serve / without a
# server error.
#
# On Linux the repository's docker-compose.yaml runs as is, with the built
# image in place of `build: .`. Docker Desktop has no host networking by
# default, so elsewhere the same mount and environment are given to
# `docker run` with a loopback port mapping.

set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

IMAGE=bifrost:gate-boot
PROJECT=bifrost-gate-boot

work=$(mktemp -d)
data="$work/data"
mkdir -p "$data"
container=""

cleanup() {
    local status=$?
    if [ -f "$work/docker-compose.yaml" ]; then
        docker compose -p "$PROJECT" -f "$work/docker-compose.yaml" logs >"$work/container.log" 2>&1 || true
        docker compose -p "$PROJECT" -f "$work/docker-compose.yaml" down >/dev/null 2>&1 || true
    elif [ -n "$container" ]; then
        docker logs "$container" >"$work/container.log" 2>&1 || true
        docker rm -f "$container" >/dev/null 2>&1 || true
    fi
    # the container wrote these as root
    docker run --rm -v "$data:/app/data" --entrypoint sh "$IMAGE" -c 'rm -rf /app/data/*' >/dev/null 2>&1 || true
    if [ "$status" -eq 0 ]; then
        rm -rf "$work"
    else
        [ -f "$work/container.log" ] && tail -n 40 "$work/container.log" >&2
        log "kept $work for inspection"
    fi
}
trap cleanup EXIT

log "building $IMAGE"
(cd "$REPO_ROOT" && quietly docker build -t "$IMAGE" .) || die "docker build failed"

# The image starts from the legacy fixture: a configuration in the shape
# master (12d9e37) wrote, with the z2m token in the url, and the state master
# left after pairing. The gate drives the admin API, so Ingress trust is
# added inside the existing `bifrost:` block — from the host (host
# networking) or the Docker bridge gateway (port mapping), which is how a
# session is skipped without inventing a password.
legacy="$REPO_ROOT/tests/fixtures/w02/legacy"
# Under compose's host networking the request reaches the bridge from the
# host's own address, not from loopback; trust that address too, and only it
host_cidr=""
if [ "$(uname -s)" = Linux ]; then
    host_ip=$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i <= NF; i++) if ($i == "src") { print $(i + 1); exit }}')
    [ -n "$host_ip" ] && host_cidr=", \"$host_ip/32\""
fi
sed -e "s/8302/$BIFROST_HTTP_PORT/" -e "s/8402/$BIFROST_HTTPS_PORT/" \
    -e "s/8502/$BIFROST_ENTM_PORT/" "$legacy/config.yaml" \
    | awk -v host_cidr="$host_cidr" '/  cert_file: cert.pem/ {
        print
        print "  auth:"
        print "    trust_ingress: true"
        print "    trusted_cidrs: [\"127.0.0.1/32\", \"172.16.0.0/12\"" host_cidr "]"
        next
    }
    { print }' >"$data/config.yaml"
cp "$legacy/state.yaml" "$data/state.yaml"
grep -q 'trust_ingress: true' "$data/config.yaml" || die "the legacy config took no auth block"

start_container() {
    if [ "$(uname -s)" = Linux ]; then
        sed -e "s|build: .|image: $IMAGE|" -e "s|container_name: bifrost|container_name: $PROJECT|" \
            "$REPO_ROOT/docker-compose.yaml" >"$work/docker-compose.yaml"
        grep -q "image: $IMAGE" "$work/docker-compose.yaml" || die "docker-compose.yaml has no 'build: .' to replace"
        docker compose -p "$PROJECT" -f "$work/docker-compose.yaml" up -d >/dev/null \
            || die "docker compose up failed"
    else
        # inside the container, listen on every interface; the mapping keeps it on loopback
        sed -i.orig 's/ipaddress: 127.0.0.1/ipaddress: 0.0.0.0/' "$data/config.yaml"
        rm -f "$data/config.yaml.orig"
        container=$(docker run -d \
            -p "127.0.0.1:$BIFROST_HTTP_PORT:$BIFROST_HTTP_PORT" \
            -e BIFROST_CONFIG=/app/data/config.yaml \
            -v "$data:/app/data" \
            "$IMAGE") || die "docker run failed"
    fi
}

stop_container() {
    if [ "$(uname -s)" = Linux ]; then
        docker compose -p "$PROJECT" -f "$work/docker-compose.yaml" down >/dev/null 2>&1 || true
    else
        docker rm -f "$container" >/dev/null 2>&1 || true
        container=""
    fi
}

start_container

base="http://127.0.0.1:$BIFROST_HTTP_PORT"
started=$SECONDS
wait_for "$base/api/config" || die "the container did not answer GET /api/config within ${BIFROST_BOOT_TIMEOUT}s"
log "answered after $((SECONDS - started))s"

[ -f "$data/cert.pem" ] || die "no cert.pem in the data directory; paths do not resolve against it"

# a new Hue user changes the state; the writer persists it after 1 s
curl -sf -X POST -d '{"devicetype":"boot#gate"}' "$base/api" >/dev/null || die "POST /api failed"
deadline=$((SECONDS + 5))
until [ -f "$data/state.yaml" ]; do
    [ "$SECONDS" -lt "$deadline" ] || die "no state.yaml in the data directory"
    sleep 0.2
done

code=$(curl -s -o /dev/null -w '%{http_code}' "$base/")
case "$code" in
    200 | 404) ;;
    *) die "GET / answered $code" ;;
esac

# A draft and an apply, so the three files every fixture-based gate cannot
# see exist and carry known content: state.yaml, config.draft.yaml and
# config.yaml.bak all live in the data directory, not in the image.
admin() {
    curl -sf -H 'x-ingress-path: /api/hassio_ingress/gate' \
        -H 'content-type: application/json' "$@"
}
document=$(admin "$base/bifrost/config") || die "GET /bifrost/config failed"
admin -X PUT -d "$(jq -c '.bridge.name = "Bifrost gate" | {revision: 0, config: .}' <<<"$document")" \
    "$base/bifrost/config/draft" >/dev/null || die "writing the draft failed"
admin -X POST -d '{"revision": 1}' "$base/bifrost/config/apply" >/dev/null || die "apply failed"

# a second draft, left unapplied: apply consumes the draft it promotes, so
# this is the one a recreated container has to still find
revision=$(admin "$base/bifrost/config/draft" | jq -r '.revision')
document=$(admin "$base/bifrost/config") || die "GET /bifrost/config failed"
admin -X PUT -d "$(jq -c --argjson revision "$revision" '.bridge.name = "Bifrost gate draft" | {revision: $revision, config: .}' <<<"$document")" \
    "$base/bifrost/config/draft" >/dev/null || die "writing the second draft failed"

for file in state.yaml config.draft.yaml config.yaml.bak; do
    [ -f "$data/$file" ] || die "no $file in the data directory"
done
hash_files() {
    local tool=sha256sum
    command -v sha256sum >/dev/null 2>&1 || tool="shasum -a 256"
    (cd "$data" && $tool state.yaml config.draft.yaml config.yaml.bak)
}

before=$(hash_files)

# Recreate the container the way a compose user does: down, then up. The
# data directory is the only thing that survives, and it must carry all
# three files unchanged.
log "recreating the container"
stop_container
start_container
wait_for "$base/api/config" || die "the recreated container did not answer within ${BIFROST_BOOT_TIMEOUT}s"
after=$(hash_files)
[ "$before" = "$after" ] || die "a recreated container changed state.yaml, config.draft.yaml or config.yaml.bak"

# existence is not enough: the draft the recreated container reads must be
# the one that was left, not an empty file with the right name
draft_name=$(admin "$base/bifrost/config/draft" | jq -r '.config.bridge.name')
[ "$draft_name" = "Bifrost gate draft" ] \
    || die "the recreated container's draft says '$draft_name', not 'Bifrost gate draft'"

log "PASS: config, certificate, state, draft and backup live in the data directory; a recreated container keeps all three files; GET / answered $code"
