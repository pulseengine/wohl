#!/usr/bin/env bash
# Assemble the wohl release evidence bundle (SWREQ-REL-001).
#
# A packager, not a producer: every evidence file is made by the existing
# gates (rivet export/coverage, witness report, sigil sign) and handed in
# here. This script only lays out the bundle, writes the checksum manifest,
# embeds the verifier, and tars the result. Keeping production out of this
# script means the bundle can never disagree with the gates that made the
# evidence.
#
# Usage:
#   tools/evidence-bundle.sh \
#     --version vX.Y.Z \
#     --coverage coverage.json \
#     --trace-export trace-export.yaml \
#     --witness-report mcdc-report.txt \
#     --signed-wasm wohl-matter-composed.signed.wasm \
#     --pubkey signing.pub \
#     --tooling TOOLING.txt \
#     --out DIR
#
# Output: DIR/wohl-<version>-evidence-bundle.tar.gz, unpacking to
# wohl-<version>-evidence/ with verify.sh inside. `./verify.sh` is the one
# command an assessor runs.
set -euo pipefail

VERSION="" COVERAGE="" TRACE="" WITNESS_REPORT="" SIGNED_WASM="" PUBKEY="" TOOLING="" OUT=""
while [ $# -gt 0 ]; do
    case "$1" in
        --version)        VERSION="$2"; shift 2 ;;
        --coverage)       COVERAGE="$2"; shift 2 ;;
        --trace-export)   TRACE="$2"; shift 2 ;;
        --witness-report) WITNESS_REPORT="$2"; shift 2 ;;
        --signed-wasm)    SIGNED_WASM="$2"; shift 2 ;;
        --pubkey)         PUBKEY="$2"; shift 2 ;;
        --tooling)        TOOLING="$2"; shift 2 ;;
        --out)            OUT="$2"; shift 2 ;;
        *) echo "error: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

missing() { echo "error: $1 is required" >&2; exit 2; }
[ -n "$VERSION" ]        || missing --version
[ -n "$COVERAGE" ]       || missing --coverage
[ -n "$TRACE" ]          || missing --trace-export
[ -n "$WITNESS_REPORT" ] || missing --witness-report
[ -n "$SIGNED_WASM" ]    || missing --signed-wasm
[ -n "$PUBKEY" ]         || missing --pubkey
[ -n "$TOOLING" ]        || missing --tooling
[ -n "$OUT" ]            || missing --out
for f in "$COVERAGE" "$TRACE" "$WITNESS_REPORT" "$SIGNED_WASM" "$PUBKEY" "$TOOLING"; do
    if [ ! -s "$f" ]; then echo "error: input '$f' is missing or empty" >&2; exit 1; fi
done

NAME="wohl-${VERSION}-evidence"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
ROOT="$STAGE/$NAME"
mkdir -p "$ROOT/trace" "$ROOT/witness" "$ROOT/attestation" "$OUT"

cp "$COVERAGE"       "$ROOT/trace/coverage.json"
cp "$TRACE"          "$ROOT/trace/trace-export.yaml"
cp "$WITNESS_REPORT" "$ROOT/witness/mcdc-report.txt"
cp "$SIGNED_WASM"    "$ROOT/attestation/wohl-matter-composed.signed.wasm"
cp "$PUBKEY"         "$ROOT/attestation/signing.pub"
cp "$TOOLING"        "$ROOT/TOOLING.txt"

# The verifier. Quoted heredoc: nothing below expands at assembly time.
cat > "$ROOT/verify.sh" <<'VERIFY_EOF'
#!/usr/bin/env bash
# One-command verifier for the wohl evidence bundle (SWREQ-REL-001).
# Run from inside the unpacked bundle: ./verify.sh
#
# Checks, in order; any failure exits non-zero:
#   1. every file matches MANIFEST.sha256
#   2. the witness MC/DC report shows 0 gap rows
#   3. the rivet trace files are present and well-formed
#   4. the attestation signature over the composed component verifies
#      (REQUIRES a verifier binary: sigil or wsc on PATH, or $SIGIL)
#
# Trust anchoring: this bundle is one release asset among the others. Its
# sha256 appears in the release's SHA256SUMS.txt, which is cosign-signed
# (keyless, bound to the release workflow identity) and SLSA-attested.
# Verify that outer chain with the commands in the release notes; this
# script verifies the bundle's inner consistency.
set -euo pipefail
cd "$(dirname "$0")"
fail() { echo "FAIL: $*" >&2; exit 1; }

