//! The Sync module's `sync_cancel` command (spec §7.10's Sync row,
//! SYS-003, §9.4 phase 2 step 3, §10; T-120; the contract's resolved
//! readings live in DECISIONS.md D-210).
//!
//! The command IS the escape hatch for a running sync: it latches the
//! live run's published [`CancelToken`] through the managed
//! [`SyncCancelSlot`] — the pinned T-284 path, `token().cancel()`,
//! unchanged from D-138/D-147 — and answers over the
//! point-of-no-return KNOWLEDGE the slot carries beside the token.
//! There is exactly ONE cancellation channel (the latched
//! `Arc<AtomicBool>` the pipeline consults at its one check point,
//! D-138's law); this command adds no second path to the token, no
//! second consultation and no second latch. What it adds is the
//! command surface: the wire answer, the registration, and the mock
//! twin.
//!
//! No SYS-005 gate (SYS-005's list is the project-*changing*
//! commands; a cancel mutates nothing) and no database access at
//! all: the slot is the only state read — managed by `setup` in
//! every mode (D-147), empty in recovery mode, where the honest
//! answer is `nothingToCancel` by construction. A run that already
//! answered cleared the slot with its lease; a cancel arriving then
//! converges the same way (the `scan_cancel` mirror: the
//! stale-click race must not error).

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};

use crate::commands::SyncCancelSlot;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::logging;

/// The `sync_cancel` command's wire answer (§7.10 Sync module, §9.4
/// phase 2 step 3, D-210): one of three states, serde-tagged the
/// [`crate::commands::SyncRunOutcome`] way (`{"outcome": …}` — no
/// member carries a payload: the run's own answer, `sync_run`'s
/// summary, holds every fact a cancel's consumer needs).
///
/// A state answer, never an `Err` (D-138's answer-shape discipline
/// mirrored: the cancel stop answers the `aborted` summary, not an
/// `Err`, because the user asked for it — a declined or empty cancel
/// is the same class of state). §10's vocabulary has no member for a
/// declined cancel (`E_SYNC_RUNNING` names a second run or a project
/// edit), and inventing one is an owner-scope decision recorded as
/// declined in D-210: the spec's own words ("the run cannot be
/// canceled", §9.4 phase 2 step 3) ride the `tooLate` tag instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub(crate) enum SyncCancelOutcome {
    /// A run is live and had not passed the point of no return when
    /// the slot was read: the request was latched onto the run's own
    /// token, and the run's single consultation — still ahead — will
    /// stop it before the backups (§9.4: a latched token stops the
    /// run there, answering the `aborted` summary). ADVISORY, and
    /// documented honestly — the answer can name `cancelled` for a
    /// run that does not stop at its consultation: (a) in the
    /// nanosecond window where the consultation is mid-flight (its
    /// read done, the point-of-no-return mark not yet), the latch
    /// lands after the read and the run proceeds — the no-op the
    /// uncancellable tail defines; (b) a pre-check stop — the D-25
    /// gate's `NeedsConfirmation`, an availability-checkpoint or
    /// phase-1 `Err` — ends the run before the consultation, and the
    /// latch is never honoured (the confirmed re-call is a full new
    /// run with a new token); (c) a run boundary between this
    /// command's slot read and its knowledge read can leave the
    /// answer describing a run other than the one latched. The run's
    /// OWN answer, `sync_run`'s summary, is the authoritative
    /// outcome in all of these; this member names the state the
    /// command acted on.
    Cancelled,
    /// The run already passed §9.4 phase 2 step 3's point of no
    /// return: the request was still latched (one behavior — the
    /// harmless no-op a token nobody consults again defines), and
    /// the run finishes on its own; its summary is the outcome.
    /// CERTAIN: the mark is latched only after the consultation
    /// completed, so a mark read after this command's latch proves
    /// the consultation read the pre-latch state.
    TooLate,
    /// No run is live: the converging no-op (the `scan_cancel`
    /// mirror — the window-close and stale-click races must not
    /// error in the loser branch). Nothing was latched, nothing
    /// written.
    NothingToCancel,
}

