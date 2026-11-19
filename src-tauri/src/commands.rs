//! The IPC commands (spec §7.10) — the recovery-screen group of the
//! System module (D-009) plus the History module's `activity_list`
//! (T-027): `app_open_data_folder`, `db_backups_list`,
//! `db_restore_backup`, `db_recreate`, `activity_list`.
//!
//! Every command returns `Result<T, AppError>` (§7.10) and is generic
//! over the Tauri runtime — the documented tauri-specta shape for
//! commands that take an [`AppHandle`], which is also what lets the
//! IPC tests drive them over the mock runtime. Two of them
//! (`db_backups_list`, `app_open_data_folder`) are read-only and safe
//! in any mode; `db_restore_backup` and `db_recreate` are
//! **recovery-mode operations**: they rename the damaged database away
//! and replace it, and they reject a call made while a healthy
//! database is already managed (the §10 screen that owns them is
//! shown exactly when it is not). `activity_list` is the "Activity"
//! window's query (§7.7): a read through the [`Db::run`] executor
//! (D-037) over the journal the action-log repository owns (D-051),
//! with LOG-005's visibility rule enforced at this boundary — user
//! entries by default, `diagnostic` only when explicitly requested.
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

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};

use crate::data_dirs;
use crate::db::{self, Db};
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::logging;
use crate::recovery::{self, DbBackupInfo};

use eunomia_store::action_log::{Action, ActionLogError, ActivityQuery, Outcome, Visibility};
use eunomia_store::storage::StorageError;

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

/// The page size `activity_list` uses when the caller sends none: a
/// desktop window's worth of entries, matching what the "Activity"
/// UI renders in one pass.
const DEFAULT_PAGE_LIMIT: i32 = 50;

/// The largest page `activity_list` serves: a bound on a single IPC
/// payload, not on the window (which paginates with `offset`). The
/// journal itself is capped by LOG-006's retention, so this is a
/// response-size guard, not a data-set guard.
const MAX_PAGE_LIMIT: i32 = 500;

/// The request DTO of `activity_list` (§7.10 History module, §7.7 UI
/// row): every field optional, every field a filter, all of them
/// combining with AND. Field names are the wire contract (snake_case,
/// the house convention of `DbBackupInfo`/`AppError`).
///
/// The closed vocabularies travel as strings in their stored spellings
/// (`"project.added"`, `"error"`) and are parsed at this boundary
/// through the store's own member parsers — one owner per vocabulary,
/// and an unknown value is `E_VALIDATION`, never a silent match-all.
/// Unknown keys are rejected (`deny_unknown_fields`): a typo'd filter
/// name must fail loudly, not silently widen the query to everything.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActivityListQuery {
    /// Inclusive lower date bound, DB-002 UTC shape
    /// (`YYYY-MM-DDTHH:MM:SS.sssZ`).
    pub(crate) from_ts: Option<String>,
    /// Inclusive upper date bound, DB-002 UTC shape.
    pub(crate) to_ts: Option<String>,
    /// The exact project id to filter by.
    pub(crate) project_id: Option<String>,
    /// The actions to filter by (the §7.7 "type" filter): the entry's
    /// action must be one of these. `None` or empty means every
    /// action.
    pub(crate) actions: Option<Vec<String>>,
    /// The result to filter by: `"ok"`, `"error"` or `"skipped"`.
    pub(crate) result: Option<String>,
    /// LOG-005: `false`/absent keeps the response to `user` entries —
    /// the "Activity" window's data; `true` includes `diagnostic`
    /// entries for the diagnostics section. Diagnostic entries are
    /// **never** returned without this explicit request.
    pub(crate) include_diagnostic: Option<bool>,
    /// The page size (1..=MAX_PAGE_LIMIT; defaults to 50).
    pub(crate) limit: Option<i32>,
    /// How many matching entries to skip (>= 0; defaults to 0).
    pub(crate) offset: Option<i32>,
}

