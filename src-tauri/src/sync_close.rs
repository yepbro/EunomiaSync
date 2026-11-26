//! The §9.4 "Closing the window during a sync" machinery (spec §9.4's
//! own paragraph, §9.4 phase 3, §9.5, §7.10's events line and Sync
//! row, §10's pointer; T-123; the contract's resolved readings live
//! in DECISIONS.md D-219).
//!
//! The flow's three halves, each behind its own seam:
//!
//! - **The interception** ([`on_close_requested`]) — the `main`
//!   window's `CloseRequested` event, matched in `lib.rs`'s one
//!   `on_window_event` closure and routed here with the window's
//!   label: with the [`crate::sync_running::SyncRunning`] flag down
//!   the close proceeds UNTOUCHED (§9.4's paragraph begins "during a
//!   sync"); with the flag up the close is PREVENTED (the wiring
//!   calls `api.prevent_close()`) and the
//!   [`SyncCloseConfirmationEvent`] ask is emitted — the dialog is
//!   the unlanded UI wave's half (UI-010's pattern), the event is
//!   this module's contract with it; with the exit already confirmed
//!   ([`SyncCloseState`]'s latch) the close is prevented SILENTLY —
//!   no second ask, the pending exit owns the ending (the process
//!   must survive the wait: the last window's close would end it
//!   mid-wait, defeating exactly what the user confirmed).
//! - **The ask** ([`SyncCloseConfirmationEvent`]) — the spec names
//!   no event (the card: "предложить"); `sync://close-confirmation`
//!   is the proposal, EMPTY payload (`null` on the wire): §9.4,
//!   UI-010 and NFR-021 make the dialog's text static localization
//!   and name no dynamic fact it needs. A refused emission is
//!   display-only degradation ([`logging::
//!   sync_close_confirmation_emit_failed`]): the window stays open
//!   and the user retries the close — the run is unaffected.
//! - **The confirmed exit** ([`sync_confirm_exit`]) — the UI's
//!   answer is a TYPED COMMAND (commands are the only UI→Rust
//!   channel, SEC-001): the first call latches
//!   [`SyncCloseState`], WAITS for the current write through the
//!   managed [`crate::commands::SyncWriteGate`] (the in-flight
//!   counter phase 3's per-project guard holds across the §9.5
//!   write AND its verdict/stamp transaction — the DoD's "the file
//!   stamped/verdict recorded" observable), and then exits through
//!   [`ConfirmExitSeams::exit`] — production
//!   [`AppHandle::exit`](tauri::AppHandle::exit), the event-loop
//!   exit path. A repeat call converges (one exit); a confirm with
//!   no sync running exits immediately (the user's standing intent —
//!   D-219 point 4): nothing is in flight, so the wait is instant.
//!
//! No SYS-005 gate (the command mutates no project data — the
//! `sync_cancel` posture), no database access (the latch and the
//! gate are the only state read, both managed by `setup` in every
//! mode — the `SessionPaths` posture), and no journal (the §7.7
//! action vocabulary has no member for an exit).
//!
//! The mock runtime boundary, recorded honestly (D-219): the mock
//! runtime delivers no real window events and cannot even construct
//! a `CloseRequested` (its `CloseRequestApi` field is private), so
//! the seam is LABEL-driven — the event matching and
//! `prevent_close` stay in the wiring, marker-pinned by source
//! scans, and the tests here pin the seam's decision table plus the
//! emitted event (the D-025/availability posture). The real
//! `AppHandle::exit` is impossible under the mock harness (its
//! `request_exit` is `unimplemented!()`), so the exit is an
//! injected seam the tests record, never a real process exit.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

use crate::commands::SyncWriteGate;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::logging;
use crate::sync_running::SyncRunning;

/// The window whose close §9.4's paragraph is about — `main`, declared
/// in `tauri.conf.json` from app start (UI-001). This module's own
/// twin of `single_instance.rs`'s constant (that module is
/// `cfg(desktop)`; the close machinery must register on every
/// platform): two constants naming the same declaration, each pinned
/// by its own literal-driven tests. While a sync runs every `main`
/// close attempt is prevented, so `main` stays open for the whole
/// run and no other window can become the last one during it — a
/// non-`main` label (future UI-002 windows) is returned untouched.
const MAIN_WINDOW_LABEL: &str = "main";

