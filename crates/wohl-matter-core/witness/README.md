# Witness MC/DC — Matter verified-core seam (SWV-MATTER-002 C5)

Feature-loop step 5: MC/DC (modified condition/decision coverage) on the
verified-core seam glue, via [`witness`](https://github.com/pulseengine/witness)
— the truth table, not a coverage percentage.

## What this fixture is

`src/lib.rs` is a standalone `no_std` `wasm32-unknown-unknown` **core module**
(witness cannot yet instrument a wasip2 Component). Its `seam_available`
decision mirrors `wait_available` / `recv_from` in
`crates/wohl-matter-core/compose/src/mcore.rs`: the transport is ready when a
packet is **buffered**, or — short-circuit — when one is **incoming** on the
`on-message-in` seam.

## How to run

`witness`'s `target/` is large and not vendored; build it from
`../../../../witness` (or wherever the witness repo lives), then:

```bash
WT=/path/to/witness/target/release/witness
cargo build --release --target wasm32-unknown-unknown
W=target/wasm32-unknown-unknown/release/wohl_matter_seam_mcdc.wasm
"$WT" instrument "$W" -o i.wasm
"$WT" run i.wasm --invoke-with-args 'available:0' \
                 --invoke-with-args 'available:1' \
                 --invoke-with-args 'available:2' -o r.json
"$WT" report --input r.json --format mcdc
```

## Finding — the seam decision is flat, so MC/DC = branch coverage here

`seam_available` is a **flat two-condition OR** (`buffered || incoming`). At
`opt-level = 1` on `wasm32-unknown-unknown`, rustc lowers it to a **branchless
bitwise OR**, so `witness` recovers **0 multi-condition decisions** — there is
no MC/DC truth table to reconstruct, and MC/DC is satisfied by (equivalent to)
branch coverage. This was confirmed against many spellings of the decision
(`||`, `if/else`, `black_box`, an opaque `#[inline(never)]` poll, the nested
`a || (!a && b)`): all collapse, because the two conditions are simple and
mergeable.

Per witness's "structured evidence, not a percentage" philosophy, **that is the
honest result**: the Matter seam glue contains no rich (nested, non-mergeable)
decision, so there is nothing for MC/DC to add over branch coverage. A real
MC/DC truth table only appears for decisions like the ISO leap-year rule
`(y%4==0 && y%100!=0) || (y%400==0)`, where the `&&` over distinct moduli keeps
a `br_if` chain rustc cannot fuse.

## Capability proof (witness works)

To show the pipeline + tool are wired and functional, the same steps on the
leap-year decision reconstruct full MC/DC with masking + unique-cause proofs:

```
decision #0 lib.rs:6: FullMcdc
  truth table:
    row 0: {c0=T}        -> T
    row 1: {c0=F, c1=T}  -> T
    row 2: {c0=F, c1=F}  -> F
  conditions:
    c0 (branch 0): proved via rows 0+2 (masking)
    c1 (branch 1): proved via rows 1+2 (unique-cause)
  decisions: 1/1 full MC/DC; conditions: 2 proved, 0 gap, 0 dead
```

So witness MC/DC is available for the verified-core line; it simply has no rich
decision to report on in the current seam glue. If a future seam grows a
multi-condition guard (e.g. a commissioning-window predicate), drop it into a
fixture here and `witness report --format mcdc` yields its truth table.
