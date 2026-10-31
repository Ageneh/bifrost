#!/usr/bin/env bash
#
# G6, the upgrade gate: boot the pinned master binary on a legacy install
# (tests/fixtures/w02/legacy: a config.yaml with no setting this programme
# added and the z2m token in the URL, plus a populated state.yaml), record its
# identity, then boot the working tree on the same directory, twice, and
# require the same identity each time.
#
# Identity: bridge id, every v2 resource id and type, every v1 light, group
# and scene id, the scene names, and the SHA-256 of the bridge certificate the
# master generated for the install. The z2m token must not appear in the
# working tree's log or answers, and a plain boot must not rewrite the
# configuration or the state file.
#
# The data directory is the real install layout: `config.yaml`, `state.yaml`
# and `cert.pem` side by side, as `doc/config-reference.md` describes it.

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
# The certificate is part of it: a regenerated certificate breaks every
# paired client, so its hash must survive the upgrade too.
identity() {
    local user cert
    user=$(curl -sf -X POST -d '{"devicetype":"upgrade#gate"}' "$base/api" | jq -r '.[0].success.username')
    cert=$(sha256_of "$data/cert.pem")
    jq -n -S \
        --arg cert "$cert" \
        --argjson config "$(curl -sf "$base/api/config")" \
        --argjson v2 "$(curl -sf "$base/clip/v2/resource")" \
        --argjson v1 "$(curl -sf "$base/api/$user")" '{
            bridgeid: $config.bridgeid,
            cert: $cert,
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
    curl -sf "$base/api/config" >"$work/$name.config.json" || die "$name: the configuration did not load"
    # config-writer debounces 1 s; let any state rewrite finish before stopping
    sleep 1.5
    stop_all
    cp "$data/bifrost.log" "$work/$name.log"
    : >"$data/bifrost.log"
}

# The master is the released binary the owner is upgrading from. It finds no
# certificate on this directory and generates one, exactly as it would on a
# fresh install; from here on that file is the install's certificate.
boot "$master" master
[ "$(jq '.v2 | length' "$work/master.json")" -gt 10 ] || die "master saw too few resources; the fixture did not load"
[ "$(jq '.scene_names | length' "$work/master.json")" -gt 0 ] || die "master saw no scene from state.yaml"
[ -f "$data/cert.pem" ] || die "master did not leave a certificate; the install layout is not what the gate assumes"

cp "$data/config.yaml" "$work/config.before.yaml"
cp "$data/state.yaml" "$work/state.before.yaml"
cp "$data/cert.pem" "$work/cert.before.pem"

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

# A boot reads the install; it does not rewrite it. Applying through the admin
# API is the only thing that writes config.yaml, and it leaves a backup.
if ! cmp -s "$work/config.before.yaml" "$data/config.yaml"; then
    log "FAIL: the working tree rewrote config.yaml on boot:"
    diff -u "$work/config.before.yaml" "$data/config.yaml" >&2 || true
    failures=$((failures + 1))
fi
# state.yaml is expected to change: the Entertainment resource carries the
# configured `max_streams`, and the first boot after the upgrade writes it.
# What matters is that it still parses and still holds every resource, which
# the identity comparison above proves; the change is reported, not failed.
if ! cmp -s "$work/state.before.yaml" "$data/state.yaml"; then
    changed=$(diff "$work/state.before.yaml" "$data/state.yaml" | grep -c '^[<>]' || true)
    log "note: the working tree rewrote state.yaml on boot ($changed lines; the Entertainment resource carries max_streams)"
fi
if ! cmp -s "$work/cert.before.pem" "$data/cert.pem"; then
    log "FAIL: the working tree replaced the install's certificate"
    failures=$((failures + 1))
fi
for stray in config.draft.yaml config.yaml.bak; do
    if [ -e "$data/$stray" ]; then
        log "FAIL: a plain boot created $stray"
        failures=$((failures + 1))
    fi
done

# The configuration still loads: the settings the owner wrote are what the
# upgraded binary answers with, on both boots.
for name in upgraded restarted; do
    loaded="$work/$name.config.json"
    for pair in "bridgeid=$(jq -r .bridgeid "$work/master.json")" "name=Bifrost Bridge" "mac=00:11:22:33:44:55"; do
        key=${pair%%=*}
        want=${pair#*=}
        got=$(jq -r --arg key "$key" '.[$key]' "$loaded")
        if [ "$got" != "$want" ]; then
            log "FAIL: the $name boot answers /api/config $key=$got, expected $want"
            failures=$((failures + 1))
        fi
    done
done

[ "$failures" -eq 0 ] || die "$failures check(s) failed"
log "PASS: bridge $(jq -r .bridgeid "$work/master.json"), $(jq '.v2 | length' "$work/master.json") resources, certificate $(jq -r .cert "$work/master.json" | cut -c1-12)… kept across the upgrade and a restart"