/// One journal entry on the `activity_list` wire: every `action_log`
/// column, with the closed vocabularies in their stored spellings
/// (the backend writes them; the UI switches on them). `details` is
/// the stored JSON as text — SEC-008's typed payload re-serialized in
/// its canonical form; parsing it stays the frontend's choice (the
/// `modified_utc` precedent of `DbBackupInfo`).
///
/// `id` and `total` are `u32`: specta forbids 64-bit integers on the
/// TS wire (the `size_bytes` decision, D-049), and an autoincrement id
/// or a row count past 2^32 is outside this app's envelope — the
/// saturating cast is the honest boundary, recorded with this task's
/// DECISIONS.md entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ActivityEntry {
    /// The autoincrement id (§6.2), saturated at `u32::MAX`.
    pub(crate) id: u32,
    /// The entry's time, DB-002 UTC ISO-8601.
    pub(crate) ts: String,
    /// The severity: `info`, `warn` or `error`.
    pub(crate) level: String,
    /// Who the entry is for (LOG-005): `user` or `diagnostic`.
    pub(crate) visibility: String,
    /// What happened, the `object.verb` spelling (`project.added`).
    pub(crate) action: String,
    /// The entity kind, when the entry names one.
    pub(crate) entity_type: Option<String>,
    /// The entity id, when the entry names one.
    pub(crate) entity_id: Option<String>,
    /// The project involved, when the entry names one.
    pub(crate) project_id: Option<String>,
    /// The sync run involved, when the entry names one.
    pub(crate) sync_run_id: Option<String>,
    /// The outcome: `ok`, `error` or `skipped`.
    pub(crate) result: String,
    /// The `details` JSON as canonical text, when the entry has any.
    pub(crate) details: Option<String>,
}

/// The `activity_list` response: one page plus the pagination meta —
/// `total` counts what the **filters** match (independent of
/// limit/offset), so the UI can render "page N of M" without a second
/// round trip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ActivityPage {
    /// The page's entries, newest first (`ts` descending, id
    /// descending within one millisecond).
    pub(crate) entries: Vec<ActivityEntry>,
    /// How many entries the filters match in total (saturated at
    /// `u32::MAX`).
    pub(crate) total: u32,
}

impl ActivityPage {
    /// Shapes a store [`ActivityPage`](eunomia_store::action_log::ActivityPage)
    /// into its wire form.
    fn from_store(page: eunomia_store::action_log::ActivityPage) -> Self {
        ActivityPage {
            entries: page
                .records
                .into_iter()
                .map(|record| ActivityEntry {
                    id: u32::try_from(record.id).unwrap_or(u32::MAX),
                    ts: record.ts,
                    level: record.level.as_str().to_owned(),
                    visibility: record.visibility.as_str().to_owned(),
                    action: record.action.as_str().to_owned(),
                    entity_type: record
                        .entity
                        .as_ref()
                        .map(|(kind, _)| kind.as_str().to_owned()),
                    entity_id: record.entity.as_ref().map(|(_, id)| id.clone()),
                    project_id: record.project_id,
                    sync_run_id: record.sync_run_id,
                    result: record.result.as_str().to_owned(),
                    details: record.details.as_ref().map(|details| {
                        serde_json::to_string(details)
                            .expect("a serde_json::Value always serializes")
                    }),
                })
                .collect(),
            total: u32::try_from(page.total).unwrap_or(u32::MAX),
        }
    }
}

/// Lists the journal for the "Activity" window and the diagnostics
/// section (§7.10 History module `activity_list`; §7.7 UI row with
/// filters by date, project, type and result; LOG-005).
///
/// The query runs through the [`Db::run`] executor like every database
/// command (D-037): one transaction off the async runtime, busy
/// retries included (D-050). Visibility is LOG-005's rule applied at
/// this boundary: the response carries `user` entries only, unless
/// `include_diagnostic` explicitly opts the diagnostics section in —
/// a `diagnostic` entry is never returned by accident, whatever the
/// other filters narrow to.
///
/// Failures reach the log with their §10 code
/// ([`logging::activity_list_failed`]); a successful read is never
/// journaled (writing the journal to read it would feed itself) and
/// gets no routine log line (nothing happened worth a file).
#[tauri::command]
#[specta::specta]
pub(crate) async fn activity_list<R: Runtime>(
    app: AppHandle<R>,
    query: ActivityListQuery,
) -> Result<ActivityPage, AppError> {
    // Split so EVERY failure path — input validation, the missing
    // database, and the executor — flows through one logging edge:
    // §10's every-error rule must not have silent branches (the round-1
    // review finding; the recovery commands log their rejections the
    // same way).
    activity_list_impl(app, query)
        .await
        .inspect_err(logging::activity_list_failed)
}

