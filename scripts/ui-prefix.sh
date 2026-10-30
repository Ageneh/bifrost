#!/usr/bin/env bash
#
# Ingress gate: the web UI shell loads at / and behind a path-prefix proxy
# (tests/fixtures/w02/nginx-prefix.conf, run in Docker) with zero asset 404s,
# and unknown Hue paths stay non-HTML 404s on both.
#
# Usage: scripts/ui-prefix.sh [--ui-dir DIR]
#   --ui-dir DIR   serve this built UI (default: tests/fixtures/w02/ui);
#                  pass "embedded" to test the copy built into the binary.
#
# Bifrost stays on loopback. On Linux the proxy shares the host network; on
# Docker Desktop it reaches the host's loopback through host.docker.internal.

set -euo pipefail
source "$(dirname "$0")/lib.sh"

: "${BIFROST_PROXY_PORT:=9402}"
NGINX_IMAGE=nginx:1.27-alpine
PREFIX=/api/hassio_ingress/w02fixture

ui_dir="$FIXTURES/ui"
while [ $# -gt 0 ]; do
    case "$1" in
        --ui-dir) ui_dir="$2"; shift 2 ;;
        *) die "unknown argument: $1" ;;
    esac
done

work=$(mktemp -d)
proxy=""
cleanup() {
    local status=$?
    [ -n "$proxy" ] && docker rm -f "$proxy" >/dev/null 2>&1
    stop_all
    if [ "$status" -eq 0 ]; then rm -rf "$work"; else log "kept $work for inspection"; fi
}
trap cleanup EXIT

bin=$(build_candidate)
write_config "$work"

ui_args=()
[ "$ui_dir" = embedded ] || ui_args=(--ui-dir "$ui_dir")
start_bifrost "$bin" "$work" "${ui_args[@]}"

if [ "$(uname -s)" = Linux ]; then
    listen="127.0.0.1:$BIFROST_PROXY_PORT"
    upstream="127.0.0.1:$BIFROST_HTTP_PORT"
    network=(--network host)
else
    listen=80
    upstream="host.docker.internal:$BIFROST_HTTP_PORT"
    network=(-p "127.0.0.1:$BIFROST_PROXY_PORT:80")
fi
sed -e "s/@LISTEN@/$listen/" -e "s/@UPSTREAM@/$upstream/" \
    "$FIXTURES/nginx-prefix.conf" >"$work/nginx.conf"
proxy=$(docker run -d --rm "${network[@]}" \
    -v "$work/nginx.conf:/etc/nginx/nginx.conf:ro" \
    "$NGINX_IMAGE") || die "cannot start the $NGINX_IMAGE proxy"

direct="http://127.0.0.1:$BIFROST_HTTP_PORT"
proxied="http://127.0.0.1:$BIFROST_PROXY_PORT$PREFIX"
wait_for "$proxied/api/config" || die "proxy did not answer; docker logs $proxy"

failures=0
fail() {
    log "FAIL: $*"
    failures=$((failures + 1))
}

status_of() {
    curl -s -o /dev/null -w '%{http_code}' "$1"
}

# Every href/src in the shell, resolved against its <base href> the way a
# browser does, must answer 200. `origin` is scheme and host only.
check_shell() {
    local origin="$1" page="$2" expected_base="$3"
    local html base ref url code count=0

    html=$(curl -sf "$origin$page") || { fail "$origin$page did not answer 200"; return; }
    base=$(sed -n 's/.*<base href="\([^"]*\)".*/\1/p' <<<"$html" | head -n1)
    [ "$base" = "$expected_base" ] || fail "$origin$page: <base href> is '$base', expected '$expected_base'"
    grep -q "window.__BIFROST_BASE__=\"$expected_base\"" <<<"$html" \
        || fail "$origin$page: window.__BIFROST_BASE__ is not '$expected_base'"

    while IFS= read -r ref; do
        case "$ref" in
            http:* | https:* | //* | data:* | '#'*) continue ;;
            /*) url="$origin$ref" ;;
            *) url="$origin$base${ref#./}" ;;
        esac
        code=$(status_of "$url")
        count=$((count + 1))
        [ "$code" = 200 ] || fail "$origin$page references $ref -> $url answered $code"
    done < <(grep -o -E '(href|src)="[^"]+"' <<<"$html" | sed -E 's/^(href|src)="(.*)"$/\2/' | grep -v -x -F "$base")

    [ "$count" -gt 0 ] || fail "$origin$page references no assets; the check would be vacuous"
    log "$origin$page: base $base, $count asset(s) checked"
}

check_not_html_404() {
    local url="$1" code type
    code=$(curl -s -o /dev/null -w '%{http_code}' "$url")
    type=$(curl -s -o /dev/null -w '%{content_type}' "$url")
    [ "$code" = 404 ] || fail "$url answered $code, expected 404"
    case "$type" in *html*) fail "$url answered $type" ;; esac
}

check_shell "$direct" / /
check_shell "$direct" /devices /
check_shell "http://127.0.0.1:$BIFROST_PROXY_PORT" "$PREFIX/" "$PREFIX/"

for origin in "$direct" "$proxied"; do
    check_not_html_404 "$origin/clip/v2/x"
    check_not_html_404 "$origin/api/x/y/z/w/v"
    check_not_html_404 "$origin/eventstream/x"
    check_not_html_404 "$origin/_nuxt/missing.js"
done

[ "$failures" -eq 0 ] || die "$failures check(s) failed"
log "PASS"
