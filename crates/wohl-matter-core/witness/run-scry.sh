#!/usr/bin/env bash
# scry sound-abstract-interpretation robustness gate on the Matter verified-core
# seam fixture (wohl#50, Track C3). Builds the fixture core module and runs
# `scry-viz check` — sound over-approximating abstract interpretation over the
# core wasm (the DO-333 leg) — asserting the analysis result is structurally
# well-formed. Red on any malformed result or analyzer crash: scry over-
# approximates, so a well-formed result on a real rustc-emitted module is a
# genuine robustness oracle.
#
# Usage:  SCRY=/path/to/scry-viz  crates/wohl-matter-core/witness/run-scry.sh
set -euo pipefail

SCRY="${SCRY:?set SCRY to the scry-viz binary}"
cd "$(dirname "$0")"

cargo build --release --target wasm32-unknown-unknown

# Find the built wasm wherever cargo put it (robust to a redirected target dir).
TD="$(cargo metadata --format-version 1 \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
W="$TD/wasm32-unknown-unknown/release/wohl_matter_seam_mcdc.wasm"
test -f "$W" || { echo "::error::fixture wasm not built at $W"; exit 1; }

# `scry-viz check` runs the sound analysis and exits non-zero on a malformed
# result (a scry-detected structural violation) or a crash; set -e propagates it.
"$SCRY" check "$W"
echo "PASS: scry sound-AI analysis of the Matter seam is well-formed"
