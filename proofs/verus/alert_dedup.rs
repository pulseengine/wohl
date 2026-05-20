//! Verus formal proofs for the AlertDispatcher dedup and rate-limit logic.
//!
//! This file mirrors the state machine of
//! `crates/wohl-alert/plain/src/engine.rs::AlertDispatcher` in Verus's
//! specification language, then proves two correctness invariants:
//!
//!   1. **Dedup invariant** (the primary obligation, Issue #7):
//!      For any two `process_alert(zone_id, alert_type, t1)` and
//!      `process_alert(zone_id, alert_type, t2)` calls with the same
//!      `(zone_id, alert_type)` and `t1 <= t2 < t1 + DEDUP_COOLDOWN_SEC`,
//!      *if the first returns `Send`, the second returns `Deduplicated`*.
//!
//!   2. **Rate-limit invariant**:
//!      After `MAX_ALERTS_PER_MINUTE` distinct alerts have been recorded
//!      within a single 60-second minute window (i.e. `minute_count`
//!      saturated and `t < minute_start + 60`), the next call returns
//!      `RateLimited` *unless* it would have been deduplicated first.
//!
//! # Relationship to the executable Rust
//!
//! Verus and the executable engine are **two specifications of the same
//! state machine**.  Verus reasons over `nat`/`int` (unbounded), while the
//! executable Rust uses `u64`/`u32` with saturating arithmetic.  The
//! relevant `process_alert` arithmetic never overflows in the regime we
//! prove about (`t < u64::MAX - DEDUP_COOLDOWN_SEC`, which is the same
//! assumption the Kani harness makes via `kani::assume`).
//!
//! Correspondence (Verus ghost -> Rust):
//!   - `GhostDispatcher.recent`        ~ `AlertDispatcher.recent[0..recent_count]`
//!   - `GhostDispatcher.minute_count`  ~ `AlertDispatcher.minute_count`
//!   - `GhostDispatcher.minute_start`  ~ `AlertDispatcher.minute_start`
//!   - `ghost_process_alert`           ~ `AlertDispatcher::process_alert`
//!     (Phases 1 + 2 only; Phase 0 subscription routing is the responsibility
//!      of the verified `relay-to::SubscriptionTable` and is modelled here
//!      as the precondition "alert is subscribed".)
//!
//! # Verification
//!
//! Run via Bazel (hermetic Verus toolchain — no manual install):
//!
//! ```bash
//! bazel test //proofs/verus:alert_dedup_verify
//! ```
//!
//! Expected output:
//!     verification results:: <N> verified, 0 errors
//!
//! The Verus toolchain version is pinned via `rules_verus` in `MODULE.bazel`.

use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Constants — kept in lock-step with `engine.rs`.
// ---------------------------------------------------------------------------

pub spec const DEDUP_COOLDOWN_SEC: nat = 300;
pub spec const MAX_ALERTS_PER_MINUTE: nat = 10;
pub spec const MAX_RECENT_ALERTS: nat = 64;
pub spec const MINUTE_SEC: nat = 60;

// ---------------------------------------------------------------------------
// Ghost domain types (mirroring `engine.rs`).
// ---------------------------------------------------------------------------

/// Ghost mirror of `DispatchAction`.
pub enum GhostAction {
    Send,
    Deduplicated,
    RateLimited,
    NotSubscribed,
}

/// Ghost mirror of an `AlertEntry`.
pub struct GhostEntry {
    pub zone_id: nat,
    pub alert_type: nat,
    pub time: nat,
}

/// Ghost mirror of the relevant state of `AlertDispatcher`.
///
/// We use `Seq<GhostEntry>` instead of a fixed-size array because the
/// dedup logic scans the prefix `recent[0..recent_count]` linearly; the
/// upper bound `MAX_RECENT_ALERTS` shows up as a precondition.
pub struct GhostDispatcher {
    pub recent: Seq<GhostEntry>,
    pub minute_count: nat,
    pub minute_start: nat,
}