/// The body of [`activity_list`], split from the command wrapper the
/// same edge logs every failure of (see there).
async fn activity_list_impl<R: Runtime>(
    app: AppHandle<R>,
    query: ActivityListQuery,
) -> Result<ActivityPage, AppError> {
    let validated = validated_activity_query(query)?;
    let database = app
        .try_state::<Db>()
        .map(|state| state.inner().clone())
        .ok_or_else(|| {
            let mut error = AppError::new(ErrorCode::DbLocked);
            error.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("NoDatabase".to_owned()),
                os_message: Some("no database is running (recovery mode?)".to_owned()),
            });
            error
        })?;
    database
        .run(move |transaction| {
            eunomia_store::action_log::list(transaction, &validated)
                .map(ActivityPage::from_store)
                .map_err(activity_log_to_app_error)
        })
        .await
}

/// Validates an [`ActivityListQuery`] into the store's
/// [`ActivityQuery`] — the IPC boundary's parse of untrusted input
/// (the principle-boundary step): every closed vocabulary and date
/// bound is checked here, and every rejection is `E_VALIDATION`
/// naming the field, the value and the reason, with nothing executed.
fn validated_activity_query(query: ActivityListQuery) -> Result<ActivityQuery, AppError> {
    let limit = query.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
        return Err(rejected_field(
            "limit",
            &limit.to_string(),
            &format!("must be between 1 and {MAX_PAGE_LIMIT}"),
        ));
    }
    let offset = query.offset.unwrap_or(0);
    if offset < 0 {
        return Err(rejected_field(
            "offset",
            &offset.to_string(),
            "must be 0 or greater",
        ));
    }
    // The same shape check the store applies (one owner:
    // `values::is_db002_shape`); doing it here gives the rejection
    // named-field params, doing it there keeps the store's contract
    // honest for Rust-side callers — the rule itself is not
    // duplicated.
    for (field, bound) in [("from_ts", &query.from_ts), ("to_ts", &query.to_ts)] {
        if let Some(found) = bound
            && !eunomia_store::values::is_db002_shape(found)
        {
            return Err(rejected_field(
                field,
                found,
                "must be a DB-002 UTC timestamp (YYYY-MM-DDTHH:MM:SS.sssZ)",
            ));
        }
    }
    if let (Some(from), Some(to)) = (&query.from_ts, &query.to_ts)
        && from > to
    {
        return Err(rejected_field(
            "from_ts",
            from,
            "the range is inverted: from_ts is later than to_ts",
        ));
    }
    // The closed vocabularies parse through the store's own member
    // parsers — an unknown wire value is a validation error naming it,
    // never a silent match-all. Duplicates are dropped on the way in:
    // the parsed set is bounded by the vocabulary's own size (21
    // members), which is what bounds the generated `action IN (…)`
    // placeholder list regardless of how long the wire array was — an
    // unbounded list could otherwise push the statement past SQLite's
    // variable limit and surface a frontend bug as `E_DB_LOCKED` (the
    // round-1 review finding).
    let mut actions = Vec::new();
    for action in query.actions.unwrap_or_default() {
        let parsed = Action::from_member(&action)
            .map_err(|_| rejected_field("actions", &action, "unknown action"))?;
        if !actions.contains(&parsed) {
            actions.push(parsed);
        }
    }
    let result = query
        .result
        .as_deref()
        .map(Outcome::from_member)
        .transpose()
        .map_err(|_| {
            rejected_field(
                "result",
                query.result.as_deref().unwrap_or_default(),
                "must be ok, error or skipped",
            )
        })?;
    // LOG-005 at the boundary: user entries only, unless the caller
    // explicitly opts the diagnostics section in.
    let visibility = if query.include_diagnostic == Some(true) {
        None
    } else {
        Some(Visibility::User)
    };
    Ok(ActivityQuery {
        from_ts: query.from_ts,
        to_ts: query.to_ts,
        project_id: query.project_id,
        actions,
        result,
        visibility,
        limit: u32::try_from(limit).expect("bounded to 1..=MAX_PAGE_LIMIT above"),
        offset: u32::try_from(offset).expect("rejected when negative above"),
    })
}