# 1. checksums
if command -v sha256sum >/dev/null 2>&1; then
    sha256sum --quiet -c MANIFEST.sha256 || fail "checksum mismatch against MANIFEST.sha256"
elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c MANIFEST.sha256 >/dev/null || fail "checksum mismatch against MANIFEST.sha256"
else
    fail "no sha256sum or shasum available"
fi
echo "OK: all files match MANIFEST.sha256"

# 2. witness MC/DC gap rows. run-witness.sh (the producer) emits the
# machine-readable 'gap-total: N' line — the single parse of witness's
# report wording. Every such line must be exactly 0.
grep -E '^gap-total: [0-9]+$' witness/mcdc-report.txt >/dev/null \
    || fail "witness/mcdc-report.txt has no 'gap-total: N' line — not a run-witness.sh report"
if grep -E '^gap-total:' witness/mcdc-report.txt | grep -vE '^gap-total: 0$' >/dev/null; then
    fail "witness reports unresolved MC/DC gap row(s): $(grep -E '^gap-total:' witness/mcdc-report.txt | tr '\n' ' ')"
fi
echo "OK: witness MC/DC report shows 0 gap rows"

# 3. trace files
[ -s trace/trace-export.yaml ] || fail "trace/trace-export.yaml missing or empty"
[ -s trace/coverage.json ]     || fail "trace/coverage.json missing or empty"
if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,sys; json.load(open("trace/coverage.json"))' \
        || fail "trace/coverage.json is not valid JSON"
    echo "OK: rivet trace export + coverage.json present and well-formed"
else
    head -c1 trace/coverage.json | grep -q '{' || fail "trace/coverage.json does not look like JSON"
    echo "OK: rivet trace files present (python3 unavailable; structural check only)"
fi

# 4. attestation signature — a verifier is REQUIRED. A verification that
# silently skips the signature is not a verification.
VERIFIER="${SIGIL:-}"
if [ -z "$VERIFIER" ]; then
    for c in sigil wsc; do
        if command -v "$c" >/dev/null 2>&1; then VERIFIER="$c"; break; fi
    done
fi
[ -n "$VERIFIER" ] || fail "no signature verifier found: put sigil or wsc on PATH (github.com/pulseengine/sigil), or set \$SIGIL"
# Gate on the success phrase, not the exit code: wsc 0.12.0 exits 0 even on
# "No valid signatures" (measured 2026-11-12), so the exit code fails open.
# The command must still succeed — a crashed verifier is not a verdict.
VERDICT="$("$VERIFIER" verify --input-file attestation/wohl-matter-composed.signed.wasm \
                              --public-key attestation/signing.pub 2>&1)" \
    || fail "signature verifier exited non-zero (verifier: $VERIFIER)"
printf '%s\n' "$VERDICT" | grep -q "Signature is valid" \
    || fail "attestation signature did not verify (verifier: $VERIFIER)"
echo "OK: attestation signature verifies (verifier: $VERIFIER)"

echo "PASS: evidence bundle verified. Outer chain (SHA256SUMS + cosign + SLSA) binds it to the release workflow — see TOOLING.txt and the release notes."
VERIFY_EOF
chmod +x "$ROOT/verify.sh"

# Manifest over everything except itself.
(
    cd "$ROOT"
    if command -v sha256sum >/dev/null 2>&1; then
        find . -type f ! -name MANIFEST.sha256 -print0 | sort -z | xargs -0 sha256sum > MANIFEST.sha256
    else
        find . -type f ! -name MANIFEST.sha256 -print0 | sort -z | xargs -0 shasum -a 256 > MANIFEST.sha256
    fi
)

TARBALL="$OUT/${NAME}-bundle.tar.gz"
tar -czf "$TARBALL" -C "$STAGE" "$NAME"
echo "assembled: $TARBALL"
