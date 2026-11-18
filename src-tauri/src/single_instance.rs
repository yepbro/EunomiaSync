//! Single-instance handoff (SYS-006): what the first instance does when a
//! second launch of the app is detected.
//!
//! The `single-instance` plugin owns the inter-process mechanism itself: a
//! named mutex (Windows), a D-Bus name (Linux) or a Unix-domain socket
//! (macOS) makes the second process exit before it reaches any app setup,
//! after handing its arguments and working directory to the callback in
//! `lib.rs`. That mechanism is a process-level OS facility and cannot be
//! exercised in a unit-test environment — it is covered by manual smoke
//! runs and the CI build matrix. What *is* unit-tested is the handler the
//! callback calls: [`focus_main_window`] resolves the main window by its
//! label and handles a missing window without panicking.
//!
//! Why show + unminimize + set_focus, in that order: on every desktop
//! platform tao's `set_focus` is a no-op for a hidden or minimized window
//! (tao 0.37.2 gates the call on visibility and the not-minimized state —
//! `platform_impl/windows/window.rs`, `platform_impl/linux/window.rs`,
//! `platform_impl/macos/window.rs`), so the window must be restored before
//! focusing. On macOS `set_focus` additionally activates the whole
//! application (`makeKeyAndOrderFront` plus `activateIgnoringOtherApps`),
//! which is exactly the SYS-006 behavior: the window of the first instance
//! is brought to the front.
//!
//! Two platform nuances are accepted rather than engineered around: on
//! Linux the window manager reports the deiconified state asynchronously,
//! so a `set_focus` sent immediately after `unminimize` can be dropped by
//! tao if the minimized flag has not cleared yet (most window managers
//! activate a window the app itself deiconifies; the manual Linux smoke
//! run covers the minimized case), and an `Ok` from the sequence is not a
//! guarantee the OS applied focus — tao silently drops focus requests it
//! considers inapplicable. The handler logs what was attempted and what
//! failed, not what the window manager eventually did.
//!
//! Diagnostics go through the logging facade (`logging.rs`) into the
//! tracing file log as structured events — the handler cannot panic on
//! the diagnostics path: tracing events never panic (a writer failure
//! drops the event), which keeps the no-abort guarantee this module had
//! with its old lossy stderr writer. That guarantee still matters: on
//! Windows the callback runs in an `extern "system"` window procedure,
//! where an unwinding panic aborts the whole process. The second
//! instance's argv and cwd are never logged: they can carry user paths,
//! and the handoff event needs no content until a feature consumes them
//! (deep links are out of v1, SEC-004).

use tauri::{AppHandle, Manager, Runtime};

use crate::logging::{self, RestoreStep};

/// The window a second launch brings back to the front. It exists from app
/// start (`tauri.conf.json` `app > windows[0]`, UI-001); the tests pin the
/// lookup by label, not by array position, so the window array may grow.
const MAIN_WINDOW_LABEL: &str = "main";

/// What [`focus_main_window`] did. Part of the handler's interface so the
/// tests can observe which branch ran: the mock runtime makes the window
/// operations no-ops that always succeed, so the outcome value is the only
/// observable seam it offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusOutcome {
    /// The `main` window existed and the restore sequence
    /// (show → unminimize → set_focus) ran against it.
    Focused,
    /// No window labelled `main` exists, so there is nothing to bring to
    /// the front — SYS-006 only ever targets that window. With tauri's
    /// defaults the process exits when its last window closes, so this
    /// state arises on shutdown races or, with a future tray icon, in a
    /// windowless running instance.
    NoMainWindow,
}

