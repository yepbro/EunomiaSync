//! The IPC commands (spec §7.10) — the recovery-screen group of the
//! System module (D-009), the first registered commands since `greet`
//! (D-037): `app_open_data_folder`, `db_backups_list`,
//! `db_restore_backup`, `db_recreate`.
//!
//! Every command returns `Result<T, AppError>` (§7.10) and is generic
//! over the Tauri runtime — the documented tauri-specta shape for
//! commands that take an [`AppHandle`], which is also what lets the
//! IPC tests drive them over the mock runtime. Two of them
//! (`db_backups_list`, `app_open_data_folder`) are read-only and safe
//! in any mode; the other two are **recovery-mode operations**: they
//! rename the damaged database away and replace it, and they reject a
//! call made while a healthy database is already managed (the §10
//! screen that owns them is shown exactly when it is not).
//!
//! # Lifecycle (the honest boundary this task fixes, DECISIONS.md
//! T-024)
//!
//! The `setup` hook continues past a `E_DB_CORRUPT` open failure
//! instead of aborting: it logs the verdict through the facade and
//! returns `Ok` with **no** `Db` managed. That state — logging up,
//! layout up, no database — is recovery mode. These commands are its
//! backend: paths come from the managed [`crate::data_dirs::DataRoot`]
//! (resolved once at startup, before the open could fail), files are
//! handled by [`crate::recovery`], and after a successful
//! recreate/restore the command itself performs the reopen: `Db::open`
//! over the replaced path, the ERR-001 daily copy, and
//! `app.manage(database)` — the §7.10 wording "replaces it with the
//! chosen copy, and reopens it", so the app continues without a
//! restart.
//!
//! # SYS-001
//!
//! The file work and the reopen run inside
//! `tauri::async_runtime::spawn_blocking` — the same executor policy
//! as `db::Db::run` (D-037): quick_check scans and file copies are
//! blocking I/O and never run on the async workers.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

use crate::data_dirs;
use crate::db::{self, Db};
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::logging;
use crate::recovery::{self, DbBackupInfo};

/// Serializes the two destructive recovery commands against each
/// other: both end in `app.manage(database)`, and tauri's `manage`
/// panics on a duplicate state — two concurrent recreates would race
/// into that panic. A caller that finds the gate locked gets a
/// validation error ("recovery already in progress") instead.
/// The async mutex (not `std`): its guard is `Send`, so it can be held
/// across the `.await` of the blocking work — exactly the window the
/// gate must cover (the inverse of D-037's std-mutex reasoning, which
/// is about guards that must *never* cross an await). The constructor
/// is not `const`, hence the `LazyLock`.
static RECOVERY_OPERATION: std::sync::LazyLock<tauri::async_runtime::Mutex<()>> =
    std::sync::LazyLock::new(|| tauri::async_runtime::Mutex::new(()));

/// Lists the database copies in `db-backups/` for the §10 recovery
/// screen: every copy's base name, size and UTC modification time,
/// newest first. Read-only, safe in any mode (§7.10 System module).
#[tauri::command]
#[specta::specta]
pub(crate) async fn db_backups_list<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<DbBackupInfo>, AppError> {
    let backups_dir =
        data_dirs::db_backups_dir(&data_root(&app).inspect_err(logging::recovery_failed)?);
    let joined = tauri::async_runtime::spawn_blocking(move || recovery::list_backups(&backups_dir))
        .await
        .map_err(db::join_failure_to_app_error)
        .and_then(|listed| listed.map_err(|error| error.app_error()));
    // §10: every error reaches the log with its code — the facade line
    // carries the code literal and the params this error built.
    joined.inspect_err(logging::recovery_failed)
}

/// Replaces the damaged database with the chosen copy and reopens it
/// (§7.10, §10): the copy must be a plain file name inside
/// `db-backups/` and must pass the startup gates (too-new/corrupt)
/// before anything is touched; the damaged database is preserved as
/// `db-backups/pre-recovery-<UTC>.sqlite3` and never deleted; the copy
/// itself is left in place. Recovery-mode only — rejected while a
/// healthy database is managed.
#[tauri::command]
#[specta::specta]
pub(crate) async fn db_restore_backup<R: Runtime>(
    app: AppHandle<R>,
    backup_name: String,
) -> Result<(), AppError> {
    run_recovery(&app, Recovery::Restore { backup_name }).await
}

/// Starts with an empty database (§7.10, §10): the damaged database is
/// preserved as `db-backups/pre-recovery-<UTC>.sqlite3` — renamed, not
/// deleted — and a fresh database is created and reopened in its
/// place. Recovery-mode only — rejected while a healthy database is
/// managed.
#[tauri::command]
#[specta::specta]
pub(crate) async fn db_recreate<R: Runtime>(app: AppHandle<R>) -> Result<(), AppError> {
    run_recovery(&app, Recovery::Recreate).await
}

