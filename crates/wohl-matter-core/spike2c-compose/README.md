# Spike 2c — PASE runs across a `wac_compose`d WIT boundary

Spike 2a proved a SPAKE2+ PASE handshake runs inside a single wasm component.
This proves it runs across a **two-component `wac_compose` graph**, with every
Matter packet crossing the WIT seam:

```
$ wac plug mcore.wasm --plug transport.wasm -o composed-matter.wasm
$ wasmtime run composed-matter.wasm
PASE-RUNS-OK: SPAKE2+ handshake completed across the wac-composed WIT seam
```

Verified 2026-05-30 (rustc 1.95.0, wasmtime 41.0.0, wac-cli 0.6.0).

## Components

- **`mcore/`** — the verified Matter core (wasi:cli command). Runs the same
  full PASE handshake as Spike 2a, but its `NetworkSend`/`NetworkReceive`
  endpoints call the **imported** `wire` seam (`push`/`pop`/`peek`) instead of
  an in-process pipe. Every packet therefore leaves the component, enters the
  `transport` shell, and comes back.
- **`transport/`** — the host transport shell (provider). Owns two packet
  queues (channel 0 = to-device, 1 = to-controller) and exports `wire`.
- **`wit/world.wit`** — the seam between them.

Sync WIT funcs busy-polled by `embassy_futures::block_on` sidestep wasmtime's
component-model *async* maturity — the cross-component call is an ordinary
synchronous import from inside rs-matter's async `poll_fn`.

## What this is (and is not)

A **local measurement oracle**, like `../spike2-exec`. Standalone cargo crates
(own `[workspace]`, excluded from the wohl workspace, not bazel targets), built
with `cargo build --target wasm32-wasip2` + `wasm-tools` + `wac` + `wasmtime`
because that path can build *and run* a composed graph locally (the bazel path
needs nix/wasi-sdk egress unavailable in the dev sandbox).

### Fidelity caveat (honest scope)

The `wire` seam here is a **hand-written simplification** — a channelled byte
pipe — NOT the exact spar-generated `matter-world` seam in
`spar/generated/matter.wit` (`on-message_in -> option<mattermessage>`,
`emit-message_out`, `on-clock_in`, `on-entropy_in`). Clock + entropy also stay
component-internal here (std monotonic clock + rs-matter's deterministic test
RNG), rather than crossing their own seam funcs.

So this proves the **architecture is runnable** (PASE over a composed transport
boundary). Binding the real spar seam — and routing clock/entropy across it —
is the job of the **rules_wasm_component bazel landing** (Spike 2d / C4 in
SWV-MATTER-002), which is also where the artifact becomes a CI gate. Per the
build directive, the landed component is built by `rules_wasm_component`, not by
the ad-hoc cargo path used here for the measurement.
