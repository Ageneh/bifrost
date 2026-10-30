#!/usr/bin/env bash
# Playwright against the release binary, not a dev server.
#
# Builds the bundle, builds the Rust binary with that bundle embedded, boots
# the binary on a temporary configuration, and runs the suite with the
# fixture server's shell and asset routes pointed at the binary. The admin
# API stays on the fixtures: it needs a zigbee network and hardware behind
# it, while the artefact under test is the bundle the binary serves.
#
# `frontend/e2e/release.spec.ts` then hashes every chunk the binary serves
# against the bundle just built, so a stale or empty embed fails the run.
#
# Usage: scripts/e2e-release.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

http_port="${BIFROST_E2E_HTTP_PORT:-4320}"
https_port="${BIFROST_E2E_HTTPS_PORT:-4321}"
entm_port="${BIFROST_E2E_ENTM_PORT:-4322}"

pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend build
cargo build --release --locked

work="$(mktemp -d)"
bridge_pid=""
cleanup() {
    if [ -n "$bridge_pid" ]; then
        kill "$bridge_pid" 2>/dev/null || true
        wait "$bridge_pid" 2>/dev/null || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT

cat >"$work/config.yaml" <<YAML
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: 127.0.0.1
  netmask: 255.0.0.0
  gateway: 127.0.0.1
  timezone: UTC
  http_port: $http_port
  https_port: $https_port
  entm_port: $entm_port

bifrost:
  state_file: state.yaml
  cert_file: cert.pem
  auth:
    trust_ingress: true
    trusted_cidrs: ["127.0.0.1/32"]

z2m: {}
YAML

target/release/bifrost --config "$work/config.yaml" &
bridge_pid=$!

# the binary is up when its session endpoint answers; it has no other
# readiness signal that does not need a session
ready=false
for _ in $(seq 1 200); do
    if curl -fsS -o /dev/null "http://127.0.0.1:$http_port/bifrost/auth/session"; then
        ready=true
        break
    fi
    sleep 0.1
done
if [ "$ready" != true ]; then
    echo "the release binary did not answer on port $http_port" >&2
    exit 1
fi

BIFROST_E2E_UPSTREAM="http://127.0.0.1:$http_port" \
BIFROST_E2E_OUTPUT="${BIFROST_E2E_OUTPUT:-frontend/test-results/release-$(date +%s)}" \
    pnpm --dir frontend test:e2e
