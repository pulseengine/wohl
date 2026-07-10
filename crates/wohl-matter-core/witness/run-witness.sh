#!/usr/bin/env bash
# Witness MC/DC gate for the Matter verified-core seam fixture (wohl#50, Track C2).
#
# Builds the fixture core module (wasm32-unknown-unknown), then instrument -> run
# -> report via `witness`, and FAILS on any unresolved MC/DC gap row. The seam
# decision is a flat `buffered || incoming` OR, which rustc lowers branchlessly
# at opt-level=1, so witness recovers 0 multi-condition decisions today (the
# documented honest finding — see README.md). This gate is therefore a
# NO-REGRESSION guard: it stays green until the seam gains a rich (nested,
# non-mergeable) decision with an uncovered condition, at which point a gap row
# appears and the gate goes red.
#
# Usage:  WITNESS=/path/to/witness  crates/wohl-matter-core/witness/run-witness.sh
set -euo pipefail

WITNESS="${WITNESS:?set WITNESS to the witness binary}"
cd "$(dirname "$0")"

cargo build --release --target wasm32-unknown-unknown

# Find the built wasm wherever cargo put it (robust to a redirected target dir).
TD="$(cargo metadata --format-version 1 \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
W="$TD/wasm32-unknown-unknown/release/wohl_matter_seam_mcdc.wasm"
test -f "$W" || { echo "::error::fixture wasm not built at $W"; exit 1; }

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
"$WITNESS" instrument "$W" -o "$WORK/i.wasm"
"$WITNESS" run "$WORK/i.wasm" \
    --invoke-with-args 'available:0' \
    --invoke-with-args 'available:1' \
    --invoke-with-args 'available:2' \
    -o "$WORK/r.json"
REPORT="$("$WITNESS" report --input "$WORK/r.json" --format mcdc)"
echo "$REPORT"

# Report line shape: "... conditions: N proved, G gap, D dead". Gate on G.
GAP="$(printf '%s\n' "$REPORT" | grep -oE '[0-9]+ gap' | grep -oE '[0-9]+' | head -1)"
GAP="${GAP:-0}"
if [ "$GAP" -ne 0 ]; then
    echo "::error::witness reports $GAP unresolved MC/DC gap row(s) on the Matter seam"
    exit 1
fi
echo "PASS: 0 MC/DC gap rows on the Matter seam (flat OR — 0 recoverable decisions, MC/DC == branch coverage)"
