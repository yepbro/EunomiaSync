//! The IPC commands (spec §7.10) — the recovery-screen group of the
//! System module (D-009), the History module's `activity_list`
//! (T-027), the Settings module's `settings_get`/`settings_set`
//! (T-030), the System module's `app_info` (T-031), the Library
//! module's `markdown_preview` (T-056), the Projects module's
//! `projects_list` (T-072), the System module's
//! `dialog_pick_folder` (T-073), the Projects module's
//! `project_add_manual` (T-074), the Projects module's
//! `exclusions_list`/`exclusion_remove` (T-078) and the Projects
//! module's `projects_reorder` (T-080):
//! `app_open_data_folder`, `db_backups_list`, `db_restore_backup`,
//! `db_recreate`, `activity_list`, `settings_get`, `settings_set`,
//! `app_info`, `markdown_preview`, `projects_list`,
//! `dialog_pick_folder`, `project_add_manual`, `exclusions_list`,
//! `exclusion_remove`, `projects_reorder`.
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
//! The settings commands (§6.5, §7.2 DEF-001…005) are the first
//! journaling producers: `settings_set` writes its `settings.changed`
//! entries (LOG-001…004) inside the operation's own transaction
//! (D-051), and both commands validate through the store's single
//! settings parse (D-039) — every rejection is `E_VALIDATION` naming
//! the key, the sent value and the reason. `app_info` is the About
//! window's fact sheet (UPD-005/REL-004 plus the §5.5 hints, and the
//! NFR-023 root-launch flag): a non-database command that answers
//! from compile-time constants, the process's effective uid and
//! managed state alone, in any mode — see its section below.
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

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};

use crate::data_dirs;
use crate::db::{self, Db};
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::logging;
use crate::markdown;
use crate::recovery::{self, DbBackupInfo};
use crate::session_paths;

use eunomia_core::project::{Color, ProjectId};
use eunomia_fs::paths::NotARepoError;
use eunomia_fs::write_access::WriteAccessError;
use eunomia_store::action_log::{
    Action, ActionLogError, ActivityQuery, Details, Entity, EntityType, Level, LogEntry, Outcome,
    Visibility, insert,
};
use eunomia_store::projects::{ListScope, ProjectState, ProjectsError};
use eunomia_store::settings::{SettingKey, SettingValue, SettingsError};
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

/// The managed [`Db`] every database command needs, with the honest
/// no-database answer (recovery mode): the recorded D-037 fallback
/// with a diagnostic naming the state — not a panic, and not a
/// pretend-empty result (an empty page, default settings) that would
/// lie about the journal/the settings table being empty.
/// `pub(crate)`: the SEC-003 check (`session_paths::validate_ui_path`)
/// shares the same honest answer for its repository half.
pub(crate) fn managed_db<R: Runtime>(app: &AppHandle<R>) -> Result<Db, AppError> {
    app.try_state::<Db>()
        .map(|state| state.inner().clone())
        .ok_or_else(|| {
            let mut error = AppError::new(ErrorCode::DbLocked);
            error.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("NoDatabase".to_owned()),
                os_message: Some("no database is running (recovery mode?)".to_owned()),
            });
            error
        })
}

/// The body of [`activity_list`], split from the command wrapper the
/// same edge logs every failure of (see there).
async fn activity_list_impl<R: Runtime>(
    app: AppHandle<R>,
    query: ActivityListQuery,
) -> Result<ActivityPage, AppError> {
    let validated = validated_activity_query(query)?;
    managed_db(&app)?
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
/// shape the recovery commands' rejections use). `pub(crate)`: the
/// SEC-003 check (`session_paths::rejected_ui_path`) builds its
/// rejection through this one builder — a single copy of the wire
/// shape (the `managed_db` reuse pattern).
pub(crate) fn rejected_field(field: &str, value: &str, reason: &str) -> AppError {
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

// ---------------------------------------------------------------------------
// Settings (T-030, §7.10 Settings module, §6.5, §7.2 DEF-001…005,
// LOG-001…004)
// ---------------------------------------------------------------------------

/// One setting on the `settings_get`/`settings_set` wire: the §6.5
/// dotted key plus the value as canonical JSON text (`"C"`, `10`,
/// `{"enabled":true,"position":"end"}`, `["node_modules","vendor"]`).
///
/// The wire stays string-shaped on purpose — the D-052 `details`
/// decision applied to the thirteen §6.5 keys: keys are parsed at this
/// boundary through the store's `SettingKey::from_key` and values
/// through `SettingValue::from_json` (D-039's single validation path),
/// so the store remains the one owner of the key list and the value
/// shapes, and no specta enum mirrors them on this side (a second
/// vocabulary to keep in sync for no behavioral gain; the
/// `specta/serde_json` feature stays disabled). Parsing the JSON text
/// stays the frontend's choice, exactly like a `details` cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct SettingItem {
    /// The §6.5 dotted key (`scan.max_depth`).
    pub(crate) key: String,
    /// The value as canonical JSON text — the exact form stored and
    /// re-read (§6.5: values live in the `value` column as JSON).
    pub(crate) value: String,
}

/// One entry of the `settings_set` changes list. `deny_unknown_fields`
/// (the [`ActivityListQuery`] rule): a typo'd field name fails the
/// deserialization loudly instead of silently sending a value-less
/// key.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingChange {
    /// The §6.5 dotted key to change.
    pub(crate) key: String,
    /// The new value as canonical JSON text (the form
    /// [`SettingItem::value`] carries).
    pub(crate) value: String,
}

/// Shapes one store key/value pair into its wire form.
fn setting_item((key, value): (SettingKey, SettingValue)) -> SettingItem {
    SettingItem {
        key: key.as_str().to_owned(),
        value: serde_json::to_string(&value.to_json())
            .expect("a serde_json::Value always serializes"),
    }
}

/// Reads every §6.5 setting with its default applied (spec §7.10
/// Settings module `settings_get`; the §7.2 window opens on this map).
///
/// The response is a **list in §6.5 table order** — the order the
/// store's `get_all` pins — not a JSON object: an object would impose
/// its own key order (sorted or arbitrary) on the Settings window's
/// sections. The query runs through [`Db::run`] like every database
/// command (D-037), so a value committed by a later `settings_set` is
/// what the next `settings_get` returns — the §7.2 "applies without a
/// restart" mechanism: nothing between the screen and the table caches
/// settings (the future scanner reads through this same repository
/// path, T-012/D-032's honest-boundary adaptation).
///
/// A successful read is never journaled and gets no routine log line
/// (the `activity_list` rationale: reading is not an event); failures
/// reach the log with their §10 code
/// ([`logging::settings_get_failed`]).
#[tauri::command]
#[specta::specta]
pub(crate) async fn settings_get<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<SettingItem>, AppError> {
    settings_get_impl(app)
        .await
        .inspect_err(logging::settings_get_failed)
}

/// The body of [`settings_get`], split so one logging edge covers every
/// failure path (the `activity_list` shape).
async fn settings_get_impl<R: Runtime>(app: AppHandle<R>) -> Result<Vec<SettingItem>, AppError> {
    managed_db(&app)?
        .run(|transaction| {
            eunomia_store::settings::get_all(transaction)
                .map(|all| all.into_iter().map(setting_item).collect::<Vec<_>>())
                .map_err(settings_cell_to_app_error)
        })
        .await
}

/// Applies a partial settings update atomically (spec §7.10 Settings
/// module `settings_set`; §6.9: "the whole operation runs in one
/// transaction"): the changes list carries **only the changed keys**
/// — the Settings screen's dirty set — and either every key applies or
/// none does. One key per call was rejected: the screen's Save would
/// be N transactions, and a failure mid-save would leave the screen
/// half applied. The response is the fresh full map (the same list
/// [`settings_get`] returns, read inside the same transaction), so the
/// saving screen gets the applied state without a second round trip.
///
/// Validation is the repository's, reached from the boundary (D-039's
/// single path): every rejection is `E_VALIDATION` naming the key, the
/// sent value and the reason, and nothing is executed for a list with
/// any invalid entry. Each changed key writes one `settings.changed`
/// journal entry (LOG-001…004) **inside the operation's transaction**
/// (D-051): a rolled-back save leaves no entry claiming it happened —
/// the entry names the key through its entity (the store's designated
/// carrier for "which setting"), and no value travels (keys are
/// identifiers, not content — SEC-008's structural line; the count is
/// the number of entries).
///
/// Failures reach the log with their §10 code
/// ([`logging::settings_set_failed`]); the journal entries themselves
/// are the success record — a successful save adds no routine file-log
/// line (the info+user+ok mirror policy, D-051).
#[tauri::command]
#[specta::specta]
pub(crate) async fn settings_set<R: Runtime>(
    app: AppHandle<R>,
    changes: Vec<SettingChange>,
) -> Result<Vec<SettingItem>, AppError> {
    settings_set_impl(app, changes)
        .await
        .inspect_err(logging::settings_set_failed)
}

/// The body of [`settings_set`], split so one logging edge covers every
/// failure path (see there).
async fn settings_set_impl<R: Runtime>(
    app: AppHandle<R>,
    changes: Vec<SettingChange>,
) -> Result<Vec<SettingItem>, AppError> {
    let validated = validated_settings_changes(changes)?;
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            for (key, value) in &validated {
                eunomia_store::settings::set(transaction, *key, value.clone())
                    .map_err(settings_write_to_app_error)?;
                // LOG-001…004 ("changing settings"), inside the
                // operation's own transaction (D-051): one entry per
                // changed key, its entity id the dotted key — the
                // store's own designated id shape for a settings
                // entry, which carries *which* setting changed; the
                // closed `Details` enum (D-051) has no keys-list
                // variant, and a value would be content, not an
                // identifier — so no details. The store is frozen for
                // this task's window (T-029 owns it), and per-key
                // entries are bounded by the vocabulary itself
                // (≤ 13), which is why no `Bulk` count is needed.
                let mut entry = LogEntry::new(
                    Action::SettingsChanged,
                    Level::Info,
                    Visibility::User,
                    Outcome::Ok,
                );
                entry.entity = Some(Entity {
                    kind: EntityType::Setting,
                    id: key.as_str(),
                });
                insert(transaction, &entry).map_err(activity_log_to_app_error)?;
                // The journal call maps through the action-log
                // mapper, not the settings one: the two error
                // families are not interchangeable, and the
                // action-log mapper keeps a hypothetical `ActionLog`
                // failure out of the settings fallback's
                // `other.to_string()` (whose `Display` can embed
                // stored text — SEC-008's os_message surface).
                // The D-051 mirror contract: one call per written
                // entry — a no-op for this info+user+ok shape (the
                // journal already holds it; `mirror_level` decides),
                // and the single place a future level/visibility
                // change would pick up its mirroring.
                logging::action_logged(&entry);
            }
            // The applied state, read in the same transaction: the
            // answer describes exactly what committed.
            eunomia_store::settings::get_all(transaction)
                .map(|all| all.into_iter().map(setting_item).collect::<Vec<_>>())
                .map_err(settings_cell_to_app_error)
        })
        .await
}

/// Validates the wire changes into typed pairs — the boundary parse of
/// untrusted input, all of it through the store's own single
/// validation path (D-039): an unknown key, a duplicate key, value
/// text that is not JSON, and every §6.5 type/range/member/shape
/// violation are `E_VALIDATION` naming the key, the sent value and the
/// reason, with nothing executed (the store's own `set` re-validation
/// is the defense-in-depth twin — see
/// [`settings_write_to_app_error`]). A valid list is bounded by the
/// vocabulary: at most thirteen distinct keys, so nothing a caller
/// sends can make the write loop unbounded.
fn validated_settings_changes(
    changes: Vec<SettingChange>,
) -> Result<Vec<(SettingKey, SettingValue)>, AppError> {
    if changes.is_empty() {
        // The loud choice (the D-039/D-052 explicit-over-silent
        // policy): an empty save that "succeeds" would hide a frontend
        // bug that computes the dirty set as always-empty — the
        // screen must call only when something changed.
        return Err(rejected_setting(
            "changes",
            "",
            "at least one change is required",
        ));
    }
    let mut validated: Vec<(SettingKey, SettingValue)> = Vec::with_capacity(changes.len());
    for change in &changes {
        let Some(key) = SettingKey::from_key(&change.key) else {
            return Err(rejected_setting(
                &change.key,
                &change.value,
                "unknown setting key (spec 6.5)",
            ));
        };
        if validated.iter().any(|(seen, _)| *seen == key) {
            // A JSON object would collapse duplicates silently
            // (last-wins); the list shape makes them visible, and
            // visible means rejected.
            return Err(rejected_setting(
                &change.key,
                &change.value,
                "the same key appears more than once",
            ));
        }
        let json: serde_json::Value = serde_json::from_str(&change.value).map_err(|error| {
            rejected_setting(
                &change.key,
                &change.value,
                &format!("the value is not valid JSON ({error})"),
            )
        })?;
        let value = SettingValue::from_json(key, &json)
            .map_err(|source| rejected_setting(key.as_str(), &change.value, &source.to_string()))?;
        validated.push((key, value));
    }
    Ok(validated)
}

/// A `settings_set` input rejection: `E_VALIDATION` with the setting
/// key, the sent value and the reason as params — the T-027
/// field/value/reason shape, with the key as the failed "field" (for a
/// settings change the "where" of UI-011 *is* the setting key; the
/// value is the caller's own input echoed back, never stored data).
fn rejected_setting(key: &str, value: &str, reason: &str) -> AppError {
    let mut error = AppError::new(ErrorCode::Validation);
    error.params.insert("key".to_owned(), key.to_owned());
    error.params.insert("value".to_owned(), value.to_owned());
    error.params.insert("reason".to_owned(), reason.to_owned());
    error
}

/// Maps a store failure of the `settings_set` **write** path onto the
/// §7.10 wire. A `Settings` rejection here is the store-contract twin
/// of the command's own validation (normally pre-empted: `set`
/// validates the same value through the same parse before writing —
/// see [`validated_settings_changes`]), so it stays `E_VALIDATION`;
/// the unreachable twin carries `key`+`reason` only — the store's
/// error does not hold the sent value, and the pre-empting path is
/// the one that adds it (a frontend message interpolating `value`
/// never sees this shape). SQLite failures go through the shared base
/// classifier (a constraint abort — the rollback path's staged
/// failure — is `E_VALIDATION` with the SQLite facts in `details`).
/// `pub(crate)` in the `activity_log_to_app_error` shape: the IPC
/// tests map their store errors through it too, and the unit tests
/// drive its arms directly.
pub(crate) fn settings_write_to_app_error(error: StorageError) -> AppError {
    match error {
        StorageError::Settings { key, source } => {
            let mut app_error = AppError::new(ErrorCode::Validation);
            app_error.params.insert("key".to_owned(), key);
            app_error
                .params
                .insert("reason".to_owned(), source.to_string());
            app_error
        }
        StorageError::Sql(source) => db::sqlite_error_to_app_error(source),
        other => storage_fallback(other),
    }
}