/// Opens the data directory in the OS file manager (§7.10). The
/// opener plugin runs from Rust only (SEC-001): the webview holds no
/// `opener:` permission, so this command is the single path from the
/// UI to the plugin — exactly what lets the app open its own data
/// directory without granting the frontend a general "open anything"
/// capability.
#[tauri::command]
#[specta::specta]
pub(crate) async fn app_open_data_folder<R: Runtime>(app: AppHandle<R>) -> Result<(), AppError> {
    let data_dir = data_root(&app).inspect_err(logging::data_folder_open_failed)?;
    open_data_folder(&app, &data_dir)
}

/// Which recovery operation `run_recovery` performs — and, for
/// `Restore`, the selection the UI made. Cloned into the blocking
/// closure (`'static`), while the original stays for the success
/// logging below.
#[derive(Clone)]
enum Recovery {
    Recreate,
    Restore { backup_name: String },
}

/// The shared body of `db_recreate` and `db_restore_backup`: guard,
/// blocking file work + reopen, then publication of the reopened
/// database as managed state (the order the `setup` hook uses:
/// daily copy first, manage last — D-046).
async fn run_recovery<R: Runtime>(app: &AppHandle<R>, operation: Recovery) -> Result<(), AppError> {
    // Recovery-mode guard: with a healthy database managed there is
    // nothing to recover, and proceeding would rename a *working*
    // database away — the §10 screen that owns these buttons is not
    // shown in that state, so the call is a UI bug or a stale screen,
    // and a validation rejection is the honest answer.
    if app.try_state::<Db>().is_some() {
        let error = rejected("a database is already running");
        logging::recovery_failed(&error);
        return Err(error);
    }
    // Serialize concurrent calls (see `RECOVERY_OPERATION`); the
    // locked-out caller gets the same shape of honest rejection.
    let _guard = RECOVERY_OPERATION.try_lock().map_err(|_| {
        let error = rejected("a recovery operation is already in progress");
        logging::recovery_failed(&error);
        error
    })?;

    let data_dir = data_root(app).inspect_err(logging::recovery_failed)?;
    let working_dir = data_dir.clone();
    let operation_moved = operation.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        // The step orders are `recovery::recreate_files`/`restore_files`
        // themselves (the seams the behavior tests pin); the restore
        // audit line rides the `on_preserved` callback into the window
        // between the preserve and the publish — after the damaged
        // file has moved, before anything replaced it — so a crash
        // between the two renames still leaves the "where did my old
        // database go?" trail in the log (the facade is usable from a
        // blocking thread; tracing is global).
        let preserved = match &operation_moved {
            Recovery::Recreate => recovery::recreate_files(&working_dir, jiff::Timestamp::now())?,
            Recovery::Restore { backup_name } => recovery::restore_files(
                &working_dir,
                backup_name,
                jiff::Timestamp::now(),
                |preserved| logging::recovery_preserved(&preserved.from, &preserved.to),
            )?,
        };
        if let Recovery::Recreate = &operation_moved
            && let Some(preserved) = &preserved
        {
            // `recreate_files` only preserves — logging here is still
            // before anything else happens to the database path.
            logging::recovery_preserved(&preserved.from, &preserved.to);
        }
        // The reopen (§7.10: "…and reopens it"): the full `Db::open`
        // gate chain runs over the replaced path, so a copy that was
        // damaged between validation and publication is still caught
        // here — the command never manages a database it did not open.
        let database = Db::open(&working_dir).map_err(recovery::RecoveryError::from)?;
        Ok::<_, recovery::RecoveryError>((preserved, database))
    })
    .await;
    // The preserve audit line was already emitted inside the closure,
    // in the window between the preserve and the publish — the
    // documented point (a second one here would read as two preserves
    // and land after the replacement, the timing round 1 rejected).
    let (_preserved, database) = match result {
        Ok(Ok(opened)) => opened,
        Ok(Err(error)) => {
            let error = error.app_error();
            logging::recovery_failed(&error);
            return Err(error);
        }
        Err(join_error) => {
            let error = db::join_failure_to_app_error(join_error);
            logging::recovery_failed(&error);
            return Err(error);
        }
    };
    let db_path = data_dirs::db_file_path(&data_dir);
    match &operation {
        // The audit lines carry both paths: where the damaged database
        // went, and what the database is now (the copy's own path is
        // derivable from the name the UI sent, which the failure paths
        // log through their params).
        Recovery::Recreate => logging::recovery_recreated(&db_path),
        Recovery::Restore { backup_name } => logging::recovery_restored(
            &data_dirs::db_backups_dir(&data_dir).join(backup_name),
            &db_path,
        ),
    }
    // The ERR-001 daily baseline of the recovered database, the same
    // step and degrade-never-abort policy as the `setup` hook
    // (D-046): the recovered database is the day's working state, and
    // the copy that produced it may itself be pruned by later
    // retention — the baseline must not depend on that.
    match database.ensure_daily_backup(jiff::Timestamp::now()) {
        Ok(eunomia_store::storage::DailyBackupOutcome::Written(path)) => {
            logging::db_backup_daily(&path);
        }
        Ok(eunomia_store::storage::DailyBackupOutcome::AlreadyPresent(_)) => {}
        Err(error) => logging::db_backup_daily_failed(&error),
    }
    app.manage(database);
    Ok(())
}

