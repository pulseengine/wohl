//! Wohl OTA Core — dual-bank firmware update state machine.
//!
//! Design-before-hardware: this crate proves the OTA state machine in
//! software *before* any board-specific glue. The AADL model in
//! `spar/wohl_firmware.aadl` (NodeBootloader, OTAManager, OTAManagerProcess)
//! declares the threads and ports — this crate is the verified core that
//! those threads will eventually call.
//!
//! ## Model
//!
//! Two firmware slots: `A` and `B`. At rest, one is `Active` (running) and
//! the other is `Standby` (free to be overwritten). Updates are streamed
//! into the standby slot, verified, then atomically swapped to become the
//! new active slot. Rollback reverts the swap.
//!
//! ## Safety invariants (Kani-verified)
//!
//! - **OTA-P01** — downloads never target the active slot
//! - **OTA-P02** — only `Ready` → `Swapping` transitions are accepted
//! - **OTA-P03** — `bytes_received` never exceeds `total_bytes`
//! - **OTA-P04** — no panics for any sequence of public API calls
//! - **OTA-P05** — `Ready` is reachable only if the supplied image digest
//!   matches `manifest.sha256` (and the signature verifies)
//! - **OTA-P06** — `rollback` only reverts a slot a prior `swap` recorded,
//!   and a second `rollback` with no intervening swap fails cleanly
//!
//! Signature verification is abstracted via [`SignatureVerifier`]. A real
//! Ed25519 backend lives in a downstream crate (out of scope here); the
//! [`AlwaysAccept`] and [`AlwaysReject`] stubs in this module are used for
//! tests and bounded model-checking.
//!
//! The image-content hash (SHA-256) is computed *outside* the core — by the
//! HAL/caller over the received bytes — and passed into [`OtaCore::verify`].
//! The core stays crypto-free: it performs only the fixed-size byte-for-byte
//! `[u8; 32]` equality against `manifest.sha256`. This mirrors the
//! delegation pattern already used for [`SignatureVerifier`].

#![allow(clippy::needless_return)]

/// Maximum manifest payload size we are willing to consider, in bytes.
/// Bounds Kani's search space and reflects the per-image budget on Gale
/// nodes (≤ 1 MiB of usable flash per slot on ESP32-class hardware).
pub const MAX_IMAGE_BYTES: u32 = 1_048_576;

/// Maximum bytes accepted in a single `write_chunk` call. Keeps Kani
/// loops small and matches CFDP file-data PDU sizing (≈ 1 KiB).
pub const MAX_CHUNK_BYTES: u32 = 1024;

// ── Slot model ──────────────────────────────────────────────────────────

/// Identifier of a firmware slot. The hardware has exactly two banks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    A,
    B,
}

impl Slot {
    /// The opposite bank — used to compute the standby of an active slot.
    pub const fn other(self) -> Slot {
        match self {
            Slot::A => Slot::B,
            Slot::B => Slot::A,
        }
    }
}

// ── Manifest ────────────────────────────────────────────────────────────

/// Update manifest streamed by the hub OTA manager. Wire format is fixed
/// (`core::mem::size_of::<OtaManifest>() == 108` bytes) so the on-node
/// bootloader can parse it without any serialization library.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OtaManifest {
    pub version: u32,
    pub size_bytes: u32,
    pub sha256: [u8; 32],
    pub signature: [u8; 64], // Ed25519
}

impl OtaManifest {
    /// Cheap structural sanity check applied *before* signature
    /// verification — catches obviously-malformed manifests so we don't
    /// waste a slot allocating space for an impossible image.
    pub fn is_well_formed(&self) -> bool {
        self.size_bytes > 0 && self.size_bytes <= MAX_IMAGE_BYTES
    }
}

// ── Signature verification (trait abstraction) ─────────────────────────

/// Pluggable signature verifier. Real Ed25519 lives in a downstream crate;
/// this trait keeps the state-machine core crypto-free and easy to verify.
pub trait SignatureVerifier {
    /// Returns `true` iff `signature` is a valid signature over a stable
    /// hash of `manifest`. Implementations must be pure and panic-free.
    fn verify(&self, manifest: &OtaManifest, signature: &[u8; 64]) -> bool;
}

/// Stub verifier that always succeeds — used in happy-path tests and to
/// prove progress-related invariants under Kani.
#[derive(Clone, Copy, Debug)]
pub struct AlwaysAccept;

impl SignatureVerifier for AlwaysAccept {
    fn verify(&self, _manifest: &OtaManifest, _signature: &[u8; 64]) -> bool {
        true
    }
}

/// Stub verifier that always rejects — used to prove that the state
/// machine never promotes an unverified image.
#[derive(Clone, Copy, Debug)]
pub struct AlwaysReject;