/// Maps a store failure of a settings **read** (`settings_get`, and
/// the `get_all` tail of `settings_set`): a `Settings` error on a read
/// is a stored cell outside its contract — corruption, not caller
/// input. `E_VALIDATION` would tell the user to fix their input (the
/// lie the T-027 review rejected for journal cells) and `E_DB_CORRUPT`
/// is the other lie D-037 already rejected (one bad cell must not
/// force the destructive recovery screen), so this is the recorded
/// fallback with its own diagnostic label — class-only text, never the
/// stored value or key (the T-027 round-2 rule: a tampered cell is
/// arbitrary database text, and `params`/`os_message` are exactly the
/// surfaces the log line and the localized message read).
/// `pub(crate)` for the same reason as
/// [`settings_write_to_app_error`].
pub(crate) fn settings_cell_to_app_error(error: StorageError) -> AppError {
    match error {
        StorageError::Settings { source, .. } => {
            let mut app_error = AppError::new(ErrorCode::DbLocked);
            app_error.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("SettingsCorrupt".to_owned()),
                os_message: Some(settings_corrupt_diagnostic(&source)),
            });
            app_error
        }
        StorageError::Sql(source) => db::sqlite_error_to_app_error(source),
        other => storage_fallback(other),
    }
}

/// The shared unreachable-fallback arm of the two settings mappers
/// (the `activity_log_to_app_error` `other` arm, same label): the
/// settings calls only produce `Settings`/`Sql` failures today, but
/// the store's error enum is wider, and the honest wire answer for an
/// unreachable variant is the recorded D-037 fallback with the family
/// name (the log target and the producing command distinguish the
/// reader from the writer).
fn storage_fallback(other: StorageError) -> AppError {
    let mut app_error = AppError::new(ErrorCode::DbLocked);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("StorageError".to_owned()),
        os_message: Some(other.to_string()),
    });
    app_error
}

/// The fixed-text diagnostic of a corrupt settings cell for
/// [`settings_cell_to_app_error`]: the failure class only, **never the
/// stored value or key** — `SettingsError`'s own `Display` embeds both
/// (the rejected number, the offending member, the problem string),
/// and a tampered cell is arbitrary database text (the
/// [`corrupt_cell_diagnostic`] rule, applied to the settings table).
fn settings_corrupt_diagnostic(source: &SettingsError) -> String {
    match source {
        SettingsError::Json(_) => "a stored settings value cell is not valid JSON".to_owned(),
        SettingsError::Type { .. } => {
            "a stored settings value cell is the wrong JSON type".to_owned()
        }
        SettingsError::Range { .. } => {
            "a stored settings value cell is outside its key's range".to_owned()
        }
        SettingsError::Member { .. } => {
            "a stored settings value cell is outside its key's vocabulary".to_owned()
        }
        SettingsError::Shape { .. } => {
            "a stored settings value cell has the wrong object shape".to_owned()
        }
    }
}

// ---------------------------------------------------------------------------
// `projects_list` (T-072, §7.10 Projects module, §7.1, §7.4 MAIN-028,
// DB-014/DB-021, D-09/DB-032, D-084)
// ---------------------------------------------------------------------------

/// One project column entry on the `projects_list` wire (§7.4 MAIN-028,
/// §7.1): the facts the matrix column renders plus the identity the
/// column's actions (archive, reorder, "Locate…") call back with, and
/// the state the "Archived projects" screen filters on. The DB types
/// never cross the boundary (the `ActivityEntry` rule): every field is
/// a wire primitive, with the closed vocabularies in their stored
/// spellings the way the settings/action-log DTOs carry them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ProjectColumnEntry {
    /// The row id (UUIDv7 TEXT, DB-002) — the identity every later
    /// project command (`project_archive`, `projects_reorder`, …)
    /// addresses.
    pub(crate) id: String,
    /// The display name (§6.9) — DB-041/NFR-020: the label is always
    /// shown and the name travels with it for the column's tooltip.
    pub(crate) name: String,
    /// The color-badge label, 1–6 visible characters (DB-042).
    pub(crate) label: String,
    /// The palette color (DB-040) as its stored member string.
    pub(crate) color: String,
    /// `active` or `archived` (§6.2 CHECK vocabulary): the default
    /// response carries actives only, so this is `"active"` there;
    /// an `include_archived` read uses it to tell the screens apart.
    pub(crate) state: String,
    /// Whether the project folder passed the D-084 availability
    /// check at this read (D-09/MAIN-028: the column's icon-and-text
    /// marking for a moved, deleted or unreadable folder).
    pub(crate) available: bool,
    /// Why [`ProjectColumnEntry::available`] is `false`, as the wire's
    /// own snake_case names of the D-084 verdicts (`missing`,
    /// `not_a_directory`, `not_readable`, `no_git_entry`) — the text
    /// half of D-09's marking; `None` when the folder is available.
    pub(crate) unavailable_reason: Option<String>,
    /// DB-021's unsynced-changes flag at this read (MAIN-028's dot
    /// icon); see [`dirty_verdict`] for the boundary this command
    /// serves it under.
    pub(crate) dirty: bool,
    /// The last successful sync's time (LOG-007/OUT-020), DB-002 UTC
    /// shape; `None` until the first sync writes it.
    pub(crate) last_synced_at: Option<String>,
}

/// Lists the projects for the main window's columns and the "Archived
/// projects" screen (§7.10 Projects module `projects_list`; §7.1; the
/// §7.4 MAIN-028 column fields).
///
/// Active projects come back in `sort_order` order (ALG-001's read
/// order — the column order PRJ-019/020 persists); `include_archived:
/// true` widens the same ordered read to every row: §6.4's archive
/// keeps a row's position, so archived entries appear at their old
/// places and carry `state: "archived"` for the screen to filter on
/// (the D-10 restore screen). The read runs through [`Db::run`] like
/// every database command (D-037), one transaction off the async
/// runtime.
///
/// Each entry's availability is probed fresh (DB-032: the window-open
/// check point) with the D-084 four-metadata-probe check — read-only
/// reuse of `eunomia_fs::paths::check_project_available`, the crate
/// untouched. The probes are blocking file I/O over live folders, so
/// they run on the blocking pool **after** the transaction (SYS-001):
/// a slow or dead folder never holds the database lock, and an
/// unavailable folder is the entry's flag + reason — D-09's
/// icon-and-text marking, never an error of the read (the sync
/// re-checks before writing, and "Locate…"/"Archive" stay available
/// actions on the entry's id).
///
/// `dirty` is served under the honest boundary [`dirty_verdict`]
/// documents: DB-014's archived half is the one part of DB-021 the
/// §6.2 row decides alone, and the live flag's owner is DB-022's
/// background hash pipeline (`state://dirty`, MAIN-028), which has
/// not landed — the DTO field is that pipeline's arrival point.
///
/// Failures reach the log with their §10 code
/// ([`logging::projects_list_failed`], the D-096 follow-up this edge
/// closed); a successful read is never journaled (the `activity_list`
/// rationale: reading is not an event).
#[tauri::command]
#[specta::specta]
pub(crate) async fn projects_list<R: Runtime>(
    app: AppHandle<R>,
    include_archived: Option<bool>,
) -> Result<Vec<ProjectColumnEntry>, AppError> {
    projects_list_impl(app, include_archived)
        .await
        .inspect_err(logging::projects_list_failed)
}

