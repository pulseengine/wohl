# wohl-matter-bridge — Design

Design doc for the **live** rs-matter integration that follows the
0.2.0 scaffold and the 0.3.0-architecture foundation landed in this
crate. The 0.3.x implementor reads this to know what to build.

Authoritative architecture record: [SWARCH-WOHL-006](../../artifacts/swarch/SWARCH-WOHL-006.yaml).

---

## 1. Why scaffold + design before live integration

Wohl's differentiator is **formally verified firmware** (Kani BMC + Verus,
`no_std`, `no_alloc`). The Matter stack (rs-matter or equivalent) is several
hundred KB of allocating Rust pulling in mDNS, TLS, BLE, and a commissioning
state machine — structurally outside Kani/Verus and outside the verified
sensor boundary by design.

Splitting the work across PRs:

1. **0.2.0** — landed the trait + cluster mapping + LoggingBridge stub.
   Small, easy to audit, no new transitive dependencies for the
   verified line. `wohl-hub` gained a `--matter` flag that exercises
   the bridge path end-to-end against the stub.
2. **0.3.0-architecture** — landed the AADL extension, rivet
   traceability artifacts, and the `RsMatterBridge` skeleton behind
   the `rs-matter-backend` feature gate (no actual rs-matter dep).
3. **0.3.x (next)** — wire the real rs-matter behind the same trait.
   The verified line is untouched; the cluster decisions are already
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
| water_leak                               | BooleanState (on WaterLeakDetector device type)         | 0x0045 | StateValue      |
| co2_warning / co2_critical               | CarbonDioxideConcentrationMeasurement                   | 0x040D | MeasuredValue   |
| pm25_warning / pm25_critical             | Pm25ConcentrationMeasurement                            | 0x042A | MeasuredValue   |
| voc_warning / voc_critical               | TotalVolatileOrganicCompoundsConcentrationMeasurement   | 0x042C | MeasuredValue   |
| door_open_too_long / door_opened_at_night| BooleanState (on ContactSensor device type)             | 0x0045 | StateValue      |
| overconsumption / power_spike / device_left_on | ElectricalPowerMeasurement                        | 0x0090 | ActivePower     |
| health_miss                              | *(internal — not bridged)*                              | —      | —               |

### Rationale per row

- **Temperature.** All four temp-alert flavors share `TemperatureMeasurement`.
  Matter has no native "freeze threshold crossed" event cluster; the controller
  observes `MeasuredValue` and applies its own threshold (Apple Home, Google,
  etc. all let users set their own alert bands). The freeze / overheat
  semantics live in the wohl notification path (push, SMS), not in Matter.
- **Water leak.** Matter 1.2 introduced a `WaterLeakDetector` **device type**
  (DTL id `0x0043`) — *not a separate cluster*. The water-leak device type
  uses `BooleanState (0x0045)` as its mandatory server cluster and
  optionally `BooleanStateConfiguration (0x0080)` for sensitivity / alarm
  latching. An earlier draft of this doc erroneously listed a
  `WaterLeakDetector cluster` at id `0x0048` — that id is actually the
  **Smoke / CO Alarm** cluster; advertising it for water leak would
  expose a cluster controllers don't recognize on that endpoint. The
  corrected bridge publishes `BooleanState::StateValue` on an endpoint
  whose device-type descriptor declares `WaterLeakDetector (0x0043)`.
  The 0.3.x implementor wires the device-type descriptor at endpoint
  registration time; `src/cluster.rs` does not enumerate
  `WaterLeakDetector` since it is not a cluster.
- **Air quality.** Three separate concentration-measurement clusters,
  all using `MeasuredValue` (0x0000). Same controller-threshold pattern
  as temperature. **Wire encoding is IEEE 754 float32** (see §7.4) —
  not int16 as an earlier draft of this doc said.
- **Door / window.** `BooleanState::StateValue` on a ContactSensor
  device-type endpoint. The "too long open" + "opened at night"
  distinction is a hub-side policy and is not exposed to Matter as a
  separate signal.