/// What [`on_close_requested`] decided — the observable seam of the
/// interception (the `availability::WindowEventReaction` shape: the
/// reaction value plus the emitted event are what the tests pin; the
/// mock runtime delivers no real close events). The wiring prevents
/// the close for every member but [`CloseDecision::Proceed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseDecision {
    /// No sync runs and no exit is pending: the close proceeds
    /// UNTOUCHED — §9.4's paragraph begins "during a sync", and with
    /// the flag down the default close is exactly what the user
    /// asked for. Nothing was prevented, nothing emitted.
    Proceed,
    /// A sync holds the flag and no exit is pending: the close was
    /// PREVENTED (the wiring's `api.prevent_close()`) and the
    /// [`SyncCloseConfirmationEvent`] ask was emitted — the UI's
    /// dialog (UI-010's pattern, the unlanded UI wave) answers it by
    /// calling [`sync_confirm_exit`] or cancelling UI-side.
    ConfirmationRequested,
    /// The exit is already pending (the user confirmed): the close
    /// was STILL prevented — the process must survive until the
    /// current write finishes, and the last window's close would end
    /// it mid-wait — but SILENTLY: no second ask, no second dialog
    /// (the pending exit owns the ending; D-219 point 7).
    ExitPending,
}

/// The close-confirmation flow's one-way latch (D-219 point 7): set
/// by the FIRST [`sync_confirm_exit`] call, read by the
/// interception. Once confirmed the app is exiting — the latch never
/// resets, and the state dies with the process (a fresh app starts
/// unlatched, the flag's own posture).
///
/// Managed by `setup` in every mode (recovery included) beside
/// [`SyncRunning`] and the gate, for the same reason they are: the
/// interception must FIND the state before the event loop can serve
/// a close, and in recovery mode no sync can run (phase 0 refuses
/// without a database), so the latch simply stays down there.
#[derive(Debug, Default)]
pub(crate) struct SyncCloseState {
    /// `true` once a confirm was accepted. The swap's Acquire/Release
    /// pair is the flag seam's — no other memory is published through
    /// this latch.
    exit_confirmed: AtomicBool,
}

impl SyncCloseState {
    /// The fresh, unconfirmed state — the state `setup` manages
    /// before anything that could fail. Deliberately not `Default` in
    /// the caller: the constructor names the fresh state in the setup
    /// code.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Takes the latch: `true` iff THIS call is the first confirm
    /// (the caller proceeds to wait and exit), `false` for every
    /// repeat (the converging no-op — one exit, never two; the
    /// idempotence discipline).
    pub(crate) fn confirm_exit(&self) -> bool {
        // The swap's answer is the PREVIOUS state: `false` means this
        // call is the first confirm (the caller proceeds — one exit);
        // `true` means a confirm was already accepted (the converging
        // repeat).
        !self.exit_confirmed.swap(true, Ordering::AcqRel)
    }

    /// Whether an exit is pending — the interception's silent-prevent
    /// arm (a close arriving while the confirmed exit waits for its
    /// write).
    pub(crate) fn exit_pending(&self) -> bool {
        self.exit_confirmed.load(Ordering::Acquire)
    }
}

/// The `sync://close-confirmation` event payload (§7.10's events
/// line, §9.4's "A confirmation is shown"; T-123, D-219 point 2):
/// EMPTY by design — `null` on the wire. §9.4, UI-010 and NFR-021
/// make the dialog's content static localization ("a sync is
/// running; exit after the current write"), and the spec names NO
/// dynamic fact the dialog needs; the UI wave owns the text and may
/// widen the payload with a follow-up if a need is ever shown.
///
/// The event name is the proposal the card asks for (the spec names
/// none): the `sync://` family — the ask exists only because a sync
/// runs — and the name §9.4's own sentence shape ("A confirmation is
/// shown") read as the close-confirmation moment, avoiding tauri's
/// own `ExitRequested` vocabulary. Emitted from Rust only; the
/// webview subscribes through the T-246 listen/unlisten grant
/// (SEC-001 — the generated `emit`/`emitTo` stay capability-denied).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) struct SyncCloseConfirmationEvent;

