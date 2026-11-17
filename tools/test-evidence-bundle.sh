#!/usr/bin/env bash
# rivet: verifies SWREQ-REL-001
#
# Fixture-driven oracle for the evidence-bundle machinery (SWV-REL-001).
# Runs on every PR without building the producer tools. The negative
# controls are the point: a verifier that cannot fail verifies nothing.
#
#   green:  assembled bundle passes its own verify.sh (stub verifier)
#   red 1:  no signature verifier on PATH          -> verify.sh fails
#   red 2:  a tampered evidence file               -> verify.sh fails
#   red 3:  witness report with non-zero gap rows  -> verify.sh fails
#   red 4:  verifier that rejects the signature    -> verify.sh fails
#   red 5:  verifier exits non-zero despite phrase -> verify.sh fails
#
# The stub verifier exercises the wiring, not the crypto. The real
# signature path runs with the sigil binary in the release workflow and
# in CI's sigil-attest gate (which includes its own tamper control).
set -euo pipefail
cd "$(dirname "$0")/.."

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# ── fixtures ──────────────────────────────────────────────────────────
F="$WORK/fixtures"; mkdir -p "$F"
printf '{"artifacts": 70, "coverage": "fixture"}\n' > "$F/coverage.json"
printf 'artifacts:\n  - id: FIXTURE-001\n'          > "$F/trace-export.yaml"
# Mirrors run-witness.sh output: the human report line(s) plus the
# machine-readable 'gap-total: N' contract line verify.sh gates on.
printf 'seam fixture — conditions: 3 proved, 0 gap, 0 dead\ngap-total: 0\n' > "$F/mcdc-report.txt"
printf 'not-really-wasm-but-signed-bytes\n'         > "$F/signed.wasm"
printf 'fixture-public-key\n'                       > "$F/signing.pub"
printf 'fixture tooling: this is a test bundle\n'   > "$F/TOOLING.txt"

# Stub verifiers (wiring only — see header).
BIN_OK="$WORK/bin-ok"; mkdir -p "$BIN_OK"
printf '#!/bin/sh\necho "Signature is valid"\n' > "$BIN_OK/sigil"
chmod +x "$BIN_OK/sigil"
BIN_BAD="$WORK/bin-bad"; mkdir -p "$BIN_BAD"
printf '#!/bin/sh\necho "Signature is INVALID"\n' > "$BIN_BAD/sigil"
chmod +x "$BIN_BAD/sigil"
# Perverse verifier: prints the success phrase but exits non-zero — pins the
# exit-code gate independently of the phrase gate.
BIN_EXIT1="$WORK/bin-exit1"; mkdir -p "$BIN_EXIT1"
printf '#!/bin/sh\necho "Signature is valid"\nexit 1\n' > "$BIN_EXIT1/sigil"
chmod +x "$BIN_EXIT1/sigil"

assemble() { # $1 = witness report file, $2 = out dir
    tools/evidence-bundle.sh --version vTEST \
        --coverage "$F/coverage.json" --trace-export "$F/trace-export.yaml" \
        --witness-report "$1" --signed-wasm "$F/signed.wasm" \
        --pubkey "$F/signing.pub" --tooling "$F/TOOLING.txt" --out "$2" >/dev/null
    tar -xzf "$2/wohl-vTEST-evidence-bundle.tar.gz" -C "$2"
}

# Minimal PATH so no real sigil/wsc leaks in from the host.
BASE_PATH="/usr/bin:/bin"

# ── green: intact bundle + stub verifier ──────────────────────────────
G="$WORK/green"; mkdir -p "$G"; assemble "$F/mcdc-report.txt" "$G"
PATH="$BIN_OK:$BASE_PATH" "$G/wohl-vTEST-evidence/verify.sh" >/dev/null \
    || { echo "FAIL: intact bundle did not verify"; exit 1; }
echo "ok: intact bundle verifies"

# Each red control must fail AND fail at ITS check — a non-zero exit for an
# unrelated earlier reason would leave the control vacuous.
expect_fail() { # $1 = label, $2 = expected FAIL-message fragment; stdin-less run via $3...
    local label="$1" frag="$2"; shift 2
    local out
    if out="$("$@" 2>&1)"; then
        echo "FAIL: $label — verify.sh passed"; exit 1
    fi
    printf '%s\n' "$out" | grep -q "$frag" \
        || { echo "FAIL: $label — failed, but not at the expected check (wanted '$frag', got: $out)"; exit 1; }
    echo "ok: $label"
}

