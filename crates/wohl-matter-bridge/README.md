# wohl-matter-bridge

Hub-side scaffold for exposing Wohl sensors as Matter bridged endpoints.

This 0.2.0 crate is the **interface + stub**, not a live rs-matter integration.
It provides:

- `MatterBridge` trait — the contract `wohl-hub` calls.
- `MatterClusterMapping` table — typed mapping from each wohl alert / reading
  kind to its Matter cluster + attribute (BooleanState, TemperatureMeasurement,
  ElectricalPowerMeasurement, …).
- `LoggingBridge` — stderr stub used by `wohl-hub --matter` to validate wiring.

The live rs-matter integration (commissioning, fabrics, mDNS, attestation) is
the **0.3.0** scope. See [`DESIGN.md`](DESIGN.md) for the full design rationale,
target rs-matter version, commissioning approach, and open questions.

## Usage in wohl-hub (0.2.0)

Run with `--matter` (or `WOHL_MATTER=1`) and the hub instantiates a
`LoggingBridge` and forwards every dispatched alert + sensor reading to it
**in addition** to the existing stdout JSON output. With the flag off,
wohl-hub behaves identically to 0.1.0.

## Why a scaffold first

Per [SWARCH-WOHL-006](../../artifacts/swarch/SWARCH-WOHL-006.yaml): the
unverified Matter stack must live on the hub, outside the sensor safety
boundary. Landing the trait + cluster mapping as data (and thoroughly
unit-testing it) lets the 0.3.0 implementor focus on the rs-matter wire
integration without renegotiating the cluster choices.