impl Event for SyncCloseConfirmationEvent {
    /// The proposed §7.10 wire name (D-219 point 2). Tauri permits
    /// exactly the characters it contains (alphanumeric, `-`, `/`,
    /// `:`, `_`).
    const NAME: &'static str = "sync://close-confirmation";
}

/// Routes one `main`-window close request to the §9.4 decision
/// (T-123): the seam `lib.rs`'s `on_window_event` wiring calls after
/// matching `WindowEvent::CloseRequested` — label-driven because the
/// mock runtime cannot construct the event itself (its
/// `CloseRequestApi` field is private, D-219 point 1), so the tests
/// drive this function with label literals and the wiring's match
/// plus `prevent_close` are pinned by source scans.
///
/// The decision table (each row pinned by the seam's suite):
///
/// | label | exit pending | flag | decision |
/// | ----- | ------------ | ---- | -------- |
/// | not `main` | — | — | [`Proceed`](CloseDecision::Proceed) (the close is not the app's) |
/// | `main` | yes | — | [`ExitPending`] (prevented silently — the exit owns the ending) |
/// | `main` | no | down | [`Proceed`] (§9.4's case is "during a sync" — the close is the user's) |
/// | `main` | no | up | [`ConfirmationRequested`] (prevented + the ask emitted) |
///
/// The ask's emission happens HERE (the `recheck_and_emit` shape —
/// the wiring only routes); a refused emission is display-only
/// degradation: logged through
/// [`logging::sync_close_confirmation_emit_failed`], never fatal —
/// the window stays open and the user retries the close, the run is
/// unaffected.
///
/// The state lookups are the panicking `app.state` (the
/// `spawn_recheck` posture): both states are managed by `setup`
/// before the event loop can serve a close, so their absence is an
/// uninitialized-app bug that must be loud, not a silently-ignored
/// close.
pub(crate) fn on_close_requested<R: Runtime>(app: &AppHandle<R>, label: &str) -> CloseDecision {
    if label != MAIN_WINDOW_LABEL {
        return CloseDecision::Proceed;
    }
    if app.state::<SyncCloseState>().exit_pending() {
        return CloseDecision::ExitPending;
    }
    if !app.state::<SyncRunning>().is_running() {
        return CloseDecision::Proceed;
    }
    if let Err(error) = SyncCloseConfirmationEvent.emit(app) {
        logging::sync_close_confirmation_emit_failed(&error);
    }
    CloseDecision::ConfirmationRequested
}

/// The injected exit operation of [`sync_confirm_exit_with`] (the
/// seam struct's alias — the `SyncRunSeams` `WriteOp`/`AbortClock`
/// shape, generic over the runtime).
type ExitOp<R> = std::sync::Arc<dyn Fn(&AppHandle<R>) + Send + Sync>;

/// The injected wait operation of [`sync_confirm_exit_with`] (the
/// same alias family).
type WaitOp = std::sync::Arc<dyn Fn(&SyncWriteGate) + Send + Sync>;

/// The injected operations of [`sync_confirm_exit_with`] — the
/// `SyncRunSeams` object-safety shape, generic over the runtime:
/// production waits on the gate's condvar and exits through
/// [`AppHandle::exit`](tauri::AppHandle::exit); the tests interpose
/// (a notification that the wait has ARRIVED — the rendezvous that
/// proves the confirm reached its wait while the write is still in
/// flight — and a recorder instead of the real exit, which is
/// impossible under the mock harness: its `request_exit` is
/// `unimplemented!()`).
pub(crate) struct ConfirmExitSeams<R: Runtime> {
    /// The exit operation: invoked once, after the wait, with the
    /// app's handle.
    pub(crate) exit: ExitOp<R>,
    /// The wait operation: invoked on the blocking pool with the
    /// managed gate. Production is [`SyncWriteGate::wait_quiet`]
    /// verbatim; a test closure may signal its arrival first (the
    /// D-089 injector pattern — the fault/rendezvous points are the
    /// caller's).
    pub(crate) wait: WaitOp,
}