impl SignatureVerifier for AlwaysReject {
    fn verify(&self, _manifest: &OtaManifest, _signature: &[u8; 64]) -> bool {
        false
    }
}

// ── State machine ──────────────────────────────────────────────────────

/// Top-level OTA state.
///
/// Invariants enforced by transition functions (see Kani harnesses for
/// machine-checkable statements):
///
/// 1. The slot referenced by `Downloading` / `Verifying` / `Ready` is the
///    *standby* slot — i.e. **never** equal to `active`.
/// 2. `bytes_received <= total_bytes` in `Downloading`.
/// 3. Transitions to `Swapping` are only legal from `Ready`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OtaState {
    /// Nothing in progress. The active slot is the only one with valid code.
    Idle,
    /// Image being streamed into `slot` (standby).
    Downloading {
        slot: Slot,
        bytes_received: u32,
        total_bytes: u32,
    },
    /// Download complete; checking manifest signature.
    Verifying { slot: Slot },
    /// Verified — a swap call will promote this slot.
    Ready { slot: Slot },
    /// Transient — the bootloader is rewriting the boot pointer.
    Swapping,
}

/// Errors that the public API can return. All variants are recoverable;
/// the caller is expected to either retry, rollback, or abort.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OtaError {
    /// API called from a state that doesn't accept it.
    WrongState,
    /// Manifest's `size_bytes` is zero or exceeds `MAX_IMAGE_BYTES`.
    BadManifest,
    /// Chunk would overrun the declared `total_bytes`.
    ChunkOverflow,
    /// Chunk size exceeds `MAX_CHUNK_BYTES`.
    ChunkTooLarge,
    /// Signature verification failed.
    BadSignature,
    /// The computed image digest does not match `manifest.sha256`.
    BadImageDigest,
    /// `finish_download` called before all bytes were received.
    Incomplete,
    /// `rollback` called with no swap to revert (no recorded `last_swap`).
    NothingToRollback,
}

/// The OTA core. Holds the current active slot and the in-flight state.
#[derive(Clone, Copy, Debug)]
pub struct OtaCore {
    active: Slot,
    state: OtaState,
    /// Manifest of the in-flight update (None outside of a download).
    pending: Option<OtaManifest>,
    /// The slot that was `active` *before* the most recent `swap`, while
    /// that swap has not yet been confirmed. `Some` means a rollback is
    /// available; `None` means there is nothing to roll back to.
    last_swap: Option<Slot>,
}

impl OtaCore {
    /// Construct an OTA core with `active` as the currently-running bank.
    pub const fn new(active: Slot) -> Self {
        Self {
            active,
            state: OtaState::Idle,
            pending: None,
            last_swap: None,
        }
    }

    /// The slot currently executing code.
    pub fn active_slot(&self) -> Slot {
        self.active
    }

    /// The slot that updates would write into (always `active.other()`).
    pub fn standby_slot(&self) -> Slot {
        self.active.other()
    }

    pub fn state(&self) -> OtaState {
        self.state
    }

    pub fn pending_manifest(&self) -> Option<OtaManifest> {
        self.pending
    }

    /// The slot a `rollback` would currently revert to, if any. `Some`
    /// after a `swap` and before a `confirm_swap`/`rollback`; `None`
    /// otherwise (fresh boot, or once the swap has been confirmed).
    pub fn rollback_target(&self) -> Option<Slot> {
        self.last_swap
    }

    /// Begin a download for the given manifest into the standby slot.
    ///
    /// Only legal from `Idle`. The destination is **always** the standby
    /// slot — callers cannot pick — which is what guarantees OTA-P01.
    pub fn start_download(&mut self, manifest: OtaManifest) -> Result<(), OtaError> {
        if !matches!(self.state, OtaState::Idle) {
            return Err(OtaError::WrongState);
        }
        if !manifest.is_well_formed() {
            return Err(OtaError::BadManifest);
        }
        self.state = OtaState::Downloading {
            slot: self.active.other(),
            bytes_received: 0,
            total_bytes: manifest.size_bytes,
        };
        self.pending = Some(manifest);
        Ok(())
    }

    /// Account for `chunk_len` bytes of payload written into the standby
    /// slot. The actual flash write happens in the caller (HAL); this core
    /// only tracks the byte counter and bounds-checks against `total_bytes`.
    pub fn write_chunk(&mut self, chunk_len: u32) -> Result<(), OtaError> {
        if chunk_len > MAX_CHUNK_BYTES {
            return Err(OtaError::ChunkTooLarge);
        }
        match self.state {
            OtaState::Downloading {
                slot,
                bytes_received,
                total_bytes,
            } => {
                // Saturating + explicit check — even if `chunk_len` is
                // huge we cannot wrap, because `MAX_CHUNK_BYTES` is small.
                let new = bytes_received.saturating_add(chunk_len);
                if new > total_bytes {
                    return Err(OtaError::ChunkOverflow);
                }
                self.state = OtaState::Downloading {
                    slot,
                    bytes_received: new,
                    total_bytes,
                };
                Ok(())
            }
            _ => Err(OtaError::WrongState),
        }
    }

