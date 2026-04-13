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
cargo test                    # Plain Rust tests
bazel test //:verus           # Verus verification (when available)
```

### Rules

- All components are no_std, no alloc
- Verified core logic in `src/core.rs` (Verus-annotated)
- Plain Rust in `plain/src/core.rs` (verus-strip output)
- Follow [Verification Guide](https://pulseengine.eu/guides/VERIFICATION-GUIDE.md)
- Use `rivet validate` for artifact YAML files