impl<R: Runtime> ConfirmExitSeams<R> {
    /// The production seams: the gate's own condvar wait, and
    /// [`AppHandle::exit`] with code 0 — the graceful path
    /// (`RunEvent::ExitRequested` → `RunEvent::Exit`, the event
    /// loop's own cleanup, tauri 2.12.6 `app.rs`).
    pub(crate) fn production() -> Self {
        Self {
            exit: std::sync::Arc::new(|app: &AppHandle<R>| app.exit(0)),
            wait: std::sync::Arc::new(|gate: &SyncWriteGate| gate.wait_quiet()),
        }
    }
}

/// Serves the confirmed exit (§7.10 Sync module `sync_confirm_exit`,
/// §9.4's closing paragraph; T-123): the UI's answer to the
/// [`SyncCloseConfirmationEvent`] ask — wait for the current atomic
/// write, then exit.
///
/// The three arms (D-219 points 3–7):
///
/// - **First call** — the latch is taken, the wait runs
///   ([`SyncWriteGate::wait_quiet`] on the blocking pool, SYS-001),
///   and the exit seam fires. The answer is `Ok(())` (the command's
///   contract is one behavior; nothing a consumer would branch on).
/// - **Repeat call** — the latch is already taken: the converging
///   no-op (one exit, never two — the double-click race the
///   idempotence discipline protects).
/// - **No sync running** — the wait is instantly satisfied (nothing
///   is in flight) and the exit proceeds: the confirm answers a
///   dialog that asked "exit after the current write?", and a run
///   ending between the ask and the answer does not cancel the
///   user's exit intent (D-219 point 4 — the standing-intent
///   reading, recorded against the converge alternative).
///
/// The only `Err` is the honest missing-state fallback (the
/// `sync_cancel` shape: the D-037 fallback with a diagnostic naming
/// the state — an uninitialized-app bug, not a panic, and not a
/// silent `Ok` that would hide the wiring gap).
#[tauri::command]
#[specta::specta]
pub(crate) async fn sync_confirm_exit<R: Runtime>(app: AppHandle<R>) -> Result<(), AppError> {
    sync_confirm_exit_with(app, ConfirmExitSeams::production())
        .await
        .inspect_err(logging::sync_confirm_exit_failed)
}

/// The body of [`sync_confirm_exit`], split the `sync_run_with` way:
/// the tests drive the wait-and-exit with a recording exit seam (the
/// real `AppHandle::exit` is impossible under the mock harness).
///
/// The wait is [`SyncWriteGate::wait_quiet`] — the condvar under the
/// gate's mutex, so no wakeup can be missed and NO polling or fixed
/// sleep exists anywhere near it (D-140) — inside
/// `tauri::async_runtime::spawn_blocking` (SYS-001: a blocking wait
/// belongs off the async threads; the wait is bounded by one write's
/// duration, §9.5's own bounded retries). The gate's region is
/// phase 3's whole per-project unit (the §9.5 write AND its
/// verdict/stamp transaction, D-219 point 5), so the exit observed
/// after the wait provably follows the DoD's "the file stamped/
/// verdict recorded" moment.
pub(crate) async fn sync_confirm_exit_with<R: Runtime>(
    app: AppHandle<R>,
    seams: ConfirmExitSeams<R>,
) -> Result<(), AppError> {
    // The missing-state fallback (the `sync_cancel` shape): the state
    // is managed by `setup` before anything that could fail, so its
    // absence is an uninitialized-app bug — the recorded D-037
    // fallback with a diagnostic naming it.
    let Some(close_state) = app.try_state::<SyncCloseState>() else {
        return Err(sync_close_state_missing());
    };
    // The latch: only the FIRST confirm proceeds (a repeat converges
    // — one exit, the double-click race's protection).
    if !close_state.confirm_exit() {
        return Ok(());
    }
    // The wait: the current atomic write's whole per-project unit,
    // through the seam's injected operation on the blocking pool
    // (SYS-001 — the production closure is the gate's condvar wait,
    // which parks the thread until a guard's drop notifies it; no
    // polling, no fixed sleep, D-140). The gate is managed beside
    // the latch in every mode; the panicking lookup is the
    // wiring-bug-must-be-loud posture.
    let gate = app.state::<SyncWriteGate>().inner().clone();
    let wait = std::sync::Arc::clone(&seams.wait);
    tauri::async_runtime::spawn_blocking(move || wait(&gate))
        .await
        .expect("the write-gate wait task must join (the condvar wait is infallible)");
    // The exit — through the seam (production: the event-loop exit
    // path; the tests: the recorder the DoD's ordering assert reads).
    (seams.exit)(&app);
    Ok(())
}

