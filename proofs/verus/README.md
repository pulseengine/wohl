# Verus proofs for Wohl

This directory holds [Verus](https://github.com/verus-lang/verus) deductive
proofs for selected Wohl components.  Verus is an SMT-backed verifier for
a superset of Rust; it lets us state and discharge correctness obligations
that go beyond what Kani's bounded model checker can rule out (e.g.
properties that quantify over all `u64` timestamps).

## Current proofs

| File | Component | Invariants |
|---|---|---|
| `alert_dedup.rs` | `wohl-alert` (`AlertDispatcher`) | dedup, rate-limit (Issue [#7](https://github.com/pulseengine/wohl/issues/7)) |

## Running

Verus is **not** invoked by `cargo` or by the regular CI workflow today.
It must be installed locally and run by hand:

```bash
# 1. Install Verus 0.2026.05.17 (pre-built macOS arm64 / linux x86_64 release):
#    https://github.com/verus-lang/verus/releases
# 2. Install the matching rust toolchain:
rustup install 1.95.0
# 3. Verify:
verus proofs/verus/alert_dedup.rs
```

Expected output:

```
verification results:: 8 verified, 0 errors
```

This output has been confirmed locally against Verus
`0.2026.05.17.e479cce` (the macOS arm64 release) for the file as
committed.

## Relationship to the executable code

Verus and the runtime Rust are **two specifications of the same state
machine**.  The `proofs/verus/` files mirror the relevant portion of the
runtime data structures into Verus's `nat`/`Seq`/`Set` ghost types and
prove the invariants on the ghost.  Each file documents the
correspondence with the source.

We chose this *separate-file* layout (rather than inline Verus annotations
in `engine.rs`) so that:

- Plain `cargo build` / `cargo test` / `cargo clippy` / Kani are unaffected.
  Verus's macro syntax (`requires`, `ensures`, `forall`, `&&&`, ...)
  doesn't parse under stock rustc, so inline annotations would require
  a `verus-strip` preprocessing step that we don't have in CI today.
- A reviewer can read the proof file in isolation without needing Verus
  installed.

The trade-off is that the Verus model and the runtime code can drift.
Mitigations:

1. Constants (`DEDUP_COOLDOWN_SEC`, `MAX_ALERTS_PER_MINUTE`, etc.) are
   declared as `spec const` in the Verus file with the same values as in
   `engine.rs`.  A drift here is immediately visible.
2. The Kani harnesses (`crates/wohl-alert/plain/src/engine.rs::kani_proofs`)
   already check the same properties on the *executable* code with bounded
   inputs — they catch drift between the model and the implementation.

## CI integration

Adding a Verus job to `.github/workflows/ci.yml` is an orchestrator
decision (Verus needs a specific nightly toolchain and a downloaded
release binary; see `MODULE.bazel` in [pulseengine/rivet][rivet] for a
Bazel-driven pattern via [rules_verus][rules_verus]).

For now the Verus proofs are run by hand by anyone who modifies
`engine.rs` or the proof file, and a follow-up issue should track wiring
this into CI.

[rivet]: https://github.com/pulseengine/rivet
[rules_verus]: https://github.com/pulseengine/rules_verus