    /// Mark the download as complete and move into `Verifying`.
    /// Fails if not all declared bytes have been received.
    pub fn finish_download(&mut self) -> Result<(), OtaError> {
        match self.state {
            OtaState::Downloading {
                slot,
                bytes_received,
                total_bytes,
            } => {
                if bytes_received != total_bytes {
                    return Err(OtaError::Incomplete);
                }
                self.state = OtaState::Verifying { slot };
                Ok(())
            }
            _ => Err(OtaError::WrongState),
        }
    }

    /// Verify the pending update and, on success, transition to `Ready`.
    ///
    /// Reaching `Ready` requires **both**:
    ///
    /// 1. `verifier` accepts the manifest signature, **and**
    /// 2. `image_digest` — the SHA-256 the caller/HAL computed over the
    ///    bytes actually streamed into the standby slot — equals
    ///    `manifest.sha256` byte-for-byte.
    ///
    /// The core stays crypto-free: the hash *computation* is delegated to
    /// the caller (same pattern as [`SignatureVerifier`]); the core only
    /// performs the fixed-size `[u8; 32]` equality. If either check fails
    /// the machine aborts back to `Idle` and clears the pending manifest —
    /// an unverified image can never become `Ready`.
    pub fn verify<V: SignatureVerifier>(
        &mut self,
        verifier: &V,
        image_digest: &[u8; 32],
    ) -> Result<(), OtaError> {
        match self.state {
            OtaState::Verifying { slot } => {
                // pending is always Some in Verifying — set in start_download
                // and never cleared until Idle is reached again.
                let m = match self.pending {
                    Some(m) => m,
                    None => {
                        // Defensive: should be unreachable. Reset to Idle
                        // rather than panic.
                        self.state = OtaState::Idle;
                        return Err(OtaError::WrongState);
                    }
                };
                // (a) Manifest signature must verify.
                if !verifier.verify(&m, &m.signature) {
                    self.state = OtaState::Idle;
                    self.pending = None;
                    return Err(OtaError::BadSignature);
                }
                // (b) The downloaded image must match the manifest hash.
                // Crypto-free: a plain fixed-size byte comparison.
                if *image_digest != m.sha256 {
                    self.state = OtaState::Idle;
                    self.pending = None;
                    return Err(OtaError::BadImageDigest);
                }
                self.state = OtaState::Ready { slot };
                Ok(())
            }
            _ => Err(OtaError::WrongState),
        }
    }

    /// Swap the standby slot in — promoting it to active. Only legal from
    /// `Ready`. After a successful swap the previous active becomes the
    /// new standby (still holds the prior image for rollback).
    ///
    /// The pre-swap active slot is recorded in `last_swap`, opening the
    /// post-swap rollback window. The window is closed by either
    /// [`confirm_swap`](Self::confirm_swap) or [`rollback`](Self::rollback).
    pub fn swap(&mut self) -> Result<(), OtaError> {
        match self.state {
            OtaState::Ready { slot } => {
                // The standby slot must match the active's other half —
                // this is structural, but assert via match for clarity.
                self.state = OtaState::Swapping;
                // Commit the swap immediately. In real hardware
                // `Swapping` would briefly straddle a reset; here we
                // model it as instantaneous from the state-machine view.
                //
                // Record the slot we are leaving so a subsequent
                // `rollback` has a concrete, verified slot to revert to.
                self.last_swap = Some(self.active);
                self.active = slot;
                self.state = OtaState::Idle;
                self.pending = None;
                Ok(())
            }
            _ => Err(OtaError::WrongState),
        }
    }

    /// Confirm (commit) the most recent swap, making the new firmware
    /// permanent. Clears `last_swap` so a later `rollback` cannot revert
    /// to a slot whose image may since have been overwritten.
    ///
    /// Idempotent: confirming with nothing pending is a harmless no-op.
    /// Only legal from `Idle` — refusing during a download/verify keeps
    /// the rollback window and the in-flight update from interleaving.
    pub fn confirm_swap(&mut self) -> Result<(), OtaError> {
        match self.state {
            OtaState::Idle => {
                self.last_swap = None;
                Ok(())
            }
            _ => Err(OtaError::WrongState),
        }
    }

