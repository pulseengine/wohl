# Wohl — Home Supervision System

*Wohl wahrt.* — Maintains the well-being of your home.

[![CI](https://github.com/pulseengine/wohl/actions/workflows/ci.yml/badge.svg)](https://github.com/pulseengine/wohl/actions/workflows/ci.yml)

Verified embedded sensor monitoring on the [PulseEngine](https://github.com/pulseengine) toolchain. Built on [Relay](https://github.com/pulseengine/relay) stream transformers. Sensor nodes publish typed streams over CCSDS; a hub routes them through six verified monitor components and dispatches alerts.

Verified. Always on.

## Components

| Component | Input | Output | Safety |
|---|---|---|---|
| Water Leak | stream\<water-event\> | stream\<alert\> | CRITICAL — immediate, no delay |
| Temperature | stream\<temperature\> | stream\<alert\> | Freeze/overheat protection |
| Air Quality | stream\<air-quality\> | stream\<alert\> | CO2/PM2.5/VOC monitoring |
| Door Watch | stream\<contact-event\> | stream\<alert\> | Open door/window detection |
| Power Meter | stream\<power-reading\> | stream\<alert\> | Usage + anomaly detection |
| Alert Dispatcher | stream\<alert\> | notifications | Dedup, rate-limit, deliver |

All component cores are `no_std`, `no_alloc`, and verified by Kani bounded model checking.

## Architecture

- **Sensor nodes** — ESP32-C3 / STM32G0 firmware, CCSDS-framed streams
- **Hub** — Raspberry Pi or STM32H7, runs the six monitors as Relay stream transformers
- **System model** — AADL specification in `spar/` (firmware threads, hardware nodes, deployed home topology)
- **Traceability** — ASPICE artifacts in `artifacts/{sysreq,swreq,swarch,swdd,verification}/`, validated by [Rivet](https://github.com/pulseengine/rivet)

## Build

```bash
# Workspace tests + proptest
cargo test --workspace

# Kani bounded model checking (per component)
for c in wohl-leak wohl-temp wohl-air wohl-door wohl-power wohl-alert; do
  cargo kani -p "$c"
done

# Cargo-fuzz smoke (60s per target)
cargo fuzz run fuzz_leak -- -max_total_time=60
cargo fuzz run fuzz_temp -- -max_total_time=60

# WASM components (fused with Meld, compiled with Synth)
bazel build //...

# ASPICE artifact traceability
rivet validate
```

Rust toolchain: `1.85.0` (workspace `rust-version`, edition 2024).

The `relay` and `rivet` repositories must be cloned as siblings of `wohl/` for path-dependencies to resolve.

## Verification

| Track | Status |
|---|---|
| Kani BMC (all 6 components) | PASS |
| proptest suites | PASS |
| cargo-fuzz (`fuzz_leak`, `fuzz_temp`) | smoke |
| Verus deductive proofs | planned — `wohl-alert` dispatcher dedup invariant |
| AADL system model | active |
| Rivet ASPICE validation | 0 errors / 20 warnings |

## License

Apache-2.0. SPDX metadata is set in each crate's `Cargo.toml`; a root `LICENSE` file is pending.

## Links

- [PulseEngine](https://github.com/pulseengine) — umbrella project
- [Relay](https://github.com/pulseengine/relay) — stream transformers
- [Rivet](https://github.com/pulseengine/rivet) — ASPICE traceability
- Issue [#1](https://github.com/pulseengine/wohl/issues/1) — CI initiative this PR closes
