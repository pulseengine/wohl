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

## Deploying

The `wohl-hub` orchestrator is published as a 4-target release tarball
(`aarch64`/`x86_64` for Linux + macOS) with cosign-signed checksums and
SLSA build provenance. Installing on a Raspberry Pi takes three steps:
download + verify the tarball, drop in `/etc/wohl/wohl.toml`, enable
the systemd unit at `deploy/systemd/wohl-hub.service`. See
[docs/INSTALL.md](docs/INSTALL.md) for the full walkthrough plus the
Docker / Compose alternative.

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

# Cargo-fuzz smoke (60s per target — coverage currently `fuzz_leak`, `fuzz_temp`)
cargo fuzz run fuzz_leak -- -max_total_time=60
cargo fuzz run fuzz_temp -- -max_total_time=60

# ASPICE artifact traceability
rivet validate
```

Toolchain: see `rust-version` in workspace `Cargo.toml`.

The `relay` and `rivet` repositories must be cloned as siblings of `wohl/` for path-dependencies to resolve.

## Verification

| Track | Status |
|---|---|
| Kani BMC (all components) | gated by CI |
| proptest suites | gated by CI |
| cargo-fuzz | smoke gated by CI; coverage expansion tracked via issues |
| Verus deductive proofs | planned for `wohl-alert` dispatcher dedup invariant |
| AADL system model | active in `spar/` |
| Rivet ASPICE validation | gated by CI |

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for the full text.

## Links

- [PulseEngine](https://github.com/pulseengine) — umbrella project
- [Relay](https://github.com/pulseengine/relay) — stream transformers
- [Rivet](https://github.com/pulseengine/rivet) — ASPICE traceability
- Issue [#1](https://github.com/pulseengine/wohl/issues/1) — CI initiative