    /// Revert the most recent, as-yet-unconfirmed swap.
    ///
    /// Succeeds **only** when `last_swap` is `Some` — i.e. a `swap` has
    /// happened and has not yet been confirmed (or already rolled back).
    /// On success it restores `active` to the recorded pre-swap slot and
    /// clears `last_swap`, so a second `rollback` with no intervening
    /// `swap` fails cleanly with [`OtaError::NothingToRollback`] — it
    /// never promotes a slot that no swap ever produced.
    ///
    /// Legal only from `Idle`: we refuse to rollback mid-update to avoid
    /// losing the in-progress image, and a fresh boot (no swap) is
    /// rejected because `last_swap` is `None`.
    pub fn rollback(&mut self) -> Result<(), OtaError> {
        match self.state {
            OtaState::Idle => match self.last_swap {
                Some(prev) => {
                    self.active = prev;
                    self.last_swap = None;
                    Ok(())
                }
                None => Err(OtaError::NothingToRollback),
            },
            _ => Err(OtaError::WrongState),
        }
    }

    /// Abort an in-flight update and return to `Idle`. Always safe.
    pub fn abort(&mut self) {
        self.state = OtaState::Idle;
        self.pending = None;
    }

    // ── Inspector helpers (used by both tests and Kani proofs) ───────

    /// Returns the slot currently being written, if any.
    pub fn target_slot(&self) -> Option<Slot> {
        match self.state {
            OtaState::Downloading { slot, .. }
            | OtaState::Verifying { slot }
            | OtaState::Ready { slot } => Some(slot),
            OtaState::Idle | OtaState::Swapping => None,
        }
    }
}

