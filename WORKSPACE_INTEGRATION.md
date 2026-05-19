# Workspace Integration for `wohl-ota`

This branch (`phase2/track-c-wohl-ota`) adds a new crate at
`crates/wohl-ota/` but **intentionally does not modify the root
`Cargo.toml`**. The orchestrator should apply the changes below before
running `cargo test --workspace` / `cargo kani` from CI.

## 1. Add to `[workspace] members`

In `/Cargo.toml`:

```diff
 members = [
     "crates/wohl-leak",
     "crates/wohl-temp",
     "crates/wohl-air",
     "crates/wohl-door",
     "crates/wohl-power",
     "crates/wohl-alert",
     "crates/wohl-hub",
     "crates/wohl-integration",
+    "crates/wohl-ota",
 ]
```

## 2. (Optional) Expose as workspace dependency

If any future crate (e.g. `wohl-hub`, integration tests, or a board-
support crate) wants to depend on `wohl-ota`, add a workspace dependency
entry alongside the other `wohl-*` entries:

```diff
 wohl-power = { path = "crates/wohl-power" }
 wohl-alert = { path = "crates/wohl-alert" }
+wohl-ota = { path = "crates/wohl-ota" }
 proptest = "1"
```

This is **not required for this PR** — `wohl-ota` is currently a leaf
crate with no internal consumers. Adding the workspace entry is cheap
and keeps the list complete for the next track.

## 3. Verification I have run locally

Against the same workspace once the two edits above are applied:

| Check | Toolchain | Result |
|---|---|---|
| `cargo build -p wohl-ota` | stable | PASS |
| `cargo test -p wohl-ota` | stable | 15 passed (11 unit + 4 proptest) |
| `cargo +1.85.0 fmt -p wohl-ota --check` | 1.85.0 | clean |
| `cargo +1.85.0 clippy -p wohl-ota --all-targets -- -D warnings` | 1.85.0 | clean |
| `cargo kani -p wohl-ota` | kani-verifier | 4 harnesses, 0 failures (OTA-P01..P04) |

## 4. CI follow-up (orchestrator)

The existing `.github/workflows/*` matrix lists each crate explicitly for
the Kani job. After integrating, add `wohl-ota` to that matrix so CI
runs the new proofs.

## 5. Out of scope (separate follow-ups)

- `cargo fuzz` target for the manifest parser — tracked by **#8**.
- Real Ed25519 implementation of `SignatureVerifier` (will live in a
  downstream crate so the core stays crypto-free and BMC-friendly).
- HAL glue that wires this state machine to ESP-IDF / Zephyr OTA APIs.
- Hub-side `OTAManagerProcess` Rust skeleton (this PR is node-side core
  only).