impl GhostDispatcher {
    /// Initial state matching `AlertDispatcher::new()`.
    pub open spec fn empty() -> Self {
        GhostDispatcher {
            recent: Seq::empty(),
            minute_count: 0,
            minute_start: 0,
        }
    }

    /// Well-formedness invariant — `recent` is bounded.
    pub open spec fn wf(self) -> bool {
        self.recent.len() <= MAX_RECENT_ALERTS
    }
}

// ---------------------------------------------------------------------------
// Spec functions that mirror the dedup and rate-limit checks in
// `process_alert`.  These are *pure* — they read the state, not mutate it.
// ---------------------------------------------------------------------------

/// `true` iff entry `i` in `d.recent` would shadow a new `(zone_id, alert_type)`
/// arriving at time `t`.  Direct transliteration of the inner-loop predicate
/// in `engine.rs`:
///
/// ```ignore
/// self.recent[idx].zone_id == zone_id
///     && self.recent[idx].alert_type == alert_type
///     && time < self.recent[idx].time.saturating_add(DEDUP_COOLDOWN_SEC)
/// ```
///
/// Note we use `+` over `nat` since saturating arithmetic on `u64` agrees
/// with `+` on `nat` whenever the operands fit in `u64` — which our
/// `t < u64::MAX - DEDUP_COOLDOWN_SEC` precondition guarantees.
pub open spec fn entry_shadows(
    e: GhostEntry,
    zone_id: nat,
    alert_type: nat,
    t: nat,
) -> bool {
    e.zone_id == zone_id
        && e.alert_type == alert_type
        && t < e.time + DEDUP_COOLDOWN_SEC
}