// ── Plain Rust tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinctive non-zero digest so digest-mismatch tests are meaningful.
    const GOOD_DIGEST: [u8; 32] = [0xABu8; 32];

    fn manifest(size: u32) -> OtaManifest {
        OtaManifest {
            version: 1,
            size_bytes: size,
            sha256: GOOD_DIGEST,
            signature: [0u8; 64],
        }
    }

    #[test]
    fn happy_path_swaps_active() {
        let mut core = OtaCore::new(Slot::A);
        assert_eq!(core.active_slot(), Slot::A);
        core.start_download(manifest(2048)).unwrap();
        assert_eq!(core.target_slot(), Some(Slot::B));
        core.write_chunk(1024).unwrap();
        core.write_chunk(1024).unwrap();
        core.finish_download().unwrap();
        core.verify(&AlwaysAccept, &GOOD_DIGEST).unwrap();
        core.swap().unwrap();
        assert_eq!(core.active_slot(), Slot::B);
        assert_eq!(core.state(), OtaState::Idle);
    }

    #[test]
    fn target_slot_is_never_active() {
        let mut core = OtaCore::new(Slot::A);
        core.start_download(manifest(64)).unwrap();
        assert_ne!(core.target_slot().unwrap(), core.active_slot());
        core.abort();

        let mut core = OtaCore::new(Slot::B);
        core.start_download(manifest(64)).unwrap();
        assert_ne!(core.target_slot().unwrap(), core.active_slot());
    }

    #[test]
    fn write_chunk_rejects_overflow() {
        let mut core = OtaCore::new(Slot::A);
        core.start_download(manifest(1024)).unwrap();
        core.write_chunk(1024).unwrap();
        assert_eq!(core.write_chunk(1), Err(OtaError::ChunkOverflow));
    }

    #[test]
    fn write_chunk_rejects_too_large() {
        let mut core = OtaCore::new(Slot::A);
        core.start_download(manifest(2 * MAX_CHUNK_BYTES)).unwrap();
        assert_eq!(
            core.write_chunk(MAX_CHUNK_BYTES + 1),
            Err(OtaError::ChunkTooLarge)
        );
    }

    #[test]
    fn finish_requires_full_image() {
        let mut core = OtaCore::new(Slot::A);
        core.start_download(manifest(1024)).unwrap();
        core.write_chunk(512).unwrap();
        assert_eq!(core.finish_download(), Err(OtaError::Incomplete));
    }

    #[test]
    fn bad_manifest_rejected() {
        let mut core = OtaCore::new(Slot::A);
        assert_eq!(core.start_download(manifest(0)), Err(OtaError::BadManifest));
        assert_eq!(
            core.start_download(manifest(MAX_IMAGE_BYTES + 1)),
            Err(OtaError::BadManifest)
        );
    }

    #[test]
    fn bad_signature_resets_to_idle() {
        let mut core = OtaCore::new(Slot::A);
        core.start_download(manifest(64)).unwrap();
        core.write_chunk(64).unwrap();
        core.finish_download().unwrap();
        assert_eq!(
            core.verify(&AlwaysReject, &GOOD_DIGEST),
            Err(OtaError::BadSignature)
        );
        assert_eq!(core.state(), OtaState::Idle);
        assert_eq!(core.active_slot(), Slot::A); // unchanged
        assert!(core.pending_manifest().is_none());
    }

    #[test]
    fn wrong_digest_never_reaches_ready() {
        // Correctly-signed manifest paired with a payload whose hash does
        // not match `manifest.sha256` must NOT promote to Ready.
        let mut core = OtaCore::new(Slot::A);
        core.start_download(manifest(64)).unwrap();
        core.write_chunk(64).unwrap();
        core.finish_download().unwrap();
        let wrong_digest = [0x11u8; 32]; // != GOOD_DIGEST
        assert_eq!(
            core.verify(&AlwaysAccept, &wrong_digest),
            Err(OtaError::BadImageDigest)
        );
        assert_eq!(core.state(), OtaState::Idle);
        assert_eq!(core.active_slot(), Slot::A); // unchanged
        assert!(core.pending_manifest().is_none());
        // And a swap cannot follow.
        assert_eq!(core.swap(), Err(OtaError::WrongState));
    }

    #[test]
    fn swap_only_from_ready() {
        let mut core = OtaCore::new(Slot::A);
        assert_eq!(core.swap(), Err(OtaError::WrongState));
        core.start_download(manifest(64)).unwrap();
        assert_eq!(core.swap(), Err(OtaError::WrongState));
        core.write_chunk(64).unwrap();
        assert_eq!(core.swap(), Err(OtaError::WrongState));
        core.finish_download().unwrap();
        assert_eq!(core.swap(), Err(OtaError::WrongState));
        core.verify(&AlwaysAccept, &GOOD_DIGEST).unwrap();
        // Now Ready — swap should succeed.
        assert!(core.swap().is_ok());
    }

    #[test]
    fn rollback_rejected_during_update() {
        let mut core = OtaCore::new(Slot::A);
        core.start_download(manifest(64)).unwrap();
        assert_eq!(core.rollback(), Err(OtaError::WrongState));
        core.abort();
    }

    #[test]
    fn rollback_fails_on_fresh_boot() {
        // No swap has happened — rollback must NOT flip the active slot.
        let mut core = OtaCore::new(Slot::A);
        assert_eq!(core.rollback_target(), None);
        assert_eq!(core.rollback(), Err(OtaError::NothingToRollback));
        assert_eq!(core.active_slot(), Slot::A); // unchanged
    }

    /// Drive a full verified update so the core is `Idle` on `to` slot.
    fn do_swap(core: &mut OtaCore) {
        core.start_download(manifest(64)).unwrap();
        core.write_chunk(64).unwrap();
        core.finish_download().unwrap();
        core.verify(&AlwaysAccept, &GOOD_DIGEST).unwrap();
        core.swap().unwrap();
    }

    #[test]
    fn rollback_reverts_recorded_swap_then_fails() {
        let mut core = OtaCore::new(Slot::A);
        do_swap(&mut core);
        assert_eq!(core.active_slot(), Slot::B);
        assert_eq!(core.rollback_target(), Some(Slot::A));

        // First rollback reverts to the recorded pre-swap slot.
        assert!(core.rollback().is_ok());
        assert_eq!(core.active_slot(), Slot::A);
        assert_eq!(core.rollback_target(), None);

        // Second rollback, no intervening swap: fails cleanly, no flip.
        assert_eq!(core.rollback(), Err(OtaError::NothingToRollback));
        assert_eq!(core.active_slot(), Slot::A);
    }

    #[test]
    fn confirm_swap_closes_rollback_window() {
        let mut core = OtaCore::new(Slot::A);
        do_swap(&mut core);
        assert_eq!(core.rollback_target(), Some(Slot::A));
        core.confirm_swap().unwrap();
        assert_eq!(core.rollback_target(), None);
        // After confirm the new firmware is permanent — rollback refused.
        assert_eq!(core.rollback(), Err(OtaError::NothingToRollback));
        assert_eq!(core.active_slot(), Slot::B);
        // confirm_swap is idempotent.
        assert!(core.confirm_swap().is_ok());
    }

    #[test]
    fn abort_is_always_safe() {
        let mut core = OtaCore::new(Slot::A);
        core.abort(); // from Idle
        core.start_download(manifest(64)).unwrap();
        core.abort();
        assert_eq!(core.state(), OtaState::Idle);
        // Active slot must not change due to an abort.
        assert_eq!(core.active_slot(), Slot::A);
    }

    #[test]
    fn slot_other_is_involutive() {
        assert_eq!(Slot::A.other().other(), Slot::A);
        assert_eq!(Slot::B.other().other(), Slot::B);
        assert_ne!(Slot::A.other(), Slot::A);
    }
}

