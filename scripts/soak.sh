#!/usr/bin/env bash
# The wave-10 soak: four streams for thirty minutes, inventory load on top.
#
# Runs `tests/soak.rs`, which is `#[ignore]`d so the normal suite stays fast,
# and keeps its output in a log next to the evidence file. The assertions are
# the wave's numbers: RSS growth under 10 %, four rows with frames on each,
# a drop ratio that does not climb, and the Resources mutex never held long.
#
# Usage: scripts/soak.sh [minutes]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

minutes="${1:-30}"
# one file, overwritten by the latest run. A per-wave name is a trap: a run
# launched without BIFROST_SOAK_LOG silently replaces the log an earlier wave's
# report cites (wave 10's did).
log="${BIFROST_SOAK_LOG:-$root/evidence/soak.log}"

BIFROST_SOAK_MINUTES="$minutes" cargo test --test soak -- --ignored --nocapture 2>&1 | tee "$log"