/// The body of [`projects_list`], split so the one logging edge
/// covers every failure path (the `activity_list` shape): the missing
/// database, the ordered read, and the blocking-pool join.
async fn projects_list_impl<R: Runtime>(
    app: AppHandle<R>,
    include_archived: Option<bool>,
) -> Result<Vec<ProjectColumnEntry>, AppError> {
    let database = managed_db(&app)?;
    // One scope, one ordered read (ALG-001's subset order): the
    // default keeps the matrix's active columns; the explicit flag
    // widens to the whole table for the archived screen.
    let scope = if include_archived == Some(true) {
        ListScope::All
    } else {
        ListScope::Active
    };
    let rows = database
        .run(move |transaction| {
            eunomia_store::projects::list(transaction, scope).map_err(projects_store_to_app_error)
        })
        .await?;
    // The availability probes (D-084) are blocking file I/O over live
    // folders: they run on the blocking pool, after the transaction —
    // a dead drive delays only this response, never the one database
    // lock, and the entries describe exactly what the read committed.
    let entries = tauri::async_runtime::spawn_blocking(move || {
        rows.into_iter()
            .map(|project| {
                // The verdict reads the row before the struct build
                // moves its fields.
                let dirty = dirty_verdict(&project);
                let (available, unavailable_reason) = availability_of(Path::new(&project.path));
                ProjectColumnEntry {
                    id: project.id,
                    name: project.name,
                    label: project.label,
                    color: project.color,
                    state: project.state.as_str().to_owned(),
                    available,
                    unavailable_reason,
                    dirty,
                    last_synced_at: project.last_synced_at,
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(db::join_failure_to_app_error)?;
    Ok(entries)
}

/// The `dirty` verdict `projects_list` serves today (DB-021 with
/// DB-014, MAIN-028) — an honest boundary, not a computation:
///
/// DB-021's rule compares the stored `last_synced_hash` with the
/// **current** build hash, and DB-022 assigns the current hashes to
/// the background recalculation pipeline that emits `state://dirty`
/// (MAIN-028's live source for the column's dot icon) — a task that
/// has not landed, and whose inputs (the library and assignment
/// repositories, the ALG-001 snapshot the build hashes) do not exist
/// in the store yet. The §6.2 row alone decides exactly one half of
/// the rule: DB-014's archived project is never counted as dirty.
///
/// For active projects, no database this app can produce holds
/// unsynced changes today: nothing that writes assignments, library
/// content or sync hashes exists yet (`last_synced_hash` starts NULL
/// with no writer, and no assignment can exist), so DB-021's
/// exception sentence shields every reachable row and the honest
/// verdict is `false`. The boundary is temporal, not structural — the
/// DB-022 pipeline task (or the first assignments command) takes the
/// fact over by feeding [`eunomia_core::dirty::is_dirty`] the real
/// build facts here; a silent constant past that point would be a
/// lie this seam exists to prevent.
fn dirty_verdict(project: &eunomia_store::projects::Project) -> bool {
    match project.state {
        // DB-014: an archived project takes part in no sync and is
        // never counted as dirty — the rule's only row-decidable half.
        ProjectState::Archived => false,
        // Active: the DB-022 boundary above — false for every row any
        // current writer can produce, and the documented seam the
        // hash-pipeline task replaces.
        ProjectState::Active => false,
    }
}

/// Runs the D-084 availability probe and shapes its verdict onto the
/// wire pair: the flag, plus the reason string when the folder failed
/// (D-09/MAIN-028 mark an unavailable project with "an icon and
/// text" — the text needs the failure class). The spellings are the
/// wire's own snake_case names of the probe's verdicts, not stored
/// database cells — single-sourced in [`crate::availability::Verdict::probe`]
/// (T-079) so the column this command serves and the
/// `state://project-availability` event can never drift apart.
fn availability_of(root: &Path) -> (bool, Option<String>) {
    let verdict = crate::availability::Verdict::probe(root);
    (verdict.available, verdict.unavailable_reason)
}

/// Maps a store failure of the `projects_list` read onto the §7.10
/// wire — the D-093 boundary: this is the first command consuming the
/// projects repository, and it maps [`ProjectsError`] beside the
/// `StorageError` shapes the commands above already unwrap (folding
/// the two store error families together stays a recorded follow-up
/// for a store-side task allowed to edit `src-tauri`). SQLite
/// failures go through the shared base classifier; everything else is
/// the recorded D-037 fallback — `E_DB_CORRUPT` remains the lie
/// D-037 rejected (one bad cell must not force the destructive
/// recovery screen). `pub(crate)` in the `activity_log_to_app_error`
/// shape: the IPC tests map their store errors through it too, and
/// the unit tests drive its arms directly.
pub(crate) fn projects_store_to_app_error(error: ProjectsError) -> AppError {
    match error {
        ProjectsError::Sql(source) => db::sqlite_error_to_app_error(source),
        // A stored cell outside its closed vocabulary: corruption, not
        // caller input — the fallback with the family's corrupt label
        // and a class-only diagnostic naming the column, never the
        // stored value (the T-027 round-2 rule: a tampered cell is
        // arbitrary database text, and `params`/`os_message` are
        // exactly the surfaces the log line and the localized message
        // read).
        ProjectsError::Member { column, .. } => projects_fallback(
            "ProjectsCorrupt",
            &format!("a stored {column} cell is outside its vocabulary"),
        ),
        // The write-path outcomes a command consuming this mapper does
        // not map itself (`exclusion_remove` pre-empts its one by-id
        // outcome — NotFound — with the typed refusal before the
        // mapper runs, and `projects_reorder` maps its own
        // `ReorderMismatch` the same way); if one ever arrives anyway,
        // the same fallback keeps the family label and class-only text
        // — never the `path_key` or id, which are stored,
        // path-derived identifiers on the same SEC-008 surfaces.
        write_outcome => projects_fallback(
            "ProjectsError",
            match &write_outcome {
                ProjectsError::DuplicatePathKey { .. } => {
                    "a duplicate path key conflict reached a command that does not map it"
                }
                ProjectsError::NotFound { .. } => {
                    "a by-id write outcome reached a command that does not map it"
                }
                ProjectsError::ReorderMismatch { .. } => {
                    "a reorder mismatch reached a command that does not map it"
                }
                ProjectsError::Member { .. } | ProjectsError::Sql(_) => {
                    "an unexpected projects store outcome"
                }
            },
        ),
    }
}

/// Builds the D-037 fallback with a family label and a fixed,
/// class-only diagnostic — the shared shape of the two
/// [`projects_store_to_app_error`] fallback arms (the
/// `storage_fallback` builder of the settings mappers).
fn projects_fallback(kind: &str, message: &str) -> AppError {
    let mut app_error = AppError::new(ErrorCode::DbLocked);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some(kind.to_owned()),
        os_message: Some(message.to_owned()),
    });
    app_error
}

// ---------------------------------------------------------------------------
// `project_add_manual` (T-074, §7.10 Projects module, §7.1
// PRJ-001/003/004/010/012, §6.9, §8 S1, §5.6 SEC-003, DB-015/DB-030/
// DB-042, D-07/D-08/D-29, LOG-001…004, D-051)
// ---------------------------------------------------------------------------

/// The `project_add_manual` response: a typed outcome string plus the
/// one payload arm that outcome carries — a struct with a
/// discriminant (not a tagged enum) in the `ActivityEntry` house
/// shape: closed vocabularies travel as strings, every field is a
/// wire primitive, and the arms are `Option` fields the UI switches on
/// with its outcome string (the `markdown_preview` DTO precedent for
/// "one response, several meanings").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ProjectAddResult {
    /// What the flow answered, one of `"added"`, `"repo_root_offered"`,
    /// `"duplicate_active"`, `"archived_duplicate"` (PRJ-001/003/004,
    /// DB-015): only failures are `Err` — the three non-add outcomes
    /// are typed answers about the user's folder, not errors of the
    /// operation (§10 has no code for "already there"; the D-096
    /// refusal-not-500 rule).
    pub(crate) outcome: String,
    /// The created row — the `"added"` arm only.
    pub(crate) project: Option<AddedProject>,
    /// The repository root the app offers instead of the picked
    /// subfolder (PRJ-003/D-08), the `"repo_root_offered"` arm only.
    /// The confirm call re-sends the SAME picked path with
    /// `accept_repo_root: true`; the root is re-derived inside the
    /// command, so SEC-003's letter (the picked folder is what the
    /// dialog chose) is never widened by an app-derived path
    /// (D-100's recorded rule).
    pub(crate) offered_root: Option<String>,
    /// The already-registered project the answer is about — the
    /// `"duplicate_active"` and `"archived_duplicate"` arms (the
    /// latter is the restore offer: the id is what the future
    /// `project_restore` command takes, PRJ-013…018).
    pub(crate) existing: Option<ExistingProject>,
}

impl ProjectAddResult {
    /// The `"added"` arm around a freshly created row and the §6.9
    /// write-access warning the post-commit probe answered (if any).
    fn added(project: &eunomia_store::projects::Project, warning: Option<String>) -> Self {
        ProjectAddResult {
            outcome: "added".to_owned(),
            project: Some(AddedProject::from_store(project, warning)),
            offered_root: None,
            existing: None,
        }
    }

    /// The `"repo_root_offered"` arm (PRJ-003): nothing was written.
    fn repo_root_offered(root: &Path) -> Self {
        ProjectAddResult {
            outcome: "repo_root_offered".to_owned(),
            project: None,
            offered_root: Some(root.display().to_string()),
            existing: None,
        }
    }

    /// The duplicate arms (PRJ-001, DB-015): the row the picked
    /// repository's key already belongs to — active is the "message is
    /// shown" case, archived is the "offer to restore" case; the
    /// stored `state` cell tells the two apart. Nothing was written.
    fn duplicate_of(existing: &eunomia_store::projects::Project) -> Self {
        ProjectAddResult {
            outcome: match existing.state {
                ProjectState::Active => "duplicate_active",
                ProjectState::Archived => "archived_duplicate",
            }
            .to_owned(),
            project: None,
            offered_root: None,
            existing: Some(ExistingProject {
                id: existing.id.clone(),
                name: existing.name.clone(),
                path: existing.path.clone(),
                state: existing.state.as_str().to_owned(),
            }),
        }
    }
}

/// The created project on the `"added"` arm of [`ProjectAddResult`]:
/// the column fields `projects_list` serves plus the write settings
/// the row copied from `default.*` (PRJ-012) — the "Project settings"
/// modal (S1) opens on exactly these facts, and the path is included
/// because PRJ-008 shows it read-only there. No store type crosses the
/// boundary (the `ProjectColumnEntry` rule): the closed vocabularies
/// travel in their stored spellings (`"A"`, `"AGENTS.md"`,
/// `"manual"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct AddedProject {
    /// The row id — the identity every later project command
    /// (`project_update`, `project_archive`, …) addresses.
    pub(crate) id: String,
    /// The canonical repository root (DB-030), the verbatim
    /// canonicalize output stored in `projects.path`.
    pub(crate) path: String,
    /// The display name derived from the root's folder name (§6.9:
    /// 1–100 after trimming; the modal lets the user edit it,
    /// PRJ-008).
    pub(crate) name: String,
    /// The color-badge label, DB-042's derivation of the name (first
    /// up to 6 characters, upper case).
    pub(crate) label: String,
    /// The palette member picked by D-29 (stored spelling).
    pub(crate) color: String,
    /// How the project was registered — always `"manual"` here
    /// (§6.2; the scan commit is `scan_commit`'s spelling).
    pub(crate) added_via: String,
    /// The appended position (DB-005: `MAX(sort_order) + 1` over the
    /// whole table, the repository's own append), saturated at
    /// `u32::MAX` — specta forbids 64-bit integers on the TS wire
    /// (the `ActivityEntry.id` decision, D-049).
    pub(crate) sort_order: u32,
    /// The project's own scheme, copied from `default.scheme`
    /// (PRJ-012/DEF-005: later default changes never reach it).
    pub(crate) scheme: String,
    /// The project's own target file, copied from
    /// `default.base_name`.
    pub(crate) base_name: String,
    /// The `AGENTS.local.md` hint flag, copied from
    /// `default.extra_local`.
    pub(crate) extra_local_enabled: bool,
    /// The `AGENTS.local.md` hint position, copied from
    /// `default.extra_local`.
    pub(crate) extra_local_position: String,
    /// The `AGENTS.project.md` hint flag, copied from
    /// `default.extra_project`.
    pub(crate) extra_project_enabled: bool,
    /// The `AGENTS.project.md` hint position, copied from
    /// `default.extra_project`.
    pub(crate) extra_project_position: String,
    /// The §6.9 write-access warning — "Write access is checked when
    /// adding (warning)": the project IS added; this carries the
    /// D-094 verdict class (`folder_not_writable`, `file_read_only`)
    /// when the probe refused. `None` when the folder is writable, and
    /// deliberately also when the probe could not answer (a refusal is
    /// not a rights verdict, D-090) — the sync preflight re-checks
    /// before any write (§6.9, error there).
    pub(crate) write_access_warning: Option<String>,
}

impl AddedProject {
    /// Shapes a stored row into its wire form around the probe's
    /// warning.
    fn from_store(project: &eunomia_store::projects::Project, warning: Option<String>) -> Self {
        AddedProject {
            id: project.id.clone(),
            path: project.path.clone(),
            name: project.name.clone(),
            label: project.label.clone(),
            color: project.color.clone(),
            added_via: project.added_via.as_str().to_owned(),
            sort_order: u32::try_from(project.sort_order).unwrap_or(u32::MAX),
            scheme: project.scheme.as_str().to_owned(),
            base_name: project.base_name.as_str().to_owned(),
            extra_local_enabled: project.extra_local.enabled,
            extra_local_position: project.extra_local.position.as_str().to_owned(),
            extra_project_enabled: project.extra_project.enabled,
            extra_project_position: project.extra_project.position.as_str().to_owned(),
            write_access_warning: warning,
        }
    }
}

/// The already-registered project of the two duplicate arms (PRJ-001,
/// DB-015): what the message names, and the id the archived arm's
/// restore offer (the future `project_restore`, T-109) calls back
/// with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ExistingProject {
    /// The existing row's id.
    pub(crate) id: String,
    /// Its display name (the duplicate message names the project).
    pub(crate) name: String,
    /// Its registered canonical path.
    pub(crate) path: String,
    /// `active` or `archived` — which duplicate arm this is.
    pub(crate) state: String,
}

/// Registers the folder the user picked in the system dialog as a
/// project (§7.10 Projects module `project_add_manual`, the D-009
/// fixed name; §7.1 PRJ-001; scenario S1).
///
/// `path` is the folder the UI's `dialog_pick_folder` answer named,
/// and the command's first act is the SEC-003 gate on it —
/// [`session_paths::validate_ui_path`], the T-073/D-100 boundary: a
/// path that neither canonicalizes onto a registered project nor onto
/// a folder picked in this session is rejected with `E_VALIDATION`
/// before anything else runs (a foreign folder never reaches the
/// `.git` check, the database, or the disk probes).
///
/// The spec flow, in order (S1: "pick a folder, `.git` check,
/// duplicate check, create the entry with default settings and a
/// color by D-29"):
///
/// 1. **Repository root** (PRJ-003/D-08, blocking pool):
///    [`eunomia_fs::paths::find_git_root`] from the canonical pick —
///    a subfolder resolves to its repository's canonical root
///    (nearest root wins, D-06). A folder outside any repository is
///    §10's `E_NOT_A_REPO`; a probe the OS refused is
///    `E_PROJECT_UNAVAILABLE` with the OS facts ("could not check" is
///    not "not a repo", D-090).
/// 2. **The root offer** (PRJ-003): when the picked folder is a
///    subfolder and `accept_repo_root` is not `Some(true)`, the answer
///    is the `"repo_root_offered"` arm — the app *offers* the root,
///    it does not silently register a different folder than the one
///    the user picked. Nothing is written. The UI's confirm re-sends
///    the same picked path with the flag; the root is re-derived
///    (idempotent converge), so SEC-003's allowlist is never widened
///    with an app-derived path (D-100).
/// 3. **Fields** (§6.9): the name defaults to the root's folder name
///    (`validate_name`) and the label to DB-042's derivation of it
///    (`default_label`, then `validate_label` — the recorded edge
///    where a control-character prefix derives an unstorable label is
///    an `E_VALIDATION` naming the field, not a panic).
/// 4. **One transaction** (D-037, D-051, §6.9 "the whole operation
///    runs in one transaction"): the DB-015 duplicate check
///    (`find_by_path_key`, either state — active answers the
///    `"duplicate_active"` message, archived the
///    `"archived_duplicate"` restore offer; both typed answers, no
///    error, nothing written); the four write settings copied from
///    `default.*` (PRJ-012/DEF-005); the D-29 color over the ACTIVE
///    projects' colors; `create` (the repository appends `sort_order`);
///    and the `project.added` journal entry (LOG-001…004,
///    info/user/ok, entity project) **inside the same transaction** —
///    a rolled-back add leaves no entry claiming it happened. The
///    `E_SYNC_RUNNING` guard of SYS-005 does not apply: that rule
///    lists the project-*editing* commands, and no sync writer exists
///    yet; the guard joins with the sync-run task.
/// 5. **Write-access warning** (§6.9, T-063/D-094): after the commit,
///    [`eunomia_fs::write_access::check_write_access`] over the
///    root's future target file, on the blocking pool (never inside
///    the transaction — a dead drive must not hold the one database
///    lock, the D-096 probe-order rule). The verdict travels as the
///    response's `write_access_warning`; the project stays added.
///
/// Failures reach the log with their §10 code
/// ([`logging::project_add_failed`]); the three typed non-add
/// outcomes log nothing (nothing happened), and a successful add is
/// the journal entry itself (the D-051 mirror policy — info+user+ok
/// is not mirrored to the file log).
#[tauri::command]
#[specta::specta]
pub(crate) async fn project_add_manual<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    accept_repo_root: Option<bool>,
) -> Result<ProjectAddResult, AppError> {
    project_add_manual_impl(app, path, accept_repo_root)
        .await
        .inspect_err(logging::project_add_failed)
}