// ── proptest property tests ─────────────────────────────────────────────

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn manifest_strategy() -> impl Strategy<Value = OtaManifest> {
        (1u32..=MAX_IMAGE_BYTES, any::<u32>(), any::<[u8; 32]>()).prop_map(
            |(size, version, sha256)| OtaManifest {
                version,
                size_bytes: size,
                sha256,
                signature: [0u8; 64],
            },
        )
    }

    proptest! {
        /// Target slot is always the other one of active, throughout the
        /// download phase.
        #[test]
        fn target_never_equals_active(
            initial in prop_oneof![Just(Slot::A), Just(Slot::B)],
            m in manifest_strategy(),
        ) {
            let mut core = OtaCore::new(initial);
            core.start_download(m).unwrap();
            prop_assert_ne!(core.target_slot().unwrap(), core.active_slot());
        }

        /// `bytes_received` never exceeds `total_bytes` after any sequence
        /// of (possibly over-large) chunks.
        #[test]
        fn bytes_received_bounded(
            m in manifest_strategy(),
            chunks in proptest::collection::vec(0u32..=MAX_CHUNK_BYTES * 2, 0..32),
        ) {
            let mut core = OtaCore::new(Slot::A);
            core.start_download(m).unwrap();
            for c in chunks {
                let _ = core.write_chunk(c);
                if let OtaState::Downloading { bytes_received, total_bytes, .. } = core.state() {
                    prop_assert!(bytes_received <= total_bytes);
                }
            }
        }

        /// Rejected signature always returns to Idle with unchanged active.
        #[test]
        fn bad_signature_is_idempotent(
            initial in prop_oneof![Just(Slot::A), Just(Slot::B)],
            m in manifest_strategy(),
        ) {
            let mut core = OtaCore::new(initial);
            core.start_download(m).unwrap();
            // Pump bytes to completion.
            let mut left = m.size_bytes;
            while left > 0 {
                let n = left.min(MAX_CHUNK_BYTES);
                core.write_chunk(n).unwrap();
                left -= n;
            }
            core.finish_download().unwrap();
            // Even with the *correct* digest, a rejected signature aborts.
            let _ = core.verify(&AlwaysReject, &m.sha256);
            prop_assert_eq!(core.state(), OtaState::Idle);
            prop_assert_eq!(core.active_slot(), initial);
        }

        /// Successful swap toggles the active slot and clears pending.
        #[test]
        fn swap_toggles_active(
            initial in prop_oneof![Just(Slot::A), Just(Slot::B)],
            size in 1u32..=4 * MAX_CHUNK_BYTES,
            digest in any::<[u8; 32]>(),
        ) {
            let m = OtaManifest {
                version: 1,
                size_bytes: size,
                sha256: digest,
                signature: [0u8; 64],
            };
            let mut core = OtaCore::new(initial);
            core.start_download(m).unwrap();
            let mut left = size;
            while left > 0 {
                let n = left.min(MAX_CHUNK_BYTES);
                core.write_chunk(n).unwrap();
                left -= n;
            }
            core.finish_download().unwrap();
            // Matching digest + accepting verifier → Ready → swap.
            core.verify(&AlwaysAccept, &digest).unwrap();
            core.swap().unwrap();
            prop_assert_eq!(core.active_slot(), initial.other());
            prop_assert_eq!(core.state(), OtaState::Idle);
            prop_assert!(core.pending_manifest().is_none());
        }

        /// Image-digest binding: even with an accepting verifier, `Ready`
        /// is reached **iff** the supplied digest equals `manifest.sha256`.
        #[test]
        fn ready_requires_matching_digest(
            m in manifest_strategy(),
            supplied in any::<[u8; 32]>(),
        ) {
            let mut core = OtaCore::new(Slot::A);
            core.start_download(m).unwrap();
            let mut left = m.size_bytes;
            while left > 0 {
                let n = left.min(MAX_CHUNK_BYTES);
                core.write_chunk(n).unwrap();
                left -= n;
            }
            core.finish_download().unwrap();
            let res = core.verify(&AlwaysAccept, &supplied);
            if supplied == m.sha256 {
                prop_assert_eq!(res, Ok(()));
                prop_assert_eq!(core.state(), OtaState::Ready { slot: Slot::B });
            } else {
                prop_assert_eq!(res, Err(OtaError::BadImageDigest));
                prop_assert_eq!(core.state(), OtaState::Idle);
            }
        }

        /// Rollback guard: a `rollback` reverts the active slot only when a
        /// prior `swap` recorded one, and is then refused until the next
        /// swap. A fresh boot is never rolled back.
        #[test]
        fn rollback_guarded_by_prior_swap(
            initial in prop_oneof![Just(Slot::A), Just(Slot::B)],
            do_first_swap in any::<bool>(),
            digest in any::<[u8; 32]>(),
        ) {
            let mut core = OtaCore::new(initial);
            if do_first_swap {
                let m = OtaManifest {
                    version: 1, size_bytes: 64, sha256: digest, signature: [0u8; 64],
                };
                core.start_download(m).unwrap();
                core.write_chunk(64).unwrap();
                core.finish_download().unwrap();
                core.verify(&AlwaysAccept, &digest).unwrap();
                core.swap().unwrap();
                prop_assert_eq!(core.active_slot(), initial.other());
                // Exactly one rollback succeeds, reverting to `initial`.
                prop_assert_eq!(core.rollback(), Ok(()));
                prop_assert_eq!(core.active_slot(), initial);
            }
            // With no (further) recorded swap, rollback fails cleanly and
            // leaves the active slot untouched.
            let before = core.active_slot();
            prop_assert_eq!(core.rollback(), Err(OtaError::NothingToRollback));
            prop_assert_eq!(core.active_slot(), before);
        }
    }
}