/// An `activity_list` input rejection: `E_VALIDATION` with the field,
/// the offending value and the reason as params (UI-011's "where" for
/// a rejected filter; the default `errors.validation` key, the same
/// shape the recovery commands' rejections use).
fn rejected_field(field: &str, value: &str, reason: &str) -> AppError {
    let mut error = AppError::new(ErrorCode::Validation);
    error.params.insert("field".to_owned(), field.to_owned());
    error.params.insert("value".to_owned(), value.to_owned());
    error.params.insert("reason".to_owned(), reason.to_owned());
    error
}

/// Maps a store failure of the `activity_list` read onto the §7.10
/// wire, by what the failure is: query-bound failures (`Timestamp`/
/// `Range` — the defense-in-depth twin of the command's own
/// validation) are `E_VALIDATION` (they name the caller's input);
/// corrupt journal cells (`Member`/`Json`/`TsCell`) take the recorded
/// D-037 fallback with an `ActionLogCorrupt` diagnostic — see the
/// arms for why neither `E_VALIDATION` nor `E_DB_CORRUPT` fits, and
/// [`corrupt_cell_diagnostic`] for why the stored value stays out of
/// the wire; SQLite failures go through the shared base classifier;
/// anything else (unreachable from a read) keeps the same fallback
/// shape. `pub(crate)` in the `sqlite_error_to_app_error` shape: the
/// IPC tests' seeding closures map their store errors through it too
/// (the exact mapping the command's own closure uses), and the unit
/// tests drive its arms directly.
pub(crate) fn activity_log_to_app_error(error: StorageError) -> AppError {
    match error {
        StorageError::ActionLog { source } => match source {
            // The caller's bounds, not the database: the honest
            // `E_VALIDATION` (normally pre-empted by the command's own
            // validation; this is the store-contract twin).
            ActionLogError::Timestamp { field, found } => {
                let mut app_error = AppError::new(ErrorCode::Validation);
                app_error
                    .params
                    .insert("field".to_owned(), field.to_owned());
                app_error.params.insert("value".to_owned(), found);
                app_error.params.insert(
                    "reason".to_owned(),
                    "must be a DB-002 UTC timestamp (YYYY-MM-DDTHH:MM:SS.sssZ)".to_owned(),
                );
                app_error
            }
            ActionLogError::Range { from, to } => {
                let mut app_error = AppError::new(ErrorCode::Validation);
                app_error.params.insert(
                    "reason".to_owned(),
                    format!("inverted date range: {from} is later than {to}"),
                );
                app_error
            }
            // A stored cell outside its contract — an unknown
            // vocabulary member, a non-JSON details cell, a `ts`
            // outside the DB-002 shape: corruption, not caller input.
            // `E_VALIDATION` would tell the user to fix their input —
            // a lie about the database (the round-1 review finding) —
            // and `E_DB_CORRUPT` is the other lie the D-037 reasoning
            // already rejected (one bad cell must not force the
            // destructive recovery screen), so this is the recorded
            // fallback with its own diagnostic label. The raw cell
            // deliberately does not travel — not in `params` (they
            // feed the localized message) and not in `os_message`
            // (they feed the log line): a tampered cell is arbitrary
            // database text, and SEC-008's surfaces are exactly those
            // two. The diagnostic names the column and the failure
            // class only; whoever debugs a tampered database reads the
            // cell from the database.
            corrupt => {
                let mut app_error = AppError::new(ErrorCode::DbLocked);
                app_error.details = Some(ErrorDetails {
                    os_code: None,
                    os_kind: Some("ActionLogCorrupt".to_owned()),
                    os_message: Some(corrupt_cell_diagnostic(&corrupt)),
                });
                app_error
            }
        },
        StorageError::Sql(source) => db::sqlite_error_to_app_error(source),
        other => {
            let mut app_error = AppError::new(ErrorCode::DbLocked);
            app_error.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("StorageError".to_owned()),
                os_message: Some(other.to_string()),
            });
            app_error
        }
    }
}

/// The fixed-text diagnostic of a corrupt journal cell for
/// [`activity_log_to_app_error`]'s fallback arm: the failing column
/// and the failure class, **never the stored value** — the variant's
/// own `Display` embeds the raw cell, and a tampered cell is arbitrary
/// database text that must not reach the log line or the wire
/// (SEC-008; the round-2 review finding).
fn corrupt_cell_diagnostic(corrupt: &ActionLogError) -> String {
    match corrupt {
        ActionLogError::Member { column, .. } => {
            format!("a stored {column} cell is outside its vocabulary")
        }
        ActionLogError::Json(_) => "a stored details cell is not valid JSON".to_owned(),
        ActionLogError::TsCell { .. } => {
            "a stored ts cell is not the DB-002 timestamp shape".to_owned()
        }
        other => format!("a stored action_log cell failed validation: {other}"),
    }
}

