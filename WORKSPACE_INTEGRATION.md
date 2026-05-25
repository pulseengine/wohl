# WORKSPACE_INTEGRATION.md

Notes for the orchestrator merging branch `0.2.0/matter-bridge-scaffold`.

## What this branch ships

A new sibling crate, `crates/wohl-matter-bridge`, plus a minimal `--matter`
flag in `crates/wohl-hub`. The new crate is currently referenced from
`wohl-hub` via a direct path dep (`wohl-matter-bridge = { path = "../wohl-matter-bridge" }`)
which builds correctly out of the worktree because both crates are members
of the same Cargo workspace tree.

However, the crate is **not yet a workspace member**, which means it does
not pick up workspace lints, won't get touched by `cargo test --workspace`,
and won't show up in `cargo build --workspace`. It does build, fmt, clippy,
and test cleanly under `cargo +1.85.0 ... -p wohl-matter-bridge -p wohl-hub`.

## Required root `Cargo.toml` edits

In `/Users/r/git/pulseengine/wohl/Cargo.toml`:

1. Add `"crates/wohl-matter-bridge"` to `[workspace] members = [...]`.
2. Add the crate to `[workspace.dependencies]` so future consumers can
   use `wohl-matter-bridge.workspace = true`.

Concrete diff:

```diff
 [workspace]
 resolver = "3"
 members = [
     "crates/wohl-leak",
     "crates/wohl-temp",
     "crates/wohl-air",
     "crates/wohl-door",
     "crates/wohl-power",
     "crates/wohl-alert",
     "crates/wohl-hub",
     "crates/wohl-integration",
     "crates/wohl-fw-door-bench",
     "crates/wohl-ota",
+    "crates/wohl-matter-bridge",
 ]

 ...

 [workspace.dependencies]
 ...
 wohl-alert = { path = "crates/wohl-alert" }
 wohl-ota = { path = "crates/wohl-ota" }
+wohl-matter-bridge = { path = "crates/wohl-matter-bridge" }
 proptest = "1"
```

After applying those edits, **also** update `crates/wohl-hub/Cargo.toml`
to use workspace inheritance for consistency:

```diff
-wohl-matter-bridge = { path = "../wohl-matter-bridge" }
+wohl-matter-bridge.workspace = true
```

This is a one-line follow-up; the current direct path form is correct
and works, it just stylistically diverges from the rest of the file.

## Verification after orchestrator edits

Run, from repo root:

```bash
cargo +1.85.0 fmt --check
cargo +1.85.0 clippy --workspace --all-targets -- -D warnings
cargo +1.85.0 test --workspace
```

All three should pass without changes to this branch.

## Verified line untouched

This PR makes **zero** edits to:

- `crates/wohl-{leak,temp,air,door,power,alert}/`
- `crates/wohl-ota/`
- `crates/wohl-fw-door-bench/`

Kani verification on `wohl-alert` (and the other verified crates) is
unaffected. The Matter scaffold lives wholly on the hub side, outside the
verified sensor / dispatcher boundary, exactly as
[SWARCH-WOHL-006](artifacts/swarch/SWARCH-WOHL-006.yaml) prescribes.

## What's NOT in this PR

- No rs-matter dependency. The Cargo.toml is deliberately
  rs-matter-free. The 0.3.0 follow-up adds it behind an
  `rs-matter-backend` feature gate. See
  `crates/wohl-matter-bridge/DESIGN.md` § 3.
- No commissioning, no mDNS, no UDP, no Matter wire bytes. 0.3.0 scope.
- No attestation cert plumbing. 0.3.x / 0.4.0 scope (gated on CSA
  vendor ID acquisition).

## Branch + PR posture

Branch: `0.2.0/matter-bridge-scaffold` (this branch).
Commit: see `git log -1 --format=%H 0.2.0/matter-bridge-scaffold`.
Not pushed. Not opened as PR. Orchestrator decides when to push + open.