/// The honest missing-state answer of [`sync_confirm_exit`] (the
/// `sync_cancel` shape): the D-037 fallback with the class-only
/// diagnostic naming the state.
fn sync_close_state_missing() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("SyncCloseStateMissing".to_owned()),
        os_message: Some("the sync close state was not managed at startup".to_owned()),
    });
    error
}

#[cfg(test)]
mod tests {
    //! The seam's own battery (the `single_instance.rs`/`availability.rs`
    //! posture): the decision table over the mock app with the states
    //! managed the way `lib.rs`'s setup manages them, the emitted
    //! event observed through the mounted registry, and the latch's
    //! lifecycle. The ipc-level halves (the DoD's parked-write wait,
    //! the registration census, the wiring's source scans) live in
    //! `ipc_tests.rs`, which owns the mock app harness the run
    //! machinery needs.

    use super::*;
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri_specta::collect_events;

    /// A mock app with the close flow's states managed the way
    /// `setup` manages them (the flag, the latch, the write gate),
    /// the ask event mounted the way the wiring's builder mounts it,
    /// and a listener that records every payload the seam emits.
    fn close_app() -> (
        tauri::App<tauri::test::MockRuntime>,
        std::sync::mpsc::Receiver<SyncCloseConfirmationEvent>,
    ) {
        let app = mock_builder()
            .build(mock_context(noop_assets()))
            .expect("failed to build the mock app");
        app.manage(SyncRunning::new());
        app.manage(SyncCloseState::new());
        app.manage(crate::commands::SyncWriteGate::new());
        tauri_specta::Builder::<tauri::test::MockRuntime>::new()
            .events(collect_events![SyncCloseConfirmationEvent])
            .mount_events(&app);
        let (sender, receiver) = std::sync::mpsc::channel();
        SyncCloseConfirmationEvent::listen(&app, move |event| {
            let _ = sender.send(event.payload);
        });
        (app, receiver)
    }

    #[test]
    fn the_event_name_is_the_proposed_wire_literal() {
        // The wire name is the frontend's whole contract (every
        // round-trip test reads the same constant on both sides, so a
        // typo would stay green) — pinned here against the proposed
        // §7.10 spelling recorded in D-219.
        assert_eq!(
            SyncCloseConfirmationEvent::NAME,
            "sync://close-confirmation",
            "the proposed event name (D-219 point 2)"
        );
    }

    #[test]
    fn the_event_payload_serializes_as_the_empty_literal() {
        // The EMPTY payload is the recorded contract (D-219 point 2):
        // `null` on the wire — the dialog needs no dynamic fact. The
        // generated TS type is `null` (specta's unit-struct rule), so
        // the spelling is the frontend's contract.
        let wire = serde_json::to_value(SyncCloseConfirmationEvent)
            .expect("the unit payload must serialize");
        assert_eq!(wire, serde_json::json!(null));
        let back: SyncCloseConfirmationEvent =
            serde_json::from_value(wire).expect("the empty payload must round-trip");
        assert_eq!(back, SyncCloseConfirmationEvent);
    }