// ── Kani bounded model checking harnesses ───────────────────────────────

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Build a manifest with a bounded size — keeps Kani's search small.
    /// `sha256` is fully nondeterministic so digest-binding proofs cover
    /// every possible manifest hash.
    fn any_small_manifest(max_size: u32) -> OtaManifest {
        let size: u32 = kani::any();
        kani::assume(size > 0 && size <= max_size);
        OtaManifest {
            version: kani::any(),
            size_bytes: size,
            sha256: kani::any(),
            signature: [0u8; 64],
        }
    }

    /// OTA-P01 — the slot targeted by any in-flight transfer is **never**
    /// the active slot, no matter the initial active or the manifest.
    #[kani::proof]
    fn ota_p01_target_never_active() {
        let initial: bool = kani::any();
        let init_slot = if initial { Slot::A } else { Slot::B };
        let mut core = OtaCore::new(init_slot);
        let m = any_small_manifest(64);
        core.start_download(m).unwrap();
        let t = core.target_slot().unwrap();
        assert!(t != core.active_slot());
        // Also during Verifying and Ready.
        let chunk: u32 = kani::any();
        kani::assume(chunk <= 64);
        // We cannot fully fill in pure Kani without unbounded loops, but
        // we can still inspect the Downloading invariant for one chunk.
        let _ = core.write_chunk(chunk);
        if let Some(t2) = core.target_slot() {
            assert!(t2 != core.active_slot());
        }
    }

    /// OTA-P02 — `swap` succeeds only from `Ready`. We test the negative
    /// half (all other states return `WrongState`) exhaustively over the
    /// states reachable in a short prefix.
    #[kani::proof]
    fn ota_p02_swap_requires_ready() {
        let initial: bool = kani::any();
        let init_slot = if initial { Slot::A } else { Slot::B };
        let mut core = OtaCore::new(init_slot);

        // From Idle: must fail.
        assert!(core.swap() == Err(OtaError::WrongState));

        // From Downloading: must fail.
        let m = OtaManifest {
            version: 1,
            size_bytes: 8,
            sha256: kani::any(),
            signature: [0u8; 64],
        };
        core.start_download(m).unwrap();
        assert!(core.swap() == Err(OtaError::WrongState));

        // After partial chunk: still Downloading, still fails.
        core.write_chunk(4).unwrap();
        assert!(core.swap() == Err(OtaError::WrongState));

        // Fill, finish → Verifying: must fail.
        core.write_chunk(4).unwrap();
        core.finish_download().unwrap();
        assert!(core.swap() == Err(OtaError::WrongState));

        // Reject signature → back to Idle: must still fail. Digest is
        // irrelevant — a rejected signature aborts before it is checked.
        let pre_active = core.active_slot();
        let digest: [u8; 32] = kani::any();
        let _ = core.verify(&AlwaysReject, &digest);
        assert!(core.state() == OtaState::Idle);
        assert!(core.active_slot() == pre_active);
        assert!(core.swap() == Err(OtaError::WrongState));
    }

    /// OTA-P03 — `bytes_received` never exceeds `total_bytes`. Property
    /// holds after any single `write_chunk` call, including nondeterministic
    /// chunk sizes (in or out of range).
    #[kani::proof]
    fn ota_p03_bytes_bounded() {
        let mut core = OtaCore::new(Slot::A);
        let m = any_small_manifest(64);
        let total = m.size_bytes;
        core.start_download(m).unwrap();

        let chunk: u32 = kani::any();
        let _ = core.write_chunk(chunk);
        match core.state() {
            OtaState::Downloading {
                bytes_received,
                total_bytes,
                ..
            } => {
                assert!(bytes_received <= total_bytes);
                assert!(total_bytes == total);
            }
            // If the chunk was rejected we may still be in Downloading
            // with bytes_received = 0, which trivially satisfies the bound.
            _ => {}
        }
    }

    /// OTA-P04 — no panic for any nondeterministic sequence of API calls.
    /// We pick one of the 8 public operations on each of 4 steps and
    /// execute it; Kani must show that none can ever panic.
    ///
    /// The unwind bound is `33`: the outer op loop runs 4 times (needs 5),
    /// but `verify`'s `[u8; 32]` digest comparison lowers to a 32-byte
    /// `memcmp` whose internal loop needs `33` to unwind fully.
    #[kani::proof]
    #[kani::unwind(33)]
    fn ota_p04_no_panic_any_sequence() {
        let initial: bool = kani::any();
        let init_slot = if initial { Slot::A } else { Slot::B };
        let mut core = OtaCore::new(init_slot);

        for _ in 0..4 {
            let op: u8 = kani::any();
            kani::assume(op < 8);
            match op {
                0 => {
                    let m = any_small_manifest(16);
                    let _ = core.start_download(m);
                }
                1 => {
                    let n: u32 = kani::any();
                    kani::assume(n <= 16);
                    let _ = core.write_chunk(n);
                }
                2 => {
                    let _ = core.finish_download();
                }
                3 => {
                    let digest: [u8; 32] = kani::any();
                    let _ = core.verify(&AlwaysAccept, &digest);
                }
                4 => {
                    let digest: [u8; 32] = kani::any();
                    let _ = core.verify(&AlwaysReject, &digest);
                }
                5 => {
                    let _ = core.swap();
                }
                6 => {
                    let _ = core.rollback();
                }
                7 => {
                    let _ = core.confirm_swap();
                }
                _ => unreachable!(),
            }
        }
        // Sanity: structural invariants still hold.
        if let Some(t) = core.target_slot() {
            assert!(t != core.active_slot());
        }
    }

    /// OTA-P05 — image-digest binding. For *any* manifest and *any*
    /// supplied digest, the machine reaches `Ready` **iff** the supplied
    /// digest equals `manifest.sha256` (the verifier here always accepts,
    /// so the digest is the sole gate). The single `write_chunk` of the
    /// full image keeps the proof loop-free.
    #[kani::proof]
    fn ota_p05_ready_requires_matching_digest() {
        let initial: bool = kani::any();
        let init_slot = if initial { Slot::A } else { Slot::B };
        let mut core = OtaCore::new(init_slot);

        // Size bounded to one chunk so the download finishes in one write.
        let m = any_small_manifest(MAX_CHUNK_BYTES);
        let manifest_hash = m.sha256;
        core.start_download(m).unwrap();
        core.write_chunk(m.size_bytes).unwrap();
        core.finish_download().unwrap();

        let supplied: [u8; 32] = kani::any();
        let res = core.verify(&AlwaysAccept, &supplied);

        if supplied == manifest_hash {
            // Matching digest: must reach Ready on the standby slot.
            assert!(res == Ok(()));
            let ready = OtaState::Ready {
                slot: init_slot.other(),
            };
            assert!(core.state() == ready);
        } else {
            // Any mismatch: must NOT be Ready — aborted to Idle.
            assert!(res == Err(OtaError::BadImageDigest));
            assert!(core.state() == OtaState::Idle);
        }
        // The contrapositive, stated directly: Ready implies a digest match.
        if let OtaState::Ready { .. } = core.state() {
            assert!(supplied == manifest_hash);
        }
    }

    /// OTA-P06 — rollback guard. `rollback` reverts the active slot only
    /// when a prior `swap` recorded one; a second `rollback` with no
    /// intervening swap fails cleanly (no panic, no slot change). We also
    /// check the fresh-boot case where no swap ever happened.
    #[kani::proof]
    fn ota_p06_rollback_guarded() {
        let initial: bool = kani::any();
        let init_slot = if initial { Slot::A } else { Slot::B };
        let do_swap_first: bool = kani::any();
        let mut core = OtaCore::new(init_slot);

        // Fresh boot: no swap recorded → rollback must refuse, no flip.
        if !do_swap_first {
            assert!(core.rollback_target().is_none());
            assert!(core.rollback() == Err(OtaError::NothingToRollback));
            assert!(core.active_slot() == init_slot);
            // Idempotent-safe: a second call also fails cleanly.
            assert!(core.rollback() == Err(OtaError::NothingToRollback));
            assert!(core.active_slot() == init_slot);
            return;
        }

        // Perform a full verified update + swap.
        let m = any_small_manifest(MAX_CHUNK_BYTES);
        let digest = m.sha256;
        core.start_download(m).unwrap();
        core.write_chunk(m.size_bytes).unwrap();
        core.finish_download().unwrap();
        core.verify(&AlwaysAccept, &digest).unwrap();
        core.swap().unwrap();
        assert!(core.active_slot() == init_slot.other());
        assert!(core.rollback_target() == Some(init_slot));

        // First rollback: succeeds, reverts to exactly the recorded slot.
        assert!(core.rollback() == Ok(()));
        assert!(core.active_slot() == init_slot);
        assert!(core.rollback_target().is_none());

        // Second rollback, no intervening swap: fails cleanly, no flip.
        assert!(core.rollback() == Err(OtaError::NothingToRollback));
        assert!(core.active_slot() == init_slot);
    }
}
