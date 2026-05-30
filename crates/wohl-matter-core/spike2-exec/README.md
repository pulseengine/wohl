# Spike 2a — PASE runs in WebAssembly (executable proof)

**Question (left open by Spike 1):** Spike 1 / SWARCH-WOHL-008 proved the rs-matter
protocol core *compiles* to a WASM component. It explicitly did **not** prove it
*runs* — "compile ≠ run." Does the security-critical PASE handshake actually
**execute** when compiled to `wasm32-wasip2`?

**Answer: yes.** A full **SPAKE2+ PASE handshake** (PBKDFParamReq/Resp →
Pake1/Pake2/Pake3 → secure session established) executes end-to-end inside a
`wasm32-wasip2` component under wasmtime:

```
$ cargo build --target wasm32-wasip2
$ wasmtime run target/wasm32-wasip2/debug/pase_exec.wasm
PASE-RUNS-OK: full SPAKE2+ handshake completed under wasmtime (wasip2)
$ echo $?
0
```

Verified 2026-05-29 with rustc 1.95.0, wasmtime 41.0.0.

## What this is (and is not)

This is a **local measurement oracle**, not a landed artifact:

- It is a standalone `cargo` binary with its own `[workspace]` table — **not** a
  member of the wohl workspace and **not** a `rules_wasm_component` Bazel target.
  Per the project's build directive, components that *land* are built via
  `rules_wasm_component`; standalone `cargo build --target wasm32-wasip2` is used
  here only because it is the one path that can both build *and* run a wasm
  component on this machine (the Bazel path needs network egress for the nix +
  wasi-sdk toolchain, which is unavailable in the dev sandbox).
- The eventual *landed* artifact — the same core behind a WIT transport seam,
  composed via `wac_compose`, built by Bazel — is Spike 2b/2c/2d.

## How it works

Mirrors rs-matter's own `rs-matter/tests/pase.rs` (in-process initiator +
`SecureChannel` responder doing a real handshake), with two changes that make it
wasip2-buildable:

1. **In-memory loopback transport.** The localhost UDP socket pair (async-io,
   the `os` feature) is replaced with a pure-Rust `NetworkSend`/`NetworkReceive`
   pipe — two packet queues, one per direction. No sockets, no `os`.

2. **A wasip2 `embassy-time` driver.** rs-matter calls `Instant::now()` /
   `Timer::after()` pervasively (transport, session, exchange, and the PASE
   handshake itself), so an `embassy-time` driver is mandatory. The `os` feature
   normally supplies `embassy-time/std`; without it, wasip2 has none. The driver
   here reads `now()` from `std::time::Instant` (wasi monotonic clock) scaled to
   `embassy_time_driver::TICK_HZ`, with a **no-op `schedule_wake`**. That is
   sufficient because `embassy_futures::block_on` busy-polls and
   `embassy_time::Timer::poll` re-checks `now()` on every poll — so internal
   timers elapse against the real clock with no reactor.

`critical-section` uses its `std` impl (rs-matter without `os` provides none).
`test_only_crypto()` supplies a deterministic RNG, so no entropy source is
needed.

See the project memory note `matter-core-wasm-buildable` and SWARCH-WOHL-008.
