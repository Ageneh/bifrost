#!/usr/bin/env bash
#
# The acceptance run, in one command.
#
# Runs every gate this programme has, in the order a failure is cheapest to
# fix, and prints one line per gate. Then it prints the hardware pass: the
# steps that need a real device, which no script can take, in the order the
# bench does them. A gate whose prerequisite is missing (docker, the browser)
# is reported as SKIP with the reason, never as a pass.
#
# Usage:
#   scripts/acceptance.sh              everything, soak included
#   scripts/acceptance.sh --no-soak    skip the thirty-minute soak
#   scripts/acceptance.sh --no-docker  skip the container gate
#
# Exit status is 0 only when every gate that ran passed.

set -uo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

soak=true
docker=true
for arg in "$@"; do
    case "$arg" in
        --no-soak) soak=false ;;
        --no-docker) docker=false ;;
        --help|-h) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown argument: $arg" ;;
    esac
done

cd "$REPO_ROOT" || die "cannot enter $REPO_ROOT"

results=()
failed=0
skipped=0

# run <gate name> <command...>
run() {
    local name="$1"
    shift
    printf '%-46s' "$name"
    local started=$SECONDS
    if output=$("$@" 2>&1); then
        printf 'PASS  %ss\n' "$((SECONDS - started))"
        results+=("PASS  $name")
    else
        printf 'FAIL  %ss\n' "$((SECONDS - started))"
        printf '%s\n' "$output" | tail -n 20 | sed 's/^/    /'
        results+=("FAIL  $name")
        failed=$((failed + 1))
    fi
}

# skip <gate name> <reason>
skip() {
    printf '%-46sSKIP  %s\n' "$1" "$2"
    results+=("SKIP  $1 ($2)")
    skipped=$((skipped + 1))
}

echo "Bifrost acceptance run — $(date -u '+%Y-%m-%dT%H:%MZ')"
echo "Commit: $(git rev-parse --short HEAD) on $(git rev-parse --abbrev-ref HEAD)"
echo

run "G1 syntax (cargo check)" cargo check --all-targets --workspace --locked
run "G2 lint (clippy baseline)" scripts/clippy-gate.sh
run "G3 tests (cargo test)" cargo test --workspace --locked
run "G4 format (cargo fmt --check)" cargo fmt --check --all
run "G5 Hue regression (goldens)" scripts/regress.sh
run "G6 upgrade from a master install" scripts/upgrade.sh
run "G7 UI behind a path prefix" scripts/ui-prefix.sh
run "G12 documented endpoints and keys" scripts/doc-check.sh

if command -v pnpm >/dev/null 2>&1; then
    run "G8 Playwright against the release binary" scripts/e2e-release.sh
else
    skip "G8 Playwright against the release binary" "pnpm is not installed"
fi

if [ "$docker" = false ]; then
    skip "G11 container boot" "--no-docker"
elif command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
    run "G11 container boot" scripts/docker-boot.sh
else
    skip "G11 container boot" "no docker daemon"
fi

if [ "$soak" = false ]; then
    skip "G9 soak (four streams, 30 min)" "--no-soak"
else
    run "G9 soak (four streams, 30 min)" scripts/soak.sh
fi

echo
echo "Gates: $(( ${#results[@]} - skipped )) ran, $failed failed, $skipped skipped."

cat <<'BENCH'

Hardware pass — not run by this script, and not verified until someone signs it:

  1. Hue app pairing .......... phone, Hue app, pairing window
  2. Desktop Hue Sync ......... desktop app, one stream
  3. Hue Sync box ............. box paired, one stream
  4. TV Ambilight + Hue ....... TV paired to a screen zone, positions match
  5. Multi-stream ............. TV and a second client at once, one stopped
  6. Home Assistant Ingress ... add-on installed, panel, apply, restart, LAN login

Each step, what to do and what success looks like is in
data/bifrost-hardware-bench.md; the record goes in evidence/hardware.md.
BENCH

[ "$failed" -eq 0 ] || exit 1
