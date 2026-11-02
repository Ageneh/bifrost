#!/usr/bin/env bash
#
# G5, the Hue regression gate: boot a binary on a fixed fixture (mock z2m
# with two lights in one room), drive the Hue surface, and compare what it
# answers with the golden files in tests/golden/.
#
# Usage:
#   scripts/regress.sh            compare the working tree with tests/golden/
#   scripts/regress.sh --golden   rewrite tests/golden/ from the pinned master
#
# Surfaces: v1 config, user and resources; v2 CLIP; firmware version; group 0
# in v1 and v2; scene create, recall and delete; eventstream order and
# Last-Event-ID replay; and every request Bifrost sends to zigbee2mqtt.
#
# Goldens come only from master (BIFROST_MASTER_COMMIT in scripts/lib.sh).
# A golden changes only in the commit that intends the change, with a wave
# report line naming the field and the reason. The differences are written to
# target/regress/regress.diff.

set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

mode=compare
case "${1:-}" in
    "") ;;
    --golden) mode=golden ;;
    *) die "unknown argument: $1" ;;
esac

GOLDEN="$REPO_ROOT/tests/golden"
OUT="$REPO_ROOT/target/regress/$mode"

work=$(mktemp -d)
sse=""
cleanup() {
    local status=$?
    [ -n "$sse" ] && kill "$sse" 2>/dev/null
    stop_all
    if [ "$status" -eq 0 ]; then rm -rf "$work"; else log "kept $work for inspection"; fi
}
trap cleanup EXIT

if [ "$mode" = golden ]; then
    bin=$(build_master)
else
    bin=$(build_candidate)
fi
mock=$(build_mock_z2m)

base="http://127.0.0.1:$BIFROST_HTTP_PORT"
rm -rf "$OUT"
mkdir -p "$OUT"

write_config "$work"
start_mock_z2m "$mock" "$FIXTURES/z2m/bridge.json" "$work/z2m.jsonl"
start_bifrost "$bin" "$work"
wait_for_lights 2

# Normalize JSON from stdin into $OUT/<name>.json, keys sorted.
save() {
    jq -S -f "$FIXTURES/normalize.jq" >"$OUT/$1.json" || die "$1: not JSON"
}

# <method> <path> [body] -> response body on stdout; fails on a non-2xx.
call() {
    local method="$1" path="$2" body="${3:-}"
    if [ -n "$body" ]; then
        curl -sf -X "$method" -H 'Content-Type: application/json' -d "$body" "$base$path"
    else
        curl -sf -X "$method" "$base$path"
    fi || die "$method $path failed"
}

# Parse a captured server-sent event stream into one JSON object per event.
sse_events() {
    awk '
        /^id: / { id = substr($0, 5) }
        /^data: / { data = substr($0, 7) }
        /^$/ { if (data != "") printf "{\"id\":\"%s\",\"data\":%s}\n", id, data; id = ""; data = "" }
    ' "$1"
}

user=$(call POST /api '{"devicetype":"regress#gate"}' | jq -r '.[0].success.username')
[ -n "$user" ] && [ "$user" != null ] || die "could not create a v1 user"

# events from here on are captured, to compare order and replay later
curl -sN "$base/eventstream/clip/v2" >"$work/events.sse" &
sse=$!
sleep 0.5

# --- reads ---
call GET /api/config | save v1-config
call GET "/api/$user" | save v1-user
for rtype in lights groups scenes sensors config; do
    call GET "/api/$user/$rtype" | save "v1-$rtype"
done
call GET /clip/v2/resource | jq '.data |= sort_by(.type, .id)' | save v2-resources
call GET /api/config | jq '{swversion, apiversion}' | save firmware

rid() {
    call GET "/clip/v2/resource/$1" | jq -r "[.data[] | select($2)] | first | .id"
}
room=$(rid room 'true')
home_group=$(call GET /clip/v2/resource/bridge_home |
    jq -r '.data[0].services[] | select(.rtype == "grouped_light") | .rid')
read -r -a lights <<<"$(call GET /clip/v2/resource/light | jq -r '[.data | sort_by(.id) | .[].id] | join(" ")')"