#[cfg(test)]
mod tests {
    //! The seam tests of the pure mappings — the opener mapping and
    //! the `activity_list` store-error mapping are pure, so they are
    //! tested directly; the opener call itself is the recorded
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

    #[test]
    fn a_query_bound_failure_maps_to_validation_naming_the_field() {
        // Arrange: the store's own rejection of a non-DB-002 bound —
        // the defense-in-depth twin of the command's validation.
        let stored = StorageError::ActionLog {
            source: ActionLogError::Timestamp {
                field: "from_ts",
                found: "2026-11-10".to_owned(),
            },
        };

        // Act
        let app_error = activity_log_to_app_error(stored);

        // Assert: the honest E_VALIDATION — a bound is caller input —
        // with the field/value/reason params (the same shape the
        // command's own validation builds).
        assert_eq!(app_error.code, ErrorCode::Validation);
        assert_eq!(app_error.message_key, "errors.validation");
        assert_eq!(
            app_error.params.get("field").map(String::as_str),
            Some("from_ts")
        );
        assert_eq!(
            app_error.params.get("value").map(String::as_str),
            Some("2026-11-10")
        );
        assert!(app_error.params.contains_key("reason"));
        assert!(app_error.details.is_none());
    }

    #[test]
    fn a_corrupt_journal_cell_maps_to_the_fallback_with_its_own_label() {
        // Arrange: the store's corrupt-cells rejection — a stored
        // action outside the closed vocabulary (the D-039 policy).
        let stored = StorageError::ActionLog {
            source: ActionLogError::Member {
                column: "action",
                found: "project.exploded".to_owned(),
            },
        };

        // Act
        let app_error = activity_log_to_app_error(stored);

        // Assert: the D-037 fallback with the `ActionLogCorrupt`
        // diagnostic — NOT E_VALIDATION (the §10 row would tell the
        // user to fix input for database corruption) and NOT
        // E_DB_CORRUPT (one bad cell must not force the recovery
        // screen). No raw cell value travels anywhere the log line or
        // the localized message can reach: `params` stays empty and
        // `os_message` is the fixed column-only diagnostic (a tampered
        // cell is arbitrary database text — SEC-008's surfaces are
        // exactly params and os_message; the round-2 review finding).
        assert_eq!(app_error.code, ErrorCode::DbLocked);
        assert_eq!(app_error.message_key, "errors.db_locked");
        assert!(app_error.params.is_empty(), "no raw cell travels in params");
        let details = app_error.details.expect("the details carry the cause");
        assert_eq!(details.os_kind.as_deref(), Some("ActionLogCorrupt"));
        assert_eq!(
            details.os_message.as_deref(),
            Some("a stored action cell is outside its vocabulary"),
            "the diagnostic names the column and class only"
        );
    }

    #[test]
    fn a_shapeless_stored_ts_cell_maps_to_the_fallback_not_validation() {
        // Arrange: the read path's rejection of a stored `ts` cell
        // outside the DB-002 shape — stored data, NOT the caller's
        // bounds (the round-2 review finding: routing it through the
        // bound variant would misreport corruption as bad input and
        // leak the raw cell into params).
        let stored = StorageError::ActionLog {
            source: ActionLogError::TsCell {
                found: "2026-11-10T00:00:00Z".to_owned(),
            },
        };

        // Act
        let app_error = activity_log_to_app_error(stored);

        // Assert: the corrupt-cells fallback, params empty, the fixed
        // ts diagnostic — the bound twin of the same shape rule keeps
        // E_VALIDATION (see `a_query_bound_failure_maps_to_validation_
        // naming_the_field`).
        assert_eq!(app_error.code, ErrorCode::DbLocked);
        assert!(app_error.params.is_empty());
        let details = app_error.details.expect("the details carry the cause");
        assert_eq!(details.os_kind.as_deref(), Some("ActionLogCorrupt"));
        assert_eq!(
            details.os_message.as_deref(),
            Some("a stored ts cell is not the DB-002 timestamp shape")
        );
    }
}