- **Power.** `ElectricalPowerMeasurement` is Matter 1.3+. ActivePower
  (0x0005) is the live wattage in milliwatts on the wire. Spike vs
  over-consumption is again hub-side policy.
- **Health miss.** Internal-only signal — the bridge drops it.
  Controllers should not see hub-internal liveness data.

## 3. rs-matter target version

Target: **rs-matter `main` HEAD, pinned by git commit sha** (NOT crates.io).

Why git-rev and not crates.io: as of 2026-05 the only published release
is `rs-matter = "0.1.0"` from July 2023, which predates the current API
surface entirely (no `DirKvBlobStore`, no `Matter::init`, different
module layout). The integration must use a `git = "..."  rev = "<sha>"`
Cargo dep against project-chip/rs-matter, with the sha pinned in this
crate's `Cargo.toml`.

Rationale:

- rs-matter is the only mature pure-Rust Matter SDK.
- It's still pre-1.0 and the published version is unusable.
- We pin a specific commit sha so reproducibility is preserved; we
  bump deliberately when upstream stabilises a relevant change.
- License: Apache-2.0, in the wohl `deny.toml` allowlist.

### MSRV reality (corrected from a previous draft)

Recent rs-matter HEAD declares `rust-version = "1.87"`. The workspace
currently pins 1.85. An earlier draft of this doc proposed a "hub-only
MSRV override" via per-crate `rust-version` keys — that does **not
isolate the toolchain** in a single-`Cargo.lock` workspace: the
resolver picks dependency versions consistently across the workspace,
and rustc rejects the whole compile if any compiled crate requires a
newer rustc than the toolchain in use.

The corrected plan, decided when the live integration PR lands:

- **Option A (likely):** raise the workspace MSRV uniformly to
  `rustc 1.87`. The verified sensor crates compile fine on 1.87 — no
  code change is needed in `wohl-{leak,temp,air,door,power,alert,ota}`,
  only the `rust-version` key in the workspace's `Cargo.toml`. CI
  toolchain bumps in lockstep.
- **Option B (deferred):** move `wohl-matter-bridge` out of the main
  workspace into a sibling workspace. Keeps the verified workspace on
  1.85. Adds composition friction.

Either way, the live integration PR makes the call explicit.

Linkage stays feature-gated:

```toml
[features]
default = []
rs-matter-backend = ["dep:rs-matter"]
```

When the feature is off (default in CI today), `cargo build` and
`cargo test` compile on 1.85 with no rs-matter transitive deps. Only
the `--features rs-matter-backend` build needs 1.87.

## 4. Commissioning approach (0.3.x)

- **QR code + manual setup code.** Generated once at first boot using
  rs-matter's `ManualPairingCode` / `QRCode` encoders (NOT a custom
  format) — Matter Core §5.1.3 mandates the Verhoeff-checksummed
  11-digit manual code and Base38-encoded TLV QR payload.
- **Persistent commissioning data.** Stored under
  `/var/lib/wohl/matter/` on Linux hub deployments via rs-matter's
  `DirKvBlobStore` (implementor of the `KvBlobStore` trait):
  - `fabric.bin` — fabric table after commissioning.
  - `acl.bin` — access-control list (per-fabric scoped by Matter ACL
    cluster 0x001F semantics — rs-matter handles this).
  - `setup-code.txt` — human-readable manual setup code, recoverable.
  - `discriminator` — 12-bit discriminator (random at first boot).
- **Commissioning window.** Default 180 s (was 60 s — bumped after
  Matter critic review; 60 s is within the Core §5.4.2 minimum but
  too tight for typical QR-scan UX on Apple Home / Google Home).
- **Factory reset.** Deleting `/var/lib/wohl/matter/` and restarting
  the hub clears commissioning. Exposed as `wohl-hub matter reset`
  for operators. The reset MUST regenerate the discriminator + setup
  code; re-imaging from a copied state directory would clone a
  credential.