/// Opens `path` in the OS file manager through the opener plugin's
/// Rust API (SEC-001: the call lives in Rust; the webview has no
/// `opener:` permission — `capabilities/default.json` stays empty).
/// Split from the command so the error mapping is testable without
/// spawning a real file-manager window: the opener call itself is the
/// one untestable seam here (it side-effects the desktop), covered by
/// the manual smoke runs and pinned as wiring by a text test.
fn open_data_folder<R: Runtime>(
    app: &AppHandle<R>,
    path: &std::path::Path,
) -> Result<(), AppError> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|error| {
            let app_error = opener_error_to_app_error(&error);
            logging::data_folder_open_failed(&app_error);
            app_error
        })
        .map(|()| logging::data_folder_opened(path))
}

/// Maps an opener-plugin failure to the §7.10 wire form. The §10
/// registry has no "open folder failed" code, so the failure lands on
/// the recorded D-037 fallback (`E_DB_LOCKED`'s "operation rejected,
/// retry" is the least harmful effect — the user retries the button
/// or opens the folder by hand) with the opener's own text in
/// `details`; the mapping is pub(crate) for the seam's tests.
pub(crate) fn opener_error_to_app_error(error: &tauri_plugin_opener::Error) -> AppError {
    let mut app_error = AppError::new(ErrorCode::DbLocked);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("OpenerError".to_owned()),
        os_message: Some(error.to_string()),
    });
    app_error
}

/// The data-directory root as resolved at startup. The state is
/// managed before the database step, so in recovery mode (the state
/// these commands exist for) it is always present; its absence is an
/// uninitialized-app bug, and the honest wire answer is the D-037
/// fallback shape with a diagnostic — not a panic, and not a silent
/// default directory.
fn data_root<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, AppError> {
    app.try_state::<data_dirs::DataRoot>()
        .map(|root| root.0.clone())
        .ok_or_else(|| {
            let mut error = AppError::new(ErrorCode::DbLocked);
            error.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("DataRootMissing".to_owned()),
                os_message: Some("the data directory was not resolved at startup".to_owned()),
            });
            error
        })
}

/// A recovery-command rejection that is a UI-input problem, not a
/// filesystem one: `E_VALIDATION` with a `reason` param (the §10 row's
/// "the whole operation rejected" is exactly right — nothing was
/// touched).
fn rejected(reason: &str) -> AppError {
    let mut error = AppError::new(ErrorCode::Validation);
    error.params.insert("reason".to_owned(), reason.to_owned());
    error
}

#[cfg(test)]
mod tests {
    //! The seam tests of the opener mapping — the mapping is pure, so
    //! it is tested directly; the opener call itself is the recorded
    //! untestable seam (module docs).

    use super::*;
    use crate::errors::ErrorCode;

    #[test]
    fn an_opener_failure_maps_to_the_fallback_code_with_its_text() {
        // Arrange: a real opener error — errno 13 wrapped the way the
        // plugin wraps a failed `Command::spawn`.
        let opener_error = tauri_plugin_opener::Error::Io(std::io::Error::from_raw_os_error(13));

        // Act
        let app_error = opener_error_to_app_error(&opener_error);

        // Assert: the D-037 fallback code, its default key, and the
        // opener's own text in `details` (nothing free-text in params).
        assert_eq!(app_error.code, ErrorCode::DbLocked);
        assert_eq!(app_error.message_key, "errors.db_locked");
        assert!(app_error.params.is_empty());
        let details = app_error.details.expect("the details carry the cause");
        assert_eq!(details.os_kind.as_deref(), Some("OpenerError"));
        assert!(
            details
                .os_message
                .as_deref()
                .is_some_and(|message| message.contains("13")),
            "the OS code travels in the message text, got: {:?}",
            details.os_message
        );
    }
}