/// The body of [`project_add_manual`], split so the one logging edge
/// covers every failure path (the `activity_list` shape) — including
/// the SEC-003 rejection, which fires before anything else.
async fn project_add_manual_impl<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    accept_repo_root: Option<bool>,
) -> Result<ProjectAddResult, AppError> {
    // SEC-003 (§5.6): the input gate. The canonical form comes back
    // from the check; the walk below then re-resolves that
    // already-canonical form (its own first step, D-086) — idempotent
    // for the normal case, and equality with the pick stays byte
    // equality on canonical forms.
    let picked = session_paths::validate_ui_path(&app, &path).await?;
    // PRJ-003/D-08: the walk-up over live folders is blocking I/O
    // (SYS-001) — a dead mount must not stall an async worker.
    let walked = picked.clone();
    let root =
        tauri::async_runtime::spawn_blocking(move || eunomia_fs::paths::find_git_root(&walked))
            .await
            .map_err(db::join_failure_to_app_error)?
            .map_err(|source| repo_lookup_failed(&source))?;
    // The database every non-offer answer needs, resolved BEFORE the
    // offer branch so recovery mode answers consistently: a picked
    // folder of any depth gets the honest `NoDatabase` state answer,
    // not an offer whose confirm could only fail the same way (the
    // round-1 review finding).
    let database = managed_db(&app)?;
    // PRJ-003: a subfolder is *offered* the root, never silently
    // re-registered — the confirm call re-sends this same picked path
    // with the flag (see the command docs for the SEC-003 reasoning).
    if root != picked && accept_repo_root != Some(true) {
        return Ok(ProjectAddResult::repo_root_offered(&root));
    }
    let name = derived_name(&root)?;
    let label_default = eunomia_core::project_validators::default_label(&name);
    let label = eunomia_core::project_validators::validate_label(&label_default)
        .map_err(|source| rejected_field("label", &label_default, &source.to_string()))?
        .to_owned();

    // The transaction half: everything the operation writes happens in
    // one `Db::run` (D-037). The moved values are what `NewProject`
    // borrows; the probe keeps its own copies outside.
    let root_key = eunomia_fs::paths::path_key(&root);
    let probe_root = root.clone();
    let outcome = database
        .run(move |transaction| {
            // DB-015: the key is unique across active AND archived
            // rows, and the check runs inside the operation's own
            // transaction — the same single-connection serialization
            // D-037 gives every writer, so the create below cannot
            // race past it.
            if let Some(existing) =
                eunomia_store::projects::find_by_path_key(transaction, &root_key)
                    .map_err(projects_store_to_app_error)?
            {
                return Ok(AddOutcome::Refused(ProjectAddResult::duplicate_of(
                    &existing,
                )));
            }
            // PRJ-012: the write settings a new project copies from
            // `default.*` — read here, inside the transaction, so the
            // copy is atomic with the create (a concurrent
            // `settings_set` cannot split them).
            let scheme = match stored_default(transaction, SettingKey::DefaultScheme)? {
                SettingValue::Scheme(scheme) => scheme,
                _ => return Err(unpaired_setting(SettingKey::DefaultScheme)),
            };
            let base_name = match stored_default(transaction, SettingKey::DefaultBaseName)? {
                SettingValue::BaseName(base_name) => base_name,
                _ => return Err(unpaired_setting(SettingKey::DefaultBaseName)),
            };
            let extra_local = match stored_default(transaction, SettingKey::DefaultExtraLocal)? {
                SettingValue::ExtraFile(extra) => extra,
                _ => return Err(unpaired_setting(SettingKey::DefaultExtraLocal)),
            };
            let extra_project = match stored_default(transaction, SettingKey::DefaultExtraProject)?
            {
                SettingValue::ExtraFile(extra) => extra,
                _ => return Err(unpaired_setting(SettingKey::DefaultExtraProject)),
            };
            // D-29/DB-041: the least-used palette color among ACTIVE
            // projects, tie-broken by a fresh UUIDv7's hash. The row's
            // own id is minted inside `create` and cannot be known
            // before the color is chosen; the store is frozen for this
            // task, so the tie-break hashes a pre-generated sibling
            // id — both are fresh UUIDv7s at decision time, which is
            // the determinism D-29's parenthetical asks of a decision
            // point (recorded in the task's DECISIONS.md entry).
            let occupied = occupied_colors(transaction)?;
            let tie_break_id = eunomia_store::values::new_id();
            let color =
                eunomia_core::color_pick::pick_color(&occupied, &ProjectId::new(tie_break_id));
            // The stored path is the verbatim canonicalize output
            // (DB-030); the lossy form is the TEXT-column boundary the
            // key and the log share (D-086) — a non-UTF-8 byte reaches
            // the row the same way it reached the key.
            let path_text = root.to_string_lossy().into_owned();
            let new = eunomia_store::projects::NewProject {
                path: &path_text,
                path_key: &root_key,
                name: &name,
                label: &label,
                color: color.as_str(),
                description: "",
                added_via: eunomia_store::projects::AddedVia::Manual,
                scheme,
                base_name,
                extra_local,
                extra_project,
            };
            // The create: a `DuplicatePathKey` here is the one write
            // race the in-transaction check above cannot produce (one
            // connection serializes every writer, D-037) — and if it
            // ever fires anyway, the honest answer is the SAME typed
            // duplicate outcome, not the read-command fallback whose
            // diagnostic would misname the flow and whose
            // `E_DB_LOCKED` code would invite a retry (the round-1
            // review finding). Convergence, never a masked conflict.
            let created = match eunomia_store::projects::create(transaction, &new) {
                Ok(created) => created,
                Err(ProjectsError::DuplicatePathKey { .. }) => {
                    let existing =
                        eunomia_store::projects::find_by_path_key(transaction, &root_key)
                            .map_err(projects_store_to_app_error)?
                            .ok_or_else(|| {
                                projects_fallback(
                                    "ProjectsError",
                                    "a duplicate path key conflict vanished on the re-read",
                                )
                            })?;
                    return Ok(AddOutcome::Refused(ProjectAddResult::duplicate_of(
                        &existing,
                    )));
                }
                Err(other) => return Err(projects_store_to_app_error(other)),
            };
            // LOG-001…004 ("adding … projects") inside the operation's
            // own transaction (D-051): entity and project carry the
            // new id; no details — the row itself holds the path
            // (this flow writes no file; the one non-file-write
            // `File` detail the app produces is `exclusion_remove`'s,
            // recorded in D-106).
            let mut entry = LogEntry::new(
                Action::ProjectAdded,
                Level::Info,
                Visibility::User,
                Outcome::Ok,
            );
            entry.entity = Some(Entity {
                kind: EntityType::Project,
                id: created.id.as_str(),
            });
            entry.project_id = Some(created.id.as_str());
            insert(transaction, &entry).map_err(activity_log_to_app_error)?;
            // The D-051 mirror contract: one call per written entry
            // (info+user+ok is the unmirrored shape today).
            logging::action_logged(&entry);
            Ok(AddOutcome::Created(created))
        })
        .await?;
    match outcome {
        // The typed refusals: nothing was written, and nothing logs —
        // the flow answered a question about the user's folder.
        AddOutcome::Refused(result) => Ok(result),
        AddOutcome::Created(project) => {
            // §6.9's "checked when adding (warning)": the probe runs
            // after the commit, on the blocking pool (a dead drive
            // delays only this response — the D-096 probe-order rule),
            // over the root's future target file — scheme C writes
            // `<STEM>.project.md` and never touches the main file
            // (D-20), so the probe asks about the file the app will
            // actually write (the round-1 review finding: probing
            // `base_name` mis-targets every scheme-C project). The
            // project stays registered whatever the verdict.
            let target = probe_root.join(probe_target_name(project.scheme, project.base_name));
            let warning = tauri::async_runtime::spawn_blocking(move || {
                eunomia_fs::write_access::check_write_access(&target)
                    .err()
                    .as_ref()
                    .and_then(write_access_warning)
            })
            .await
            .map_err(db::join_failure_to_app_error)?;
            Ok(ProjectAddResult::added(&project, warning))
        }
    }
}

/// The transaction half of [`project_add_manual`]: the created row, or
/// the typed refusal the duplicate check answered (the wire result is
/// built around either once the probe has run).
enum AddOutcome {
    /// The row was created and journaled; the commit is the caller's
    /// `Db::run`.
    Created(eunomia_store::projects::Project),
    /// The duplicate check answered instead — active message or
    /// archived-restore offer; nothing was written.
    Refused(ProjectAddResult),
}

/// Maps the root-lookup failure onto §10 (the PRJ-003 half of the
/// flow): the negative lookup (`NoGitEntry` — every ancestor probed,
/// no `.git` anywhere) is `E_NOT_A_REPO`, §10's row for "a folder
/// without `.git` is being added"; a refused probe or a refused
/// canonicalization is `E_PROJECT_UNAVAILABLE` — "could not check" is
/// not "checked and absent" (the D-090 split; §10 keeps the two
/// apart, and the user actions differ: retry or fix access vs. pick
/// the repository root). The walked path travels in `params`
/// (SEC-008: paths may) and the OS refusal, when there is one, in
/// `details` (§10's first-class log fact).
fn repo_lookup_failed(source: &NotARepoError) -> AppError {
    let code = match source {
        NotARepoError::NoGitEntry { .. } => ErrorCode::NotARepo,
        NotARepoError::Canonicalize { .. } | NotARepoError::NotReadable { .. } => {
            ErrorCode::ProjectUnavailable
        }
    };
    let mut app_error = AppError::new(code);
    app_error
        .params
        .insert("path".to_owned(), source.path().to_owned());
    match source.io_error() {
        Some(io) => app_error.with_os_error(io),
        None => app_error,
    }
}

/// The default display name of a new project (S1): the repository
/// root's own folder name, §6.9-validated through the core validator —
/// trimmed, 1–100 characters, code points. A canonical root always has
/// a folder name except the filesystem root itself (`/`, `C:\`), whose
/// empty derivation is the honest `E_VALIDATION` naming the field (a
/// repository mounted at the filesystem root is named by the user in
/// the settings modal afterwards — but the app cannot register a
/// nameless row, and the CHECKs would reject it anyway).
fn derived_name(root: &Path) -> Result<String, AppError> {
    let raw = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    eunomia_core::project_validators::validate_name(&raw)
        .map_err(|source| rejected_field("name", &raw, &source.to_string()))
}

/// Reads one §6.5 setting the new project copies, through the store's
/// single read path (D-039): a missing row answers the §6.5 default, a
/// corrupt cell answers the settings corrupt fallback
/// ([`settings_cell_to_app_error`], never `E_VALIDATION` — the T-027
/// round-2 rule: corruption is not the caller's input).
fn stored_default(
    transaction: &rusqlite::Transaction<'_>,
    key: SettingKey,
) -> Result<SettingValue, AppError> {
    eunomia_store::settings::get(transaction, key).map_err(settings_cell_to_app_error)
}

/// The wrong-variant arm of a default-setting read: `get` re-parses
/// every stored cell through the key's §6.5 contract, so a value of
/// the wrong variant reaching the destructure means the read's own
/// validation was bypassed — unreachable, but typed honestly instead
/// of `unwrap`ped. The synthesized `SettingsError::Type` rides the
/// shared mapper, whose diagnostic is class-only (the fields never
/// reach the wire or the log).
fn unpaired_setting(key: SettingKey) -> AppError {
    settings_cell_to_app_error(StorageError::Settings {
        key: key.as_str().to_owned(),
        source: SettingsError::Type {
            expected: "the key's §6.5 type",
            found: "another variant",
        },
    })
}

/// The D-29 input: the colors held by ACTIVE projects, one entry per
/// project (a multiset — repetitions count, the archived contribute
/// nothing, DB-014). The `color` column has no schema CHECK (§6.2),
/// so a stored cell outside the palette can only be corruption or a
/// newer app version — the D-096 corrupt-cells fallback with a
/// class-only diagnostic, never a guess (and never the stored value on
/// a SEC-008 surface).
fn occupied_colors(transaction: &rusqlite::Transaction<'_>) -> Result<Vec<Color>, AppError> {
    eunomia_store::projects::list(transaction, ListScope::Active)
        .map_err(projects_store_to_app_error)?
        .into_iter()
        .map(|project| {
            eunomia_core::project_validators::validate_color(&project.color).map_err(|_| {
                projects_fallback(
                    "ProjectsCorrupt",
                    "a stored color cell is outside its palette",
                )
            })
        })
        .collect()
}

/// The §9 target-file name the write-access probe checks: scheme C
/// writes `<STEM>.project.md` and never edits the main file (D-20), so
/// the probe must ask about the file the app will actually write —
/// [`eunomia_core::backup_name::target_file_name`] is the single owner
/// of that mapping (the round-1 review finding: probing the row's
/// `base_name` mis-targets every scheme-C project). The store's
/// vocabulary twins are converted member-by-member — both enums carry
/// the same schema literals, and the match is compile-checked on both
/// sides, so a new member of either cannot drift silently.
fn probe_target_name(
    scheme: eunomia_store::settings::Scheme,
    base_name: eunomia_store::settings::BaseName,
) -> &'static str {
    use eunomia_core::project::{BaseName, Scheme};
    let core_scheme = match scheme {
        eunomia_store::settings::Scheme::A => Scheme::A,
        eunomia_store::settings::Scheme::B => Scheme::B,
        eunomia_store::settings::Scheme::C => Scheme::C,
    };
    let core_base_name = match base_name {
        eunomia_store::settings::BaseName::AgentsMd => BaseName::AgentsMd,
        eunomia_store::settings::BaseName::ClaudeMd => BaseName::ClaudeMd,
    };
    eunomia_core::backup_name::target_file_name(core_scheme, core_base_name)
}

/// Shapes the §6.9 warning from the probe's verdict: the two rights
/// verdicts map to their wire classes; a refused probe maps to `None`
/// — "could not check" is not "no rights" (the D-090 split), the
/// project is added without a warning, and the sync preflight (where
/// the same probe's failure is an `E_NO_WRITE_ACCESS` skip, §9.4)
/// re-checks before any write.
fn write_access_warning(source: &WriteAccessError) -> Option<String> {
    match source {
        WriteAccessError::FolderNotWritable { .. } => Some("folder_not_writable".to_owned()),
        WriteAccessError::FileReadOnly { .. } => Some("file_read_only".to_owned()),
        WriteAccessError::ProbeFailed { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// `exclusions_list` and `exclusion_remove` (T-078, §7.10 Projects
// module, §6.4 "Path exclusion", D-10/PRJ-013…018, LOG-001…004, D-051,
// D-093, D-098)
// ---------------------------------------------------------------------------

/// One path exclusion on the `exclusions_list` wire (§6.2
/// `project_exclusions`, the D-10 "Excluded paths" screen): the §6.2
/// row's every column as a wire primitive — the `ActivityEntry` rule
/// that no store type crosses the boundary, with the closed
/// vocabulary (`reason`) in its stored spelling the way the
/// settings/action-log DTOs carry theirs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ExclusionEntry {
    /// The row id (UUIDv7 TEXT, DB-002) — the identity
    /// `exclusion_remove` addresses; the screen's "Remove" button
    /// calls back with it, never with a path.
    pub(crate) id: String,
    /// The canonical path (DB-030), stored verbatim — the screen's
    /// display text: an exclusion IS a path fact.
    pub(crate) path: String,
    /// The identity key (DB-030/DB-031) — the string the scan
    /// registry's excluded set holds (ALG-053, D-098), carried so the
    /// scan UI can correlate a skipped repository with its exclusion
    /// row without re-deriving the key. A pure text transform of
    /// `path` (the `path_key` function, D-086): no fact the path
    /// itself does not already carry.
    pub(crate) path_key: String,
    /// Why the path is excluded (§6.2 CHECK vocabulary):
    /// `"removed_scanned"` or `"manual"`, the stored spelling.
    pub(crate) reason: String,
    /// When the exclusion was created, DB-002 UTC ISO-8601.
    pub(crate) created_at: String,
}

impl ExclusionEntry {
    /// Shapes a stored exclusion row into its wire form.
    fn from_store(exclusion: eunomia_store::projects::Exclusion) -> Self {
        ExclusionEntry {
            id: exclusion.id,
            path: exclusion.path,
            path_key: exclusion.path_key,
            reason: exclusion.reason.as_str().to_owned(),
            created_at: exclusion.created_at,
        }
    }
}

/// Lists the excluded paths for the D-10 "Excluded paths" screen
/// (§7.10 Projects module `exclusions_list`; §6.4: a path exclusion
/// is what keeps an archived scan-found project from resurfacing as a
/// scan candidate, PRJ-013…018).
///
/// The read runs through the [`Db::run`] executor like every database
/// command (D-037), one transaction off the async runtime, and
/// answers in the repository's own stable order — `created_at`, the
/// UUIDv7 id second: oldest first, insertion order inside one
/// millisecond. A fresh database answers an empty list — the screen's
/// empty state, not an error and not a null payload. Recovery mode
/// (no database) answers the honest [`managed_db`] fallback, never a
/// pretend-empty list that would lie about the exclusions table being
/// empty (the `projects_list` rule).
///
/// SEC-003 note: the response carries the database's own path facts
/// out to the UI, but the command accepts NO path input — there is
/// nothing to gate, and no UI-typed path can reach the filesystem or
/// the store through it. The inverse direction (`exclusion_remove`)
/// is id-addressed for the same reason.
///
/// A successful read is never journaled (the `activity_list`
/// rationale: reading is not an event); failures reach the log with
/// their §10 code ([`logging::exclusions_list_failed`]).
#[tauri::command]
#[specta::specta]
pub(crate) async fn exclusions_list<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<ExclusionEntry>, AppError> {
    exclusions_list_impl(app)
        .await
        .inspect_err(logging::exclusions_list_failed)
}

/// The body of [`exclusions_list`], split so the one logging edge
/// covers every failure path (the `activity_list` shape).
async fn exclusions_list_impl<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<ExclusionEntry>, AppError> {
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            eunomia_store::projects::list_exclusions(transaction)
                .map(|rows| rows.into_iter().map(ExclusionEntry::from_store).collect())
                .map_err(projects_store_to_app_error)
        })
        .await
}

