# CLAUDE.md

## Wohl — Home Supervision System

*Wohl wahrt.* — Maintains the well-being of your home.

Built on [Relay](https://github.com/pulseengine/relay) stream transformers and the [PulseEngine](https://github.com/pulseengine) toolchain. Verified. Always on.

### Architecture

Sensor nodes (ESP32/STM32, Synth → Gale) publish typed streams.
Hub (Raspberry Pi/Kiln or STM32H7/Gale) runs monitor components.
Each monitor is a stream transformer: `stream<sensor-data> → stream<alert>`.

### Components

| Component | Input | Output | Safety |
|---|---|---|---|
| Water Leak | stream\<water-event\> | stream\<alert\> | CRITICAL — immediate, no delay |
| Temperature | stream\<temperature\> | stream\<alert\> | Freeze/overheat protection |
| Air Quality | stream\<air-quality\> | stream\<alert\> | CO2/PM2.5/VOC monitoring |
| Door Watch | stream\<contact-event\> | stream\<alert\> | Open door/window detection |
| Power Meter | stream\<power-reading\> | stream\<alert\> | Usage + anomaly detection |
| Alert Dispatcher | stream\<alert\> | notifications | Dedup, rate-limit, deliver |

### Build

```bash
cargo test --workspace        # Plain Rust tests + proptest
cargo kani -p <crate>         # Kani BMC, e.g. cargo kani -p wohl-leak
cargo fuzz run <target> -- -max_total_time=60
rivet validate                # ASPICE artifact traceability
```

### Rules

- All components are no_std, no alloc
- Verified core logic in `crates/wohl-*/plain/src/lib.rs` (Kani BMC harnesses on `engine.rs`)
- Verification floor: Kani BMC + proptest + cargo-fuzz on all 6 components
- Verus deductive proofs prove the `wohl-alert` dispatcher dedup + rate-limit invariants (`proofs/verus/alert_dedup.rs`, Bazel-gated); a conformance proptest (`verus_conformance` in `engine.rs`) ties the proven ghost model to the executable engine — see [#7](https://github.com/pulseengine/wohl/issues/7) (closed)
- Use `rivet validate` for artifact YAML files