- **First-boot UX.** wohl-hub logs the QR code as ASCII to stderr on
  first boot when no fabric data exists. Operators photograph it
  with the Home app.

## 5. Multi-admin (multi-fabric) behavior

Wohl bridge accepts **multiple fabrics simultaneously** — a user
adding the bridge to Apple Home then later to Google Home keeps
both working without re-pairing.

- rs-matter supports multi-fabric out of the box; we just don't
  fail-fast on a second commissioner.
- The Matter ACL cluster (0x001F) is **fabric-scoped natively**:
  the ACL attribute is a fabric-scoped list, each fabric writes its
  own entries, and entries from fabric A are invisible to fabric B.
  rs-matter handles this automatically — we don't write per-fabric
  ACL plumbing.
- Subscriptions are **per-active-subscription, per-subscriber**.
  When publishing an attribute update, the bridge hands the change
  to rs-matter's report engine and lets it fan-out to subscribed
  fabrics at each subscriber's negotiated MinInterval / MaxInterval.
- The "Share with another platform" flow (Matter Multi-Admin) issues
  a new pairing code from an already-paired controller. The bridge
  does not need special handling — rs-matter exposes a node-operational
  credentials API for this.

Open: do we expose a "show second pairing code" affordance in
wohl-hub's CLI, or rely entirely on the first-fabric controller's UI
to drive multi-admin? **Default to controller-driven; revisit if
operators complain.**

## 6. Attestation certificates

- **0.2.0 (shipped):** N/A — no Matter wire bits.
- **0.3.0-architecture (shipped):** N/A — feature gate exists, no
  Matter wire bits yet.
- **0.3.x dev:** stub DAC (Device Attestation Certificate) using
  rs-matter example certs. Bridge will commission with "uncertified
  device" warnings in controllers, which is fine for dev/internal
  builds. Vendor ID `0xFFF1` (Matter Test Vendor 1) is the official
  CSA-reserved test allocation — not impersonation, but products
  using it MUST NOT claim CSA certification.
- **0.3.x or 0.4.0 production:** real DAC issued by CSA. Requires:
  - CSA vendor ID (apply with Connectivity Standards Alliance,
    one-time fee + annual maintenance).
  - Per-bridge-batch CD (Certification Declaration) tied to the
    certified Matter Bridge device type.
  - PAA/PAI chain — either using the CSA test PAA or our own PAA
    rooted at a CSA-recognized authority.
  - Secure storage of the device-specific DAC key. On Linux hub
    deployments this lands in `/var/lib/wohl/matter/dac.key`,
    mode 0600 owned by the wohl service user. Matter Core §6.2.2
    recommends but does not mandate HSM/SE — a 0600 file is the
    minimum-conforming baseline; CSA certification may require a
    tamper-evidence story.

Vendor-ID acquisition is the long-pole — it gates the production cert
chain, not the engineering work.

## 7. Open questions for the 0.3.x implementor

1. **Endpoint id allocation.** wohl-hub today has three separate id
   spaces (zone, contact, circuit). The bridge needs a stable
   1:1 → Matter-endpoint mapping. Matter endpoint id is `u16` so
   the space is large; the earlier proposal of flat namespaces
   (zones 1..=99, contacts 100..=199, circuits 200..=255) caps each
   space artificially. Better proposal: tag the high byte with the
   kind (`(kind << 8) | native_id`) or use a register-on-demand
   model and persist the assignment under
   `/var/lib/wohl/matter/endpoints.toml`. **Decide before
   commissioning ships.**