/// Physically deletes one path exclusion (§7.10 Projects module
/// `exclusion_remove`; §6.4's one physical-delete entity — "the path
/// is found by a scan again"): the D-10 "Excluded paths" screen's
/// "Remove" button.
///
/// `id` is a row id from [`exclusions_list`] — the command is
/// id-addressed and accepts no path, by design (SEC-003's spirit: a
/// row is chosen from the screen's own list, and no UI-typed path can
/// become a filesystem or store fact through this command). An id
/// that matches no row is the typed `E_VALIDATION` refusal naming the
/// field — a stale screen or a replayed click; the whole operation is
/// rejected and nothing is written. The end state converges either
/// way (the path is not excluded), and the refusal leaves no partial
/// state behind, so a retry after a refresh is always safe (the
/// make-operations-idempotent rule: the store's `delete_exclusion`
/// owns the same rule one layer down — an absent row deletes
/// nothing).
///
/// One transaction (D-037/D-051, §6.9 "the whole operation runs in
/// one transaction"): the by-id lookup through the repository's
/// ordered list (the store's public face has no by-id read, D-093;
/// the list is exactly the screen's own data, screen-sized), the
/// physical `DELETE`, and the LOG-001…004 journal entry — a
/// `project.changed` record at info/user/ok whose `Details::File`
/// carries the removed path. The frozen `Action` vocabulary (D-051)
/// has no exclusion-specific member and no `EntityType` names an
/// exclusion, so the path — SEC-008 permits paths, and it is exactly
/// the fact the screen shows — identifies the removed row; a future
/// store-side task may grow an `exclusion.removed` member and take
/// the entry over. A failure between the writes rolls both back: the
/// journal never claims a removal that did not commit (the D-051
/// in-transaction rule, pinned here in the settings rollback test's
/// shape).
///
/// Failures reach the log with their §10 code
/// ([`logging::exclusion_remove_failed`]); a successful removal is
/// the journal entry itself (the D-051 mirror policy — info+user+ok
/// is not mirrored to the file log).
#[tauri::command]
#[specta::specta]
pub(crate) async fn exclusion_remove<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<(), AppError> {
    exclusion_remove_impl(app, id)
        .await
        .inspect_err(logging::exclusion_remove_failed)
}

/// The body of [`exclusion_remove`], split so the one logging edge
/// covers every failure path (the `activity_list` shape).
async fn exclusion_remove_impl<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), AppError> {
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            // The by-id lookup: the store's public face is the ordered
            // list (D-093 — no by-id read), which is exactly the data
            // the exclusions screen itself renders, so the find is
            // bounded by the screen's own size.
            let exclusions = eunomia_store::projects::list_exclusions(transaction)
                .map_err(projects_store_to_app_error)?;
            let Some(exclusion) = exclusions.iter().find(|row| row.id == id) else {
                // The typed refusal: caller input that names no row —
                // the honest §10 fit of the frozen registry (no
                // generic not-found code exists; the D-037 fallback
                // would misname a stale screen as a database
                // problem). Nothing was written.
                return Err(rejected_field("id", &id, "no exclusion with this id"));
            };
            // §6.4's physical delete. The store's own NotFound cannot
            // fire here — the row was just read inside this same
            // transaction on the one connection (D-037) — and if it
            // ever does anyway, the shared mapper's fallback keeps
            // the family label instead of guessing.
            eunomia_store::projects::delete_exclusion(transaction, &id)
                .map_err(projects_store_to_app_error)?;
            // LOG-001…004 inside the operation's own transaction
            // (D-051): `project.changed` with the removed path as the
            // File detail — the frozen-vocabulary reasoning is on the
            // command docs.
            let mut entry = LogEntry::new(
                Action::ProjectChanged,
                Level::Info,
                Visibility::User,
                Outcome::Ok,
            );
            entry.details = Some(Details::File {
                path: PathBuf::from(&exclusion.path),
            });
            insert(transaction, &entry).map_err(activity_log_to_app_error)?;
            // The D-051 mirror contract: one call per written entry
            // (info+user+ok is the unmirrored shape today).
            logging::action_logged(&entry);
            Ok(())
        })
        .await
}

// ---------------------------------------------------------------------------
// `projects_reorder` (T-080, §7.10 Projects module, §7.1 PRJ-019/020,
// DB-005, LOG-001…004, D-051, D-093)
// ---------------------------------------------------------------------------

/// The three D-093 mismatch forms of a reorder input, counted at the
/// command's own boundary — the active-set twin of the store's
/// [`ProjectsError::ReorderMismatch`] payload (the same three numbers,
/// judged before any write runs).
#[derive(Debug)]
struct ReorderForms {
    /// Input ids that name no active project (an unknown id, or an
    /// archived one — a stale screen's list).
    unknown: Vec<String>,
    /// Active projects the input omits.
    missing: usize,
    /// Ids the input repeats.
    duplicates: usize,
}

impl ReorderForms {
    /// Whether any form is present — the one condition that refuses
    /// the reorder.
    fn any(&self) -> bool {
        !self.unknown.is_empty() || self.missing > 0 || self.duplicates > 0
    }
}

/// Builds the full-table permutation a reorder writes from the stored
/// order and the UI's active list (PRJ-019/020): the new active order
/// is zipped into the stored table order — every slot the stored table
/// fills with an **active** project receives the next id of the input,
/// and every archived row keeps exactly its own slot and `sort_order`
/// (the least destructive reading of D-093/§6.4: archive/restore
/// already keep a row's position, and a column drag must not move the
/// rows the matrix does not even render).
///
/// Returns the permutation plus how many positions actually change —
/// the no-op verdict that keeps a replayed drag from writing (and
/// journaling) anything — or the three mismatch forms when the input
/// does not name the active set exactly once each.
fn composed_reorder<'a>(
    stored: &[(&'a str, bool)],
    ordered_ids: &'a [String],
) -> Result<(Vec<&'a str>, usize), ReorderForms> {
    let mut seen: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(ordered_ids.len());
    let mut duplicates = 0usize;
    for id in ordered_ids {
        if !seen.insert(id.as_str()) {
            duplicates += 1;
        }
    }
    let active: std::collections::HashSet<&str> = stored
        .iter()
        .filter(|(_, is_active)| *is_active)
        .map(|(id, _)| *id)
        .collect();
    let unknown: Vec<String> = ordered_ids
        .iter()
        .filter(|id| !active.contains(id.as_str()))
        .cloned()
        .collect();
    let missing = active.difference(&seen).count();
    let forms = ReorderForms {
        unknown,
        missing,
        duplicates,
    };
    if forms.any() {
        return Err(forms);
    }
    let mut input = ordered_ids.iter().map(String::as_str);
    let mut permutation: Vec<&str> = Vec::with_capacity(stored.len());
    let mut changed = 0usize;
    for (id, is_active) in stored {
        let new_id = if *is_active {
            input
                .next()
                .expect("the input names exactly the active set")
        } else {
            *id
        };
        if *id != new_id {
            changed += 1;
        }
        permutation.push(new_id);
    }
    Ok((permutation, changed))
}

/// A `projects_reorder` input rejection: `E_VALIDATION` in the
/// field/value/reason shape (the [`rejected_field`] builder's list
/// form), with the reason naming all three counts. The value echoes
/// the offending caller ids — capped, so a buggy caller's unbounded
/// list cannot bloat the envelope or its log line — or, when the
/// offending form is missing-only, the count of ids the caller did
/// send; stored ids never travel on these surfaces (the D-096 rule:
/// `params` and the log line are exactly what the localization and
/// the diagnostics read).
fn rejected_reorder(forms: &ReorderForms, sent: usize) -> AppError {
    // The per-id bound (UTF-8 code points, so a multi-byte id cannot
    // panic the cut): a real project id is a 36-char UUIDv7, which
    // passes intact — only an absurdly long caller string is cut.
    const ECHO_LIMIT: usize = 5;
    const ID_CHARS: usize = 64;
    let value = if forms.unknown.is_empty() {
        format!("{sent} ids sent")
    } else {
        let echo = forms.unknown[..forms.unknown.len().min(ECHO_LIMIT)]
            .iter()
            .map(|id| {
                let bounded: String = id.chars().take(ID_CHARS).collect();
                bounded
            })
            .collect::<Vec<_>>()
            .join(", ");
        if forms.unknown.len() > ECHO_LIMIT {
            format!("{echo} (+{} more)", forms.unknown.len() - ECHO_LIMIT)
        } else {
            echo
        }
    };
    rejected_field(
        "ordered_ids",
        &value,
        &format!(
            "the list must name every active project exactly once: \
             {} not an active project, {} missing, {} repeated",
            forms.unknown.len(),
            forms.missing,
            forms.duplicates
        ),
    )
}

/// Persists a new column order (§7.10 Projects module
/// `projects_reorder`; §7.1 PRJ-019/020: the order is changed by
/// dragging a column or through the "Move left / Move right" menu, and
/// "the order is saved right away" — the AC pair: the order is kept
/// after a restart).
///
/// `ordered_ids` is the complete list of **active** project ids in the
/// new order — the columns the matrix renders, exactly the ids
/// `projects_list`'s default active read serves (PRJ-019/020 reorders
/// columns; the archived screen is not a column). Positions are
/// table-wide (DB-005, D-093), so the command composes the full
/// permutation inside its own transaction: the new active order is
/// zipped into the stored table order with every archived row keeping
/// its own slot ([`composed_reorder`] — the D-093 "splices the move
/// in" recipe over `list(ListScope::All)`), and the store's
/// [`reorder`](eunomia_store::projects::reorder) renumbers the whole
/// table `1..=N`.
///
/// One transaction (D-037/D-051, §6.9 "an entity is never saved
/// partly"): the ordered read, the composition, the renumbering and
/// the LOG-001…004 journal entry — `project.changed` ("changing
/// projects") at info/user/ok with [`Details::Bulk`] carrying how many
/// positions moved, and no entity or project id: a reorder is one fact
/// about many projects at once, the bulk parenthetical's shape. A
/// failure between the writes rolls both back (the journal never
/// claims an order that did not commit).
///
/// Idempotent (the replayed-drag rule): a list that composes to the
/// stored order is a **verified no-op** — nothing is written and
/// nothing is journaled (the journal must not claim a change that
/// never happened, D-051), so re-sending the same order as many times
/// as the UI retries converges with zero side effects. The shortcut
/// yields only while the stored positions are the gapless `1..=N`
/// DB-005 asks for: a tampered table is renumbered by the same call
/// that serves a drag (the one writer that restores the invariant).
///
/// A list that does not name the active set exactly once each — an
/// unknown or archived id, a missing active, a repeated id (a stale
/// screen's list) — is the typed `E_VALIDATION` refusal naming all
/// three counts ([`rejected_reorder`]): the whole operation is
/// rejected before the first write, and the table is provably
/// untouched. The store's own `ReorderMismatch` cannot follow a
/// passing validation (one connection serializes the read and the
/// write, D-037) but is mapped through the same refusal if it ever
/// fires anyway — convergence, never a masked conflict (the D-103
/// duplicate-race rule).
///
/// Failures reach the log with their §10 code
/// ([`logging::projects_reorder_failed`]); a successful reorder is the
/// journal entry itself (the D-051 mirror policy — info+user+ok is not
/// mirrored to the file log).
#[tauri::command]
#[specta::specta]
pub(crate) async fn projects_reorder<R: Runtime>(
    app: AppHandle<R>,
    ordered_ids: Vec<String>,
) -> Result<(), AppError> {
    projects_reorder_impl(app, ordered_ids)
        .await
        .inspect_err(logging::projects_reorder_failed)
}

/// The body of [`projects_reorder`], split so the one logging edge
/// covers every failure path (the `activity_list` shape).
async fn projects_reorder_impl<R: Runtime>(
    app: AppHandle<R>,
    ordered_ids: Vec<String>,
) -> Result<(), AppError> {
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            // D-093: positions are table-wide, so the whole table in
            // its current order is the base the move is spliced into —
            // `list(Active)` alone is never a valid permutation.
            let rows = eunomia_store::projects::list(transaction, ListScope::All)
                .map_err(projects_store_to_app_error)?;
            let stored: Vec<(&str, bool)> = rows
                .iter()
                .map(|project| {
                    (
                        project.id.as_str(),
                        matches!(project.state, ProjectState::Active),
                    )
                })
                .collect();
            let (permutation, changed) =
                composed_reorder(&stored, &ordered_ids).map_err(|forms| {
                    // The typed refusal: caller input that does not
                    // name the active set — nothing was written.
                    rejected_reorder(&forms, ordered_ids.len())
                })?;
            if changed == 0 {
                // The replayed-drag no-op — but only when the stored
                // positions are ALREADY the gapless `1..=N` DB-005
                // requires: a tampered, non-gapless table falls
                // through to the renumbering, which is the one writer
                // that restores the invariant. No shipped write path
                // can produce such a table (creation appends `MAX + 1`
                // and archive/restore keep positions, D-093) — the
                // guard exists so the shortcut never preserves damage.
                let gapless = rows.iter().enumerate().all(|(index, row)| {
                    row.sort_order == i64::try_from(index + 1).expect("a table's length fits i64")
                });
                if gapless {
                    return Ok(());
                }
            }
            // DB-005's renumbering, still inside the operation's one
            // transaction. A `ReorderMismatch` here is the
            // store-contract twin of the validation above (normally
            // pre-empted — the one connection serializes the read and
            // the write, D-037): the same typed refusal, so a caller
            // retry converges instead of meeting a misnamed fallback.
            eunomia_store::projects::reorder(transaction, &permutation).map_err(
                |error| match error {
                    ProjectsError::ReorderMismatch {
                        unknown,
                        missing,
                        duplicates,
                    } => rejected_reorder(
                        &ReorderForms {
                            unknown,
                            missing,
                            duplicates,
                        },
                        permutation.len(),
                    ),
                    other => projects_store_to_app_error(other),
                },
            )?;
            // LOG-001…004 ("changing projects") inside the operation's
            // own transaction (D-051): one entry for the whole reorder,
            // the bulk parenthetical's shape — the count of POSITIONS
            // that moved rides `Details::Bulk`; no entity and no
            // project id, because the change is about many projects at
            // once and the Activity window's project filter must not
            // mis-attribute it to one of them. The tampered-table
            // repair path (a non-gapless table renumbered with zero
            // positions moved) journals the same entry with `changed:
            // 0` — kept deliberately: the operation DID rewrite the
            // table, and a silent database rewrite with no journal
            // trace would defeat the journal's audit purpose (the
            // round-2 review's alternative, skipped here on purpose).
            let mut entry = LogEntry::new(
                Action::ProjectChanged,
                Level::Info,
                Visibility::User,
                Outcome::Ok,
            );
            entry.details = Some(Details::Bulk {
                changed: u64::try_from(changed).expect("a table wider than u64 rows is impossible"),
            });
            insert(transaction, &entry).map_err(activity_log_to_app_error)?;
            // The D-051 mirror contract: one call per written entry
            // (info+user+ok is the unmirrored shape today).
            logging::action_logged(&entry);
            Ok(())
        })
        .await
}

