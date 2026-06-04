# Sigil attestation — Matter verified-core component (SWV-MATTER-002, step 6)

Feature-loop step 6: sign the verified-core build artifact with
[`sigil`](https://github.com/pulseengine/sigil) and confirm the **detached
verifier accepts the signed artifact and rejects a tampered one**.

## The chain (demonstrated)

`sign-verify.sh` runs `sigil keygen → sign → verify`, plus a tamper check.
Demonstrated 2026-06-04 on a Matter verified-core wasm artifact:

```
$ sigil keygen -k sk.key -K pk.key
  Secret key saved to [sk.key]     # Ed25519, 65 B
  Public key saved to [pk.key]     # 33 B
$ sigil sign -i module.wasm -o signed.wasm -k sk.key
$ sigil verify --input-file signed.wasm  --public-key pk.key
  Signature is valid.              # ACCEPT
$ # flip one byte of signed.wasm →
$ sigil verify --input-file tampered.wasm --public-key pk.key
  No valid signatures              # REJECT
```

Run it yourself:

```bash
SIGIL=/path/to/sigil/target/release/sigil \
ARTIFACT=/path/to/some-matter-core.wasm \
  ./sign-verify.sh
```

`sigil verify` reports the verdict in its output (`Signature is valid` vs
`No valid signatures`); the script greps for that rather than relying on the
exit code.

## Scope / productionization

This is the local demonstration of the chain on a verified-core wasm artifact.
The production form is a **build-stage attestation in CI**: after
`bazel build //:wohl-matter-composed`, sign the composed component with `sigil`
(or the existing `release.yml` SLSA + cosign path) so downstream consumers can
verify what was built. That CI step is the remaining productionization; the
chain itself is shown working here.

The witness MC/DC evidence (`../witness/`) feeds the same attestation story:
`witness attest` emits a DSSE envelope that `sigil`'s `wsc verify` consumes, so
the coverage evidence can be signed alongside the component.
