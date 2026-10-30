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

write_config "$data"

if [ "$(uname -s)" = Linux ]; then
    sed -e "s|build: .|image: $IMAGE|" -e "s|container_name: bifrost|container_name: $PROJECT|" \
        "$REPO_ROOT/docker-compose.yaml" >"$work/docker-compose.yaml"
    grep -q "image: $IMAGE" "$work/docker-compose.yaml" || die "docker-compose.yaml has no 'build: .' to replace"
    docker compose -p "$PROJECT" -f "$work/docker-compose.yaml" up -d >/dev/null \
        || die "docker compose up failed"
else
    # inside the container, listen on every interface; the mapping keeps it on loopback
    sed -i.orig 's/ipaddress: 127.0.0.1/ipaddress: 0.0.0.0/' "$data/config.yaml"
    rm "$data/config.yaml.orig"
    container=$(docker run -d \
        -p "127.0.0.1:$BIFROST_HTTP_PORT:$BIFROST_HTTP_PORT" \
        -e BIFROST_CONFIG=/app/data/config.yaml \
        -v "$data:/app/data" \
        "$IMAGE") || die "docker run failed"
fi

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
log "PASS: config, certificate and state live in the data directory; GET / answered $code"
