# wohl-matter-bridge — Design

Design doc for the **live** rs-matter integration that follows the
0.2.0 scaffold landed in this crate. The 0.3.0 implementor reads this
to know what to build.

Authoritative architecture record: [SWARCH-WOHL-006](../../artifacts/swarch/SWARCH-WOHL-006.yaml).

---

## 1. Why scaffold + design before live integration

Wohl's differentiator is **formally verified firmware** (Kani BMC + Verus,
`no_std`, `no_alloc`). The Matter stack (rs-matter or equivalent) is several
hundred KB of allocating Rust pulling in mDNS, TLS, BLE, and a commissioning
state machine — structurally outside Kani/Verus and outside the verified
sensor boundary by design.

Splitting the work in two PRs:

1. **0.2.0 (this PR)** — land the trait + cluster mapping + stub. Small,
   easy to audit, no new transitive dependencies for the verified line.
   `wohl-hub` gets a `--matter` flag that exercises the bridge path
   end-to-end against the stub.
2. **0.3.0** — wire the real rs-matter behind the same trait. The
   verified line is untouched; the cluster decisions are already
   reviewed and unit-tested; the diff is concentrated in one place.

This keeps the verification claim ("verified end-to-end on the sensor
line") defensible: the Matter stack is explicitly hub-side and explicitly
outside the verified boundary.

## 2. Cluster mapping decisions

Encoded in `src/cluster.rs` as a typed enum match, unit-tested in
`mapping_for_alert` / `mapping_for_reading`. Summary:

| Wohl kind                                | Matter cluster                                          | id     | Attribute       |
|------------------------------------------|---------------------------------------------------------|--------|-----------------|
| freeze / overheat / rapid_drop / rapid_rise | TemperatureMeasurement                               | 0x0402 | MeasuredValue   |
| water_leak                               | BooleanState (Matter 1.0 fallback)                      | 0x0045 | StateValue      |
| co2_warning / co2_critical               | CarbonDioxideConcentrationMeasurement                   | 0x040D | MeasuredValue   |
| pm25_warning / pm25_critical             | Pm25ConcentrationMeasurement                            | 0x042A | MeasuredValue   |
| voc_warning / voc_critical               | TotalVolatileOrganicCompoundsConcentrationMeasurement   | 0x042C | MeasuredValue   |
| door_open_too_long / door_opened_at_night| BooleanState                                            | 0x0045 | StateValue      |
| overconsumption / power_spike / device_left_on | ElectricalPowerMeasurement                        | 0x0090 | ActivePower     |
| health_miss                              | *(internal — not bridged)*                              | —      | —               |

### Rationale per row

- **Temperature.** All four temp-alert flavors share `TemperatureMeasurement`.
  Matter has no native "freeze threshold crossed" event cluster; the controller
  observes `MeasuredValue` and applies its own threshold (Apple Home, Google,
  etc. all let users set their own alert bands). The freeze / overheat
  semantics live in the wohl notification path (push, SMS), not in Matter.
- **Water leak.** Matter 1.2 added the `WaterLeakDetector` device type
  (cluster 0x0048), but as of 2026-05 Apple Home and Google Home both
  fall back to BooleanState. We ship the BooleanState mapping for
  broadest compatibility and keep `WaterLeakDetector` in the enum for
  forward-compat. The 0.3.0 implementor may dual-publish.
- **Air quality.** Three separate concentration-measurement clusters,
  all using `MeasuredValue` (0x0000). Same controller-threshold pattern
  as temperature.
- **Door / window.** `BooleanState::StateValue` is the universal
  pattern. The "too long open" + "opened at night" distinction is a
  hub-side policy and is not exposed to Matter as a separate signal.
- **Power.** `ElectricalPowerMeasurement` is Matter 1.3+. ActivePower
  (0x0005) is the live wattage. Spike vs over-consumption is again
  hub-side policy.
- **Health miss.** Internal-only signal — the bridge drops it.
  Controllers should not see hub-internal liveness data.

## 3. rs-matter target version

Target: **rs-matter 0.1.x** (latest patch at integration time).

Rationale:

- rs-matter is the only mature pure-Rust Matter SDK.
- It's still pre-1.0; we pin a tilde range (`~0.1`) to take bugfix
  releases without an unreviewed minor bump.
- As of writing, recent rs-matter releases require **rustc 1.87**, above
  the workspace MSRV (1.85). The 0.3.0 PR raises the **hub-only** MSRV
  via a `rust-version` override in `crates/wohl-hub/Cargo.toml` and
  `crates/wohl-matter-bridge/Cargo.toml`. The verified sensor crates
  stay on 1.85.

Linkage will be feature-gated:

```toml
[features]
default = []
rs-matter-backend = ["dep:rs-matter"]
```

So the trait + stub remain buildable on 1.85 even after the live impl lands.

## 4. Commissioning approach (0.3.0)

- **QR code + manual setup code.** Generated once at first boot. Printed
  on stderr (so journald captures it) and written to disk for repeat
  display.
- **Persistent commissioning data.** Stored under
  `/var/lib/wohl/matter/` on Linux hub deployments:
  - `fabric.bin` — fabric table after commissioning (sealed via systemd
    credentials or DPAPI-equivalent in future).
  - `acl.bin` — access-control list.
  - `setup-code.txt` — human-readable manual setup code, recoverable.
  - `discriminator` — 12-bit discriminator (random at first boot).
- **Factory reset.** Deleting `/var/lib/wohl/matter/` and restarting the
  hub clears commissioning. We expose this as `wohl-hub matter reset`
  for operators.
- **First-boot UX.** wohl-hub logs the QR code as ASCII to stderr on
  first boot when no fabric data exists. Operators photograph it with
  the Home app.

## 5. Multi-admin (multi-fabric) behavior

Wohl bridge should accept **multiple fabrics simultaneously** — a user
adding the bridge to Apple Home then later to Google Home must keep
both working without re-pairing.

- rs-matter supports multi-fabric out of the box; we just don't
  fail-fast on a second commissioner.
- Each fabric gets its own ACL entry; we publish the same attribute
  updates to all fabrics.
- The "Share with another platform" flow (Matter Multi-Admin) issues a
  new pairing code from an already-paired controller. Our bridge does
  not need special handling — rs-matter exposes a node-operational
  credentials API for this.

Open: do we expose a "show second pairing code" affordance in
wohl-hub's CLI, or rely entirely on the first-fabric controller's UI
to drive multi-admin? **Default to controller-driven; revisit if
operators complain.**

## 6. Attestation certificates

- **0.2.0 (this PR):** N/A — no Matter wire bits.
- **0.3.0 dev:** stub DAC (Device Attestation Certificate) using rs-matter
  example certs. Bridge will commission with "uncertified device"
  warnings in controllers, which is fine for dev/internal builds.
- **0.3.x or 0.4.0 production:** real DAC issued by CSA. Requires:
  - CSA vendor ID (apply with Connectivity Standards Alliance,
    one-time fee + annual maintenance).
  - Per-bridge-batch CD (Certification Declaration) tied to the
    certified Matter Bridge device type.
  - PAA/PAI chain — either using the CSA test PAA or our own PAA
    rooted at a CSA-recognized authority.
  - Secure storage of the device-specific DAC key. On Linux hub
    deployments this lands in `/var/lib/wohl/matter/dac.key`,
    permissioned to the wohl service user.

Vendor-ID acquisition is the long-pole — it gates the production cert
chain, not the engineering work.

## 7. Open questions for the 0.3.0 implementor

1. **Endpoint id allocation.** wohl-hub today has three separate id
   spaces (zone, contact, circuit). The bridge needs a stable
   1:1 → Matter-endpoint mapping. Proposal: deterministic flattening
   (`endpoint_id = kind_offset + native_id`, e.g. zones 1..=99,
   contacts 100..=199, circuits 200..=255). Persist the map under
   `/var/lib/wohl/matter/endpoints.toml` to keep ids stable across
   controller re-syncs. **Decide before commissioning ships.**

2. **WaterLeakDetector dual-publish.** Do we publish to both
   `BooleanState` (0x0045) *and* `WaterLeakDetector` (0x0048) so
   newer controllers get the proper device type, or stick with
   `BooleanState`-only for simplicity? Real-world controller
   support (Apple/Google) drives this — needs a test pass against
   each platform when the impl lands.

3. **Reading throttle / rate limit.** Should the bridge throttle
   high-frequency readings (e.g. 1Hz power) before publishing, or
   pass them through and rely on rs-matter's subscription
   reporting cadence? Leaning toward "pass through, let
   rs-matter subscription cadence handle it" — but verify the
   bridge doesn't accidentally hot-loop the radio.

4. **Unit-conversion contract.** Matter cluster attributes have
   specific units that differ from wohl's internal representation.
   The 0.2.0 stub passes raw wohl values through; the live
   `RsMatterBridge` must apply conversions at the bridge boundary
   (not inside the verified monitors):

   | Wohl internal | Matter attribute target | Conversion |
   |---|---|---|
   | Temperature: signed centi-°C (already) | `TemperatureMeasurement::MeasuredValue` int16 centi-°C | passthrough |
   | Power: watts (signed, f64-ish or i32) | `ElectricalPowerMeasurement::ActivePower` int64 milliwatts | `× 1000` (cast saturating) |
   | CO₂: ppm | `CarbonDioxideConcentrationMeasurement::MeasuredValue` float32 ppm | cast |
   | PM2.5: µg/m³ | `Pm25ConcentrationMeasurement::MeasuredValue` float32 µg/m³ | passthrough (verify cluster scaling) |
   | VOC: ppb | `TotalVolatileOrganicCompoundsConcentrationMeasurement::MeasuredValue` float32 ppm or ppb (per cluster spec scaling) | check cluster scaling attr |
   | Contact (door): bool | `BooleanState::StateValue` bool — ContactSensor device type: `true`=closed, `false`=open | **passthrough but check polarity** |
   | Water presence: bool | `BooleanState::StateValue` bool — WaterLeakDetector device type: `true`=leak detected | **passthrough — opposite polarity from contact** |

   Encode the conversion in a single `bridge_value` helper alongside
   the cluster mapping so the policy is centralized and unit-testable.
   Property test: round-trip a wohl value through `bridge_value` and
   confirm the cluster wire encoding round-trips back.
