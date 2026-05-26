# spar/generated/

WIT interface signatures **generated from the AADL model** in `../*.aadl` via
`spar codegen --format wit`.

Do not hand-edit. Regenerate after AADL changes:

```bash
spar codegen --root Wohl_Home::StarterHome.Deployed \
  --format wit --output /tmp/wohl-wit \
  spar/*.aadl
cp /tmp/wohl-wit/wit/*.wit spar/generated/
```

## Files

| File | Source process | Notes |
|---|---|---|
| `monitors.wit` | `Wohl_Firmware::HubMonitorProcess.Impl` | The verified monitor + dispatcher chain on the hub. |
| `ota.wit` | `Wohl_Firmware::OTAManagerProcess.Impl` | OTA delivery + attestation collection. |
| `fw.wit` | `Wohl_Firmware::ClimateFirmwareProcess.Impl` | Sensor-node firmware ports (example: ClimateNode). |
| `matter.wit` | `Wohl_Matter::MatterBridgeProcess.Impl` | Matter Bridge component set (0.3.0). |

## Relationship to `crates/*/wit/`

The per-crate WIT files under `crates/wohl-{leak,temp,air,door,power,alert}/wit/`
are hand-crafted **application interfaces** (semantic types, business rules,
init / process-event style entry points). They predate the spar-generates-WIT
convention adopted in 0.3.0.

The spar-generated files in this directory are the **architectural port
signatures** derived from AADL — one per AADL process. They sit one layer
below the application interfaces and are how a future wasm-componentized
bridge crate (or any future spar-driven component) would bind to its host.

Eventual convergence: the per-crate hand-crafted files should be replaced
with spar-generated ones once the AADL covers the application-level
interface surface (currently it only covers process ports). That's a
follow-up, not part of 0.3.0-architecture.