/// Serves the sync cancel request (§7.10 Sync module `sync_cancel`,
/// SYS-003): latches the live run's published token — cancelling the
/// clone IS cancelling the run's (one `Arc`, D-147's pinned path) —
/// and answers one of three states over the slot's
/// point-of-no-return knowledge.
///
/// The three arms:
///
/// - [`SyncCancelOutcome::Cancelled`] — a run is live and had not
///   passed §9.4 phase 2 step 3's point of no return when the slot
///   was read: the request is latched, the run's single consultation
///   is still ahead, and it will stop the run before the backups
///   (answering the `aborted` summary, D-138's law — the run's own
///   answer is the authoritative outcome).
/// - [`SyncCancelOutcome::TooLate`] — the run already passed the
///   point of no return: the latch is still performed (one behavior —
///   "cancel through the slot IS the existing check"), and it is the
///   harmless no-op §9.4 phase 2 step 3 defines for a token nobody
///   consults again; the run finishes on its own.
/// - [`SyncCancelOutcome::NothingToCancel`] — no run is live: the
///   converging no-op (the `scan_cancel` mirror — the window-close
///   and stale-click races must not error in the loser branch).
///
/// A state answer, never an `Err` (D-138's answer-shape discipline:
/// a declined or empty cancel is a state, not a system failure —
/// §10's vocabulary names no code for it). The only `Err` is the
/// honest missing-state fallback of the managed slot itself (an
/// uninitialized-app bug, the `scan_cancel` shape).
#[tauri::command]
#[specta::specta]
pub(crate) async fn sync_cancel<R: Runtime>(
    app: AppHandle<R>,
) -> Result<SyncCancelOutcome, AppError> {
    sync_cancel_impl(app)
        .await
        .inspect_err(logging::sync_cancel_failed)
}

/// The body of [`sync_cancel`], split so the one logging edge covers
/// every failure path (the `sync_run` shape) — which here means the
/// missing-state fallback alone: the command owns no database access,
/// so no store refusal can reach the wire through it.
async fn sync_cancel_impl<R: Runtime>(app: AppHandle<R>) -> Result<SyncCancelOutcome, AppError> {
    // The missing-state fallback (the `managed_session` shape): the
    // state is managed by `setup` before anything that could fail,
    // so its absence is an uninitialized-app bug — the recorded
    // D-037 fallback with a diagnostic naming it, not a panic, and
    // not a silent Ok that would hide the wiring gap.
    let Some(slot) = app.try_state::<SyncCancelSlot>() else {
        return Err(sync_cancel_slot_missing());
    };
    // The pinned path (T-284): the token the slot publishes — the
    // same latched `Arc` the pipeline consults at its ONE check
    // point. With no run live there is nothing to latch: the
    // converging no-op answers first.
    let Some(token) = slot.token() else {
        return Ok(SyncCancelOutcome::NothingToCancel);
    };
    // THE latch — one behavior, unconditional (D-147's law: a
    // post-point latch is the harmless no-op §9.4 defines; the
    // knowledge below shapes only the ANSWER, never the act).
    token.cancel();
    // The KNOWLEDGE read, after the latch (D-210 point 5): the
    // certainty split is recorded on the DTO. `tooLate` is certain —
    // the latch set means the consultation completed and read the
    // pre-latch state; `cancelled` is advisory — the run's own
    // answer is the authority.
    Ok(if slot.is_past_point_of_no_return() {
        SyncCancelOutcome::TooLate
    } else {
        SyncCancelOutcome::Cancelled
    })
}

/// The honest missing-state answer of [`sync_cancel`] (the
/// `scan_cancel` shape): the D-037 fallback with the class-only
/// diagnostic naming the state.
fn sync_cancel_slot_missing() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("SyncCancelSlotMissing".to_owned()),
        os_message: Some("the sync cancel slot was not managed at startup".to_owned()),
    });
    error
}