// ---------------------------------------------------------------------------
// `markdown_preview` (T-056, §7.10 Library module, SEC-005, D-15, D-009)
// ---------------------------------------------------------------------------

/// The `markdown_preview` response (§7.10 Library module): the
/// sanitized preview HTML plus D-15's non-blocking Markdown warnings.
/// A named struct since T-056, which carried only `html` in the shape
/// T-057's warnings list was already planned for; the `warnings` field
/// joined in a coordinated bindings regeneration (T-057) without
/// reshaping the wire the UI already consumes — `html` keeps its
/// position and meaning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct MarkdownPreview {
    /// The sanitized HTML (SEC-005): pulldown-cmark's render with
    /// every raw-HTML fragment of the text escaped into visible text
    /// and no resource-loading element anywhere —
    /// [`markdown::preview`] is the pipeline and its policy.
    pub(crate) html: String,
    /// The Markdown warnings (D-15, §7.3 LIB-002/009…016's "Markdown
    /// warnings (for example, an unclosed code block) that do not
    /// block saving"): one entry per finding, in source order, empty
    /// for clean text. Pure data for the editor's Preview UI — the
    /// save path never reads them and the command never fails because
    /// of them ([`markdown::MarkdownWarning`] is the taxonomy,
    /// [`markdown::preview`] the detection, in the same single parse
    /// pass as the HTML).
    pub(crate) warnings: Vec<markdown::MarkdownWarning>,
}

/// Renders the editor's Markdown into sanitized preview HTML with the
/// Markdown warnings alongside (§7.10 Library module `markdown_preview`,
/// the D-009 name registry; SEC-005, D-15, §7.3 LIB-002/009…016).
///
/// Stateless and **non-database**: it reads no managed state and does
/// no I/O, so there is nothing to route through [`Db::run`], and
/// SYS-001's executor policy is not triggered either — its
/// `spawn_blocking` list is DB, files, scan and sync (blocking *I/O*),
/// while this render is pure CPU with no lock and no `.await` inside,
/// and realistic inputs stay in the milliseconds range: the editor's
/// §6.9 text ceiling (DB-006: 262,144 characters) bounds what the UI
/// sends, though the command itself does not enforce that ceiling
/// (see the validation note below). It answers in any mode, recovery
/// included. There is no §6.9 validation here on purpose: this is the
/// editor's live preview of whatever the field currently holds, and
/// the save path (the validating constructors of the
/// instruction/variant tasks) is where text length and reserved
/// markers are enforced. The warnings follow the same reading — they
/// observe the draft as-is (D-15: "warnings only … does not block
/// writing") and are computed in the same single parse pass as the
/// HTML. The `Result` shape is §7.10's uniform command contract, not a
/// claim of fallibility: the pipeline is total, so no `Err` is
/// reachable today — and there is no log edge for the same reason
/// (§10 logs errors; none can occur).
#[tauri::command]
#[specta::specta]
pub(crate) async fn markdown_preview(text: String) -> Result<MarkdownPreview, AppError> {
    let preview = markdown::preview(&text);
    Ok(MarkdownPreview {
        html: preview.html,
        warnings: preview.warnings,
    })
}

// ---------------------------------------------------------------------------
// `app_info` (T-031, §7.10 System module, §7.9 UPD-005/REL-004, §5.5)
// ---------------------------------------------------------------------------

/// The app identifier (UPD-005/REL-004/D-020): the bundle ID and the
/// last path component of the data directory on every OS (§5.5). A
/// spec literal, deliberately not a runtime read of
/// `app.config().identifier`: the identifier is a build-time constant
/// of this app, and the runtime config would answer with the mock
/// runtime's own default in the IPC tests, making the spec value
/// unpinnable there. A unit test pins the literal equal to the
/// `tauri.conf.json` field (the config's home, D-020), so the command
/// cannot drift from the config that actually locates the data
/// directory.
const APP_IDENTIFIER: &str = "pro.yepbro.eunomiasync";

/// The repository link (UPD-005/REL-004): a spec literal — no
/// `repository` field exists in `Cargo.toml` or `tauri.conf.json`
/// today (so `env!("CARGO_PKG_REPOSITORY")` would be empty), and the
/// spec fixes the URL itself.
const REPOSITORY_URL: &str = "https://github.com/yepbro/EunomiaSync";

/// The pure half of the NFR-023 root seam: root is uid `0` on every
/// Unix, so the predicate is one comparison. A free function over a
/// raw `u32` (not a libc type) so it is testable on every platform.
/// Production code uses it on Linux alone (the platform twin of
/// [`running_as_root`] answers `false` without a uid), so the gate
/// keeps it in Linux builds and every test build — a non-Linux
/// release build drops it instead of carrying dead code.
#[cfg(any(test, target_os = "linux"))]
pub(crate) fn is_root(uid: u32) -> bool {
    uid == 0
}

/// The uid the OS gives files this process creates — the **effective**
/// user id (`geteuid`, the `id -u` answer). Chosen over the real uid
/// because NFR-023's rationale is ownership — "the created files will
/// be owned by root" follows the effective uid — and because the two
/// coincide for every launch path a desktop app really has (direct,
/// `sudo`, `pkexec`); see the task's DECISIONS.md entry. The real read
/// is Linux only (NFR-023 scopes the root warning there, and with it
/// the crate's single `unsafe` block lives only there).
#[cfg(target_os = "linux")]
pub(crate) fn current_uid() -> u32 {
    // SAFETY: `geteuid` is a pure libc query — async-signal-safe, no
    // arguments to validate, no failure mode, nothing held across the
    // call: it answers the calling process's effective user id and can
    // do nothing else.
    unsafe { libc::geteuid() }
}

/// The off-Linux twin of [`current_uid`]: no uid is read there (the
/// platform twin of [`running_as_root`] never looks at one), so the
/// `0` is inert — a placeholder that keeps the command body
/// platform-uniform, not a sentinel the caller must interpret.
#[cfg(not(target_os = "linux"))]
pub(crate) fn current_uid() -> u32 {
    0
}

/// The NFR-023 policy for a given uid: on Linux — the only platform
/// the spec assigns a root warning to — the flag is the root
/// predicate; the uid is the launch's effective id the command reads.
#[cfg(target_os = "linux")]
pub(crate) fn running_as_root(uid: u32) -> bool {
    is_root(uid)
}

/// The non-Linux half of the NFR-023 platform semantics: the spec
/// assigns the root warning to Linux alone, so the flag is `false` on
/// Windows and macOS regardless of uid — by this compile-time branch,
/// not by an untested convention. (macOS could detect root the same
/// way; the spec's letter wins. Recorded in the task's DECISIONS.md
/// entry.)
#[cfg(not(target_os = "linux"))]
pub(crate) fn running_as_root(_uid: u32) -> bool {
    false
}

/// The About window's fact sheet (§7.10 System module `app_info`;
/// UPD-005: version, identifier, author, license, repository link;
/// the §5.5 pair: the data directory and whether this launch created
/// the database from scratch).
///
/// Field sources, one each — see the task's DECISIONS.md entry for
/// the full rationale: `version` is the src-tauri crate version
/// (`CARGO_PKG_VERSION`, which `tauri.conf.json` mirrors — pinned by
/// a unit test), `identifier` and `repository` are the spec literals
/// above, `author` is `CARGO_PKG_AUTHORS` (the `src-tauri/Cargo.toml`
/// field D-020 names, `Den Brooks <denis@yepbro.pro>`), `license` is
/// `CARGO_PKG_LICENSE` (the workspace's `MIT OR Apache-2.0` — the
/// SPDX form of UPD-005/REL-004's "dual license: MIT License and
/// Apache License, Version 2.0", matching the `LICENSE-MIT`/
/// `LICENSE-APACHE` pair in the repository root), `data_dir` is the
/// resolved data-directory root (the managed [`data_dirs::DataRoot`],
/// D-049), and `db_created_fresh` is the launch-time fact
/// [`Db::open`] observed (see the command docs). `running_as_root`
/// (NFR-023) is the process's effective-uid fact — see the seam
/// functions above it. The path renders
/// through `display()` — a lossy display string for the About window;
/// the OS-level "open it" action goes through `app_open_data_folder`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct AppInfo {
    /// The app version (`CARGO_PKG_VERSION`; `tauri.conf.json`
    /// mirrors it).
    pub(crate) version: String,
    /// The bundle identifier `pro.yepbro.eunomiasync` (UPD-005,
    /// REL-004, D-020).
    pub(crate) identifier: String,
    /// The author, `Name <email>` (the Cargo manifest form of
    /// UPD-005's "Den Brooks, denis@yepbro.pro").
    pub(crate) author: String,
    /// The license as an SPDX expression (`MIT OR Apache-2.0` — the
    /// dual license of UPD-005/REL-004).
    pub(crate) license: String,
    /// The repository link (UPD-005/REL-004).
    pub(crate) repository: String,
    /// The data-directory root resolved at startup (§5.5), as a
    /// display string.
    pub(crate) data_dir: String,
    /// Whether this launch created the database from scratch (§5.5
    /// tip: deleted data → "the app starts with an empty DB and says
    /// so"). `false` without a running database (recovery mode): no
    /// fresh database was created, and the recovery screen — not this
    /// flag — is the honest state display.
    pub(crate) db_created_fresh: bool,
    /// Whether this launch runs with root privileges on Linux (NFR-023)
    /// — the fact behind the UI warning "the created files will be
    /// owned by root". `false` on every other platform: the spec
    /// assigns the warning to Linux alone (the compile-time branch is
    /// [`running_as_root`]).
    pub(crate) running_as_root: bool,
}

/// Serves the About window's facts (§7.10 System module `app_info`).
///
/// A **non-database** command by construction: every field is a
/// compile-time constant or a managed-state read, so there is no
/// blocking work and nothing to route through [`Db::run`] (SYS-001's
/// executor policy is for DB/file operations; this one has neither —
/// the D-049 `DataRoot` pattern instead). It answers in any mode,
/// recovery included: the About window must not depend on a healthy
/// database, and `db_created_fresh` degrades to `false` there rather
/// than failing the whole sheet.
///
/// The only reachable failure is the missing `DataRoot` state (an
/// uninitialized-app bug — the state is managed before anything that
/// could fail); it answers the D-037 fallback shape with a
/// `DataRootMissing` diagnostic, like the recovery commands.
#[tauri::command]
#[specta::specta]
pub(crate) async fn app_info<R: Runtime>(app: AppHandle<R>) -> Result<AppInfo, AppError> {
    app_info_impl(&app, current_uid()).inspect_err(logging::app_info_failed)
}

/// The body of [`app_info`], split so the one logging edge covers its
/// every failure path (the `activity_list` shape). `uid` is the
/// process's effective id the command read — a parameter (not a read
/// inside) so the tests can drive both halves of the root flag's
/// truth table without becoming root themselves.
pub(crate) fn app_info_impl<R: Runtime>(app: &AppHandle<R>, uid: u32) -> Result<AppInfo, AppError> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        identifier: APP_IDENTIFIER.to_owned(),
        author: env!("CARGO_PKG_AUTHORS").to_owned(),
        license: env!("CARGO_PKG_LICENSE").to_owned(),
        repository: REPOSITORY_URL.to_owned(),
        data_dir: data_root(app)?.display().to_string(),
        db_created_fresh: app
            .try_state::<Db>()
            .is_some_and(|database| database.created_fresh()),
        running_as_root: running_as_root(uid),
    })
}

// ---------------------------------------------------------------------------
// `dialog_pick_folder` (T-073, §7.10 System module, §5.6 SEC-001/SEC-003,
// §5.3, D-009)
// ---------------------------------------------------------------------------

/// Opens the system folder picker from Rust (§7.10 System module
/// `dialog_pick_folder`, the D-009 fixed name; §5.6) and remembers
/// the picked folder for the session: the answer is the picked path
/// in its canonical form (DB-030 — the form the future
/// `project_add_manual`/`scan_start` will register and probe), or
/// `None` when the user cancelled, in which case nothing is
/// remembered.
///
/// SEC-001: the dialog plugin is invoked from this Rust command only
/// — the webview holds no `dialog:` permission
/// (`capabilities/default.json` stays plugin-free), so this command
/// is the single path from the UI to the native picker. That is
/// exactly what lets SEC-003 work: every folder the UI may later name
/// passed through here (and landed in the session allowlist) or is a
/// registered project — [`session_paths::validate_ui_path`] enforces
/// the pair for every path the UI sends.
///
/// The plugin call is the one untestable seam here (it opens a real
/// desktop dialog; the same treatment `app_open_data_folder` gives
/// its opener call): the whole flow lives in
/// [`dialog_pick_folder_with`] behind the `pick` parameter, the
/// production seam is [`plugin_pick_folder`], and the wiring is
/// pinned textually (the registration pin in `ipc_tests.rs`).
#[tauri::command]
#[specta::specta]
pub(crate) async fn dialog_pick_folder<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<String>, AppError> {
    dialog_pick_folder_with(app, plugin_pick_folder::<R>)
        .await
        .inspect_err(logging::dialog_pick_folder_failed)
}

/// The body of [`dialog_pick_folder`], split so the one logging edge
/// covers every failure path (the `activity_list` shape) and so the
/// tests drive the whole flow with their own `pick` closure (the
/// plugin seam, module docs above).
///
/// The picker parks its calling thread for as long as the dialog is
/// open, and canonicalizing the fresh pick is a metadata walk — both
/// are blocking work and run on the blocking pool (SYS-001), never on
/// an async worker. The pick is canonicalized **before** it is
/// remembered and answered, so the allowlist only ever holds canonical
/// forms (SEC-003 compares canonical to canonical); a pick whose
/// folder vanished in that window is
/// [`picked_folder_vanished`]'s typed answer, and nothing is
/// remembered for it.
pub(crate) async fn dialog_pick_folder_with<R, F>(
    app: AppHandle<R>,
    pick: F,
) -> Result<Option<String>, AppError>
where
    R: Runtime,
    F: FnOnce(&AppHandle<R>) -> Result<Option<PathBuf>, AppError> + Send + 'static,
{
    let session_owner = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let picked = pick(&app)?;
        match picked {
            None => Ok(None),
            Some(raw) => eunomia_fs::paths::canonicalize(&raw)
                .map(Some)
                .map_err(|source| picked_folder_vanished(&raw, &source)),
        }
    })
    .await
    .map_err(db::join_failure_to_app_error)??;
    let Some(canonical) = picked else {
        // Cancel: the allowlist must not grow, and the honest answer
        // to the UI is "nothing was picked" (§7.10: "nothing on
        // cancel").
        return Ok(None);
    };
    session_paths::remember_pick(&session_owner, canonical.clone())?;
    Ok(Some(canonical.display().to_string()))
}