/// Brings the first instance's main window back in front of the user
/// (SYS-006): shows it if hidden, unminimizes it if minimized, and focuses
/// it, which raises it above other windows.
///
/// The restore steps run unconditionally: each is harmless when the window
/// is already in the requested state (Windows sets an already-set flag,
/// Linux re-runs `show_all`/deiconify, macOS re-raises through
/// `makeKeyAndOrderFront`), and the order matters because `set_focus`
/// alone would do nothing for a hidden or minimized window (see the
/// module docs).
///
/// Individual window operations are logged and skipped on error instead of
/// propagated: this runs inside the single-instance callback after the
/// user launched the app again, and a failed window-state change must not
/// crash the running first instance.
pub(crate) fn focus_main_window<R: Runtime>(app: &AppHandle<R>) -> FocusOutcome {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        logging::single_instance_no_main_window();
        return FocusOutcome::NoMainWindow;
    };

    let mut failed_steps = 0;
    if let Err(error) = window.show() {
        failed_steps += 1;
        logging::single_instance_step_failed(RestoreStep::Show, &error);
    }
    if let Err(error) = window.unminimize() {
        failed_steps += 1;
        logging::single_instance_step_failed(RestoreStep::Unminimize, &error);
    }
    if let Err(error) = window.set_focus() {
        failed_steps += 1;
        logging::single_instance_step_failed(RestoreStep::Focus, &error);
    }
    logging::single_instance_handed_off(failed_steps);
    FocusOutcome::Focused
}

#[cfg(test)]
mod tests {
    //! Tests run on the tauri mock runtime (the `tauri` `test` feature —
    //! the same harness the IPC tests use). The mock window dispatchers
    //! make `show`/`unminimize`/`set_focus` no-ops that always succeed and
    //! the state probes always report a visible, restored window, so the
    //! OS-level focus result is deliberately not what these tests pin:
    //! they pin the observable seam the handler itself owns — that the
    //! `main` label is what selects the window (expressed through the
    //! returned outcome, in both directions) and that a missing window is
    //! handled without panicking. The inter-process single-instance
    //! mechanism (instance lock, second process exiting, callback
    //! dispatch) and the real focus effects are process/OS-level and
    //! covered by manual smoke runs and the CI build matrix on all three
    //! platforms.

    use super::*;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    /// Builds a mock app carrying the `main` window, as the real app has
    /// it from `tauri.conf.json` from startup. The label is the literal
    /// from that declaration — deliberately not `MAIN_WINDOW_LABEL`, so a
    /// mutated lookup constant cannot satisfy its own test.
    fn mock_app_with_main_window() -> tauri::App<tauri::test::MockRuntime> {
        let app = mock_builder()
            .build(mock_context(noop_assets()))
            .expect("failed to build the mock app");
        tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("failed to create the mock `main` window");
        app
    }

    #[test]
    fn focuses_the_main_window_when_it_exists() {
        // Arrange: an app with the `main` window (the real app always has
        // one, declared in tauri.conf.json).
        let app = mock_app_with_main_window();

        // Act & Assert: the handler must resolve the window by its label
        // and run the restore sequence. A `NoMainWindow` outcome here
        // means the lookup regressed (wrong label or getter).
        assert_eq!(focus_main_window(app.handle()), FocusOutcome::Focused);
    }

    #[test]
    fn a_non_main_window_is_not_focused() {
        // Arrange: an app whose only window carries a non-main label, as
        // the planned UI-002 windows will. Any label other than `main`
        // must not be selected — the handler targets the label, never
        // "the first window that exists".
        let app = mock_builder()
            .build(mock_context(noop_assets()))
            .expect("failed to build the mock app");
        tauri::WebviewWindowBuilder::new(&app, "preview", Default::default())
            .build()
            .expect("failed to create the mock `preview` window");

        // Act & Assert: a lookup that picks any existing window passes
        // the "main window exists" test above, so this is the direction
        // that pins label selection.
        assert_eq!(focus_main_window(app.handle()), FocusOutcome::NoMainWindow);
    }

    #[test]
    fn missing_main_window_is_reported_instead_of_panicking() {
        // Arrange: an app with no `main` window. With tauri's defaults
        // closing the last window ends the process, so this state mostly
        // arises during shutdown races — or, with a future tray icon, in
        // a windowless running instance; either way the handler must
        // cope.
        let app = mock_builder()
            .build(mock_context(noop_assets()))
            .expect("failed to build the mock app");

        // Act & Assert: a second launch arriving in that state must not
        // crash the first instance just because its window is gone.
        assert_eq!(focus_main_window(app.handle()), FocusOutcome::NoMainWindow);
    }
}