/// `true` iff *some* entry in `d.recent` shadows the incoming alert.
pub open spec fn would_dedup(
    d: GhostDispatcher,
    zone_id: nat,
    alert_type: nat,
    t: nat,
) -> bool {
    exists|i: int|
        0 <= i < d.recent.len()
            && entry_shadows(#[trigger] d.recent[i], zone_id, alert_type, t)
}

/// The post-state of the "reset minute counter if new window" step.
pub open spec fn after_minute_reset(d: GhostDispatcher, t: nat) -> GhostDispatcher {
    if t >= d.minute_start + MINUTE_SEC {
        GhostDispatcher { minute_count: 0, minute_start: t, ..d }
    } else {
        d
    }
}

/// `true` iff the rate-limit gate fires in state `d` at time `t`.
/// Mirrors `if self.minute_count >= MAX_ALERTS_PER_MINUTE { ... RateLimited }`
/// applied *after* the minute-window reset.
pub open spec fn would_rate_limit(d: GhostDispatcher, t: nat) -> bool {
    after_minute_reset(d, t).minute_count >= MAX_ALERTS_PER_MINUTE
}

// ---------------------------------------------------------------------------
// The ghost transition function — equivalent to phases 1 + 2 of
// `AlertDispatcher::process_alert`.  Returns `(new_state, action)`.
//
// Pre-condition `is_subscribed == true` corresponds to phase 0 (the
// verified relay-to routing decision).  When `is_subscribed == false`
// the executable code returns `NotSubscribed` immediately; the ghost
// model skips that path because it's already covered by relay-to's
// own verification.
// ---------------------------------------------------------------------------

pub open spec fn ghost_process_alert(
    d: GhostDispatcher,
    zone_id: nat,
    alert_type: nat,
    t: nat,
) -> (GhostDispatcher, GhostAction) {
    let d1 = after_minute_reset(d, t);
    if would_dedup(d1, zone_id, alert_type, t) {
        (d1, GhostAction::Deduplicated)
    } else if d1.minute_count >= MAX_ALERTS_PER_MINUTE {
        (d1, GhostAction::RateLimited)
    } else {
        let new_recent = if d1.recent.len() < MAX_RECENT_ALERTS {
            d1.recent.push(GhostEntry { zone_id, alert_type, time: t })
        } else {
            d1.recent
        };
        let d2 = GhostDispatcher {
            recent: new_recent,
            minute_count: d1.minute_count + 1,
            minute_start: d1.minute_start,
        };
        (d2, GhostAction::Send)
    }
}

// ---------------------------------------------------------------------------
// Helper lemma — a Send transition appends the just-processed entry.
// ---------------------------------------------------------------------------

/// If `ghost_process_alert` returns `Send`, then the resulting `recent`
/// sequence contains an entry for `(zone_id, alert_type, t)` somewhere.
proof fn lemma_send_records_entry(
    d: GhostDispatcher,
    zone_id: nat,
    alert_type: nat,
    t: nat,
)
    requires
        d.wf(),
        d.recent.len() < MAX_RECENT_ALERTS, // ensure the push happens
        ({
            let (_, a) = ghost_process_alert(d, zone_id, alert_type, t);
            a is Send
        }),
    ensures
        ({
            let (d2, _) = ghost_process_alert(d, zone_id, alert_type, t);
            exists|i: int|
                0 <= i < d2.recent.len()
                    && (#[trigger] d2.recent[i]) == (GhostEntry { zone_id, alert_type, time: t })
        }),
{
    let d1 = after_minute_reset(d, t);
    let (d2, _a) = ghost_process_alert(d, zone_id, alert_type, t);
    // d2.recent == d1.recent.push(new_entry); the witness is index len.
    let new_entry = GhostEntry { zone_id, alert_type, time: t };
    let pushed = d1.recent.push(new_entry);
    assert(d2.recent == pushed);
    assert(pushed[d1.recent.len() as int] == new_entry);
}

// ---------------------------------------------------------------------------
// The dedup invariant — the primary obligation from Issue #7.
//
// Statement:
//   Let `d0` be any well-formed dispatcher with `recent.len() < MAX_RECENT_ALERTS`
//   so that the first call can record.  Let
//       (d1, a1) = ghost_process_alert(d0, zone_id, alert_type, t1)
//       (d2, a2) = ghost_process_alert(d1, zone_id, alert_type, t2)
//   with `t1 <= t2 < t1 + DEDUP_COOLDOWN_SEC`.
//   If `a1 == Send` then `a2 == Deduplicated`.
//
// This is the precise property that motivates the deductive proof:
// Kani's BMC only checks bounded values of `t1`, `t2`, `zone_id`, etc.
// Verus proves it *for all* nats.
// ---------------------------------------------------------------------------

pub proof fn theorem_dedup_invariant(
    d0: GhostDispatcher,
    zone_id: nat,
    alert_type: nat,
    t1: nat,
    t2: nat,
)
    requires
        d0.wf(),
        d0.recent.len() < MAX_RECENT_ALERTS,
        t1 <= t2,
        t2 < t1 + DEDUP_COOLDOWN_SEC,
        ({
            let (_d1, a1) = ghost_process_alert(d0, zone_id, alert_type, t1);
            a1 is Send
        }),
    ensures
        ({
            let (d1, _a1) = ghost_process_alert(d0, zone_id, alert_type, t1);
            let (_d2, a2) = ghost_process_alert(d1, zone_id, alert_type, t2);
            a2 is Deduplicated
        }),
{
    let (d1, a1) = ghost_process_alert(d0, zone_id, alert_type, t1);
    // Because a1 == Send, the push branch was taken, so the new entry
    // sits at index `after_minute_reset(d0, t1).recent.len()` of d1.recent.
    let d0_post = after_minute_reset(d0, t1);
    let new_entry = GhostEntry { zone_id, alert_type, time: t1 };

    // The first call did not dedup and did not rate-limit.
    assert(!would_dedup(d0_post, zone_id, alert_type, t1));
    assert(d0_post.minute_count < MAX_ALERTS_PER_MINUTE);

    // The push branch ran (we required d0.recent.len() < MAX_RECENT_ALERTS,
    // and after_minute_reset doesn't change recent).
    assert(d0_post.recent.len() < MAX_RECENT_ALERTS);
    assert(d1.recent == d0_post.recent.push(new_entry));

    // The new entry shadows (zone_id, alert_type, t2):
    //   new_entry.time == t1, so t2 < new_entry.time + DEDUP_COOLDOWN_SEC
    //   is exactly t2 < t1 + DEDUP_COOLDOWN_SEC.
    assert(entry_shadows(new_entry, zone_id, alert_type, t2));

    // The witness for `would_dedup` after minute-reset on d1:
    // after_minute_reset doesn't change `recent`, so the entry persists.
    let d1_post = after_minute_reset(d1, t2);
    assert(d1_post.recent == d1.recent);

    // Index of the appended entry.
    let idx: int = d0_post.recent.len() as int;
    assert(0 <= idx < d1_post.recent.len());
    assert(d1_post.recent[idx] == new_entry);
    assert(entry_shadows(d1_post.recent[idx], zone_id, alert_type, t2));

    // Conclude would_dedup, which forces the Deduplicated branch.
    assert(would_dedup(d1_post, zone_id, alert_type, t2));
}

// ---------------------------------------------------------------------------
// The rate-limit invariant.
//
// Statement:
//   If `d.minute_count >= MAX_ALERTS_PER_MINUTE` and `t < d.minute_start + 60`
//   and the incoming `(zone_id, alert_type, t)` would not be deduplicated,
//   then `ghost_process_alert` returns `RateLimited`.
//
// This is the dual of the dedup invariant — it states that rate-limiting
// kicks in when the per-minute budget is saturated.
// ---------------------------------------------------------------------------

pub proof fn theorem_rate_limit_invariant(
    d: GhostDispatcher,
    zone_id: nat,
    alert_type: nat,
    t: nat,
)
    requires
        d.minute_count >= MAX_ALERTS_PER_MINUTE,
        t < d.minute_start + MINUTE_SEC,
        !would_dedup(d, zone_id, alert_type, t),
    ensures
        ({
            let (_, a) = ghost_process_alert(d, zone_id, alert_type, t);
            a is RateLimited
        }),
{
    // The minute-reset branch is *not* taken because t < minute_start + 60.
    assert(!(t >= d.minute_start + MINUTE_SEC));
    let d1 = after_minute_reset(d, t);
    assert(d1 == d);
    // Dedup check fails by hypothesis.
    assert(!would_dedup(d1, zone_id, alert_type, t));
    // Rate-limit gate fires.
    assert(d1.minute_count >= MAX_ALERTS_PER_MINUTE);
}

// ---------------------------------------------------------------------------
// Sanity lemma: first alert (empty state, subscribed) returns Send.
// This is the base-case Kani harness ALERT-P03 asserts as a precondition;
// we prove it deductively here so the dedup-invariant theorem composes.
// ---------------------------------------------------------------------------

pub proof fn lemma_first_alert_sends(zone_id: nat, alert_type: nat, t: nat)
    ensures
        ({
            let d0 = GhostDispatcher::empty();
            let (_, a) = ghost_process_alert(d0, zone_id, alert_type, t);
            a is Send
        }),
{
    let d0 = GhostDispatcher::empty();
    let d1 = after_minute_reset(d0, t);
    // Empty recent => no shadow possible.
    assert(d1.recent.len() == 0);
    assert(!would_dedup(d1, zone_id, alert_type, t));
    // minute_count is 0 on both branches of the reset.
    assert(d1.minute_count == 0);
    assert(d1.minute_count < MAX_ALERTS_PER_MINUTE);
    // And the push happens (0 < MAX_RECENT_ALERTS).
    assert(d1.recent.len() < MAX_RECENT_ALERTS);
}

} // verus!

fn main() {}