/// The production picker seam: the dialog plugin's Rust-side API
/// (SEC-001 — never the webview's), `blocking_pick_folder`, mapped
/// onto the command's internal `Option<PathBuf>` answer. The plugin
/// answer's `FilePath` is converted with `into_path()` (a `file://`
/// URL spelling becomes the path it names; desktop pickers answer
/// plain paths, the conversion is the documented safety net), and a
/// conversion failure takes the [`dialog_error_to_app_error`]
/// fallback. The plugin requires being registered
/// (`tauri_plugin_dialog::init()` in `lib.rs`, pinned by
/// `config_tests.rs`) — an unregistered call would panic in the
/// plugin's state lookup, which is why this seam is never driven in
/// tests.
#[cfg(desktop)]
fn plugin_pick_folder<R: Runtime>(app: &AppHandle<R>) -> Result<Option<PathBuf>, AppError> {
    use tauri_plugin_dialog::DialogExt;
    match app.dialog().file().blocking_pick_folder() {
        None => Ok(None),
        Some(picked) => picked
            .into_path()
            .map(Some)
            .map_err(|error| dialog_error_to_app_error(&error.to_string())),
    }
}

/// The non-desktop twin of [`plugin_pick_folder`] (the
/// `running_as_root` platform-twin pattern): the plugin's folder
/// picker is a desktop API (`#[cfg(desktop)]` on the crate method),
/// and every build this app ships is desktop (the single-instance
/// plugin is desktop-gated the same way). The twin keeps the command
/// compilable for a hypothetical mobile target and answers the
/// recorded fallback instead of opening anything.
#[cfg(not(desktop))]
fn plugin_pick_folder<R: Runtime>(_app: &AppHandle<R>) -> Result<Option<PathBuf>, AppError> {
    Err(dialog_error_to_app_error(
        "the system folder picker is not available on this platform",
    ))
}

/// Maps a dialog-plugin failure to the §7.10 wire form — the
/// `opener_error_to_app_error` shape: the §10 registry has no
/// "dialog failed" code, so the failure lands on the recorded D-037
/// fallback (the least harmful effect — the user retries the button)
/// with the plugin's own text in `details`. Takes the rendered text
/// (not a typed plugin error) because the only reachable typed error
/// is `tauri_plugin_fs::Error` — the plugin re-exports the type from
/// `tauri-plugin-fs`, and naming it would add a direct
/// `tauri-plugin-fs` edge this command does not need.
/// `pub(crate)` for the seam's tests, the opener mapping's shape.
pub(crate) fn dialog_error_to_app_error(error_text: &str) -> AppError {
    let mut app_error = AppError::new(ErrorCode::DbLocked);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("DialogError".to_owned()),
        os_message: Some(error_text.to_owned()),
    });
    app_error
}

/// The §10 mapping for a pick whose folder cannot be canonicalized:
/// the dialog only returns existing folders, so this is the race
/// where the folder moved, was deleted or lost read access between
/// the pick and the bookkeeping — §10's `E_PROJECT_UNAVAILABLE` row
/// ("The project folder moved, was deleted, or cannot be read") is
/// exactly the fact. The picked path travels in `params` (SEC-008:
/// paths may) and the OS refusal in `details`
/// ([`AppError::with_os_error`], §10's first-class log fact). Nothing
/// is remembered for a vanished pick.
fn picked_folder_vanished(
    picked: &Path,
    source: &eunomia_fs::paths::CanonicalizeError,
) -> AppError {
    let mut app_error = AppError::new(ErrorCode::ProjectUnavailable);
    app_error
        .params
        .insert("path".to_owned(), picked.display().to_string());
    app_error.with_os_error(source.io_error())
}

#[cfg(test)]
mod tests {
    //! The seam tests of the pure mappings — the opener mapping and
    //! the `activity_list`/settings store-error mappings are pure, so
    //! they are tested directly; the opener call itself is the
    //! recorded untestable seam (module docs).

    use super::*;
    use crate::errors::ErrorCode;

    #[test]
    fn a_dialog_failure_maps_to_the_fallback_code_with_its_text() {
        // Arrange: the seam takes the plugin error's rendered text
        // (the `tauri-plugin-fs` error type is not nameable without a
        // new dependency edge — see `dialog_error_to_app_error`).

        // Act
        let app_error = dialog_error_to_app_error("failed to convert file URL to path");

        // Assert: the D-037 fallback code, its default key, and the
        // plugin's own text in `details` (nothing free-text in
        // params) — the opener mapping's shape.
        assert_eq!(app_error.code, ErrorCode::DbLocked);
        assert_eq!(app_error.message_key, "errors.db_locked");
        assert!(app_error.params.is_empty());
        let details = app_error.details.expect("the details carry the cause");
        assert_eq!(details.os_kind.as_deref(), Some("DialogError"));
        assert_eq!(
            details.os_message.as_deref(),
            Some("failed to convert file URL to path")
        );
    }

    #[test]
    fn a_vanished_pick_maps_to_project_unavailable_with_the_os_facts() {
        // Arrange: a real canonicalization failure over a path that
        // does not exist — the only shape the pick bookkeeping can
        // fail with (the dialog returns existing folders; this is the
        // vanish race).
        let vanished = Path::new("/definitely/not/here/eunomia-t073");
        let source = eunomia_fs::paths::canonicalize(vanished)
            .expect_err("a missing path must fail canonicalization");

        // Act
        let app_error = picked_folder_vanished(vanished, &source);

        // Assert: §10's E_PROJECT_UNAVAILABLE row, the picked path in
        // params (SEC-008: paths may), and the OS refusal in details.
        assert_eq!(app_error.code, ErrorCode::ProjectUnavailable);
        assert_eq!(app_error.message_key, "errors.project_unavailable");
        assert_eq!(
            app_error.params.get("path").map(String::as_str),
            Some("/definitely/not/here/eunomia-t073")
        );
        let details = app_error.details.expect("the OS refusal travels");
        assert!(details.os_code.is_some(), "the raw OS code is present");
        assert!(details.os_message.is_some());
    }

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

    #[test]
    fn a_settings_write_rejection_maps_to_validation_naming_the_key() {
        // Arrange: the store's own rejection of a write value — the
        // defense-in-depth twin of the command's validation (normally
        // pre-empted by `validated_settings_changes`).
        let stored = StorageError::Settings {
            key: "scan.max_depth".to_owned(),
            source: SettingsError::Range {
                value: 21,
                min: 1,
                max: 20,
            },
        };

        // Act
        let app_error = settings_write_to_app_error(stored);

        // Assert: the honest E_VALIDATION — the write path's Settings
        // failures are about the caller's value — naming the key and
        // the reason (the command's own validation adds the sent
        // value).
        assert_eq!(app_error.code, ErrorCode::Validation);
        assert_eq!(app_error.message_key, "errors.validation");
        assert_eq!(
            app_error.params.get("key").map(String::as_str),
            Some("scan.max_depth")
        );
        assert!(
            app_error
                .params
                .get("reason")
                .is_some_and(|reason| reason.contains("21")),
            "the reason carries the store's own wording, got: {:?}",
            app_error.params
        );
    }

    #[test]
    fn corrupt_settings_cells_map_to_class_only_diagnostics() {
        // Every SettingsError class a stored cell can fail with: the
        // answer is the D-037 fallback with a fixed, class-only
        // diagnostic — never E_VALIDATION (the §10 row would blame the
        // user's input for database corruption) and never the stored
        // value or key in params or os_message (a tampered cell is
        // arbitrary database text; SEC-008's surfaces are exactly
        // those two).
        let cases: [(SettingsError, &str); 5] = [
            (
                SettingsError::Json(serde_json::from_str::<()>("nope").expect_err("must error")),
                "a stored settings value cell is not valid JSON",
            ),
            (
                SettingsError::Type {
                    expected: "integer",
                    found: "string",
                },
                "a stored settings value cell is the wrong JSON type",
            ),
            (
                SettingsError::Range {
                    value: 9999,
                    min: 1,
                    max: 20,
                },
                "a stored settings value cell is outside its key's range",
            ),
            (
                SettingsError::Member {
                    found: "Q".to_owned(),
                },
                "a stored settings value cell is outside its key's vocabulary",
            ),
            (
                SettingsError::Shape {
                    problem: "unknown field \"payload\"".to_owned(),
                },
                "a stored settings value cell has the wrong object shape",
            ),
        ];
        for (source, expected) in cases {
            let app_error = settings_cell_to_app_error(StorageError::Settings {
                key: "tampered.key".to_owned(),
                source,
            });
            assert_eq!(app_error.code, ErrorCode::DbLocked);
            assert_eq!(app_error.message_key, "errors.db_locked");
            assert!(
                app_error.params.is_empty(),
                "no stored cell travels in params"
            );
            let details = app_error.details.expect("the details carry the cause");
            assert_eq!(details.os_kind.as_deref(), Some("SettingsCorrupt"));
            assert_eq!(
                details.os_message.as_deref(),
                Some(expected),
                "the diagnostic names the class only, never the stored value"
            );
        }
    }

    // ---- `projects_list` store-error mapping pins (T-072) --------------

    #[test]
    fn a_corrupt_projects_cell_maps_to_the_fallback_with_its_own_label() {
        // Arrange: the store's corrupt-cells rejection — a stored
        // vocabulary member outside the CHECK vocabulary (the D-039
        // policy; only corruption or a newer app version can produce
        // it, the CHECKs make the API path unreachable).
        let stored = ProjectsError::Member {
            column: "state",
            found: "vanished".to_owned(),
        };

        // Act
        let app_error = projects_store_to_app_error(stored);

        // Assert: the D-037 fallback with the `ProjectsCorrupt`
        // diagnostic — NOT E_VALIDATION (the §10 row would blame the
        // user's input for database corruption) and NOT E_DB_CORRUPT
        // (one bad cell must not force the destructive recovery
        // screen), and no raw cell on any surface the log line or the
        // localized message reads: `params` empty, `os_message` the
        // fixed column-only diagnostic (the T-027 round-2 rule).
        assert_eq!(app_error.code, ErrorCode::DbLocked);
        assert_eq!(app_error.message_key, "errors.db_locked");
        assert!(app_error.params.is_empty(), "no raw cell travels in params");
        let details = app_error.details.expect("the details carry the cause");
        assert_eq!(details.os_kind.as_deref(), Some("ProjectsCorrupt"));
        assert_eq!(
            details.os_message.as_deref(),
            Some("a stored state cell is outside its vocabulary"),
            "the diagnostic names the column and class only"
        );
    }

