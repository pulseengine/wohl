#!/usr/bin/env bash
# Sigil attestation chain for a Matter verified-core wasm artifact
# (feature-loop step 6 / SWV-MATTER-002). Signs a wasm module with an Ed25519
# key and confirms the detached verifier accepts the signed artifact and
# rejects a tampered one.
#
# Usage: SIGIL=/path/to/sigil ARTIFACT=/path/to/module.wasm ./sign-verify.sh
set -euo pipefail
SIGIL="${SIGIL:-sigil}"
ARTIFACT="${ARTIFACT:?set ARTIFACT to a .wasm module}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

"$SIGIL" keygen -k "$WORK/sk.key" -K "$WORK/pk.key"
"$SIGIL" sign  -i "$ARTIFACT" -o "$WORK/signed.wasm" -k "$WORK/sk.key"

# Accept: the signed artifact must verify.
"$SIGIL" verify --input-file "$WORK/signed.wasm" --public-key "$WORK/pk.key" \
    | grep -q "Signature is valid" || { echo "FAIL: signed artifact did not verify"; exit 1; }
echo "OK: signed artifact verifies"

# Reject: a tampered copy must NOT verify.
cp "$WORK/signed.wasm" "$WORK/tampered.wasm"
printf '\xFF' | dd of="$WORK/tampered.wasm" bs=1 seek=200 count=1 conv=notrunc 2>/dev/null
if "$SIGIL" verify --input-file "$WORK/tampered.wasm" --public-key "$WORK/pk.key" \
    | grep -q "Signature is valid"; then
    echo "FAIL: tampered artifact verified (chain broken)"; exit 1
fi
echo "OK: tampered artifact rejected"
echo "PASS: sigil attestation chain intact"
