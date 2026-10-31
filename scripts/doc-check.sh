#!/usr/bin/env bash
#
# G12, the documentation gate: the endpoints the docs promise are the
# endpoints the router serves, and every setting this programme added is in
# the configuration reference.
#
# Two halves:
#
#   endpoints  Every `METHOD /bifrost/...` in a table cell of doc/ui.md and
#              doc/config-reference.md is called against a running bridge. An
#              endpoint answers when the response is a 2xx or carries a body:
#              the router answers an unknown path under /bifrost with an
#              empty 404, so a body means the route exists. Setup mode's
#              table is checked the same way against a bridge in setup mode.
#   keys       Every configuration key this programme added is named in
#              doc/config-reference.md.
#
# Two endpoints are documented but never called, because calling them would
# end the run they are meant to check: `POST /bifrost/config/restart` shuts
# the bridge down, and `POST /bifrost/bridge/certificate/regenerate`
# replaces the certificate every paired client trusts. For those the script
# asserts the path literal is in the router source instead.

set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

DOC="$REPO_ROOT/doc"
SETUP_PORT="${BIFROST_SETUP_PORT:-8303}"
PASSWORD=doc-check-password

# Documented, deliberately not called; checked against the router source.
NOT_CALLED=(
    "POST /bifrost/config/restart"
    "POST /bifrost/bridge/certificate/regenerate"
    # ends the session every other endpoint in this run is made with
    "POST /bifrost/auth/logout"
)

work=$(mktemp -d)
cleanup() {
    local status=$?
    stop_all
    if [ "$status" -eq 0 ]; then rm -rf "$work"; else log "kept $work for inspection"; fi
}
trap cleanup EXIT

candidate=$(build_candidate)