2. **WaterLeakDetector device-type endpoint registration.** Each
   water-presence endpoint must be declared with the WaterLeakDetector
   device type (DTL `0x0043`) on the endpoint descriptor — that's
   what tells the controller "interpret BooleanState::StateValue
   on this endpoint as a leak indicator". This is endpoint-level
   metadata in rs-matter's `Endpoint` configuration, not a
   cluster choice.

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
   | Temperature: signed centi-°C (already) | `TemperatureMeasurement::MeasuredValue` **int16 in 0.01 °C** | passthrough (range fits int16: -327.68 °C .. 327.67 °C, plenty for indoor) |
   | Power: watts (signed, f64-ish or i32) | `ElectricalPowerMeasurement::ActivePower` **int64 milliwatts** | `× 1000` (cast saturating) |
   | CO₂: ppm | `CarbonDioxideConcentrationMeasurement::MeasuredValue` **IEEE 754 float32** | `as f32` cast. Publish `MeasurementUnit` (0x0008) alongside to declare `ppm`. |
   | PM2.5: μg/m³ | `Pm25ConcentrationMeasurement::MeasuredValue` **IEEE 754 float32** | `as f32` cast. `MeasurementUnit` = μg/m³ (verify against rs-matter defaults). |
   | VOC: index value | `TotalVolatileOrganicCompoundsConcentrationMeasurement::MeasuredValue` (expects concentration) | **MISMATCH — see §7.5.** A Sensirion-style index is not a concentration. |
   | Contact (door): bool | `BooleanState::StateValue` on ContactSensor (DTL `0x0015`): `true`=closed, `false`=open | passthrough |
   | Water presence: bool | `BooleanState::StateValue` on WaterLeakDetector (DTL `0x0043`): `true`=leak detected | passthrough (opposite physical meaning of contact's `true`, same bridge boolean; device-type descriptor on the endpoint disambiguates) |

   An earlier draft of this table said the concentration clusters
   used int16. That was wrong: ConcentrationMeasurement-family
   MeasuredValue is **IEEE 754 float32** (Matter App Cluster Spec
   1.3 §2.x ConcentrationMeasurement base). Cluster ids and
   polarities have been independently re-checked against the spec
   and the Matter Device Type Library; see also §2 (cluster mapping)
   for the water-leak device-type-vs-cluster correction.

   Encode the conversion in a single `bridge_value` helper alongside
   the cluster mapping so the policy is centralized and unit-testable.
   Property test: round-trip a wohl value through `bridge_value` and
   confirm the cluster wire encoding round-trips back.

5. **VOC unit semantics.** Wohl's `ReadingKind::Voc` carries an
   *index value* (a Sensirion-style 0..500 dimensionless quantity),
   not a concentration. The Matter VOC cluster expects a real
   concentration (ppm/ppb/μg/m³). Publishing the index as if it
   were a concentration will mislead controllers. Options for the
   0.3.x implementor:
   (a) Omit VOC publish to Matter (drop the mapping in `cluster.rs`
       and don't expose a VOC endpoint).
   (b) Define a vendor-specific cluster carrying the index.
   (c) Map the index through a calibrated Sensirion conversion (the
       only reliable mapping requires per-sensor calibration data
       wohl doesn't currently capture).
   Recommendation: (a) until the calibration story is clearer.

6. **PASE rate-limit ownership.** SWREQ-MATTER-002 requires the
   Matter Core §5.1.1.7 limit (max 20 PASE failures per fabric per
   24h). Decision needed: does the wohl bridge layer enforce this
   itself, or trust rs-matter's implementation? If rs-matter,
   SWV-MATTER-001's "rate-limit verified by attempted-failure load
   test" actually verifies rs-matter, not wohl.

7. **Verified-line backpressure boundary.** The bridge runs as a
   trait call on `wohl-hub`'s dispatcher thread. A slow / panicking
   `publish_alert` impl will delay the next dispatcher iteration —
   the Verus dedup invariant proves "at most one alert per key in
   dispatcher state", says nothing about downstream subscribers
   preserving ordering or freshness. Either (a) document the
   liveness coupling honestly in SWARCH-WOHL-007, or (b) make
   `publish_*` non-blocking (channel send, async-spawn) so a stuck
   bridge cannot delay the verified dispatcher.