    #[test]
    fn unreachable_write_outcomes_map_to_the_fallback_without_stored_text() {
        // Arrange: the three write-path outcomes a list read cannot
        // produce, each carrying a stored, path-derived identifier.
        let outcomes = [
            ProjectsError::DuplicatePathKey {
                path_key: "/repos/secret-project".to_owned(),
            },
            ProjectsError::NotFound {
                id: "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned(),
            },
            ProjectsError::ReorderMismatch {
                unknown: vec!["0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned()],
                missing: 1,
                duplicates: 0,
            },
        ];

        // Act
        for outcome in outcomes {
            let app_error = projects_store_to_app_error(outcome);

            // Assert: the same D-037 fallback with the family label and
            // class-only text — the identifier never reaches `params`
            // or `os_message` (SEC-008's surfaces; a stored id or
            // path_key is database text, exactly what the rule keeps
            // off the wire).
            assert_eq!(app_error.code, ErrorCode::DbLocked);
            assert_eq!(app_error.message_key, "errors.db_locked");
            assert!(app_error.params.is_empty());
            let details = app_error.details.expect("the details carry the cause");
            assert_eq!(details.os_kind.as_deref(), Some("ProjectsError"));
            let message = details.os_message.expect("the diagnostic is present");
            assert!(
                !message.contains("/repos/secret-project") && !message.contains("0193a2bb"),
                "no stored identifier travels in the diagnostic, got: {message}"
            );
        }
    }

    // ---- `projects_reorder` seam tests (T-080) -------------------------

    #[test]
    fn a_reorder_splices_the_active_order_around_the_archived_slots() {
        // The composition rule (D-093's table-wide positions): the
        // stored order [A1, Ar1, A2, Ar2, A3] (actives and archived
        // interleaved, archived at their own slots) with the UI's
        // active order [A3, A1, A2] composes to [A3, Ar1, A1, Ar2, A2]
        // — every active slot takes the next input id in sequence, the
        // archived rows keep their exact positions — with exactly the
        // three active positions counted as changed.
        let stored = [
            ("a1", true),
            ("r1", false),
            ("a2", true),
            ("r2", false),
            ("a3", true),
        ];
        let input = ["a3", "a1", "a2"].map(str::to_owned);

        let (permutation, changed) = composed_reorder(&stored, &input).expect("a valid active set");

        assert_eq!(permutation, ["a3", "r1", "a1", "r2", "a2"]);
        assert_eq!(changed, 3, "only the three active slots moved");
    }

    #[test]
    fn a_reorder_that_composes_to_the_stored_order_is_a_verified_noop() {
        // The replayed-drag rule: re-sending the current active order
        // composes to the stored permutation — zero positions change,
        // so the command writes and journals nothing. The single-active
        // and empty-table shapes are the same rule's boundaries (one
        // column cannot move; an empty matrix has nothing to move).
        let stored: [(&str, bool); 3] = [("a1", true), ("r1", false), ("a2", true)];
        let same = ["a1", "a2"].map(str::to_owned);
        let (_, changed) = composed_reorder(&stored, &same).expect("the current order is valid");
        assert_eq!(changed, 0);

        let single = [("only", true)];
        let (_, changed) =
            composed_reorder(&single, &["only".to_owned()]).expect("the single column is valid");
        assert_eq!(changed, 0);

        let empty: [(&str, bool); 0] = [];
        let (permutation, changed) =
            composed_reorder(&empty, &[]).expect("an empty matrix accepts its empty order");
        assert_eq!(permutation, Vec::<&str>::new());
        assert_eq!(changed, 0);
    }

    #[test]
    fn every_reorder_mismatch_form_is_counted_before_any_write() {
        // The three D-093 forms at the command's active-set boundary:
        // an unknown id (never a project), an archived id (a project,
        // but not an active one — the stale-screen shape), a missing
        // active, and a repeated id — each counted, none written.
        let stored = [("a1", true), ("r1", false), ("a2", true)];

        // An unknown id is `unknown` (and the actives it displaced are
        // `missing`).
        let forms = composed_reorder(&stored, &["a1".to_owned(), "ghost".to_owned()])
            .expect_err("an unknown id must be rejected");
        assert_eq!(forms.unknown, ["ghost".to_owned()]);
        assert_eq!(forms.missing, 1, "a2 is absent from the list");
        assert_eq!(forms.duplicates, 0);
        assert!(forms.any());

        // An archived id is the same `unknown` bucket at this boundary:
        // it names no active project.
        let forms = composed_reorder(&stored, &["r1".to_owned(), "a2".to_owned()])
            .expect_err("an archived id must be rejected");
        assert_eq!(forms.unknown, ["r1".to_owned()]);
        assert_eq!(forms.missing, 1, "a1 is absent from the list");
        assert_eq!(forms.duplicates, 0);

        // A repeated id is counted on its own.
        let forms = composed_reorder(
            &stored,
            &["a1".to_owned(), "a2".to_owned(), "a2".to_owned()],
        )
        .expect_err("a repeated id must be rejected");
        assert!(forms.unknown.is_empty());
        assert_eq!(forms.missing, 0);
        assert_eq!(forms.duplicates, 1);

        // The empty list against a non-empty active set is the pure
        // missing form.
        let forms = composed_reorder(&stored, &[]).expect_err("an empty list must be rejected");
        assert!(forms.unknown.is_empty());
        assert_eq!(forms.missing, 2);
        assert_eq!(forms.duplicates, 0);
    }

    #[test]
    fn a_reorder_refusal_carries_the_field_the_offending_ids_and_the_counts() {
        // The `rejected_field` shape, list form: the field names the
        // whole input, the value echoes only the offending CALLER ids
        // (never stored ones — the D-096 rule), and the reason names
        // all three counts so one assertion shape covers every form.
        let forms = ReorderForms {
            unknown: vec!["ghost".to_owned(), "r1".to_owned()],
            missing: 2,
            duplicates: 1,
        };

        let app_error = rejected_reorder(&forms, 5);

        assert_eq!(app_error.code, ErrorCode::Validation);
        assert_eq!(app_error.message_key, "errors.validation");
        assert_eq!(
            app_error.params.get("field").map(String::as_str),
            Some("ordered_ids")
        );
        assert_eq!(
            app_error.params.get("value").map(String::as_str),
            Some("ghost, r1"),
            "the value echoes the offending caller ids"
        );
        assert_eq!(
            app_error.params.get("reason").map(String::as_str),
            Some(
                "the list must name every active project exactly once: \
                 2 not an active project, 2 missing, 1 repeated"
            )
        );

        // The missing-only form: no caller id offends, so the value
        // names what the caller DID send (the empty-list boundary).
        let forms = ReorderForms {
            unknown: Vec::new(),
            missing: 2,
            duplicates: 0,
        };
        let app_error = rejected_reorder(&forms, 0);
        assert_eq!(
            app_error.params.get("value").map(String::as_str),
            Some("0 ids sent")
        );

        // The echo is capped: a buggy caller's unbounded unknown list
        // cannot bloat the envelope or its log line — neither by count…
        let forms = ReorderForms {
            unknown: (0..8).map(|index| format!("ghost-{index}")).collect(),
            missing: 0,
            duplicates: 0,
        };
        let app_error = rejected_reorder(&forms, 8);
        assert_eq!(
            app_error.params.get("value").map(String::as_str),
            Some("ghost-0, ghost-1, ghost-2, ghost-3, ghost-4 (+3 more)")
        );
        // …nor by a single arbitrarily long id (each is cut at 64 code
        // points; a real UUIDv7 id passes intact).
        let forms = ReorderForms {
            unknown: vec!["x".repeat(100)],
            missing: 0,
            duplicates: 0,
        };
        let app_error = rejected_reorder(&forms, 1);
        let expected = "x".repeat(64);
        assert_eq!(
            app_error.params.get("value").map(String::as_str),
            Some(expected.as_str())
        );
    }

    #[test]
    fn the_availability_reasons_carry_the_d084_verdict_classes() {
        // The wire spellings of the D-084 verdicts: every arm of
        // `availability_of` mapped from a real fixture, so a
        // copy-paste regression in the spelling table fails here. The
        // `missing` arm is additionally pinned end-to-end by the IPC
        // test's never-created folder; these fixtures cover the other
        // three.
        use std::fs;
        let temp = tempfile::tempdir().expect("a temp directory must be creatable");

        // A healthy repository: available, no reason.
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join(".git")).expect("the fixture repository must be creatable");
        assert_eq!(availability_of(&repo), (true, None));

        // A readable folder without `.git`: NoGitEntry.
        let plain = temp.path().join("plain");
        fs::create_dir_all(&plain).expect("the plain folder must be creatable");
        assert_eq!(
            availability_of(&plain),
            (false, Some("no_git_entry".to_owned()))
        );

        // A plain file at the registered path: NotADirectory.
        let file = temp.path().join("file");
        fs::write(&file, b"not a folder").expect("the fixture file must be creatable");
        assert_eq!(
            availability_of(&file),
            (false, Some("not_a_directory".to_owned()))
        );

        // A folder that cannot be listed: NotReadable — the mode-bit
        // fixture the eunomia-fs suite uses, root-gated the same way
        // (mode bits are not enforced for root, so the precondition
        // cannot be set up there).
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let locked = temp.path().join("locked");
            fs::create_dir_all(&locked).expect("the locked fixture must be creatable");
            let untestable = fs::metadata(&locked).is_ok_and(|metadata| metadata.uid() == 0);
            if untestable {
                eprintln!(
                    "skipping the not_readable arm: running as root — mode \
                     bits are not enforced, the precondition cannot be set up"
                );
            } else {
                fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))
                    .expect("removing all mode bits must work");
                let verdict = availability_of(&locked);
                // Restore even when the assertion below panics, so
                // TempDir's teardown can remove the directory.
                fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))
                    .expect("restoring mode bits must work");
                assert_eq!(
                    verdict,
                    (false, Some("not_readable".to_owned())),
                    "a 000-mode folder must map to the not_readable reason"
                );
            }
        }
    }

    // ---- `project_add_manual` seam tests (T-074) -----------------------

    #[test]
    fn a_negative_root_lookup_is_e_not_a_repo_with_the_picked_path() {
        // Arrange: the walk's negative verdict — every ancestor probed,
        // no `.git` anywhere (PRJ-003/D-08). No OS call failed, so the
        // §10 row's own situation: "Project not added … Pick the
        // repository root".
        let source = NotARepoError::NoGitEntry {
            path: "/home/user/plain".to_owned(),
        };

        // Act
        let app_error = repo_lookup_failed(&source);

        // Assert: E_NOT_A_REPO with its default key, the walked path in
        // params (SEC-008: paths may), and no details — a fact about the
        // tree, not an OS failure.
        assert_eq!(app_error.code, ErrorCode::NotARepo);
        assert_eq!(app_error.message_key, "errors.not_a_repo");
        assert_eq!(
            app_error.params.get("path").map(String::as_str),
            Some("/home/user/plain")
        );
        assert!(app_error.details.is_none());
    }

    #[test]
    fn a_refused_root_probe_is_project_unavailable_with_the_os_facts() {
        // Arrange: the walk's refused probe — "could not check" is not
        // "checked and absent" (D-090), so the §10 row is
        // E_PROJECT_UNAVAILABLE, not E_NOT_A_REPO. Both io-backed
        // variants of the walk share the arm; the Canonicalize one is
        // the vanish race the pick itself can also hit (D-100).
        let refused = NotARepoError::NotReadable {
            path: "/home/user/locked".to_owned(),
            source: std::io::Error::from_raw_os_error(13),
        };
        let vanished = NotARepoError::Canonicalize {
            source: eunomia_fs::paths::canonicalize(Path::new("/definitely/not/here/t074"))
                .expect_err("a missing path must fail canonicalization"),
        };

        for source in [&refused, &vanished] {
            // Act
            let app_error = repo_lookup_failed(source);

            // Assert: the §10 row, the walked path in params, and the OS
            // refusal in details (§10's first-class log fact).
            assert_eq!(app_error.code, ErrorCode::ProjectUnavailable);
            assert_eq!(app_error.message_key, "errors.project_unavailable");
            assert!(app_error.params.contains_key("path"));
            let details = app_error.details.expect("the OS refusal travels");
            assert!(details.os_code.is_some(), "the raw OS code is present");
        }
    }

    #[test]
    fn the_derived_name_is_the_root_folder_name_section_69_validated() {
        // The S1 default: the repository root's own folder name, through
        // the core validator (trim, 1–100 code points). The edges a
        // canonical root can actually produce: a plain name, a padded
        // one (a folder literally named with edge spaces — legal on
        // Unix), a 100/101-character name, and the filesystem root
        // itself (no file name at all → the honest E_VALIDATION, since
        // the schema CHECKs would reject an empty name anyway).
        assert_eq!(
            derived_name(Path::new("/home/user/eunomia-repo")).unwrap(),
            "eunomia-repo"
        );
        assert_eq!(derived_name(Path::new("/home/user/ pad ")).unwrap(), "pad");
        let hundred = "r".repeat(100);
        assert_eq!(
            derived_name(Path::new("/home/user").join(&hundred).as_path()).unwrap(),
            hundred
        );
        let too_long = derived_name(Path::new("/home/user").join("r".repeat(101)).as_path())
            .expect_err("a 101-character folder name must be rejected");
        assert_eq!(too_long.code, ErrorCode::Validation);
        assert_eq!(
            too_long.params.get("field").map(String::as_str),
            Some("name")
        );
        assert!(
            too_long
                .params
                .get("reason")
                .is_some_and(|reason| reason.contains("101")),
            "the reason carries the validator's own wording: {:?}",
            too_long.params
        );
        let nameless = derived_name(Path::new("/")).expect_err("the filesystem root has no name");
        assert_eq!(nameless.code, ErrorCode::Validation);
        assert_eq!(
            nameless.params.get("field").map(String::as_str),
            Some("name")
        );
    }

    #[test]
    fn write_access_verdicts_map_to_warning_classes_and_refusals_stay_silent() {
        // The §6.9 warning classes on the wire: the two rights verdicts
        // (T-063/D-094) name their class; a refused probe answers None
        // — "could not check" is not "no rights" (the D-090 split), and
        // the sync preflight re-checks where a refusal is a skip.
        let folder = WriteAccessError::FolderNotWritable {
            path: "/repo/AGENTS.md".to_owned(),
            source: std::io::Error::from_raw_os_error(13),
        };
        assert_eq!(
            write_access_warning(&folder).as_deref(),
            Some("folder_not_writable")
        );
        let file = WriteAccessError::FileReadOnly {
            path: "/repo/AGENTS.md".to_owned(),
        };
        assert_eq!(
            write_access_warning(&file).as_deref(),
            Some("file_read_only")
        );
        let refused = WriteAccessError::ProbeFailed {
            path: "/repo/AGENTS.md".to_owned(),
            source: std::io::Error::other("vanished mid-probe"),
        };
        assert_eq!(write_access_warning(&refused), None);
    }

    // ---- `app_info` field-source pins (T-031) --------------------------

    /// Parses the bundled `tauri.conf.json` — the config tauri compiles
    /// into the binary — for the drift pins below.
    fn bundled_config() -> serde_json::Value {
        serde_json::from_str(include_str!("../tauri.conf.json"))
            .expect("tauri.conf.json must parse as JSON")
    }

    #[test]
    fn the_identifier_literal_matches_the_bundled_app_config() {
        // `APP_IDENTIFIER` is a spec literal (UPD-005/REL-004), while the
        // config field is what actually locates the data directory
        // (D-020); this pin keeps the two from drifting apart silently —
        // changing either one alone fails here.
        let config = bundled_config();
        assert_eq!(
            config["identifier"],
            serde_json::json!(APP_IDENTIFIER),
            "the app_info identifier literal and the tauri.conf.json \
             identifier must agree (D-020)"
        );
    }

    #[test]
    fn the_crate_version_matches_the_bundled_app_config() {
        // `app_info` serves `CARGO_PKG_VERSION` while the bundler and the
        // updater read `tauri.conf.json`'s `version` — two declarations of
        // one fact that must not drift: a bump of either alone fails here.
        let config = bundled_config();
        assert_eq!(
            config["version"],
            serde_json::json!(env!("CARGO_PKG_VERSION")),
            "the src-tauri crate version and the tauri.conf.json version \
             must agree"
        );
    }

    // ---- `running_as_root` seam tests (T-032, NFR-023) ---------------

    #[test]
    fn uid_zero_is_root_and_regular_uids_are_not() {
        // The card's acceptance pair ("under root true, under a regular
        // user false"), driven through the seam — the test suite cannot
        // become root itself. Uid 0 is root on every Unix; 1000 is the
        // first regular user on a default Linux install; 65534 is
        // `nobody`, the largest typical uid — a one-comparison
        // predicate's whole truth table is its boundary values.
        assert!(is_root(0), "uid 0 is root");
        assert!(!is_root(1000), "a regular user's uid is not root");
        assert!(!is_root(65534), "`nobody`'s uid is not root");
    }

    // The wiring pin that used to sit here (`running_as_root() ==
    // is_root(current_uid())`) was vacuous in an unprivileged run —
    // both sides read false, so a hardcoded-false flag passed it. The
    // round-1 review replaced it: `app_info_impl` now takes the uid as
    // a parameter, and the IPC tests drive both halves with chosen
    // uids (see `the_sheets_root_flag_answers_the_uid`), which a
    // hardcoded flag fails in either direction.

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn the_flag_is_false_off_linux() {
        // NFR-023's platform scope, pinned where it applies: outside
        // Linux the flag is false by the compile-time branch. This
        // test runs only under a non-Linux test build — today's CI
        // matrix builds but does not test the Windows/macOS legs, so
        // it is dormant on this machine and the Linux CI (recorded in
        // the task's DECISIONS.md entry); it exists to pin the
        // semantics for any future non-Linux test run.
        assert!(
            !running_as_root(0),
            "NFR-023 assigns the root warning to Linux alone — even uid 0 \
             does not raise it elsewhere"
        );
    }
}