    #[test]
    fn a_close_with_no_sync_proceeds_without_asking() {
        // §9.4's paragraph begins "during a sync": with the flag down
        // the close is the user's — nothing prevented, nothing
        // emitted (the wiring's prevent follows the decision, and no
        // ask payload exists for a listener to receive).
        let (app, asks) = close_app();
        let decision = on_close_requested(app.handle(), "main");
        assert_eq!(
            decision,
            CloseDecision::Proceed,
            "no sync, no pending exit — the close proceeds untouched"
        );
        assert!(
            asks.try_recv().is_err(),
            "no confirmation was requested, so no event exists"
        );
        drop(app);
    }

    #[test]
    fn a_close_during_a_sync_is_intercepted_and_the_ask_repeats_after_a_cancel() {
        // The §9.4 case: the flag is up (an externally-held guard, the
        // way the sync-running suite holds it), the close is
        // intercepted — the ask is emitted with the EMPTY payload,
        // and the decision names the prevented-and-asked state. A
        // second close on the SAME state (no confirm yet) asks again:
        // the interception is stateless per event — the user
        // cancelled the dialog and tried again (D-219 point 7's
        // re-ask reading).
        let (app, asks) = close_app();
        let _guard = app
            .state::<SyncRunning>()
            .try_acquire()
            .expect("the flag is free before the run-shaped hold");
        assert_eq!(
            on_close_requested(app.handle(), "main"),
            CloseDecision::ConfirmationRequested,
            "a sync runs — the close is prevented and the ask emitted"
        );
        assert_eq!(
            asks.recv_timeout(std::time::Duration::from_secs(30))
                .expect("the ask must be delivered"),
            SyncCloseConfirmationEvent,
            "the ask carries the empty payload"
        );
        // Cancel-then-retry: nothing latched, the ask repeats.
        assert_eq!(
            on_close_requested(app.handle(), "main"),
            CloseDecision::ConfirmationRequested
        );
        assert!(
            asks.recv_timeout(std::time::Duration::from_secs(30))
                .is_ok(),
            "the retry asks again — the interception is stateless per event"
        );
        drop(_guard);
        drop(app);
    }

    #[test]
    fn a_non_main_window_close_is_not_the_apps() {
        // The label discrimination (the D-025 both-directions pin):
        // a non-`main` label is not the app's close — even with the
        // flag up it proceeds untouched (future UI-002 windows; while
        // a sync runs `main` is pinned open, so another window can
        // never be the last one during a run).
        let (app, asks) = close_app();
        let _guard = app
            .state::<SyncRunning>()
            .try_acquire()
            .expect("the flag is free before the run-shaped hold");
        assert_eq!(
            on_close_requested(app.handle(), "preview"),
            CloseDecision::Proceed,
            "a non-main window's close is not §9.4's case"
        );
        assert!(
            asks.try_recv().is_err(),
            "no ask was made for another window"
        );
        drop(_guard);
        drop(app);
    }

    #[test]
    fn a_close_after_a_confirmed_exit_is_prevented_silently() {
        // The pending-exit arm (D-219 point 7): once a confirm was
        // accepted, a close arriving while the exit waits is STILL
        // prevented (the last window's close would end the process
        // mid-wait) but SILENTLY — no second ask, no second dialog.
        let (app, asks) = close_app();
        let state = app.state::<SyncCloseState>();
        assert!(
            state.confirm_exit(),
            "the first confirm takes the latch (the command's own act)"
        );
        assert_eq!(
            on_close_requested(app.handle(), "main"),
            CloseDecision::ExitPending,
            "the exit owns the ending — the close is absorbed silently"
        );
        assert!(
            asks.try_recv().is_err(),
            "no second ask exists for the pending exit"
        );
        assert!(
            !state.confirm_exit(),
            "the repeat confirm converges (one exit, never two)"
        );
        drop(app);
    }

    #[test]
    fn the_latch_is_fresh_per_app() {
        // The latch dies with the process (the flag's own posture):
        // a fresh app starts unlatched — a stale latch from a prior
        // session would silently swallow every close of the next one.
        let (app, _asks) = close_app();
        assert!(
            !app.state::<SyncCloseState>().exit_pending(),
            "a fresh app has no pending exit"
        );
        drop(app);
    }
}