# --- group 0: every light must be driven ---
call PUT "/api/$user/groups/0/action" '{"on": false}' | save v1-group0-action
call PUT "/clip/v2/resource/grouped_light/$home_group" '{"on": {"on": true}}' | save v2-group0-action

# --- scene create, recall, delete ---
scene_body=$(jq -n --arg room "$room" --arg l1 "${lights[0]}" --arg l2 "${lights[1]}" '{
    type: "scene",
    metadata: {name: "Regress"},
    group: {rid: $room, rtype: "room"},
    actions: [
        {target: {rid: $l1, rtype: "light"}, action: {on: {on: true}, dimming: {brightness: 50}}},
        {target: {rid: $l2, rtype: "light"}, action: {on: {on: false}}}
    ]
}')
created=$(call POST /clip/v2/resource/scene "$scene_body")
scene=$(jq -r '.data[0].rid' <<<"$created")
[ -n "$scene" ] && [ "$scene" != null ] || die "scene create returned no id: $created"
# the new scene's id is random: name it, so the goldens stay stable
named() { sed "s/$scene/<scene>/g"; }

named <<<"$created" | save scene-create
call GET "/clip/v2/resource/scene/$scene" | named | save scene-get
call PUT "/clip/v2/resource/scene/$scene" '{"recall": {"action": "active"}}' | named | save scene-recall
call GET "/api/$user/scenes" | named | save scene-v1
call DELETE "/clip/v2/resource/scene/$scene" | named | save scene-delete
# the scene goes once zigbee2mqtt confirms the removal, which the mock never
# does: the status records whatever the binary answers until then
curl -s -o /dev/null -w '{"status": %{http_code}}' "$base/clip/v2/resource/scene/$scene" | save scene-after-delete

# let the last events and z2m requests land
sleep 1.5
kill "$sse" 2>/dev/null || true
wait "$sse" 2>/dev/null || true
sse=""

sse_events "$work/events.sse" >"$work/events.jsonl"
[ -s "$work/events.jsonl" ] || die "the eventstream delivered no events"

# --- eventstream replay: resuming after the first event replays the rest ---
first=$(head -n1 "$work/events.jsonl" | jq -r .id)
curl -sN --max-time 2 -H "Last-Event-ID: $first" "$base/eventstream/clip/v2" >"$work/replay.sse" || true
sse_events "$work/replay.sse" >"$work/replay.jsonl"
if ! diff <(tail -n +2 "$work/events.jsonl" | jq -r .id) <(jq -r .id "$work/replay.jsonl") >"$work/replay.diff"; then
    die "Last-Event-ID $first did not replay the later events in order: $(cat "$work/replay.diff")"
fi

# ids carry the wall clock; the order and content of the blocks is compared
jq -s '[.[] | .data | map(.id = "<event>")]' "$work/events.jsonl" | named | save eventstream

# --- what Bifrost asked zigbee2mqtt to do, in order ---
jq -s . "$work/z2m.jsonl" | named | save z2m-requests
jq -e 'map(select(.topic | endswith("/set"))) | length > 0' "$OUT/z2m-requests.json" >/dev/null \
    || die "no z2m set request was sent; group 0 and scene recall reached no light"

stop_all

if [ "$mode" = golden ]; then
    mkdir -p "$GOLDEN"
    find "$GOLDEN" -name '*.json' -delete
    cp "$OUT"/*.json "$GOLDEN/"
    log "wrote $(find "$OUT" -name '*.json' | wc -l | tr -d ' ') golden files from $BIFROST_MASTER_COMMIT to tests/golden/"
    exit 0
fi

diff_file="$REPO_ROOT/target/regress/regress.diff"
if diff -ru "$GOLDEN" "$OUT" -x README.md >"$diff_file"; then
    log "PASS: $(find "$OUT" -name '*.json' | wc -l | tr -d ' ') surfaces match the $BIFROST_MASTER_COMMIT goldens"
else
    cat "$diff_file"
    die "output differs from tests/golden; see target/regress/regress.diff"
fi
