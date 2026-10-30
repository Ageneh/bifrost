#!/usr/bin/env bash
#
# G6, the upgrade gate: boot the pinned master binary on a legacy install
# (tests/fixtures/w02/legacy: a config.yaml with the z2m token in the URL and
# a populated state.yaml), record its identity, then boot the working tree on
# the same directory, twice, and require the same identity each time.
#
# Identity: bridge id, every v2 resource id and type, every v1 light, group
# and scene id, and the scene names. The z2m token must not appear in the
# working tree's log or answers.

set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

TOKEN=upgrade-fixture-6f1c2a

work=$(mktemp -d)
cleanup() {
    local status=$?
    stop_all
    if [ "$status" -eq 0 ]; then rm -rf "$work"; else log "kept $work for inspection"; fi
}
trap cleanup EXIT

master=$(build_master)
candidate=$(build_candidate)
mock=$(build_mock_z2m)

base="http://127.0.0.1:$BIFROST_HTTP_PORT"
data="$work/data"
mkdir -p "$data"
cp "$FIXTURES/legacy/config.yaml" "$FIXTURES/legacy/state.yaml" "$data/"
grep -q "token=$TOKEN" "$data/config.yaml" || die "the legacy fixture lost its URL token"

# The identity a paired Hue app depends on, sorted so order cannot matter.
identity() {
    local user
    user=$(curl -sf -X POST -d '{"devicetype":"upgrade#gate"}' "$base/api" | jq -r '.[0].success.username')
    jq -n -S \
        --argjson config "$(curl -sf "$base/api/config")" \
        --argjson v2 "$(curl -sf "$base/clip/v2/resource")" \
        --argjson v1 "$(curl -sf "$base/api/$user")" '{
            bridgeid: $config.bridgeid,
            v2: [$v2.data[] | {type, id, id_v1}] | sort_by(.type, .id),
            v1_lights: $v1.lights | keys,
            v1_groups: $v1.groups | keys,
            v1_scenes: $v1.scenes | to_entries | map({id: .key, name: .value.name}),
            scene_names: [$v2.data[] | select(.type == "scene") | .metadata.name] | sort
        }'
}

# Boot `bin` on the shared directory, record its identity in `name`, stop.
boot() {
    local bin="$1" name="$2"
    start_mock_z2m "$mock" "$FIXTURES/legacy/z2m.json" "$work/z2m-$name.jsonl"
    start_bifrost "$bin" "$data"
    wait_for_lights 2
    identity >"$work/$name.json" || die "$name: could not read the identity"
    # config-writer debounces 1 s; let any state rewrite finish before stopping
    sleep 1.5
    stop_all
    cp "$data/bifrost.log" "$work/$name.log"
    : >"$data/bifrost.log"
}

boot "$master" master
[ "$(jq '.v2 | length' "$work/master.json")" -gt 10 ] || die "master saw too few resources; the fixture did not load"
[ "$(jq '.scene_names | length' "$work/master.json")" -gt 0 ] || die "master saw no scene from state.yaml"

cp "$data/config.yaml" "$work/config.before.yaml"
boot "$candidate" upgraded
boot "$candidate" restarted

failures=0
for name in upgraded restarted; do
    if ! diff -u "$work/master.json" "$work/$name.json"; then
        log "FAIL: identity after the $name boot differs from master"
        failures=$((failures + 1))
    fi
    if grep -q "$TOKEN" "$work/$name.log"; then
        log "FAIL: the z2m token appears in the $name log"
        failures=$((failures + 1))
    fi
done

if ! cmp -s "$work/config.before.yaml" "$data/config.yaml"; then
    log "note: the working tree rewrote config.yaml:"
    diff -u "$work/config.before.yaml" "$data/config.yaml" >&2 || true
fi

[ "$failures" -eq 0 ] || die "$failures check(s) failed"
log "PASS: bridge $(jq -r .bridgeid "$work/master.json"), $(jq '.v2 | length' "$work/master.json") resources kept across the upgrade and a restart"