# ── red 1: no verifier available ──────────────────────────────────────
run_noverifier() { PATH="$BASE_PATH" SIGIL='' "$G/wohl-vTEST-evidence/verify.sh"; }
expect_fail "refuses without a signature verifier" "no signature verifier" run_noverifier

# ── red 2: tampered evidence file ─────────────────────────────────────
T="$WORK/tamper"; mkdir -p "$T"; assemble "$F/mcdc-report.txt" "$T"
printf 'tampered\n' >> "$T/wohl-vTEST-evidence/trace/trace-export.yaml"
run_tampered() { PATH="$BIN_OK:$BASE_PATH" "$T/wohl-vTEST-evidence/verify.sh"; }
expect_fail "rejects a tampered file" "checksum mismatch" run_tampered

# ── red 3: witness gap rows (first decision clean — head -1 must not mask it)
printf 'conditions: 2 proved, 0 gap, 0 dead\nconditions: 1 proved, 2 gap, 0 dead\ngap-total: 2\n' > "$F/mcdc-gap.txt"
P="$WORK/gap"; mkdir -p "$P"; assemble "$F/mcdc-gap.txt" "$P"
run_gap() { PATH="$BIN_OK:$BASE_PATH" "$P/wohl-vTEST-evidence/verify.sh"; }
expect_fail "rejects non-zero witness gap rows" "gap row" run_gap

# ── red 4: verifier rejects the signature ─────────────────────────────
run_badsig() { PATH="$BIN_BAD:$BASE_PATH" "$G/wohl-vTEST-evidence/verify.sh"; }
expect_fail "rejects a bad signature verdict" "did not verify" run_badsig

# ── red 5: verifier exits non-zero despite printing the phrase ────────
run_exit1() { PATH="$BIN_EXIT1:$BASE_PATH" "$G/wohl-vTEST-evidence/verify.sh"; }
expect_fail "rejects a non-zero verifier exit" "exited non-zero" run_exit1

# ── red 6: trace file MISSING and dropped from the manifest ───────────
# The checksum step passes over whatever the manifest lists, so a producer
# bug that both loses the file and omits its manifest line must be caught
# by the dedicated presence check, not ride through vacuously green.
M="$WORK/notrace"; mkdir -p "$M"; assemble "$F/mcdc-report.txt" "$M"
rm "$M/wohl-vTEST-evidence/trace/trace-export.yaml"
grep -v 'trace/trace-export.yaml' "$M/wohl-vTEST-evidence/MANIFEST.sha256" \
    > "$M/manifest.tmp" && mv "$M/manifest.tmp" "$M/wohl-vTEST-evidence/MANIFEST.sha256"
run_notrace() { PATH="$BIN_OK:$BASE_PATH" "$M/wohl-vTEST-evidence/verify.sh"; }
expect_fail "rejects a missing trace file (even when the manifest omits it)" \
    "trace/trace-export.yaml missing or empty" run_notrace

# ── red 7: trace file EMPTY with a matching manifest entry ────────────
# sha256 of the empty string is a valid checksum, so an emptied file with a
# refreshed manifest line passes step 1 — the -s presence check is what
# must go red.
E="$WORK/emptytrace"; mkdir -p "$E"; assemble "$F/mcdc-report.txt" "$E"
: > "$E/wohl-vTEST-evidence/trace/trace-export.yaml"
EMPTY_SHA="e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
sed "s|^[0-9a-f]\{64\}\(  ./trace/trace-export.yaml\)\$|$EMPTY_SHA\1|" \
    "$E/wohl-vTEST-evidence/MANIFEST.sha256" > "$E/manifest.tmp" \
    && mv "$E/manifest.tmp" "$E/wohl-vTEST-evidence/MANIFEST.sha256"
run_emptytrace() { PATH="$BIN_OK:$BASE_PATH" "$E/wohl-vTEST-evidence/verify.sh"; }
expect_fail "rejects an empty trace file (checksum alone would pass)" \
    "trace/trace-export.yaml missing or empty" run_emptytrace

echo "PASS: evidence-bundle machinery — 1 green, 7 red controls"
