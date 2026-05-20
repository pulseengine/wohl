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

Verus runs via Bazel through the [rules_verus][rules_verus] ruleset — the
toolchain is downloaded hermetically, no manual install needed:

```bash
bazel test //proofs/verus:alert_dedup_verify
```

The Verus toolchain version is pinned in `MODULE.bazel` (the `verus`
extension). `cargo build` / `cargo test` / `cargo clippy` / Kani are
unaffected — Verus is a separate `bazel test` gate.

Expected: the test passes with `verification results:: 8 verified, 0 errors`.

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
  a `verus-strip` preprocessing step that we don't have today.
- A reviewer can read the proof file in isolation.

The trade-off is that the Verus model and the runtime code can drift.
Mitigations:

1. Constants (`DEDUP_COOLDOWN_SEC`, `MAX_ALERTS_PER_MINUTE`, etc.) are
   declared as `spec const` in the Verus file with the same values as in
   `engine.rs`.  A drift here is immediately visible.
2. The Kani harnesses (`crates/wohl-alert/plain/src/engine.rs::kani_proofs`)
   already check the same properties on the *executable* code with bounded
   inputs — they catch drift between the model and the implementation.

## CI integration

The `bazel test //proofs/verus:alert_dedup_verify` target above is
CI-ready. Wiring it into `.github/workflows/ci.yml` as a job is tracked
as a follow-up — Issue [#7](https://github.com/pulseengine/wohl/issues/7)
stays open until that job lands.

[rules_verus]: https://github.com/pulseengine/rules_verus