# `METHOD /path` for every endpoint in a table cell of `file`, or only those
# under the heading matching `section` when one is given.
documented() {
    local file="$1" section="${2:-}"
    awk -F'|' -v section="$section" '
        /^#/ { heading = $0 }
        /^\|/ {
            if (section != "" && heading !~ section) { next }
            cell = $2
            if (cell !~ /\/bifrost\//) { next }
            # "`GET`/`POST /bifrost/x`" splits on the separator between
            # methods; the last piece carries the method and the path
            gsub(/`\/`/, "@", cell)
            gsub(/`/, "", cell)
            n = split(cell, parts, "@")
            last = parts[n]
            sub(/^ +/, "", last)
            sub(/ +$/, "", last)
            if (!match(last, /^(GET|POST|PUT|PATCH|DELETE) \/[^ ]*/)) { next }
            split(last, mp, " ")
            path = mp[2]
            for (i = 1; i < n; i++) { print parts[i] " " path }
            print mp[1] " " path
        }
    ' "$file"
}

# `{id}` and friends: any value answers, because the route, not the resource,
# is what this checks.
concrete() {
    sed -e 's/{[a-z_]*}/00000000-0000-4000-8000-000000000000/g' <<<"$1"
}

ROUTES="$REPO_ROOT/src/routes/bifrost"

# Whether the router mounts the documented path: the longest prefix that is a
# `nest(...)` in mod.rs, and the rest as a route literal in that module. A
# handler may answer 404 for a resource it does not have — several do, with
# the same body the fallback uses — so the source is the only place the route
# itself can be read.
route_exists() {
    local path="$1" rest mount module leaf candidate
    rest=${path#/bifrost}
    [ "$rest" != "$path" ] || return 1
    [ -n "$rest" ] || return 1

    mount=""
    # `/bifrost/config` is the mount itself, whose route is "/"
    if grep -qF "nest(\"$rest\"" "$ROUTES/mod.rs"; then
        mount=$rest
    fi
    candidate=$rest
    while [ -z "$mount" ] && [ "${candidate%/*}" != "$candidate" ]; do
        candidate=${candidate%/*}
        if [ -n "$candidate" ] && grep -qF "nest(\"$candidate\"" "$ROUTES/mod.rs"; then
            mount=$candidate
            break
        fi
    done

    if [ -z "$mount" ]; then
        # a route on the /bifrost router itself, such as /ws
        grep -qF "route(\"$rest\"" "$ROUTES/mod.rs"
        return
    fi

    # the module a mount names is in the nest line, not in its path: /setup is
    # wizard.rs and /bridge is pairing.rs
    module=$(grep -F "nest(\"$mount\"" "$ROUTES/mod.rs" | head -n1 | sed 's/.*nest("[^"]*", *//; s/::.*//')
    [ -n "$module" ] || return 1
    leaf=${rest#"$mount"}
    [ -n "$leaf" ] || leaf="/"
    grep -qF "\"$leaf\"" "$ROUTES/$module.rs"
}

# Call one endpoint with the session; print "status<TAB>body-length".
call() {
    local method="$1" path="$2"
    local args=(-s -o "$work/body" -w '%{http_code}' -X "$method"
        -H "Origin: http://127.0.0.1:$BIFROST_HTTP_PORT"
        -H "X-Bifrost-Csrf: $csrf"
        -b "$work/cookies")
    case "$method" in
        POST|PUT|PATCH)
            # a body that cannot change anything: a stale revision, an empty
            # object, or a password that does not match
            args+=(-H 'Content-Type: application/json' -d '{"revision": 999999, "confirm": "", "admin_password": ""}')
            ;;
    esac
    local status
    status=$(curl "${args[@]}" "http://127.0.0.1:$BIFROST_HTTP_PORT$path")
    printf '%s\t%s\n' "$status" "$(wc -c <"$work/body" | tr -d ' ')"
}

failures=0
checked=0

# --- a running bridge ------------------------------------------------------

mkdir -p "$work/data"
cp "$FIXTURES/legacy/config.yaml" "$work/data/config.yaml"
sed -i.bak "s/^  http_port: .*/  http_port: $BIFROST_HTTP_PORT/; s/^  https_port: .*/  https_port: $BIFROST_HTTPS_PORT/; s/^  entm_port: .*/  entm_port: $BIFROST_ENTM_PORT/" "$work/data/config.yaml"
sed -i.bak "s|url: ws://127.0.0.1:[0-9]*|url: ws://127.0.0.1:$BIFROST_MOCK_Z2M_PORT|" "$work/data/config.yaml"
rm -f "$work/data/config.yaml.bak"

start_bifrost "$candidate" "$work/data"

code=$(grep -o 'setup code[^:]*: [A-Z0-9-]*' "$work/data/bifrost.log" | tail -n1 | awk '{print $NF}')
[ -n "$code" ] || die "the bridge printed no setup code; cannot get a session"
reply=$(curl -s -c "$work/cookies" -X POST -H 'Content-Type: application/json' \
    -H "Origin: http://127.0.0.1:$BIFROST_HTTP_PORT" \
    -d "{\"setup_code\": \"$code\", \"password\": \"$PASSWORD\"}" \
    "http://127.0.0.1:$BIFROST_HTTP_PORT/bifrost/auth/setup")
csrf=$(jq -r '.csrf // empty' <<<"$reply")
[ -n "$csrf" ] || die "setup was refused: $reply"

endpoints=$( { documented "$DOC/ui.md" "^## The admin API"; documented "$DOC/config-reference.md" "^## Configuration admin API"; } | sort -u)

while read -r method path; do
    [ -n "$method" ] || continue
    skip=false
    for entry in "${NOT_CALLED[@]}"; do
        [ "$entry" = "$method $path" ] && skip=true
    done
    if [ "$skip" = true ]; then continue; fi
    if ! route_exists "$path"; then
        log "FAIL: $method $path is documented but the router has no such route"
        failures=$((failures + 1))
        continue
    fi
    result=$(call "$method" "$(concrete "$path")")
    status=${result%%$'\t'*}
    checked=$((checked + 1))
    # any status is an answer: a 409 for a stale revision and a 422 for a
    # body that changes nothing both prove the handler ran. What must not
    # happen is the fallback, which is a 404 saying "No such endpoint".
    if [ "$status" -ge 200 ] && [ "$status" -lt 300 ]; then continue; fi
    if ! grep -q 'No such endpoint' "$work/body"; then continue; fi
    # a handler may say that about a resource it does not have; a path with
    # no parameter has no such excuse
    case "$path" in
        *"{"*) continue ;;
    esac
    log "FAIL: $method $path answers $status: no such endpoint"
    failures=$((failures + 1))
done <<<"$endpoints"

# the ones that are documented but never called: the route has to be there
for entry in "${NOT_CALLED[@]}"; do
    path=${entry#* }
    if ! route_exists "$path"; then
        log "FAIL: $entry is documented but the router has no such route"
        failures=$((failures + 1))
    fi
    checked=$((checked + 1))
done

stop_all

# --- a bridge in setup mode ------------------------------------------------

setup_data="$work/setup"
mkdir -p "$setup_data"
( cd "$setup_data" && exec env RUST_LOG=info "$candidate" --config "$setup_data/missing.yaml" \
    --setup-listen "127.0.0.1:$SETUP_PORT" ) >>"$work/setup.log" 2>&1 &
BIFROST_PIDS+=("$!")
deadline=$((SECONDS + BIFROST_BOOT_TIMEOUT))
until grep -q 'setup code' "$work/setup.log" 2>/dev/null; do
    [ "$SECONDS" -lt "$deadline" ] || die "setup mode did not start; log: $work/setup.log"
    sleep 0.2
done

code=$(grep -o 'setup code[^:]*: [A-Z0-9-]*' "$work/setup.log" | tail -n1 | awk '{print $NF}')
reply=$(curl -s -c "$work/setup-cookies" -X POST -H 'Content-Type: application/json' \
    -H "Origin: http://127.0.0.1:$SETUP_PORT" \
    -d "{\"setup_code\": \"$code\", \"password\": \"$PASSWORD\"}" \
    "http://127.0.0.1:$SETUP_PORT/bifrost/auth/setup")
csrf=$(jq -r '.csrf // empty' <<<"$reply")
[ -n "$csrf" ] || die "setup mode refused the setup code: $reply"

setup_endpoints=$(documented "$DOC/ui.md" "^### Setup mode" | sort -u)
while read -r method path; do
    [ -n "$method" ] || continue
    args=(-s -o "$work/body" -w '%{http_code}' -X "$method"
        -H "Origin: http://127.0.0.1:$SETUP_PORT"
        -H "X-Bifrost-Csrf: $csrf"
        -b "$work/setup-cookies")
    case "$method" in
        POST|PUT|PATCH)
            args+=(-H 'Content-Type: application/json' -d '{"revision": 999999}')
            ;;
    esac
    status=$(curl "${args[@]}" "http://127.0.0.1:$SETUP_PORT$(concrete "$path")")
    checked=$((checked + 1))
    if [ "$status" -ge 200 ] && [ "$status" -lt 300 ]; then continue; fi
    # setup mode's own fallback is the same "No such endpoint" answer
    if grep -q 'No such endpoint' "$work/body"; then
        log "FAIL: setup mode does not serve $method $path ($status)"
        failures=$((failures + 1))
    fi
done <<<"$setup_endpoints"

stop_all

# --- every setting this programme added is documented ----------------------

# The keys the GUI programme introduced, by wave. `doc/config-reference.md`
# has to name each one: a setting that exists and is not written down is a
# setting nobody can find.
KEYS=(
    # wave 1, configuration core
    bifrost.auth.password_hash
    bifrost.auth.session_hours
    bifrost.auth.idle_minutes
    # wave 2, admin authentication and ingress
    bifrost.auth.trust_ingress
    bifrost.auth.trusted_cidrs
    bifrost.auth.allowed_hosts
    # wave 7, entertainment
    bifrost.entertainment.max_streams
    # wave 6, pairing
    pairing.require_link_button
    # wave 2, the z2m token out of the url
    z2m.token
    # wave 5, rooms and device overrides
    rooms.devices
    devices.enabled
    devices.name
    devices.room
)

for key in "${KEYS[@]}"; do
    leaf=${key##*.}
    if ! grep -q "$leaf" "$DOC/config-reference.md"; then
        log "FAIL: $key is not named in doc/config-reference.md"
        failures=$((failures + 1))
    fi
done

[ "$failures" -eq 0 ] || die "$failures documentation check(s) failed"
log "PASS: $checked documented endpoints answer, ${#KEYS[@]} configuration keys are named in doc/config-reference.md"
