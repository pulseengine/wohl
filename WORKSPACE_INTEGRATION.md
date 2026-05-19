# Workspace integration — `wohl-fw-door`

This crate (`crates/wohl-fw-door/`) is **not yet** a workspace member.
This document lists the exact edits the orchestrator needs to make to
`Cargo.toml` and (optionally) `.github/workflows/ci.yml` to integrate
the firmware into the workspace and into CI.

The firmware itself has been verified standalone — see the *Verification*
section near the bottom — by temporarily adding the crate to the
`members` list, running the four gates, then reverting that one line.
After the orchestrator applies the change below, the same verification
will pass from the workspace.

---

## 1. `Cargo.toml` — add the firmware crate to the workspace

**One-line edit.** Add `crates/wohl-fw-door` to the `members` list:

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
+    "crates/wohl-fw-door",
 ]
```

That is the **only** change strictly required.

### Optional but recommended — share the HAL pin

If you'd like firmware versions to be pinned at the workspace level (so
future `wohl-fw-climate`, `wohl-fw-air`, etc. share a single HAL
version), add four `[workspace.dependencies]` lines:

```diff
 [workspace.dependencies]
 relay-lc = { path = "../relay/crates/relay-lc" }
 …
 proptest = "1"
+# Firmware shared deps (consumed by crates/wohl-fw-*)
+cortex-m       = { version = "0.7.7",  default-features = false }
+cortex-m-rt   = { version = "0.7.5",  default-features = false }
+panic-halt    = { version = "1.0.0",  default-features = false }
+stm32g0xx-hal = { version = "0.2.0",  features = ["stm32g031", "rt"] }
```

…and then in `crates/wohl-fw-door/Cargo.toml` you'd switch the
`[target.'cfg(target_os = "none")'.dependencies]` lines from explicit
versions to `{ workspace = true }`. This is **purely cosmetic**; the
crate works as-is with versions pinned locally. Defer this until a
second firmware crate exists.

## 2. `.github/workflows/ci.yml` — out of scope

Per the task spec, **do not touch CI in this PR**. The firmware target
needs either:

- a `rustup target add thumbv6m-none-eabi` step (cheap), or
- a separate `cross` matrix entry (more isolated).

Both options should be discussed and added in a follow-up PR. Until
then, the existing `cargo test --workspace` job will exercise
`wohl-fw-door`'s host-side tests automatically once the crate is added
to `members` — because `cargo test` cross-compiles to the *host* by
default, and on the host the firmware binary collapses to a
zero-cost stub (`fn main() {}`), avoiding any need for the ARMv6-M
toolchain in CI.

---

## Verification (already run locally on this branch)

From the worktree root, with the one-line `members` edit applied
temporarily, all four gates passed:

```
cargo build -p wohl-fw-door --target thumbv6m-none-eabi --release   # PASS
cargo test  -p wohl-fw-door                                         # PASS — 16/16 tests
cargo fmt   -p wohl-fw-door --check                                 # PASS
cargo clippy -p wohl-fw-door --all-targets -- -D warnings           # PASS
```

Cross-clippy (`cargo clippy -p wohl-fw-door --target thumbv6m-none-eabi
--lib --bins -- -D warnings`) is also clean — only `--all-targets` on
the bare-metal target fails, because the dev-dependency `proptest`
pulls in non-`no_std` crates. That's expected and not a regression.

## Files added by this PR

```
boards/stm32g0/README.md                     # Hardware/pin docs
crates/wohl-fw-door/Cargo.toml               # Crate manifest
crates/wohl-fw-door/build.rs                 # memory.x → OUT_DIR
crates/wohl-fw-door/memory.x                 # STM32G031K8 linker map
crates/wohl-fw-door/.cargo/config.toml       # default target + linker flags
crates/wohl-fw-door/src/lib.rs               # Pure-logic library root
crates/wohl-fw-door/src/ccsds.rs             # 14-byte CCSDS encoder
crates/wohl-fw-door/src/debounce.rs          # Reed-switch debouncer
crates/wohl-fw-door/src/door.rs              # State machine
crates/wohl-fw-door/src/main.rs              # MCU binary (entry, USART)
WORKSPACE_INTEGRATION.md                     # This file
```

## Files NOT touched

- `Cargo.toml` — orchestrator applies the one-line `members` edit.
- `Cargo.lock` — regenerated when `members` is updated.
- `.github/workflows/ci.yml` — follow-up.
- `crates/wohl-*/` (existing monitors) — untouched.
- `crates/wohl-hub/` — untouched.
