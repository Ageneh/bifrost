#!/usr/bin/env bash
#
# G3, the clippy gate: run clippy over the workspace and fail when it reports
# an error or more distinct warnings than clippy-baseline.txt allows.

set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

baseline_file="$REPO_ROOT/clippy-baseline.txt"
allowed=$(sed -n 's/^warnings=//p' "$baseline_file")
toolchain=$(sed -n 's/^toolchain=//p' "$baseline_file")
[ -n "$allowed" ] || die "no warnings= line in clippy-baseline.txt"

actual_toolchain=$(rustc --version | awk '{print $2}')
if [ "$actual_toolchain" != "$toolchain" ]; then
    log "note: baseline measured with $toolchain, running $actual_toolchain; lint changes can move the count"
fi

output=$(cd "$REPO_ROOT" && cargo clippy --all-targets --workspace --locked 2>&1) || {
    printf '%s\n' "$output" >&2
    die "cargo clippy failed"
}

# one line per diagnostic: its message and the location it points at, so the
# same warning reported for the lib and the lib test counts once
diagnostics=$(awk '
    /^(warning|error)(\[[^]]*\])?: / { message = $0; next }
    /^ *--> / { if (message != "") { print message " @ " $2; message = "" } }
' <<<"$output" | sort -u)

errors=$(grep -c '^error' <<<"$diagnostics" || true)
warnings=$(grep -c '^warning' <<<"$diagnostics" || true)

[ "$errors" -eq 0 ] || die "clippy reported $errors error(s)"
if [ "$warnings" -gt "$allowed" ]; then
    die "clippy reports $warnings distinct warnings, the baseline allows $allowed"
fi
log "PASS: $warnings distinct warnings, baseline $allowed"
