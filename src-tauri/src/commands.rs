//! The IPC commands (spec §7.10) — the recovery-screen group of the
//! System module (D-009), the History module's `activity_list`
//! (T-027), the Settings module's `settings_get`/`settings_set`
//! (T-030), the System module's `app_info` (T-031), the Library
//! module's `markdown_preview` (T-056), the Projects module's
//! `projects_list` (T-072), the System module's
//! `dialog_pick_folder` (T-073), the Projects module's
//! `project_add_manual` (T-074), the Projects module's
//! `exclusions_list`/`exclusion_remove` (T-078), the Projects
//! module's `projects_reorder` (T-080), the Sync module's
//! `sync_run` (T-118, D-138), the Library module's `library_get`
//! (T-083, D-187) and the Library module's instruction
//! lifecycle `instruction_create`/`instruction_update`/
//! `instruction_hide`/`instruction_restore` (T-328, D-188):
//! `app_open_data_folder`, `db_backups_list`, `db_restore_backup`,
//! `db_recreate`, `activity_list`, `settings_get`, `settings_set`,
//! `app_info`, `markdown_preview`, `projects_list`,
//! `dialog_pick_folder`, `project_add_manual`, `exclusions_list`,
//! `exclusion_remove`, `projects_reorder`, `sync_run`,
//! `library_get`, `instruction_create`, `instruction_update`,
//! `instruction_hide`, `instruction_restore`.
//!
//! This module also owns the rules a NEW project row is built by —
//! the derived §6.9 fields, the PRJ-012 copy of the `default.*`
//! settings, D-29's color and the `project.added` journal entry
//! ([`ProjectFields`], [`WriteDefaults`], [`ColorOccupancy`],
//! [`register_project`]) — since T-077: `project_add_manual` and the
//! scan module's `scan_commit` (PRJ-002's "Add selected", `scan.rs`)
//! both call that ONE implementation, so the two add flows cannot
//! drift apart (D-179).
//!
//! Since T-105 the project-*changing* commands (`project_add_manual`,
//! `exclusion_remove`, `projects_reorder`, and `scan_commit` since
//! T-077) run the SYS-005 gate first: while a sync holds the global
//! `sync_running` flag (SYS-004, `sync_running.rs`), they answer the
//! typed `E_SYNC_RUNNING` refusal and write nothing — see
//! [`reject_during_sync`].
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
//! recreate/restore the command itself performs the reopen — `Db::open`
//! over the replaced path — and then runs
//! [`crate::start_database_session`], the tail the `setup` hook runs
//! over a healthy launch: the §9.4 sweep, the DB-022 commit hook, the
//! ERR-001 daily copy, `app.manage(database)`, and the state services
//! over the published database (dirty recalculation, target-file
//! watch, availability startup pass, daily retention). The §7.10
//! wording "replaces it with the chosen copy, and reopens it", so the
//! app continues without a restart — armed like a launch, which until
//! T-322 it was not: the recovery published a database and nothing
//! else, and the session ran without its services until a restart.
//!
//! # SYS-001
//!
//! The file work, the reopen and the session tail run inside
//! `tauri::async_runtime::spawn_blocking` — the same executor policy
//! as `db::Db::run` (D-037): quick_check scans and file copies are
//! blocking I/O and never run on the async workers, and the tail's
//! ERR-001 daily copy is a `VACUUM INTO` — a file copy too.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

use jiff::Timestamp;

use crate::availability;
use crate::data_dirs;
use crate::db::{self, Db};
use crate::dirty_recalc::DirtyState;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::file_watch;
use crate::logging;
use crate::markdown;
use crate::recovery::{self, DbBackupInfo};
use crate::session_paths;
use crate::sync_phase0;
use crate::sync_phase1;
use crate::sync_phase2;
use crate::sync_phase3;
use crate::sync_phase4;
use crate::sync_running;

use eunomia_core::gaps::heading_level_gaps;
use eunomia_core::library::{
    Heading as CoreHeading, HeadingLevel, Instruction as CoreInstruction, InstructionState, ItemId,
    LibraryItem as CoreLibraryItem,
};
use eunomia_core::library_validators::{
    LibraryFieldError, validate_instruction_title, validate_variant_text,
};
use eunomia_core::markers::{BEGIN_MARKER, END_MARKER, MarkerLine};
use eunomia_core::project::{Color, ProjectId};
use eunomia_core::version_snapshot::{
    instruction_snapshot, instruction_snapshot_hash, variant_snapshot, variant_snapshot_hash,
};
use eunomia_fs::atomic::{AtomicWriteError, WrittenFile};
use eunomia_fs::backup::{BackupError, BackupOutcome, BackupTarget};
use eunomia_fs::paths::NotARepoError;
use eunomia_fs::write_access::WriteAccessError;
use eunomia_store::action_log::{
    Action, ActionLogError, ActivityQuery, Details, Entity, EntityType, Level, LogEntry, Outcome,
    Visibility, insert,
};
use eunomia_store::library::{
    ItemKind, ItemScope, ItemState, ItemUpdate, LibraryError, LibraryItem, NewLibraryItem,
    NewVariant, Variant,
};
use eunomia_store::library_versions::{
    InstructionOp, Recorded, RevisionError, VariantOp, record_instruction, record_variant,
};
use eunomia_store::projects::{ListScope, ProjectState, ProjectsError};
use eunomia_store::settings::{SettingKey, SettingValue, SettingsError};
use eunomia_store::storage::StorageError;
use eunomia_store::sync_runs::abort_run;

/// Serializes the two destructive recovery commands against each
/// other: both end in the shared session tail, which publishes the
/// reopened database with `app.manage(database)`, and tauri's
/// `manage` keeps the FIRST state of a type and answers `false` for a
/// duplicate — it neither replaces nor panics (2.12.6, `src/state.rs`
/// `StateManager::set`). Without the gate two concurrent calls would
/// interleave their preserve/replace/reopen steps over the one
/// database path, and only whichever reached the publication first
/// would have its handle managed — the other's reopen, silently
/// discarded. A caller that finds the gate locked gets a validation
/// error ("recovery already in progress") instead. The gate is taken
/// BEFORE the recovery-mode guard, which then runs under it: a guard
/// is a fact only at the moment it is checked, and only the gate keeps
/// that moment and the tail in one uninterrupted region.
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
/// `Restore`, the selection the UI made. Moved into the blocking
/// closure (`'static`): the file mechanics and the audit lines that
/// name the selection both run there.
enum Recovery {
    Recreate,
    Restore { backup_name: String },
}

/// The shared body of `db_recreate` and `db_restore_backup`: guard,
/// blocking file work + reopen, then the session tail every open path
/// runs — [`crate::start_database_session`], the function the `setup`
/// hook runs over a healthy launch (T-322). Until it was shared, the
/// recovery mirrored only the sweep, the daily copy and the
/// publication and armed none of the state services: the recovered
/// session ran without a commit hook, a dirty service, a watcher, an
/// availability startup pass or a retention task until a restart.
async fn run_recovery<R: Runtime>(app: &AppHandle<R>, operation: Recovery) -> Result<(), AppError> {
    // Serialize concurrent calls FIRST (see `RECOVERY_OPERATION`); the
    // locked-out caller gets the same shape of honest rejection. The
    // gate deliberately precedes the recovery-mode guard below: the
    // guard is a check, and a check is a fact only at the moment it is
    // made. Under the old order, the guard checked first, and that
    // left a window — a restore that passed it, waited, and then took
    // the gate after a concurrent recovery had
    // finished would preserve that recovery's WORKING database away,
    // publish nothing (the tail manages no second `Db`), and answer
    // success while the session's handle kept pointing at the file
    // just renamed away; only `db_recreate`'s own healthy-file guard
    // stood in that window's way (T-322's round-1 review, P2-1). Under
    // the gate no other recovery can exist between the check and the
    // tail.
    let _guard = RECOVERY_OPERATION.try_lock().map_err(|_| {
        let error = rejected("a recovery operation is already in progress");
        logging::recovery_failed(&error);
        error
    })?;
    // Recovery-mode guard, held with the gate for the whole operation:
    // with a healthy database managed there is nothing to recover, and
    // proceeding would rename a *working* database away — the §10
    // screen that owns these buttons is not shown in that state, so
    // the call is a UI bug or a stale screen, and a validation
    // rejection is the honest answer. The same guard is what keeps the
    // session's one-shot service start one-shot: a retry from a stale
    // screen never reaches the tail.
    if app.try_state::<Db>().is_some() {
        let error = rejected("a database is already running");
        logging::recovery_failed(&error);
        return Err(error);
    }

    let data_dir = data_root(app).inspect_err(logging::recovery_failed)?;
    let tail_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        // The step orders are `recovery::recreate_files`/`restore_files`
        // themselves (the seams the behavior tests pin); the restore
        // audit line rides the `on_preserved` callback into the window
        // between the preserve and the publish — after the damaged
        // file has moved, before anything replaced it — so a crash
        // between the two renames still leaves the "where did my old
        // database go?" trail in the log (the facade is usable from a
        // blocking thread; tracing is global).
        let preserved = match &operation {
            Recovery::Recreate => recovery::recreate_files(&data_dir, Timestamp::now())?,
            Recovery::Restore { backup_name } => {
                recovery::restore_files(&data_dir, backup_name, Timestamp::now(), |preserved| {
                    logging::recovery_preserved(&preserved.from, &preserved.to)
                })?
            }
        };
        if let Recovery::Recreate = &operation
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
        let database = Db::open(&data_dir).map_err(recovery::RecoveryError::from)?;
        let db_path = data_dirs::db_file_path(&data_dir);
        match &operation {
            // The audit lines carry both paths: where the damaged
            // database went, and what the database is now (the copy's
            // own path is derivable from the name the UI sent, which
            // the failure paths log through their params). Emitted
            // right after the reopen and before the tail, as before.
            Recovery::Recreate => logging::recovery_recreated(&db_path),
            Recovery::Restore { backup_name } => logging::recovery_restored(
                &data_dirs::db_backups_dir(&data_dir).join(backup_name),
                &db_path,
            ),
        }
        // The session tail, inside the blocking closure with the rest
        // of the file work (SYS-001): the §9.4 sweep, the commit hook,
        // the ERR-001 daily copy (a `VACUUM INTO` — a file copy), the
        // publication and the state services — the same function, in
        // the same order, as a healthy startup. The tail is infallible
        // (every refusal degrades through the facade), so the command
        // answers success exactly when the database is published AND
        // armed; a failure above leaves nothing published and the
        // parked start untouched for the user's next attempt (T-322).
        crate::start_database_session(&tail_app, database);
        Ok::<_, recovery::RecoveryError>(())
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            let error = error.app_error();
            logging::recovery_failed(&error);
            Err(error)
        }
        Err(join_error) => {
            let error = db::join_failure_to_app_error(join_error);
            logging::recovery_failed(&error);
            Err(error)
        }
    }
}

/// Opens `path` in the OS file manager through the opener plugin's
/// Rust API (SEC-001: the call lives in Rust; the webview has no
/// `opener:` permission — `capabilities/default.json` grants only
/// the event-subscription pair `core:event:allow-listen`/`-unlisten`).
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

/// The §7.10 envelope of the `E_SYNC_RUNNING` refusal (§10's row:
/// "A second run, or a project edit during a sync" → "Command
/// rejected"): the code with its default `message_key`, empty params
/// and no details. Empty on purpose: the §10 row names no path,
/// project or field for this situation — the message is the same
/// "a sync is in progress, wait for it to finish" whatever the
/// command was — and there is no OS fact to attach (the refusal is a
/// policy answer, not a failure of the system). `pub(crate)`: the
/// IPC tests tie this envelope to the seam refusal the sync pipeline
/// (T-112+) will render the same way.
pub(crate) fn sync_running_refusal() -> AppError {
    AppError::new(ErrorCode::SyncRunning)
}

/// The managed global flag, with the honest missing-state answer (the
/// `managed_db`/`managed_session` shape): the state is managed by
/// `setup` before anything that could fail, so its absence is an
/// uninitialized-app bug — the recorded D-037 fallback with a
/// diagnostic naming it, not a panic, and not a silently-permissive
/// gate. The returned [`tauri::State`] derefs to `&SyncRunning`, so
/// callers use the seam directly. `pub(crate)`: the `sync_status`
/// command (T-121, `sync_reads.rs`) reads the flag through the same
/// fallback-carrying lookup — one truth with the gate.
pub(crate) fn managed_sync_flag<'a, R: Runtime>(
    app: &'a AppHandle<R>,
) -> Result<tauri::State<'a, sync_running::SyncRunning>, AppError> {
    app.try_state::<sync_running::SyncRunning>().ok_or_else(|| {
        let mut error = AppError::new(ErrorCode::DbLocked);
        error.details = Some(ErrorDetails {
            os_code: None,
            os_kind: Some("SyncRunningMissing".to_owned()),
            os_message: Some("the sync-running flag was not managed at startup".to_owned()),
        });
        error
    })
}

/// The SYS-004 start seam: what the `sync_run` command (T-118/D-138)
/// calls as §9.4 phase 0 step 1 — "atomically take
/// `sync_running`; otherwise `E_SYNC_RUNNING`". `Ok` carries the
/// [`sync_running::SyncGuard`] the pipeline holds for the whole run,
/// snapshot to summary (phase 4 step 5's "Reset `sync_running`" is
/// the guard's drop); `Err` is the typed refusal already rendered as
/// the §7.10 envelope through [`sync_running_refusal`] — the one
/// mapping from the seam's [`sync_running::AlreadyRunning`] marker,
/// so the second start's wire answer and the commands' gate answer
/// cannot drift apart (deliberately not a `From`: the errors.rs
/// no-blanket-conversion rule). The phase-0 snapshot the run then
/// builds MUST be one `Db::run` read transaction — the
/// serialization argument in `sync_running.rs`'s module docs rests
/// on it.
pub(crate) fn begin_sync<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<sync_running::SyncGuard, AppError> {
    // The marker carries no payload; the envelope is the whole wire
    // answer (see `sync_running_refusal`).
    managed_sync_flag(app)?
        .try_acquire()
        .map_err(|_| sync_running_refusal())
}

/// The SYS-005 gate every project-*changing* command runs as its
/// first act: while a sync holds the global `sync_running` flag
/// (SYS-004), the command answers the typed `E_SYNC_RUNNING`
/// refusal instead of mutating — "commands that change projects …
/// return `E_SYNC_RUNNING` during a sync". Today that is
/// [`project_add_manual`], [`exclusion_remove`],
/// [`projects_reorder`], — since T-077, it creates project rows —
/// `scan_commit` (`scan.rs`), — since T-108 —
/// [`crate::project_archive::project_archive`], — since T-109 —
/// [`crate::project_restore::project_restore`], — since T-110 —
/// [`crate::project_relink::project_relink`], and — since T-106 —
/// [`crate::project_update::project_update`]: the mutation commands
/// that exist; the
/// spec's `project_remove` and P1 `backup_restore` join this gate as
/// they land. Reads are never
/// gated (SYS-005 names the changing commands only).
///
/// A **check**, not an acquire: only the sync pipeline holds the
/// guard ([`begin_sync`], `sync_running.rs` module docs) — the flag
/// means "a sync is running", and a mutation holding it would make
/// the status lie and block a sync start behind an edit. The check
/// is a plain atomic load, deliberately the cheapest act of the
/// command: during a sync the refusal is the answer whatever the
/// input is (the mode gate precedes per-input validation — SEC-003
/// for the add flow — which a foreign path still never passes, it
/// just meets the mode answer first), and a command that slips in
/// just before a sync starts commits cleanly on either side of the
/// snapshot: the D-037 executor serializes it against the snapshot
/// read on the one connection.
///
/// `pub(crate)`: the scan commit (`scan.rs`) runs the same gate as
/// its first act.
pub(crate) fn reject_during_sync<R: Runtime>(app: &AppHandle<R>) -> Result<(), AppError> {
    if managed_sync_flag(app)?.is_running() {
        return Err(sync_running_refusal());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `sync_run` (T-118, §7.10 Sync module, §9.4 the pipeline, D-25/D-130…D-134)
// ---------------------------------------------------------------------------

/// The `sync_run` command's wire answer (§7.10 Sync module, §9.4 phase 1
/// step 6 and phase 4 step 5): either the D-25 confirmation request, or
/// the finished run's summary.
///
/// The two variants are the command's only outcomes besides `Err`:
/// a run that reaches phase 2 cannot be stopped (§9.4 phase 2 step 3),
/// so there is no "cancelled while writing" member — a cancel lands
/// before the backups and comes back as the `aborted` verdict of
/// [`SyncRunOutcome::Completed`], the same verdict a `success`/
/// `partial`/`failed` run answers with (the store's §6.2 vocabulary
/// literal, read back from the run row).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub(crate) enum SyncRunOutcome {
    /// §9.4 phase 1 step 6 (D-25): the build is empty over a non-empty
    /// file for at least one project, `confirm_empty` was not `true`,
    /// and the run stopped before the backups — nothing was written,
    /// no backup was made, the flag is free, and the UI re-calls the
    /// command with `confirm_empty=true` after the user confirms (a
    /// full new run) or simply does not (declining cancels the whole
    /// run — this run's row is already terminal).
    NeedsConfirmation {
        /// The ids of the projects awaiting confirmation, in
        /// `sort_order` — [`Phase1Outcome::needs_confirmation`]'s
        /// list. Ids, not names: the dialog maps them through the
        /// projects the matrix already renders, so no project name
        /// text rides the wire twice.
        projects: Vec<String>,
    },
    /// The run reached a terminal status (`success`/`partial`/`failed`
    /// — phase 4's counted verdict — or `aborted` — a stop before the
    /// backups), and the summary is the committed rows read back.
    Completed { summary: SyncRunSummary },
}

/// One finished (or stopped) sync run's summary — §9.4's verdict with
/// MAIN-024's counter chips and per-project rows, read back from the
/// run's committed rows (the same counted truth [`finish_run`] wrote)
/// so the answer cannot disagree with the database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct SyncRunSummary {
    /// The run row's id (UUIDv7 TEXT, DB-002).
    pub(crate) run_id: String,
    /// The run's verdict — the store's §6.2 status literal
    /// (`success`/`partial`/`failed`/`aborted`; `interrupted` reaches
    /// this DTO only through the cold-start reader, `sync_last_result`
    /// over a crash-recovered attempt's row — T-121).
    pub(crate) verdict: String,
    /// When the run started (DB-002), the flag-take moment phase 0
    /// recorded.
    pub(crate) started_at: String,
    /// When the run reached its terminal status (DB-002); `None` never
    /// reaches this DTO (a terminal row always carries it).
    pub(crate) finished_at: Option<String>,
    /// The per-status counters, exactly the row's cells: MAIN-024's
    /// "Written", "Unchanged", "Skipped", "Failed" chips. `i32` on the
    /// wire (specta forbids BigInt-style types); the store's i64
    /// counts of projects always fit.
    pub(crate) projects_total: i32,
    pub(crate) projects_written: i32,
    pub(crate) projects_unchanged: i32,
    pub(crate) projects_skipped: i32,
    pub(crate) projects_failed: i32,
    /// The run row's free-text `details` cell — the §9.4 empty-run
    /// message on a `success` over no active projects; the abort
    /// lines this command writes on its stopped-run paths.
    pub(crate) details: Option<String>,
    /// Every project's outcome, in `sort_order` — the skip and error
    /// rows MAIN-024 renders with their `E_*` code.
    pub(crate) projects: Vec<SyncRunProjectEntry>,
}

/// One project's outcome inside the summary — the `sync_run_projects`
/// row's cells on the wire (§6.2 status literal + the §10 code of a
/// refusal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct SyncRunProjectEntry {
    /// The project row id.
    pub(crate) project_id: String,
    /// The row's status literal (`written`/`unchanged`/`skipped`/
    /// `failed`/`not_started`).
    pub(crate) status: String,
    /// The target path phase 0 built for (§9.4 phase 1 step 2's
    /// spelling, before symlink resolution).
    pub(crate) target_path: Option<String>,
    /// The §10 code of a `skipped`/`failed` outcome.
    pub(crate) error_code: Option<String>,
    /// The short, class-only message of a refusal (paths and codes
    /// only, never file content — SEC-008).
    pub(crate) error_message: Option<String>,
}

/// The cooperative cancel seam (§9.4's "declining cancels the whole
/// run" and the `sync_cancel` entry of §7.10): a shared atomic flag the
/// pipeline consults at its ONE cancellation point — after the D-25
/// gate, before phase 2 ("After phase 2 succeeds, the run cannot be
/// canceled", §9.4 phase 2 step 3). Minimal by the card's ruling: a
/// bool, not a channel — the phases between the check and phase 2 are
/// already running, and everything after phase 2 is uncancellable by
/// spec, so a latched flag checked once is the whole policy.
///
/// The token the command drives the pipeline with is PUBLISHED for
/// the run's lifetime through the managed [`SyncCancelSlot`] (T-258,
/// D-138's recorded follow-up landed), so the `sync_cancel` command
/// (T-120) reaches this same latch; the check point and the abort
/// shape it triggers land here, tested through [`sync_run_with`].
#[derive(Debug, Clone, Default)]
pub(crate) struct CancelToken {
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl CancelToken {
    /// A token nobody has cancelled.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Latches the cancel request (Release: the pipeline's check must
    /// not read it early). The `sync_cancel` command's act (T-120),
    /// over the clone the slot publishes.
    pub(crate) fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Whether a cancel was requested before the check point (Acquire,
    /// the flag seam's own ordering pair).
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Test-support identity probe for [`CancelToken`](CancelToken):
/// whether two tokens share one latch (the `Arc` inside) — the
/// discriminator that pins the managed slot's published token to the
/// very latch the pipeline's ONE check point reads, which no
/// behavioral outcome can (a post-point cancel is a no-op on any
/// token). Exists only under `cfg(test)`; no production caller can
/// reach it (the `Db::set_busy_timeout_for_tests` shape).
#[cfg(test)]
impl CancelToken {
    pub(crate) fn shares_latch(&self, other: &CancelToken) -> bool {
        Arc::ptr_eq(&self.cancelled, &other.cancelled)
    }
}

/// The managed slot publishing the LIVE run's cancel token (D-138's
/// recorded follow-up, landed here — T-258): [`sync_run_with`]
/// publishes the token it drives the pipeline with at the run's start
/// and clears it at the run's end, so the `sync_cancel` command
/// (§7.10, T-120) reaches the ONE cooperative cancel point through
/// the same latched token the pipeline consults (§9.4 phase 2
/// step 3's point of no return) — no second cancel mechanism exists
/// or is needed: `token().cancel()` IS the cancel, and its semantics
/// are D-138's law unchanged (a latch before the check point stops
/// the run before the backups; a latch after it is the no-op the
/// uncancellable tail already defines).
///
/// **The lifecycle — one mechanism, the [`sync_running::SyncGuard`]
/// shape (D-123).**
///
/// | moment | the slot holds |
/// | ------ | -------------- |
/// | no run live (fresh app, between runs, recovery mode) | `None` |
/// | `sync_run_with` entered, before the phase-0 flag take | the start's token (`publish`: set-if-empty) |
/// | phase 0 answered `Ok` — the flag is held | the flag holder's token (`publish_owned`, unconditional: the pre-take race's loser must not stay published over the live run — the round-1 P2-1 fix) |
/// | the run runs (phases 0–4) | the run's token — a refused second start publishes nothing and clears nothing (its token is not current) |
/// | the run answered — every exit: the finish, the D-25 stop, the cancel stop, the P3-4 wrapper's `Err`, a panic's unwind | `None` again (the [`CancelLease`] RAII drop's `clear_if_current`; the finish path drops the lease beside the guard, before the never-fatal aftermath — the D-138 P2-2 shape) |
/// | the process dies mid-run | the slot dies with it (in-memory state); the corpse row is the startup `interrupted` recovery's case (§9.4, T-258) |
///
/// The publication is TWO steps by construction: the pre-take
/// `publish` (set-if-empty) closes the missed-cancel window — a
/// cancel latched while phase 0 builds its snapshot is honored at the
/// check point — and the post-take `publish_owned` (unconditional,
/// the flag holder alone) reconciles the two races that decide the
/// slot and the run (the round-1 P2-1 fix): a start that pre-published
/// but lost the flag would otherwise stay published over the winner,
/// and its lease's exit would empty the slot mid-run. `publish` being
/// set-if-empty means a second start can never evict a live token,
/// and `clear_if_current` clears only the lease's OWN latch, so no
/// run's exit can unpublish another's. A stale token cannot exist in
/// a live process: every exit drops the lease, and a release-build
/// panic aborts the process (the slot dies with it, the flag's own
/// `panic = "abort"` caveat).
///
/// Managed by `setup` in every mode (recovery included) beside
/// [`sync_running::SyncRunning`], for the same reason the flag is:
/// the `sync_cancel` command (T-120) must FIND the state in every
/// mode — in recovery mode it simply stays empty (a sync refuses at
/// phase 0).
#[derive(Debug, Default)]
pub(crate) struct SyncCancelSlot {
    /// `std::sync::Mutex` — the guard never crosses an await point
    /// (each method body is one lock); `None` means no run is live.
    token: std::sync::Mutex<Option<CancelToken>>,
    /// The point-of-no-return KNOWLEDGE (T-120, D-210): `true` from
    /// the moment the live run's ONE consultation fell through (the
    /// check read not-cancelled — the run enters §9.4 phase 2's
    /// uncancellable tail) until the lease clears the slot. An
    /// `AtomicBool` beside the token, not inside the mutex: the
    /// pipeline marks it once, the command reads it once, and
    /// neither takes the other's lock. KNOWLEDGE ONLY — the pipeline
    /// never reads it, no pipeline branch consults it, and it latches
    /// nothing: the single cancellation channel stays the token
    /// (D-138's law, the pinned T-284 path). Reset together with the
    /// token by `clear_if_current`, so the flag is always a fact
    /// about the run whose token is published (only the flag holder
    /// marks — a losing start never reaches a consultation — and the
    /// flag holder's lease always clears).
    past_point: std::sync::atomic::AtomicBool,
}

impl SyncCancelSlot {
    /// An empty slot — the state `setup` manages before anything that
    /// could fail.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Publishes `token` only when the slot is EMPTY: the live run's
    /// publisher wins, and a concurrent second start (the
    /// `E_SYNC_RUNNING` path) publishes nothing — it never evicts the
    /// running run's token.
    fn publish(&self, token: &CancelToken) {
        let mut slot = self
            .token
            .lock()
            .expect("the cancel slot must not be poisoned");
        if slot.is_none() {
            *slot = Some(token.clone());
        }
    }

    /// The flag holder's re-assertion of ownership: stores `token`
    /// UNCONDITIONALLY, over whatever a losing start's pre-take
    /// `publish` left (the round-1 P2-1 fix). Ownership of the slot
    /// and ownership of the run are decided by different races —
    /// `publish` is set-if-empty, the flag is `compare_and_swap` — so
    /// a start that pre-published first but lost the flag would
    /// otherwise leave ITS token in the slot, and its lease's exit
    /// would then empty the slot over the LIVE run (a `sync_cancel`
    /// arriving next would find no run to cancel). Only the flag
    /// holder reaches this call, so exactly one run re-asserts; the
    /// loser's later `clear_if_current` no-ops over the winner's
    /// latch (`ptr_eq`), and the pre-take `publish`'s window — a
    /// cancel latching during phase 0's snapshot build — is kept.
    fn publish_owned(&self, token: &CancelToken) {
        *self
            .token
            .lock()
            .expect("the cancel slot must not be poisoned") = Some(token.clone());
    }

    /// The live run's token, if one is published — the `sync_cancel`
    /// command's (T-120) read; cancelling the clone is cancelling the
    /// run's (one `Arc` latches both spellings).
    pub(crate) fn token(&self) -> Option<CancelToken> {
        self.token
            .lock()
            .expect("the cancel slot must not be poisoned")
            .clone()
    }

    /// The pipeline's single knowledge mark (T-120, D-210): called
    /// ONCE per run, by [`sync_run_with`] immediately after the ONE
    /// consultation falls through — the check read not-cancelled, so
    /// the run enters §9.4 phase 2's uncancellable tail and a cancel
    /// latched from here on is the no-op the spec defines. Only the
    /// flag holder reaches the call site, so exactly one run marks;
    /// the lease's `clear_if_current` resets the flag with the token
    /// when that run answers.
    pub(crate) fn mark_past_point_of_no_return(&self) {
        self.past_point
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// The knowledge read (T-120): whether the live run — the run
    /// whose token the slot publishes — has passed §9.4 phase 2
    /// step 3's point of no return. `true` is read only by the
    /// `sync_cancel` command, after it latched the token. The
    /// CERTAINTY of the too-late verdict comes from the mark's
    /// PLACEMENT, not the atomics: the mark is stored only on the
    /// consultation's fall-through path, so a `true` read after a
    /// latch proves the consultation completed and read the
    /// pre-latch value (a latch before the consultation would have
    /// stopped the run and left the mark unset) — the Acquire/
    /// Release pair only makes that write visible in time.
    pub(crate) fn is_past_point_of_no_return(&self) -> bool {
        self.past_point.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Clears the slot only while it still holds exactly `token`'s
    /// latch — a lease that published nothing (the slot was occupied)
    /// clears nothing, so no run's exit unpublishes another's token.
    /// The knowledge flag resets with the token (one lifecycle, the
    /// field's own doc): the next run starts with the point of no
    /// return unmarked.
    fn clear_if_current(&self, token: &CancelToken) {
        let mut slot = self
            .token
            .lock()
            .expect("the cancel slot must not be poisoned");
        let current = slot.as_ref();
        if current.is_some_and(|current| Arc::ptr_eq(&current.cancelled, &token.cancelled)) {
            *slot = None;
            self.past_point
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }
}

/// The publication's RAII other half (the [`sync_running::SyncGuard`]
/// shape, D-123): clearing the slot on EVERY exit of
/// [`sync_run_with`] — held across the run, dropped at the answer;
/// an `Err`'s early return and a panic's unwind drop it the same way,
/// and the finish path drops it explicitly beside the guard, before
/// the never-fatal aftermath (a wedged after-run seam must not leave
/// a published token behind a finished run — the D-138 P2-2
/// reasoning, applied to the slot). The drop's state read is the
/// non-panicking `try_state` on purpose: publication already proved
/// the state exists (a panicking lookup there), and a `Drop` must not
/// turn a wiring bug into an abort mid-unwind.
struct CancelLease<R: Runtime> {
    app: AppHandle<R>,
    token: CancelToken,
}

impl<R: Runtime> Drop for CancelLease<R> {
    fn drop(&mut self) {
        if let Some(slot) = self.app.try_state::<SyncCancelSlot>() {
            slot.clear_if_current(&self.token);
        }
    }
}

// ---------------------------------------------------------------------------
// The write-in-flight gate (T-123, §9.4's "Closing the window during
// a sync" — "The app finishes the current atomic write and then
// exits"; D-219 point 5): the one fact the confirmed exit waits on.
// ---------------------------------------------------------------------------

/// The write-in-flight gate — the one fact §9.4's closing paragraph
/// makes load-bearing: which phase-3 per-project write unit is in
/// flight RIGHT NOW. Phase 3's loop takes a [`WriteInFlightGuard`]
/// per project (before the blocking-pool §9.5 write) and drops it
/// after the verdict/stamp transaction (§9.4 phase 3 step 2) — the
/// guard's region IS the per-project unit, the DoD's "the file
/// stamped/verdict recorded" moment — and the `sync_confirm_exit`
/// command waits on [`SyncWriteGate::wait_quiet`] before it exits
/// (T-123, D-219).
///
/// The region covers phase 3's writes only, by the recorded reading:
/// §9.4's "atomic write" is §9.5's unit, and phase 2's backup copies
/// are not target writes. The honest boundary (D-219 point 5a): the
/// wait covers the write in flight WHEN THE WAIT BEGINS — a write
/// STARTING after the quiet observation races the process death, and
/// that arm is carried by exactly the machinery §9.4 itself defines
/// for a mid-write death: §9.5's old-or-new atomicity plus the
/// startup `interrupted` sweep (D-147). A "freeze new writes" gate
/// would need a pipeline stop §9.4 phase 2 step 3 forbids past the
/// point of no return.
///
/// Managed by `setup` in every mode beside [`SyncCancelSlot`] (the
/// pipeline-resident posture: phase 3 — a `crate::commands` importer
/// already — takes the guards, `sync_close` reads the wait as a
/// sibling consumer). [`Clone`] because the confirm command moves a
/// share into its blocking wait (the `Arc` behind it is the whole
/// cost).
#[derive(Clone, Debug, Default)]
pub(crate) struct SyncWriteGate {
    inner: std::sync::Arc<SyncWriteGateInner>,
}

/// The gate's shared half: the in-flight count under the mutex the
/// condvar pairs with — the count is the fact, the condvar is the
/// wakeup, and both live under ONE lock so no wakeup can be missed
/// (the `while` re-check under the mutex is the miss-proof shape).
#[derive(Debug, Default)]
struct SyncWriteGateInner {
    in_flight: std::sync::Mutex<usize>,
    write_finished: std::sync::Condvar,
}

impl SyncWriteGate {
    /// The quiet gate a fresh app starts from — the state `setup`
    /// manages before anything that could fail. Deliberately not
    /// `Default` in the caller: the constructor names the fresh state
    /// in the setup code.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Enters one write's region: counts the caller's unit as in
    /// flight. Called by phase 3's per-project loop (the guard's
    /// placement there is Rule 7's whole-region rule — the guard
    /// lives across the `spawn_blocking` write AND the verdict
    /// transaction's `.await`).
    pub(crate) fn enter(&self) -> WriteInFlightGuard {
        let mut in_flight = self
            .inner
            .in_flight
            .lock()
            .expect("the write gate must not be poisoned");
        *in_flight += 1;
        WriteInFlightGuard {
            inner: std::sync::Arc::clone(&self.inner),
        }
    }

    /// Waits until no write unit is in flight — the confirmed exit's
    /// wait (§9.4: "The app finishes the current atomic write and
    /// then exits"). BLOCKING by design: the condvar's `wait` parks
    /// the calling thread until a guard's drop notifies it, so the
    /// caller runs this on the blocking pool (SYS-001; the confirm
    /// command's `spawn_blocking`) and NO polling or fixed sleep
    /// exists anywhere near the wait (D-140 — the wait's return IS
    /// the rendezvous). Bounded by one write unit's duration (§9.5's
    /// own bounded retries).
    pub(crate) fn wait_quiet(&self) {
        let mut in_flight = self
            .inner
            .in_flight
            .lock()
            .expect("the write gate must not be poisoned");
        while *in_flight > 0 {
            in_flight = self
                .inner
                .write_finished
                .wait(in_flight)
                .expect("the write gate must not be poisoned");
        }
    }

    /// Whether no write unit is in flight — the moment-of-read
    /// observability seam the gate's own suite uses. No production
    /// caller consults it (the confirm command WAITS, it never
    /// polls), so it exists only under `cfg(test)` (the
    /// `Db::set_busy_timeout_for_tests` shape).
    #[cfg(test)]
    pub(crate) fn is_quiet(&self) -> bool {
        *self
            .inner
            .in_flight
            .lock()
            .expect("the write gate must not be poisoned")
            == 0
    }
}

/// One write unit's in-flight lease (the [`sync_running::SyncGuard`]
/// RAII shape): counted in on [`SyncWriteGate::enter`], counted out
/// and announced in `Drop` — the notify wakes exactly the waiter the
/// count's return to zero matters to. `Send + Sync` by construction
/// (an `Arc`), so phase 3's loop holds it across the write's and the
/// verdict transaction's `.await`s — the whole region the gate
/// protects (Rule 7: a take-and-immediately-drop is the bug the
/// rule exists for).
pub(crate) struct WriteInFlightGuard {
    inner: std::sync::Arc<SyncWriteGateInner>,
}

impl Drop for WriteInFlightGuard {
    fn drop(&mut self) {
        let mut in_flight = self
            .inner
            .in_flight
            .lock()
            .expect("the write gate must not be poisoned");
        *in_flight = in_flight
            .checked_sub(1)
            .expect("a write guard only ever drops on a counted unit");
        // notify_all, not notify_one: every waiter re-checks under
        // the mutex, so the shape needs no wakeup accounting.
        self.inner.write_finished.notify_all();
    }
}

// ---------------------------------------------------------------------------
// The `sync://progress` event (T-119, §7.10's events line, SYS-003,
// MAIN-026, §9.4): the run's live "k of N" and the current phase,
// emitted no more than ten times per second. D-202 holds the readings.
// ---------------------------------------------------------------------------

/// The minimum spacing between two `sync://progress` emissions
/// (SYS-003: "no more than 10 times per second" — one tenth of a
/// second between consecutive emissions). Pinned by a unit test
/// against the literal, the `dirty_recalc::DEBOUNCE` pin shape. The
/// scan twin ([`crate::scan::PROGRESS_MIN_INTERVAL`]) stays scan's
/// own const — its doc ties it to ALG-053's walk; two literals for
/// one SYS-003 bound, each pinned by its own suite (D-202).
pub(crate) const SYNC_PROGRESS_MIN_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);

/// §9.4's phase words on the `sync://progress` wire, serialized as
/// the spec's own lower-case spellings — the [`crate::scan::
/// ScanClassWire`] shape. No `start` member: phase 0 is
/// command-internal (the flag, the ALG-001 snapshot build, the row —
/// one step with no per-project loop the event could report on, and
/// no injectable seam inside it), so its completion IS preflight's
/// start, and the first emission announces `preflight` (D-202).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) enum SyncPhaseWire {
    /// §9.4 phase 1 — the per-project preflight.
    #[serde(rename = "preflight")]
    Preflight,
    /// §9.4 phase 2 — the per-project backups.
    #[serde(rename = "backups")]
    Backups,
    /// §9.4 phase 3 — the per-project atomic writes.
    #[serde(rename = "write")]
    Write,
    /// §9.4 phase 4 — the finish.
    #[serde(rename = "finish")]
    Finish,
}

/// The `sync://progress` event payload (§7.10, SYS-003, MAIN-026):
/// the live "k of N" region's two numbers and the current phase —
/// `done` counts the run's projects with a TERMINAL §9.4 verdict
/// (`skipped`/`unchanged` from phase 1's transaction on, plus
/// `written`/`failed` as phase 3 completes each write; the per-write
/// report rides the write's own completion, so it can lead the
/// verdict row's transaction by the microseconds between them — a
/// display-only preview, never a claim the row is already visible),
/// `total` is the run's active project count, constant for the whole
/// run (MAIN-026's region reads one scale per run, never rescales
/// mid-run).
///
/// The event name is the spec's literal wire name, pinned by the
/// manual [`Event`] implementation (the D-112 ruling: the derive
/// macro cannot produce the `://` spelling). Emitted from Rust only;
/// the webview subscribes through the T-246 listen/unlisten grant
/// (SEC-001 — the generated `emit`/`emitTo` stay capability-denied).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) struct SyncProgressEvent {
    /// The phase the run is in (§9.4's own word).
    pub(crate) phase: SyncPhaseWire,
    /// The run's projects with a terminal §9.4 verdict so far (k).
    pub(crate) done: u32,
    /// The run's active project count (N, fixed at phase 0).
    pub(crate) total: u32,
}

impl Event for SyncProgressEvent {
    /// The §7.10 wire name, verbatim. Tauri permits exactly the
    /// characters it contains (alphanumeric, `-`, `/`, `:`, `_`).
    const NAME: &'static str = "sync://progress";
}

/// The rate-bounded `sync://progress` reporter (SYS-003): the ONE
/// seam every progress fact consults — the four phase-boundary
/// reports the command makes between phases and the per-write
/// reports phase 3's injected operation carries (D-202).
///
/// Shared across the command's task and the write wrapper's
/// blocking-pool closures (an [`Arc`]), so the last-emission instant
/// lives behind a [`std::sync::Mutex`]; the guard covers exactly the
/// rate decision and the EMIT is the caller's, outside the lock. The
/// clock is the injected [`SyncRunSeams::progress_clock`] (the D-197
/// abort-clock pattern): production answers [`std::time::Instant::now`],
/// the tests answer simulated instants — the ≤10/s bound is a
/// property of simulated windows, never wall seconds (the D-140
/// posture; no fixed sleeps anywhere near the rate tests).
pub(crate) struct SyncProgressReporter {
    clock: ProgressClock,
    min_interval: std::time::Duration,
    state: std::sync::Mutex<SyncProgressReporterState>,
}

/// The reporter's shared half: when the last emission fired (the
/// first report has emitted nothing yet).
struct SyncProgressReporterState {
    last_emit: Option<std::time::Instant>,
}

impl SyncProgressReporter {
    /// The reporter over the seam's clock (the `min_interval` is
    /// SYS-003's literal, [`SYNC_PROGRESS_MIN_INTERVAL`]).
    pub(crate) fn new(clock: ProgressClock) -> Self {
        Self {
            clock,
            min_interval: SYNC_PROGRESS_MIN_INTERVAL,
            state: std::sync::Mutex::new(SyncProgressReporterState { last_emit: None }),
        }
    }

    /// Records one progress fact and decides its emission:
    /// `Some(payload)` iff this fact may reach the wire now — the
    /// first report, or the first after a full `min_interval` since
    /// the last emission. The coalescing rule is the scan
    /// [`ProgressTracker`](crate::scan) rule: a suppressed report's
    /// state is simply not sent, and the next emission carries its
    /// OWN newest state — never a stale intermediate.
    pub(crate) fn report(
        &self,
        phase: SyncPhaseWire,
        done: u32,
        total: u32,
    ) -> Option<SyncProgressEvent> {
        // The clock is read INSIDE the critical section (round-2's
        // hardening note, taken): reading it before the lock would
        // let an overlapping caller hand an earlier instant to a
        // `last_emit` a later one installed, and `duration_since`
        // panics on reversed instants. The shipped pipeline never
        // overlaps the two callers (phase 3 awaits each write), but
        // the invariant is structural here, not incidental.
        let mut state = self
            .state
            .lock()
            .expect("the sync progress reporter must not be poisoned");
        let now = (self.clock)();
        let may_emit = match state.last_emit {
            None => true,
            Some(last) => now.duration_since(last) >= self.min_interval,
        };
        if !may_emit {
            return None;
        }
        state.last_emit = Some(now);
        Some(SyncProgressEvent { phase, done, total })
    }
}

/// The progress clock (the `SyncRunSeams::progress_clock` arm) —
/// `Sync` for the same structural reason as the other arms: the seam
/// struct crosses the command's await points whole, and the reporter
/// consults it from phase 3's blocking-pool closures too.
type ProgressClock = Arc<dyn Fn() -> std::time::Instant + Send + Sync>;

/// A `usize` count on the `sync://progress` wire (u32, the
/// `wire_count` reasoned-panic posture: a run's project count fits).
fn progress_count(count: usize) -> u32 {
    u32::try_from(count).expect("a run's project count fits the wire's u32")
}

/// Emits one `sync://progress` fact through the shared reporter —
/// the display-only seam: the reporter's rate gate decides whether
/// the fact reaches the wire, and a refused emission is logged and
/// never fatal (the run's ANSWER carries the terminal state the
/// events preview — the scan callback's posture).
fn emit_sync_progress<R: Runtime>(
    app: &AppHandle<R>,
    progress: &SyncProgressReporter,
    phase: SyncPhaseWire,
    done: u32,
    total: u32,
) {
    if let Some(payload) = progress.report(phase, done, total)
        && let Err(error) = payload.emit(app)
    {
        logging::sync_progress_emit_failed(&error);
    }
}

/// The injected pipeline operations of [`sync_run_with`] — the
/// phase-2/3 `D-089 injector pattern` bundled for one call site:
/// production closes over the real `eunomia-fs` operations, the tests
/// substitute the fault points of spec 15.1 (a failing copy, a parked
/// or refusing write). The other phase seams (free space, the marker
/// clock, the dirty pass) stay un-injected here: their fault behavior
/// is pinned by the phases' own suites, and no command-level test in
/// this task needs to reach through them.
pub(crate) struct SyncRunSeams {
    /// Phase 2's copy operation — production
    /// [`backup_target_file`](eunomia_fs::backup::backup_target_file).
    pub(crate) backup: BackupOp,
    /// Phase 3's §9.5 write operation — production
    /// [`sync_phase3::production_write`].
    pub(crate) write: WriteOp,
    /// The ABORT's clock (D-197, the command-level shape of D-185's
    /// semantics): consulted at most once per run, on the abort paths
    /// the command drives ITSELF — the D-25 gate's stop and the cancel
    /// checkpoint's stop (both through [`stop_run`]) and phase 2's
    /// abort ([`sync_phase2::run_with_abort_clock`]) — answering the
    /// instant the run row goes terminal. Production is the real
    /// clock; the tests answer a fixed instant, which makes the
    /// abort's stamp an oracle instead of a presence check. The
    /// post-phase-0 `Err` wrapper (`pipeline_refusal`) keeps the real
    /// clock: it is the best-effort corpse cleanup on a path whose
    /// FIRST error is the answer.
    pub(crate) abort_clock: AbortClock,
    /// The PROGRESS clock (T-119, SYS-003's rate limit — the D-197
    /// abort-clock pattern applied to the `sync://progress` seam):
    /// [`SyncProgressReporter`] consults it on every report — the
    /// four phase-boundary reports and phase 3's per-write reports —
    /// and the spacing it observes bounds the emissions at ten per
    /// second. Production is the real clock; the tests answer
    /// simulated instants (a frozen clock, a jumping one), which
    /// makes the rate bound a property of simulated windows, never
    /// wall seconds (the D-140 posture).
    pub(crate) progress_clock: ProgressClock,
}

/// Phase 2's injected copy operation (the `SyncRunSeams::backup` arm).
type BackupOp = Arc<dyn Fn(&BackupTarget<'_>) -> Result<BackupOutcome, BackupError> + Send + Sync>;

/// Phase 3's injected §9.5 write operation (the `SyncRunSeams::write`
/// arm) — `Sync` because phase 3 shares it over an `Arc` across the
/// per-project blocking closures.
type WriteOp = Arc<dyn Fn(&Path, &[u8]) -> Result<WrittenFile, AtomicWriteError> + Send + Sync>;

/// The abort clock (the `SyncRunSeams::abort_clock` arm) — `Sync` for
/// the same structural reason as the other two arms: the seam struct
/// crosses the command's await points whole.
type AbortClock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

impl SyncRunSeams {
    /// The production seams: the real `eunomia-fs` operations and the
    /// real clocks for the abort's instant and the progress rate.
    pub(crate) fn production() -> Self {
        Self {
            backup: Arc::new(eunomia_fs::backup::backup_target_file),
            write: Arc::new(crate::sync_phase3::production_write),
            abort_clock: Arc::new(Timestamp::now),
            progress_clock: Arc::new(std::time::Instant::now),
        }
    }
}

/// Serves the "Run sync" button (§7.10 Sync module `sync_run`, §9.4 the
/// pipeline): the whole run — flag, snapshot build, preflight, the
/// D-25 gate, backups, atomic writes, the finish — through the
/// production seams, with §10's every-error rule served by the
/// [`logging::sync_run_failed`] edge.
#[tauri::command]
#[specta::specta]
pub(crate) async fn sync_run<R: Runtime>(
    app: AppHandle<R>,
    confirm_empty: Option<bool>,
) -> Result<SyncRunOutcome, AppError> {
    sync_run_with(
        app,
        confirm_empty,
        CancelToken::new(),
        SyncRunSeams::production(),
    )
    .await
    .inspect_err(logging::sync_run_failed)
}

/// The body of [`sync_run`], split the `dialog_pick_folder_with` way so
/// the one logging edge covers every failure path and the tests drive
/// the pipeline with their own cancel token and seams.
///
/// **The composition, §9.4's own order.**
///
/// 0. The cancel-token publication (T-258) — the token is placed in
///    the managed [`SyncCancelSlot`] BEFORE phase 0's flag take (a
///    cancel latched during the snapshot build is honored at the
///    check point; a token whose take is refused is a harmless latch
///    the [`CancelLease`] clears on the way out), and the lease holds
///    the clearing duty for every exit below.
/// 1. Phase 0 ([`sync_phase0::run`]) — the flag-guarded start: a
///    second run meets the typed `E_SYNC_RUNNING` refusal here, the
///    snapshot is built in one read transaction, and the `running` row
///    is recorded. The outcome (the [`sync_running::SyncGuard`]
///    included) is held for the run: whatever path this function takes
///    out — confirmation, cancel, error, finish — the guard's RAII
///    drop is §9.4 phase 4 step 5's "Reset `sync_running`"
///    (D-123's contract, `Phase0Outcome`'s field), and on the finish
///    path the drop is explicit at phase 4's own end, BEFORE the
///    never-fatal aftermath, so an after-run seam that never returns
///    cannot wedge SYS-004 behind a finished run (the round-1 P2-2
///    fix).
/// 2. DB-032's "before a sync" availability checkpoint
///    ([`availability::recheck_and_emit`]) — after the flag, before
///    phase 1: the mutations are gated, so the pass reads a stable
///    project list; its diff is the event's business (phase 1's
///    per-project probes stay the run's own gate), its failure refuses
///    the run.
/// 3. Phase 1 — preflight, over the built content; then the D-25 gate:
///    a non-empty [`Phase1Outcome::needs_confirmation`] list without
///    `confirm_empty=true` answers
///    [`SyncRunOutcome::NeedsConfirmation`] with the list, the run row
///    terminal `aborted`, and nothing else changed (no file, no
///    backup, no stamp) — the UI's confirm re-calls the command as a
///    full new run.
/// 4. The cancel check point — the one place the token is consulted,
///    before phase 2; a latched token aborts the run (the D-131
///    `E_DISK_FULL` tail shape: the row goes `aborted`, phase 1's
///    verdicts stay) and answers the `aborted` summary.
/// 5. Phases 2 → 3 → 4 — backups, the per-project writes, the finish;
///    after phase 2 no stop exists (§9.4's own point of no return).
/// 6. The watcher's after-sync refresh
///    ([`file_watch::refresh_after_sync`], SYS-008) and the finish's
///    never-fatal reports (D-134): reported through
///    [`logging::sync_run_degraded`], never `Err` — the verdict is
///    committed, and the next trigger retries everything that refused.
/// 7. The summary — the committed rows read back ([`summary_of`]).
///
/// **The progress emissions (T-119, SYS-003, D-202).** Throughout the
/// run, [`SyncProgressReporter`] emits the `sync://progress` event:
/// one report immediately before each of phases 1–4 begins (only
/// when the run reaches the phase — the availability checkpoint's
/// refusal, the D-25 gate and the cancel point all stop the run
/// BEFORE the phase their report would announce), plus one report
/// per write through the WRAPPED [`SyncRunSeams::write`] operation —
/// the only place k moves within a phase. `done` counts terminal §9.4
/// verdicts, `total` is phase 0's project count, and the reporter's
/// rate gate bounds the whole stream at ten emissions per second
/// (the clock is the seam's [`SyncRunSeams::progress_clock`]). A
/// refused emission is display-only degradation — logged, never
/// fatal. Nothing emits after the finish boundary: the terminal
/// state is the ANSWER, the summary (the scan posture).
///
/// **The error wrapper (the P3-4 ruling, D-133's recorded follow-up).**
/// An `Err` after phase 0 whose run row is still `running` gets its
/// terminal status: [`abort_run`] with the error's own §10 literal and
/// a class-only stage line, decided by the ROW's state inside the one
/// transaction — never by the error's code, which proves nothing about
/// the row (the shared SQL classifier maps `SQLITE_FULL` to
/// `E_DISK_FULL` from any phase's transaction; the round-1 P1 fix).
/// The abort is best-effort — a refused abort (the store that just
/// failed failing again) leaves the row `running`, is logged through
/// the degraded edge, and returns the ORIGINAL error; that row is
/// exactly what the startup `interrupted` recovery exists for. No
/// per-project reset rides these aborts: the phase-2 reset belongs to
/// the backup abort's own transaction (§9.4 phase 2 step 2), and a
/// `written`/`unchanged` row claims what actually happened (the
/// D-131 tail posture, spelled for every stop point).
pub(crate) async fn sync_run_with<R: Runtime>(
    app: AppHandle<R>,
    confirm_empty: Option<bool>,
    cancel: CancelToken,
    seams: SyncRunSeams,
) -> Result<SyncRunOutcome, AppError> {
    // The data-directory root phases 1 (free space) and 2/4 (the §5.5
    // tree) need — resolved before the flag so a missing `DataRoot`
    // (an uninitialized-app bug) does not churn the flag at all.
    let data_dir = data_root(&app)?;

    // Step 0 — publish the cancel token (T-258, D-138's recorded
    // follow-up): the managed slot the `sync_cancel` command
    // reads. BEFORE the flag take (the missed-cancel window closes:
    // phase 0's snapshot build takes real time, and a cancel arriving
    // over it must find the latch), set-if-empty (a refused second
    // start never evicts the live run's token). The lease is the
    // clearing half: every exit of this function — including the
    // `Err`s below and a panic's unwind — drops it, and the finish
    // path drops it explicitly beside the guard.
    app.state::<SyncCancelSlot>().publish(&cancel);
    let _cancel_lease = CancelLease {
        app: app.clone(),
        token: cancel.clone(),
    };

    // Step 1 — phase 0. An `Err` here leaves no run row (the flag is
    // RAII-released inside the phase), so there is nothing to abort.
    let phase0 = sync_phase0::run(&app).await?;
    // The flag holder re-asserts slot ownership (the round-1 P2-1
    // fix): the pre-take `publish` and the flag's `compare_and_swap`
    // are separate races, and a start that pre-published first but
    // lost the flag must not stay published over THIS run — its lease
    // exit would empty the slot while the live run still needs it.
    // Only the flag holder reaches here, so exactly one run
    // re-asserts; the loser's `clear_if_current` no-ops over this
    // latch.
    app.state::<SyncCancelSlot>().publish_owned(&cancel);
    let run_id = phase0.run.id.clone();

    // The run's progress reporter (T-119, SYS-003): the rate-bounded
    // `sync://progress` seam the four phase-boundary reports and
    // phase 3's per-write reports consult, over the seam's injected
    // clock (D-202). Shared behind an `Arc` because the write
    // wrapper's closure — phase 3's own injection point — reports
    // from the blocking pool.
    let progress = Arc::new(SyncProgressReporter::new(Arc::clone(&seams.progress_clock)));
    // MAIN-026's N: the run's active project count, fixed at phase 0
    // — the one scale the live "k of N" region reads for the whole
    // run (captured before the outcome is destructured below).
    let progress_total = progress_count(phase0.projects.len());

    // Step 2 — DB-032's "before a sync" checkpoint. The diff is the
    // event's business; a refusal stops the run (a pass that could not
    // see the project list cannot be "before a sync").
    if let Err(error) = availability::recheck_and_emit(&app).await {
        return Err(pipeline_refusal(&app, &run_id, "the availability checkpoint", error).await);
    }

    // The preflight boundary (T-119): the run is live and phase 1 is
    // the phase it is in — MAIN-026's region opens at zero. Emitted
    // AFTER the availability checkpoint on purpose: a refusal there
    // stops the run before preflight, and an emission names a phase
    // the run actually reached (D-202's boundary-truthfulness rule).
    emit_sync_progress(&app, &progress, SyncPhaseWire::Preflight, 0, progress_total);

    // Step 3 — phase 1 over the built content. The free-space seam is
    // the production closure the phase's docs prescribe.
    let free_probe = {
        let dir = data_dir.clone();
        move || eunomia_fs::free_space::free_bytes(&dir)
    };
    let phase1 =
        match sync_phase1::run(&app, &phase0, free_probe, std::time::SystemTime::now()).await {
            Ok(outcome) => outcome,
            Err(error) => {
                return Err(pipeline_refusal(&app, &run_id, "the preflight", error).await);
            }
        };

    // The run's decided-so-far count (k, D-202): phase 1's terminal
    // verdicts — every `skipped`/`unchanged` project; the `ready`
    // projects join the count as phase 3 writes them. Computed once:
    // the backups and write boundaries and every per-write report
    // read the same base.
    let progress_decided = progress_count(
        phase1
            .projects
            .iter()
            .filter(|(_, verdict)| {
                !matches!(verdict, crate::sync_phase1::PreflightVerdict::Ready(_))
            })
            .count(),
    );

    // §10's per-project skip lines over the outcome's structured OS
    // facts (the store row has no column for them — D-131's round-1
    // P3-5 added the details for exactly this consumption), logged
    // HERE — right after the phase — so every later exit (the D-25
    // gate, the cancel stop, a phase-2/3 `Err`) still carries them:
    // phase 4 journals only finished runs (the round-2 P3 fix).
    for (_project_id, verdict) in &phase1.projects {
        if let crate::sync_phase1::PreflightVerdict::Skipped {
            code,
            message,
            details,
            ..
        } = verdict
        {
            logging::sync_project_refused(
                false,
                code.as_str(),
                details.as_ref().and_then(|facts| facts.os_code),
                details.as_ref().and_then(|facts| facts.os_kind.as_deref()),
                message,
            );
        }
    }

    // Phase 1 froze everything it read into its own outcome, so the
    // built snapshot's memory goes here; the guard is the only half
    // the rest of the run needs (the round-1 P3-4 finding).
    let sync_phase0::Phase0Outcome {
        guard, projects, ..
    } = phase0;
    drop(projects);

    // The D-25 gate (§9.4 phase 1 step 6): a non-empty confirmation
    // list without the flag answers the list and stops the run — the
    // aborted row is this command's recorded reading of "changes
    // nothing": §9.4's phrase is about the target files (phase 1's
    // own header), while a `running` row behind a released flag would
    // lie about the app's state until the next restart.
    let confirmation = phase1.needs_confirmation();
    if !confirmation.is_empty() && confirm_empty != Some(true) {
        stop_run(
            &app,
            &run_id,
            "",
            &format!(
                "awaiting the empty-build confirmation for {} project(s) (D-25)",
                confirmation.len()
            ),
            crate::sync_phase2::db002_stamp((seams.abort_clock)()),
        )
        .await?;
        logging::sync_run_aborted("awaiting the empty-build confirmation");
        return Ok(SyncRunOutcome::NeedsConfirmation {
            projects: confirmation.iter().map(|id| (*id).to_owned()).collect(),
        });
    }

    // Step 4 — the cancel check point: the one consultation, before
    // phase 2 (§9.4 phase 2 step 3's point of no return). The stop is
    // the D-131 tail shape — the row `aborted`, the verdicts phase 1
    // wrote stay — and the answer is the `aborted` summary, not an
    // `Err`: the user asked for this.
    if cancel.is_cancelled() {
        stop_run(
            &app,
            &run_id,
            "",
            "cancelled by the user before the backups",
            crate::sync_phase2::db002_stamp((seams.abort_clock)()),
        )
        .await?;
        logging::sync_run_aborted("cancelled by the user before the backups");
        let summary = summary_of(&app, &run_id).await?;
        return Ok(SyncRunOutcome::Completed { summary });
    }

    // The point of no return is PASSED (§9.4 phase 2 step 3): the
    // consultation above has read the token, and from here to the
    // answer no stop exists. The KNOWLEDGE mark (T-120, D-210): the
    // one write the `sync_cancel` command's answer reads — placed
    // immediately after the consultation, so the mark answers "this
    // run's cancel window closed". Never a second consultation, never
    // a gate the pipeline reads: the token stays the single
    // cancellation channel. Only the flag holder reaches here, and
    // this run's lease resets the flag with the token at the exit.
    app.state::<SyncCancelSlot>().mark_past_point_of_no_return();

    // The backups boundary (T-119): phase 1's verdicts are decided
    // rows, so k opens at the decided count. Emitted after the gate
    // and the cancel point — a run stopped at either never reached
    // the backups, and an emission names a phase the run reached
    // (D-202's boundary-truthfulness rule, pinned by the gate test).
    emit_sync_progress(
        &app,
        &progress,
        SyncPhaseWire::Backups,
        progress_decided,
        progress_total,
    );

    // Step 5 — phases 2 → 3 → 4, each refusing through the wrapper.
    // Phase 2 gets the SAME abort clock as the command's own stops
    // (D-197): the D-25 gate's stop, this cancel checkpoint and the
    // backup abort — the three abort paths the command drives itself —
    // all answer the one seam, so an interrupted run's stamp is one
    // injectable fact at any of them. The backup instant stays the
    // real clock — it is the run's own instant, not the seam's.
    let backup = Arc::clone(&seams.backup);
    let abort_clock = Arc::clone(&seams.abort_clock);
    if let Err(error) = sync_phase2::run_with_abort_clock(
        &app,
        &phase1,
        &data_dir,
        move |target| backup(target),
        Timestamp::now(),
        move || abort_clock(),
    )
    .await
    {
        return Err(pipeline_refusal(&app, &run_id, "the backups", error).await);
    }

    // The write boundary (T-119): phase 2 decided no verdicts (a
    // backup copies bytes, never a row), so k still reads the
    // phase-1 count; the per-write reports below advance it.
    emit_sync_progress(
        &app,
        &progress,
        SyncPhaseWire::Write,
        progress_decided,
        progress_total,
    );

    // Phase 3's write operation, WRAPPED for progress (T-119, D-202):
    // the command owns the composition, so the wrapper lives here —
    // phase 3 calls the seam it always called, and each completed
    // write reports k advancing one project at a time. A write
    // ATTEMPT decides its project (success and failure both produce
    // a row), so the counter counts attempts; a phase-3 `Err` (the
    // join wrapper's case) aborts the run past this point and no
    // further emission happens — the row's terminal state is the
    // answer's. The closure runs on the blocking pool, so the
    // reporter rides an `Arc` and the emit is display-only (a
    // refusal is logged, never fatal — the scan callback's posture).
    let write = Arc::clone(&seams.write);
    let write_progress = Arc::clone(&progress);
    let writes_seen = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let progress_app = app.clone();
    let phase3 = match sync_phase3::run(
        &app,
        &phase1,
        move |target: &Path, contents: &[u8]| {
            let outcome = write(target, contents);
            let completed = 1 + writes_seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            emit_sync_progress(
                &progress_app,
                &write_progress,
                SyncPhaseWire::Write,
                progress_decided.saturating_add(completed),
                progress_total,
            );
            outcome
        },
        eunomia_core::marker_clock::now_marker_clock,
        Timestamp::now(),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            return Err(pipeline_refusal(&app, &run_id, "the write phase", error).await);
        }
    };

    // §10's per-project failure lines (the skip loop already ran right
    // after phase 1 — the two loops read disjoint sets: phase 3
    // processes only `ready` projects).
    for (_project_id, outcome) in &phase3.results {
        if let crate::sync_phase3::WriteOutcome::Failed {
            code,
            message,
            details,
            ..
        } = outcome
        {
            logging::sync_project_refused(
                true,
                code.as_str(),
                details.as_ref().and_then(|facts| facts.os_code),
                details.as_ref().and_then(|facts| facts.os_kind.as_deref()),
                message,
            );
        }
    }

    // The finish boundary (T-119): phase 3 decided every remaining
    // project — k has reached N. Nothing emits AFTER this: the
    // terminal state rides the command's answer (the summary), the
    // scan posture (D-202).
    emit_sync_progress(
        &app,
        &progress,
        SyncPhaseWire::Finish,
        progress_total,
        progress_total,
    );

    // The dirty seam through a closure that owns its handle (the
    // higher-ranked bound: a bare fn item's returned future borrows the
    // argument, so the phase's `FnOnce(&AppHandle<R>) -> F` wants the
    // cloned-handle shape the phase tests use).
    let dirty_seam = |app: &AppHandle<R>| {
        let app = app.clone();
        async move { crate::sync_phase4::production_dirty(&app).await }
    };
    // Phase 1's outcome rides along (T-124/D-223): phase 4 reads the
    // skipped verdicts' OS facts from it — the one §10 field the
    // committed rows cannot store.
    let phase4 = match sync_phase4::run(
        &app,
        &phase1,
        &phase3,
        &data_dir,
        Timestamp::now(),
        dirty_seam,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            return Err(pipeline_refusal(&app, &run_id, "the finish", error).await);
        }
    };

    // §9.4 phase 4 step 5's reset happens HERE, at the run's own end:
    // the guard leaves the pipeline before the never-fatal aftermath
    // (SYS-008's refresh) and the summary read, so a seam that never
    // returns cannot wedge SYS-004 behind a finished run (the round-1
    // P2-2 fix). The cancel lease drops the same way, beside it: a
    // wedged after-run seam must not leave a published token behind a
    // finished run either. An `Err` above unwinds through the guard's
    // Drop — and the lease's — the same release.
    drop(guard);
    drop(_cancel_lease);

    // Step 6 — the never-fatal aftermath: the watcher's after-sync
    // refresh (SYS-008; its class-only §10 code is all that rides the
    // line — SEC-008) and the finish's three reports (D-134).
    let watch_failure = match file_watch::refresh_after_sync(&app).await {
        Ok(_) => None,
        Err(error) => Some(error.code.as_str().to_owned()),
    };
    if !phase4.retention.failures.is_empty()
        || !phase4.journal.failures.is_empty()
        || phase4.dirty.failure.is_some()
        || watch_failure.is_some()
    {
        logging::sync_run_degraded(
            &phase4.retention.failures,
            &phase4.journal.failures,
            phase4.dirty.failure.as_deref(),
            watch_failure.as_deref(),
        );
    }

    // Step 7 — the summary off the committed rows.
    let summary = summary_of(&app, &run_id).await?;
    Ok(SyncRunOutcome::Completed { summary })
}

/// The P3-4 error wrapper (see [`sync_run_with`]'s docs): give a
/// post-phase-0 `Err` its terminal run row when the run is not
/// already terminal, then hand the ORIGINAL error back — the store's
/// refusal of the abort itself is reported, never swapped in (the
/// first error names the cause; the second only compounds it).
///
/// When the abort lands (T-124/D-223, reading 4b), the same
/// transaction journals the run's `sync.run_aborted` entry —
/// fail-then-log, D-051's read over this pipeline: the failed
/// phase's own transaction rolled back, so the abort transaction IS
/// where the error's durable journal claim lands, carrying the
/// ORIGINAL error's §10 code and OS code (its `details` half; a
/// refusal no OS fact backs journals the code alone). A row already
/// terminal answers `Ok(false)` with no second entry (the phase's
/// own in-phase abort already wrote both the row and its entry); a
/// refused abort leaves the row `running` and journals nothing — no
/// claim of an outcome that never landed.
///
/// **The decision is the row's state, never the error's code** (the
/// round-1 P1 fix): a code like `E_DISK_FULL` proves nothing about the
/// row — the shared SQL classifier maps `SQLITE_FULL` to it from ANY
/// phase's transaction, all of which leave the row `running`. The two
/// phases that DO write their terminal row before returning an error
/// (phase 1's free-space tail, phase 2's backup abort) are recognized
/// by exactly that row state inside the abort transaction, so a
/// second abort can never meet the store's wrong-state refusal.
/// `pub(crate)`: the behavior suite drives it directly (the
/// classifier-shaped `E_DISK_FULL` over a `running` row).
pub(crate) async fn pipeline_refusal<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
    stage: &str,
    error: AppError,
) -> AppError {
    let code = error.code.as_str();
    let os_code = error.details.as_ref().and_then(|facts| facts.os_code);
    match stop_run_if_running(
        app,
        run_id,
        code,
        &format!("the run stopped at {stage}: {code}"),
        os_code,
    )
    .await
    {
        Ok(_) => error,
        Err(abort_error) => {
            // The best-effort half: the row stays `running` — the
            // startup `interrupted` recovery's case — and BOTH facts
            // reach the log (the command edge logs the original; this
            // line names the abort that could not land, its own edge
            // because the run did NOT finish — the round-2 P2 fix).
            logging::sync_run_abort_refused(abort_error.code.as_str());
            error
        }
    }
}

/// The run repository's counter on the wire's integer — the store's
/// i64 count of projects into specta-safe i32 (BigInt types are
/// forbidden on the wire to avoid precision loss). A run cannot hold
/// `i32::MAX` projects; the reasoned-panic posture over that bound
/// (the phase-2 `size_bytes` shape).
fn wire_count(count: i64) -> i32 {
    i32::try_from(count).expect("a run's project count fits the wire's i32")
}

/// Writes a user-shaped stop's terminal row: `aborted` with the
/// caller-supplied `finished_at` — the abort clock's answer in the
/// DB-002 spelling (D-197) — and a class-only details line. The `code`
/// is the §10 literal of
/// what stopped the run where one exists (the P3-4 wrapper passes the
/// error's own code); the two user-shaped stops — the D-25 gate, the
/// cancel point — are decisions, not §10 situations, and pass the
/// empty literal: the store's writer takes `&str` (NULL is not an
/// input it offers), so "" is the no-code spelling and the first run
/// reader's reading is pinned at [`run_error_code_or_none`] (D-138's
/// obligation, landed as the one seam — T-258).
async fn stop_run<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
    code: &str,
    details: &str,
    finished_at: String,
) -> Result<(), AppError> {
    let run_id = run_id.to_owned();
    let code = code.to_owned();
    let details = details.to_owned();
    managed_db(app)?
        .run(move |tx| {
            abort_run(tx, &run_id, &finished_at, &code, &details)
                .map_err(db::sync_runs_to_app_error)
        })
        .await
}

/// The P3-4 wrapper's abort: [`stop_run`]'s row write, decided by the
/// ROW's state inside the one transaction — a row no longer
/// `running` (a phase's own in-phase abort, a finish that raced)
/// answers `Ok(false)` untouched; only a still-`running` corpse is
/// aborted. Returns whether the abort landed.
///
/// When it lands, the same transaction journals the run's
/// `sync.run_aborted` entry (T-124/D-223, reading 4b): error/error,
/// the `sync_run` entity, and `Details::Failure` with the ORIGINAL
/// error's §10 code and OS code — the `code`/`os_code` pair
/// [`pipeline_refusal`] extracted from it (fail-then-log: the failed
/// phase's transaction rolled back, and this abort transaction is
/// where the error's durable journal claim lands). No path: §10
/// names the path "of the file the failure is about", and a
/// pipeline-level stop is about the run, not one file (the failed
/// phase's own per-project entries carry their paths when they
/// exist). `os_code` is the error's own fact — `None` for a store
/// error or any refusal no syscall backs, absent from the JSON then.
async fn stop_run_if_running<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
    code: &str,
    details: &str,
    os_code: Option<i32>,
) -> Result<bool, AppError> {
    let run_id = run_id.to_owned();
    let code = code.to_owned();
    let details = details.to_owned();
    let finished_at = eunomia_store::values::now_utc();
    managed_db(app)?
        .run_with_mirror(move |tx, mirror| {
            let running = eunomia_store::sync_runs::get(tx, &run_id)
                .map_err(db::sync_runs_to_app_error)?
                .is_some_and(|row| row.status == eunomia_store::sync_runs::SyncRunStatus::Running);
            if !running {
                return Ok(false);
            }
            abort_run(tx, &run_id, &finished_at, &code, &details)
                .map_err(db::sync_runs_to_app_error)?;
            // The error abort's journal claim (T-124/D-223): inside
            // the abort's own transaction, after the row it
            // describes — the `code`/`os_code` pair is the original
            // error's, and the shape failure of a non-§10 `code`
            // (tampering only — every `ErrorCode` literal is registry
            // shaped, pinned by the errors suite) keeps the entry at
            // the cost of the payload. The T-324/D-228 mirror records
            // inside the closure; the executor flushes it with this
            // abort transaction's own commit (the fail-then-log
            // shape, D-051 — the failed phase rolled back, this
            // transaction is where the error's durable claims land).
            let mut entry = eunomia_store::action_log::LogEntry::new(
                eunomia_store::action_log::Action::SyncRunAborted,
                eunomia_store::action_log::Level::Error,
                eunomia_store::action_log::Visibility::User,
                eunomia_store::action_log::Outcome::Error,
            );
            entry.entity = Some(eunomia_store::action_log::Entity {
                kind: eunomia_store::action_log::EntityType::SyncRun,
                id: run_id.as_str(),
            });
            entry.sync_run_id = Some(run_id.as_str());
            entry.details = eunomia_store::action_log::FailureCode::new(code.as_str())
                .ok()
                .map(|failure_code| eunomia_store::action_log::Details::Failure {
                    code: failure_code,
                    path: None,
                    os_code,
                });
            eunomia_store::action_log::insert(tx, &entry).map_err(activity_log_to_app_error)?;
            mirror.record(&entry);
            Ok(true)
        })
        .await
}

/// D-138's first-reader obligation, landed as the one seam (T-258):
/// the RUN row's `error_code` cell spells "no code" TWO ways — `NULL`
/// ([`finish_run`]'s verdicts, the startup `interrupted` sweep) and
/// the empty literal `""` (the two user-shaped stops, whose store
/// writer takes `&str`; no §10 code names a user decision). The first
/// reader of the cell — the `sync_last_result` command (§7.10, T-121,
/// `sync_reads.rs`) — collapses both spellings to "no code" through
/// this seam, and every later reader reuses it instead of
/// re-deciding the spelling. The PER-PROJECT rows' codes are not
/// this helper's business: a `skipped`/`failed` row carries a real
/// §10 literal by definition, and phase 4's journal already
/// shape-checks those against §10.
pub(crate) fn run_error_code_or_none(error_code: Option<&str>) -> Option<&str> {
    match error_code {
        None | Some("") => None,
        Some(code) => Some(code),
    }
}

/// Reads one run's committed truth back as the wire summary — the
/// verdict and counters `finish_run` wrote (or the abort this command
/// wrote), and every per-project row in `sort_order`. Called only over
/// terminal rows (the finish, the cancel stop), so a missing row here
/// names a broken invariant, not a data state. `pub(crate)`: the
/// `sync_last_result` command (T-121, `sync_reads.rs`) reuses the
/// same committed-rows read over the last attempt's row — one DTO,
/// one read shape (D-217's ruling).
pub(crate) async fn summary_of<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
) -> Result<SyncRunSummary, AppError> {
    let run_id = run_id.to_owned();
    managed_db(app)?
        .run(move |tx| {
            let run = eunomia_store::sync_runs::get(tx, &run_id)
                .map_err(db::sync_runs_to_app_error)?
                .ok_or_else(run_row_missing_refusal)?;
            let rows = eunomia_store::sync_runs::list_projects(tx, &run_id)
                .map_err(db::sync_runs_to_app_error)?;
            Ok(SyncRunSummary {
                run_id: run.id,
                verdict: run.status.as_str().to_owned(),
                started_at: run.started_at,
                finished_at: run.finished_at,
                // The store's i64 counts of projects on the wire's
                // i32 — a run cannot hold two billion projects; the
                // reasoned-panic posture over that bound, the
                // `size_bytes` shape.
                projects_total: wire_count(run.projects_total),
                projects_written: wire_count(run.projects_written),
                projects_unchanged: wire_count(run.projects_unchanged),
                projects_skipped: wire_count(run.projects_skipped),
                projects_failed: wire_count(run.projects_failed),
                details: run.details,
                projects: rows
                    .into_iter()
                    .map(|row| SyncRunProjectEntry {
                        project_id: row.project_id,
                        status: row.status.as_str().to_owned(),
                        target_path: row.target_path,
                        error_code: row.error_code,
                        error_message: row.error_message,
                    })
                    .collect(),
            })
        })
        .await
}

/// The class-only refusal for a run row that vanished between its
/// terminal write and the summary read (the FK keeps the row alive;
/// a miss names a broken invariant — the phase-1
/// `project_missing_refusal` family).
fn run_row_missing_refusal() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("SyncRunRowMissing".to_owned()),
        os_message: Some(
            "the run row the summary was about is missing from the sync_runs table".to_owned(),
        ),
    });
    error
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
/// settings (the scan command reads the same table through its own
/// repository path, T-012/D-032's honest-boundary adaptation).
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
        .run_with_mirror(move |transaction, mirror| {
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
                // The T-324/D-228 mirror contract: record inside the
                // transaction, flush only after the commit — a no-op
                // for this info+user+ok shape (the journal already
                // holds it; `mirror_level` decides at flush), and the
                // single place a future level/visibility change
                // would pick up its mirroring.
                mirror.record(&entry);
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
    /// icon); see [`dirty_verdict`] for the cache-backed boundary
    /// this command serves it under (D-121, closed by T-254).
    pub(crate) dirty: bool,
    /// SYS-008's external-change flag at this read (MAIN-028's
    /// "externally modified" marking): the watch service's last known
    /// verdict, served under the same cache-backed boundary as
    /// [`ProjectColumnEntry::dirty`] — see [`watch_verdict`] (D-137's
    /// round-1 P1-1, landed by T-257).
    pub(crate) external_changed: bool,
    /// SYS-009's target-deleted hint at this read (MAIN-028's "target
    /// deleted" marking), from the same [`watch_verdict`] read; the
    /// two flags are mutually exclusive in the service's verdict.
    pub(crate) file_deleted: bool,
    /// SYS-010's lost-watch notice at this read (the non-blocking
    /// "this file is not being watched" marking, D-155): the watch
    /// service's last recorded refusal class for the project's
    /// target file, in the closed vocabulary `"gone" | "limit" |
    /// "other"` ([`file_watch::refusal_wire_class`]) — `None` when
    /// the file is watched (or no full pass has recorded yet). See
    /// [`unwatched_reason`] for the cache-backed boundary this
    /// command serves it under; the notice is deliberately a polled
    /// read, never an event (a refused watch is a persistent fact,
    /// D-137's P2-4 loop ruling).
    pub(crate) unwatched_reason: Option<String>,
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
/// `dirty` is the dirty service's last known verdict per project
/// (MAIN-028's dot icon), read from the managed [`DirtyState`]
/// cache through [`dirty_verdict`] — the same session state the
/// `state://dirty` diff is taken against (D-121, closed by T-254),
/// so every served verdict is one the service truthfully recorded.
/// The event is at-least-once and may carry a superseded change, so
/// a consumer reconciling the two treats this read as the truth.
/// The verdicts are read after the blocking availability probes, so
/// a slow folder cannot make this response arrive carrying dirt
/// older than an event the webview already applied. Before the
/// service's first pass the cache is empty and every verdict reads
/// clean — the service's own baseline; DB-014's archived half stays
/// row-decidable. The read is not gated by `E_SYNC_RUNNING`
/// (the T-105 gate covers project-*changing* commands; a read
/// during a sync is safe and answers the verdicts the running pass
/// will refresh).
///
/// `external_changed`/`file_deleted` are the SAME shape of read for
/// §5.7's watch verdicts (SYS-008/SYS-009, MAIN-028's "externally
/// modified"/"target deleted" markings, D-137's round-1 P1-1):
/// one [`file_watch::FileWatchState::snapshot`] of the managed
/// [`file_watch::FileWatchState`] cache — the state the
/// `state://file-changed` diff is taken against — taken after the
/// probes for the same staleness rule, served per row through
/// [`watch_verdict`]. The startup verdicts therefore reach ANY
/// caller of this command, not only a webview that was already
/// listening when the event fired (Tauri does not replay events);
/// the cold-start carrier is exactly what the follow-up existed
/// for.
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
    let probed: Vec<(eunomia_store::projects::Project, bool, Option<String>)> =
        tauri::async_runtime::spawn_blocking(move || {
            rows.into_iter()
                .map(|project| {
                    let (available, unavailable_reason) = availability_of(Path::new(&project.path));
                    (project, available, unavailable_reason)
                })
                .collect()
        })
        .await
        .map_err(db::join_failure_to_app_error)?;
    // The dirty verdicts are read from the session cache AFTER the
    // probes (the round-1 review's ordering fix): probes over dead
    // folders can stall for seconds, and a verdict snapshot taken
    // before them would let this response arrive carrying dirt older
    // than a `state://dirty` event the webview already applied —
    // reading after shrinks the race to the IPC boundary. The dirty
    // service is managed by `setup` before anything that could fail
    // (recovery mode included, D-112's shape), so a missing state is
    // an uninitialized-app bug; the honest degraded answer is the
    // clean baseline, not a failed read over a display flag —
    // [`dirty_verdict`]'s absent ≡ clean, one truth with the
    // service's diff rule.
    let dirty_cache = app.try_state::<DirtyState>();
    // The §5.7 watch verdicts (T-257): ONE snapshot read of the watch
    // service's session cache, taken AFTER the probes for the same
    // staleness rule the dirty read states above — probes over dead
    // folders can stall for seconds, and a snapshot taken before
    // them would let this response arrive carrying verdicts older
    // than a `state://file-changed` event the webview already
    // applied. One read, then per-row map lookups (linear in the
    // observed set, no re-read per row); the map is the carrier the
    // service's own `snapshot()` prepares (D-137's round-1 P1-1).
    // A missing state is the [`DirtyState`] degrade posture:
    // `setup` manages it before anything that could fail (recovery
    // mode included), so its absence is an uninitialized-app bug,
    // and the honest degraded answer is the clean baseline —
    // [`watch_verdict`]'s absent ≡ clean, one truth with the
    // service's diff rule.
    let watch_flags = watch_verdicts_of(&app);
    // The SYS-010 lost-watch notice (D-155): ONE read of the same
    // session cache's refused-watch recording, taken beside the
    // verdict snapshot for the same staleness rule, then per-row map
    // lookups. The notice is a polled read — never an event (a
    // refused watch is persistent; D-137's P2-4 loop ruling).
    let lost_watches = unwatched_reasons_of(&app);
    let entries = probed
        .into_iter()
        .map(|(project, available, unavailable_reason)| {
            // The verdict reads the row before the struct build moves
            // its fields.
            let dirty = dirty_verdict(dirty_cache.as_deref(), &project);
            let (external_changed, file_deleted) = watch_verdict(&watch_flags, &project, available);
            let lost_watch = unwatched_reason(
                lost_watches.get(project.id.as_str()).map(String::as_str),
                &project,
                available,
            );
            ProjectColumnEntry {
                id: project.id,
                name: project.name,
                label: project.label,
                color: project.color,
                state: project.state.as_str().to_owned(),
                available,
                unavailable_reason,
                dirty,
                external_changed,
                file_deleted,
                unwatched_reason: lost_watch,
                last_synced_at: project.last_synced_at,
            }
        })
        .collect::<Vec<_>>();
    Ok(entries)
}

/// DB-021's verdict as `projects_list` serves it (MAIN-028; D-121's
/// routed follow-up, closed by T-254): the row-decidable DB-014 half
/// plus the live flag read from the session cache the
/// `state://dirty` service maintains.
///
/// An archived project is never counted as dirty (DB-014 — the
/// rule's only half the §6.2 row decides alone, decided here
/// whatever a stale cache entry might still say). An active
/// project's verdict is the [`DirtyState`] cache's last recorded
/// value — the same map the service diffs its next `state://dirty`
/// event against, so a read is always consistent with the cache the
/// service last recorded. The event itself is at-least-once and may
/// carry a superseded change the cache no longer holds (the
/// service's overlap contract), so a consumer reconciling the two
/// treats this read as the truth — the event updates, the read
/// corrects. The divergence is transient in both directions: the
/// event may lead the cache by the service's emit→record window, and
/// the read lags it exactly that long — the next read settles either
/// way. A project the cache has not observed yet reads as the
/// baseline **clean** (D-121's diff rule: absent ≡ clean), and so
/// does a missing state entirely — `setup` manages it before
/// anything that could fail (recovery mode included), so its absence
/// is an uninitialized-app bug, and the honest degraded answer is
/// the clean baseline, not a failed read over a display flag.
///
/// The lookups are per row, not one atomic snapshot (the round-1
/// review's note): a concurrent pass recording between two rows can
/// mix pre- and post-pass verdicts in one response — every value
/// still a verdict the cache truthfully held, and cheaper than a
/// map clone per read; the next read (or event) settles it.
/// `pub(crate)`: the `sync_status` command (T-121, `sync_reads.rs`)
/// aggregates the SAME verdicts into the panel's dirty count — one
/// truth with the column and the event, never a second derivation.
pub(crate) fn dirty_verdict(
    cache: Option<&DirtyState>,
    project: &eunomia_store::projects::Project,
) -> bool {
    match project.state {
        // DB-014: an archived project takes part in no sync and is
        // never counted as dirty — the rule's only row-decidable
        // half, checked before the cache so a stale entry (archived
        // after the last pass, before the next one prunes it) cannot
        // leak a dot onto an archived column.
        ProjectState::Archived => false,
        // Active: the service's last known verdict, clean baseline
        // for the not-yet-observed.
        ProjectState::Active => cache
            .map(|cache| cache.last_verdict(&project.id))
            .unwrap_or(false),
    }
}

/// §5.7's watch verdict pair as `projects_list` serves it (SYS-008/
/// SYS-009, MAIN-028's "externally modified"/"target deleted"
/// markings; D-137's round-1 P1-1, landed by T-257): the watch
/// service's last recorded flags, read from the one
/// [`file_watch::FileWatchState::snapshot`] map this command took
/// after its blocking probes (the `dirty_verdict` staleness rule).
///
/// The shape mirrors [`dirty_verdict`], with the watch set's own
/// second row-decidable half: SYS-007's watch set is the ACTIVE,
/// AVAILABLE projects', and the service's full pass PRUNES every
/// leaver from its cache (an archived or unavailable project
/// contributes no verdict — `blocking_half` excludes both from the
/// covered set). This command probes availability itself, fresher
/// than the cache's last pass, so the row half is decided from the
/// row's `state` AND this response's own `available` fact — BEFORE
/// the cache — and the window between a folder dying (or a row being
/// archived — `project_archive`, since T-108) and the next full
/// pass's prune
/// cannot leak a stale marking onto a column that shows
/// unavailable/archived. An active, available project the service
/// has not observed (before its first pass, a service that never
/// started) reads as the clean baseline `(false, false)` — the
/// service's own diff rule (absent ≡ clean), one truth with the
/// `state://file-changed` event. A consumer reconciling the event
/// with this read treats the read as the truth; the event updates,
/// the read corrects — the same at-least-once posture as the dirty
/// read above.
fn watch_verdict(
    flags: &std::collections::HashMap<String, (bool, bool)>,
    project: &eunomia_store::projects::Project,
    available: bool,
) -> (bool, bool) {
    match (project.state, available) {
        // SYS-007: archived or unavailable rows are outside the watch
        // set; the row decides alone, whatever a stale cache entry
        // says.
        (ProjectState::Archived, _) | (ProjectState::Active, false) => (false, false),
        // Active and available: the service's last recorded pair,
        // clean baseline for the not-yet-observed.
        (ProjectState::Active, true) => flags
            .get(project.id.as_str())
            .copied()
            .unwrap_or((false, false)),
    }
}

/// SYS-010's lost-watch notice as `projects_list` serves it per row
/// (D-155): the watch service's last recorded refusal class for the
/// project's target file, `None` when nothing was lost — the
/// non-blocking notice the UI renders from this carrier. The shape
/// mirrors [`watch_verdict`] with the same second row-decidable
/// half: SYS-007's watch set is the ACTIVE, AVAILABLE projects', so
/// an archived row or a row this response just probed unavailable
/// reads `None` BEFORE the cache — "lost from observation" would be
/// a lie for a file that is not supposed to be watched (the stale
/// window between a folder dying and the next full pass's prune
/// cannot leak a lost-watch notice onto a column that shows
/// unavailable/archived). An active, available project the service
/// has not observed (before its first pass, a service that never
/// started, recovery mode) reads `None` too — the clean baseline,
/// one truth with the service's own recording rule. A consumer
/// re-reading this notice treats the read as the truth; it is a
/// poll, never an event.
fn unwatched_reason(
    recorded: Option<&str>,
    project: &eunomia_store::projects::Project,
    available: bool,
) -> Option<String> {
    match (project.state, available) {
        // SYS-007: outside the watch set by design — nothing to lose.
        (ProjectState::Archived, _) | (ProjectState::Active, false) => None,
        // Active and available: the recorded refusal class, if any.
        (ProjectState::Active, true) => recorded.map(str::to_owned),
    }
}

/// Takes the one [`file_watch::FileWatchState::unwatched_projects`]
/// read a `projects_list` call serves its rows from: the recorded
/// lost-watch classes keyed by project id, in their wire spellings.
/// A missing state (an uninitialized-app bug — `setup` manages it
/// before anything that could fail, recovery mode included) answers
/// the empty map, the no-losses baseline — the [`DirtyState`]
/// degrade posture, never a failed read over display flags.
fn unwatched_reasons_of<R: Runtime>(
    app: &AppHandle<R>,
) -> std::collections::HashMap<String, String> {
    app.try_state::<file_watch::FileWatchState>()
        .map(|state| {
            state
                .unwatched_projects()
                .into_iter()
                .map(|(id, kind)| (id, file_watch::refusal_wire_class(kind).to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// Takes the one [`file_watch::FileWatchState::snapshot`] read a
/// `projects_list` call serves its rows from: the managed state's
/// current verdicts keyed by project id. A missing state (an
/// uninitialized-app bug — `setup` manages it before anything that
/// could fail, recovery mode included) answers the empty map, the
/// clean baseline for every row — the [`DirtyState`] degrade
/// posture, never a failed read over display flags.
fn watch_verdicts_of<R: Runtime>(
    app: &AppHandle<R>,
) -> std::collections::HashMap<String, (bool, bool)> {
    app.try_state::<file_watch::FileWatchState>()
        .map(|state| {
            state
                .snapshot()
                .into_iter()
                .map(|change| {
                    (
                        change.project_id,
                        (change.external_changed, change.file_deleted),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
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
            existing: Some(ExistingProject::of(existing)),
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
/// `"manual"`). The same DTO is the `"added"` entry's payload of the
/// scan commit (T-077, `scan.rs`) — one wire shape for one created row.
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
    /// How the project was registered (§6.2): `"manual"` from
    /// `project_add_manual`, `"scan"` from `scan_commit`.
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
    /// warning. `pub(crate)` for the scan commit's `"added"` entries
    /// (T-077): both add flows serve one created row the same way.
    pub(crate) fn from_store(
        project: &eunomia_store::projects::Project,
        warning: Option<String>,
    ) -> Self {
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
/// restore offer (`project_restore`, T-109) calls back with. The scan
/// commit's `"already_added"`/`"archived"` entries carry the same
/// view (T-077).
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

impl ExistingProject {
    /// The wire view of one already-registered row — the constructor
    /// behind both duplicate arms of the manual add and the scan
    /// commit's non-add entries (T-077).
    pub(crate) fn of(project: &eunomia_store::projects::Project) -> Self {
        ExistingProject {
            id: project.id.clone(),
            name: project.name.clone(),
            path: project.path.clone(),
            state: project.state.as_str().to_owned(),
        }
    }
}

/// Registers the folder the user picked in the system dialog as a
/// project (§7.10 Projects module `project_add_manual`, the D-009
/// fixed name; §7.1 PRJ-001; scenario S1).
///
/// `path` is the folder the UI's `dialog_pick_folder` answer named,
/// and the command's first act is the SYS-005 sync gate on it —
/// while a sync holds `sync_running`, the answer is the typed
/// `E_SYNC_RUNNING` refusal before anything else runs (see
/// [`reject_during_sync`]); with the flag down, the SEC-003 gate —
/// [`session_paths::validate_ui_path`], the T-073/D-100 boundary —
/// is the first per-input act: a path that neither canonicalizes
/// onto a registered project nor onto a folder picked in this
/// session is rejected with `E_VALIDATION` before anything else
/// runs (a foreign folder never reaches the `.git` check, the
/// database, or the disk probes).
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
///    not "not a repo", D-090); a root whose canonical form is not
///    valid UTF-8 is the D-086-addendum encoding rejection (keys and
///    stored paths are lossless), the same envelope the dialog and
///    session gates answer.
/// 2. **The root offer** (PRJ-003): when the picked folder is a
///    subfolder and `accept_repo_root` is not `Some(true)`, the answer
///    is the `"repo_root_offered"` arm — the app *offers* the root,
///    it does not silently register a different folder than the one
///    the user picked. Nothing is written. The UI's confirm re-sends
///    the same picked path with the flag; the root is re-derived
///    (idempotent converge), so SEC-003's allowlist is never widened
///    with an app-derived path (D-100).
/// 3. **Fields** (§6.9), through the shared
///    [`ProjectFields::derive`] — the one implementation the scan
///    commit (T-077) calls too: the name defaults to the root's
///    folder name (`validate_name`) and the label to DB-042's
///    derivation of it (`default_label`, then `validate_label` — the
///    recorded edge where a control-character prefix derives an
///    unstorable label is an `E_VALIDATION` naming the field, not a
///    panic). The description enters as the empty constant (the add
///    flow collects none, §7.1) through `validate_description` — the
///    D-151 NUL addendum's save boundary: the stored value is
///    validator-passed by construction, the silent `U+0000` strip
///    included.
/// 4. **One transaction** (D-037, D-051, §6.9 "the whole operation
///    runs in one transaction"): the DB-015 duplicate check
///    (`find_by_path_key`, either state — active answers the
///    `"duplicate_active"` message, archived the
///    `"archived_duplicate"` restore offer; both typed answers, no
///    error, nothing written); then the shared [`register_project`]
///    — the four write settings copied from `default.*`
///    (PRJ-012/DEF-005, read inside the transaction so a concurrent
///    `settings_set` cannot split the copy from the create); the D-29
///    color over the ACTIVE projects' colors; `create` (the
///    repository appends `sort_order`); and the `project.added`
///    journal entry (LOG-001…004, info/user/ok, entity project)
///    **inside the same transaction** — a rolled-back add leaves no
///    entry claiming it happened. The
///    SYS-005 sync gate runs before the whole flow (the command's
///    first act above): during a sync the add answers
///    `E_SYNC_RUNNING` and nothing is written; the spec's own
///    project-*editing* list joins the same gate — `project_update`
///    since T-106, `project_relink` since T-110, and
///    `project_remove` and P1 `backup_restore` as they land.
/// 5. **Write-access warning** (§6.9, T-063/D-094): after the commit,
///    [`eunomia_fs::write_access::check_write_access`] over the
///    root's future target file, on the blocking pool (never inside
///    the transaction — a dead drive must not hold the one database
///    lock, the D-096 probe-order rule). The verdict travels as the
///    response's `write_access_warning`; the project stays added.
/// 6. **The watch refresh** (§5.7 SYS-007, T-257/D-137's follow-up
///    2): the committed row joins the watched set, so the answer
///    waits for one FULL pass through
///    [`file_watch::refresh_after_mutation`] — the BOUNDED twin of
///    the seam the sync pipeline runs after its writes (the same
///    `Refresh` command and ack, never a second refresh path; the
///    budget since T-280/D-156, so a pass stuck on a dead mount
///    degrades instead of hanging this answer), on the service task,
///    its blocking half on the pool. The
///    refresh is never-fatal aftermath (D-138's watch posture): the
///    project IS added whatever the pass answers — a refusal or the
///    budget's expiry degrades with its class-only warn
///    ([`logging::file_watch_mutation_refresh_degraded`], T-278: the
///    gone service, the pass's §10 failure, the timeout — one line
///    per command; the command is the episode), and the next anchor
///    (an availability change, a debounced file event, the next sync
///    or mutation) re-converges the set, because every full pass
///    re-derives it from the project table. The other SYS-005-gated
///    mutations change no watched file (`projects_reorder` moves
///    sort_order, `exclusion_remove` touches no project row) and run
///    no refresh; the spec's scheme/name/restore/relink commands call
///    the same seam (`project_update`'s arm since T-107,
///    `project_restore`'s since T-109, `project_relink`'s since
///    T-110), and the archive (T-108) is the deliberate non-member:
///    the leaver exits the set at the next anchor's pass.
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
/// `pub(crate)` for the ipc suite's call-site logging tests (T-278),
/// which drive it directly on the test thread so the thread-local
/// capture sees its own lines (the `pipeline_refusal` precedent; a
/// real-IPC invoke runs on runtime threads the capture cannot see).
pub(crate) async fn project_add_manual_impl<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    accept_repo_root: Option<bool>,
) -> Result<ProjectAddResult, AppError> {
    // SYS-005 (§5.4): the mode gate runs before everything — while a
    // sync holds `sync_running`, the add is refused with
    // `E_SYNC_RUNNING` whatever the input is (see
    // `reject_during_sync`; the SEC-003 gate below is the input gate
    // and still never lets a foreign folder through — during a sync
    // the foreign path simply meets the mode answer first).
    reject_during_sync(&app)?;
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
    // The §6.9 fields plus the identity — the shared "Fields" step
    // (see the command docs, step 3); the rejection of an unstorable
    // derived field is the `E_VALIDATION` envelope naming the field.
    let fields = ProjectFields::derive(&root)?;

    // The transaction half: everything the operation writes happens in
    // one `Db::run` (D-037). The probe keeps its own copy of the root
    // outside.
    let probe_root = root.clone();
    let outcome = database
        .run_with_mirror(move |transaction, mirror| {
            // DB-015: the key is unique across active AND archived
            // rows, and the check runs inside the operation's own
            // transaction — the same single-connection serialization
            // D-037 gives every writer, so the create below cannot
            // race past it.
            if let Some(existing) =
                eunomia_store::projects::find_by_path_key(transaction, &fields.path_key)
                    .map_err(projects_store_to_app_error)?
            {
                return Ok(AddOutcome::Refused(ProjectAddResult::duplicate_of(
                    &existing,
                )));
            }
            // The shared create step (see the command docs, step 4):
            // the PRJ-012 copy and D-29's input are read HERE, inside
            // the transaction, so the copy is atomic with the create
            // (a concurrent `settings_set` cannot split them).
            let defaults = WriteDefaults::load(transaction)?;
            let mut occupancy = ColorOccupancy::load(transaction)?;
            Ok(
                match register_project(
                    transaction,
                    mirror,
                    &fields,
                    eunomia_store::projects::AddedVia::Manual,
                    &defaults,
                    &mut occupancy,
                )? {
                    Registered::Created(created) => AddOutcome::Created(created),
                    // The one write race the check above cannot
                    // produce (one connection serializes every writer,
                    // D-037) — and if it ever fires anyway, the honest
                    // answer is the SAME typed duplicate outcome, not
                    // the read-command fallback whose diagnostic would
                    // misname the flow and whose `E_DB_LOCKED` code
                    // would invite a retry (the round-1 review finding
                    // of D-103). Convergence, never a masked conflict.
                    Registered::Existing(existing) => {
                        AddOutcome::Refused(ProjectAddResult::duplicate_of(&existing))
                    }
                },
            )
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
            // Step 6 — the watch refresh (§5.7 SYS-007, T-257): the
            // committed row joins the watched set, so one FULL pass
            // runs through the mutation seam — the same awaited pass
            // the sync pipeline runs after its writes (one refresh
            // path). When the pass answers within the budget, the
            // converged set and the emitted first-observation diff
            // are in place by the time the answer lands (a
            // pre-existing differing target file is therefore
            // already reported, SYS-008's literal reading); on
            // expiry the verdict arrives with the NEXT anchor's
            // event/read instead (D-144's reconciliation rule).
            // BOUNDED since T-280 (D-156): the interactive
            // answer waits at most the recorded budget — a pass
            // stuck on a dead mount degrades instead of hanging
            // the response, and the pass keeps converging on the
            // service task whatever this caller does. NEVER-FATAL
            // (the command docs' step 6): the project IS added
            // whatever the pass answers — the refusal arms below
            // (recovery mode is unreachable here, the flow holds a
            // database; the real case is a watcher backend that
            // refused to start, when no service task exists and the
            // send fails fast) degrade with their class-only warn,
            // and the next watch anchor re-converges the set (the
            // warn line is `file_watch_mutation_refresh_degraded`,
            // T-278 — the D-156 addendum's class set: gone, the
            // pass's §10 failure, the timeout; every class the caller
            // OBSERVES is logged exactly once — a pass failure the
            // budget outlives answers the timeout here and is logged
            // nowhere else, the D-156 addendum's recorded window).
            if let Err(refused) = file_watch::refresh_after_mutation(&app).await {
                logging::file_watch_mutation_refresh_degraded(&refused);
            }
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

// ---------------------------------------------------------------------------
// The new-project rules both add flows share (T-077, D-179): §6.9,
// DB-030, DB-042, PRJ-012, D-29, LOG-001…004, D-051
//
// ONE implementation of "how a repository root becomes a project row".
// `project_add_manual` (S1) and the scan commit (PRJ-002's "Add
// selected", `scan.rs`) both call the four items below, so a change to
// an add rule — a new `default.*` key, a new derivation — lands in one
// place and cannot leave the two flows disagreeing.
// ---------------------------------------------------------------------------

/// Everything a new project row derives from its canonical repository
/// root before the store is involved: S1's "Fields" step (§6.9) plus
/// the row's identity (DB-030).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectFields {
    /// The root's folder name, `validate_name`-passed (§6.9: 1–100
    /// characters after trimming).
    pub(crate) name: String,
    /// DB-042's derivation of the name (`default_label`, then
    /// `validate_label`: 1–6 visible characters).
    pub(crate) label: String,
    /// The add flows collect no description (§7.1): the constant empty
    /// text, `validate_description`-passed (the D-151 NUL addendum's
    /// save boundary — a NUL never reaches the column).
    pub(crate) description: String,
    /// The registered identity of the root — the key `find_by_path_key`
    /// answers and `create` stores (DB-030/DB-031).
    pub(crate) path_key: String,
    /// The stored `projects.path`: the canonical root's verbatim UTF-8
    /// text (DB-030; the D-086 addendum keeps keys and stored paths
    /// lossless).
    pub(crate) path: String,
}

impl ProjectFields {
    /// S1's "Fields" step over a canonical repository root. Pure over
    /// the path text — no I/O: `root` is the canonical form the caller
    /// already resolved.
    ///
    /// # Errors
    ///
    /// `E_VALIDATION` naming the failed field: the folder name the
    /// name validator refuses (the filesystem root itself has none —
    /// the app cannot register a nameless row the schema CHECKs would
    /// reject), and the recorded D-079 edge where a legal name's DB-042
    /// derivation is a label §6.9 cannot store (a control-character
    /// prefix). A root whose canonical text is not valid UTF-8 answers
    /// the D-086 encoding rejection — unreachable past a caller that
    /// went through `canonicalize` (its own gate refused), typed
    /// instead of assumed.
    pub(crate) fn derive(root: &Path) -> Result<Self, AppError> {
        let name = derived_name(root)?;
        let label_default = eunomia_core::project_validators::default_label(&name);
        let label = eunomia_core::project_validators::validate_label(&label_default)
            .map_err(|source| rejected_field("label", &label_default, &source.to_string()))?
            .to_owned();
        // The description crosses the same save boundary even though
        // the add flows collect none (§7.1): the value stored today is
        // the constant empty string — the PRJ-008 modal's edit path
        // landed with `project_update` (T-106; the modal's read half
        // is the UI task's) — routed through the
        // validator so every description THESE FLOWS store is
        // validator-passed by construction (the store itself does not
        // enforce it — store-level writers must route through the
        // validator, the D-151 NUL addendum's seam).
        let description = eunomia_core::project_validators::validate_description("")
            .map_err(|source| rejected_field("description", "", &source.to_string()))?;
        let path_key = eunomia_fs::paths::path_key(root)
            .map_err(|source| rejected_non_utf8_path(source.path()))?;
        let path = root
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| rejected_non_utf8_path(&root.to_string_lossy()))?;
        Ok(Self {
            name,
            label,
            description,
            path_key,
            path,
        })
    }
}

/// The PRJ-012 copy: the four write settings a new project takes from
/// the `default.*` keys at its creation. The copy is a copy — later
/// default changes never reach the row (DEF-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WriteDefaults {
    /// `default.scheme` → `projects.scheme`.
    pub(crate) scheme: eunomia_store::settings::Scheme,
    /// `default.base_name` → `projects.base_name`.
    pub(crate) base_name: eunomia_store::settings::BaseName,
    /// `default.extra_local` → the `AGENTS.local.md` hint columns.
    pub(crate) extra_local: eunomia_store::settings::ExtraFile,
    /// `default.extra_project` → the `AGENTS.project.md` hint columns.
    pub(crate) extra_project: eunomia_store::settings::ExtraFile,
}

impl WriteDefaults {
    /// Reads the four keys through the store's single read path
    /// (D-039) inside the caller's transaction — so the copy is atomic
    /// with the create that consumes it (a concurrent `settings_set`
    /// cannot split them). A missing row answers the §6.5 default; a
    /// corrupt cell answers the settings corrupt fallback
    /// ([`settings_cell_to_app_error`], never `E_VALIDATION` — the
    /// T-027 round-2 rule: corruption is not the caller's input).
    ///
    /// # Errors
    ///
    /// The settings corrupt fallback for a cell that cannot be read,
    /// and the recorded D-037 fallback for a value of the wrong variant
    /// (unreachable: `get` re-parses through the key's §6.5 contract).
    pub(crate) fn load(transaction: &rusqlite::Transaction<'_>) -> Result<Self, AppError> {
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
        let extra_project = match stored_default(transaction, SettingKey::DefaultExtraProject)? {
            SettingValue::ExtraFile(extra) => extra,
            _ => return Err(unpaired_setting(SettingKey::DefaultExtraProject)),
        };
        Ok(Self {
            scheme,
            base_name,
            extra_local,
            extra_project,
        })
    }
}

/// D-29's input over ONE transaction: how many ACTIVE projects hold
/// each palette color, read once when the transaction starts and kept
/// current by [`record`](Self::record) as the transaction creates rows
/// — so the Nth row of a batch is colored against the active set as it
/// stands at that moment, earlier rows of the same batch included
/// (exactly what the manual add's per-add read gives a sequence of
/// single adds).
///
/// `pick_color` (core, the rule's one owner) takes the occupied colors
/// as a multiset slice and reads only the counts RELATIVE to the
/// least-used member: the least count selects the candidates and the
/// candidate count indexes the tie-break. Subtracting that minimum from
/// every count changes neither — so this type hands `pick_color` the
/// reduced multiset (one entry per unit above the minimum), whose size
/// is bounded by the active set that existed when the transaction
/// began and never grows with the batch. A batch is therefore linear
/// in its rows; re-listing the table per row would be quadratic (and a
/// growing full multiset would re-count it per row).
#[derive(Debug)]
pub(crate) struct ColorOccupancy {
    /// One count per palette member, in `Color::ALL` order.
    counts: [usize; Color::ALL.len()],
}

impl ColorOccupancy {
    /// The occupancy of a multiset of colors, one entry per ACTIVE
    /// project (the `occupied_colors` shape).
    pub(crate) fn from_colors(occupied: impl IntoIterator<Item = Color>) -> Self {
        let mut counts = [0usize; Color::ALL.len()];
        for color in occupied {
            counts[color_index(color)] += 1;
        }
        Self { counts }
    }

    /// The D-29 input: the colors held by the ACTIVE projects, read
    /// through the projects repository inside the caller's
    /// transaction. The `color` column has no schema CHECK (§6.2), so a
    /// stored cell outside the palette can only be corruption or a
    /// newer app version — the D-096 corrupt-cells fallback with a
    /// class-only diagnostic, never a guess.
    ///
    /// # Errors
    ///
    /// The store fallbacks of the active list read
    /// ([`projects_store_to_app_error`]) and of a stored color outside
    /// the palette (`ProjectsCorrupt`).
    pub(crate) fn load(transaction: &rusqlite::Transaction<'_>) -> Result<Self, AppError> {
        Ok(Self::from_colors(occupied_colors(transaction)?))
    }

    /// The color the next created row takes by D-29 over the counts as
    /// they stand: the least-used palette member, ties broken by the
    /// SHA-256 of `tie_break` (core's `pick_color`, the rule's one
    /// owner). Pure — [`record`](Self::record) the answer once the row
    /// exists.
    pub(crate) fn pick(&self, tie_break: &ProjectId) -> Color {
        // The reduced multiset (see the type docs): the minimum
        // exists — the palette is never empty.
        let least = *self
            .counts
            .iter()
            .min()
            .expect("the palette is never empty");
        let mut reduced: Vec<Color> = Vec::new();
        for (color, count) in Color::ALL.iter().zip(self.counts.iter()) {
            for _ in least..*count {
                reduced.push(*color);
            }
        }
        eunomia_core::color_pick::pick_color(&reduced, tie_break)
    }

    /// Records that an ACTIVE project now holds `color` — called once
    /// per created row, so the rest of the transaction sees it.
    pub(crate) fn record(&mut self, color: Color) {
        self.counts[color_index(color)] += 1;
    }
}

/// The palette position of a member — an invariant, not an input
/// check: `Color` has no value outside `Color::ALL`.
fn color_index(color: Color) -> usize {
    Color::ALL
        .iter()
        .position(|member| *member == color)
        .expect("every Color is a palette member")
}

/// What [`register_project`] answered.
#[derive(Debug)]
pub(crate) enum Registered {
    /// The row was created and journaled; the caller's `Db::run`
    /// commits it.
    Created(eunomia_store::projects::Project),
    /// `create` met the `path_key` UNIQUE constraint and the re-read
    /// found the row that holds the key — nothing was written by this
    /// call. Reachable only when no duplicate check preceded the call
    /// in the same transaction (one connection serializes every
    /// writer, D-037); the honest answer is still the existing row,
    /// never a masked conflict.
    Existing(eunomia_store::projects::Project),
}

/// The in-transaction create step both add flows share: D-29's color
/// over the occupancy, the store's `create` (the repository appends
/// `sort_order`), and the `project.added` journal entry (LOG-001…004,
/// info/user/ok, entity and project naming the new row) inside the
/// SAME transaction (D-051) — a rolled-back operation leaves no entry
/// claiming it happened, and the T-324/D-228 mirror buffer holds the
/// entry's record until the commit flushes it.
///
/// The caller owns the duplicate check ([`eunomia_store::projects::
/// find_by_path_key`], DB-015) and maps its typed answer; this step
/// only converges if `create` reports the conflict anyway.
///
/// # Errors
///
/// `E_DB_LOCKED`-coded store fallbacks: a `create` the SQLite CHECKs
/// refused or the database could not carry (through
/// [`projects_store_to_app_error`]), the base classifier's verdict for
/// a failed journal insert ([`activity_log_to_app_error`]), and the
/// D-037 fallback when a duplicate conflict vanishes on the re-read.
pub(crate) fn register_project(
    transaction: &rusqlite::Transaction<'_>,
    mirror: &mut crate::logging::MirrorBuffer,
    fields: &ProjectFields,
    added_via: eunomia_store::projects::AddedVia,
    defaults: &WriteDefaults,
    occupancy: &mut ColorOccupancy,
) -> Result<Registered, AppError> {
    // D-29/DB-041: the least-used palette color among ACTIVE
    // projects, tie-broken by a fresh UUIDv7's hash. The row's own id
    // is minted inside `create` and cannot be known before the color
    // is chosen; the tie-break therefore hashes a pre-generated
    // sibling id — both are fresh UUIDv7s at decision time, which is
    // the determinism D-29's parenthetical asks of a decision point
    // (D-103).
    let color = occupancy.pick(&ProjectId::new(eunomia_store::values::new_id()));
    // The stored path is the verbatim canonicalize output (DB-030) —
    // and, by the D-086 addendum, always valid UTF-8 (`ProjectFields`
    // refused anything else), so the TEXT-column boundary between the
    // row and the key is lossless.
    let new = eunomia_store::projects::NewProject {
        path: &fields.path,
        path_key: &fields.path_key,
        name: &fields.name,
        label: &fields.label,
        color: color.as_str(),
        description: &fields.description,
        added_via,
        scheme: defaults.scheme,
        base_name: defaults.base_name,
        extra_local: defaults.extra_local,
        extra_project: defaults.extra_project,
    };
    let created = match eunomia_store::projects::create(transaction, &new) {
        Ok(created) => created,
        Err(ProjectsError::DuplicatePathKey { .. }) => {
            let existing = eunomia_store::projects::find_by_path_key(transaction, &fields.path_key)
                .map_err(projects_store_to_app_error)?
                .ok_or_else(|| {
                    projects_fallback(
                        "ProjectsError",
                        "a duplicate path key conflict vanished on the re-read",
                    )
                })?;
            return Ok(Registered::Existing(existing));
        }
        Err(other) => return Err(projects_store_to_app_error(other)),
    };
    // LOG-001…004 ("adding … projects") inside the operation's own
    // transaction (D-051): entity and project carry the new id; no
    // details — the row itself holds the path (these flows write no
    // file; the one non-file-write `File` detail the app produces is
    // `exclusion_remove`'s, recorded in D-106).
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
    // The T-324/D-228 mirror contract: record inside the transaction,
    // flush only after the commit (info+user+ok is the unmirrored
    // shape today).
    mirror.record(&entry);
    // The row and its entry both exist (uncommitted): its color is
    // occupied from here on — recorded only now, so every recorded
    // color belongs to a fully written row whatever failed earlier.
    occupancy.record(color);
    Ok(Registered::Created(created))
}

/// The D-086-addendum encoding rejection (owner ruling 2026-11-20,
/// escalation variant 2): a path whose text is not valid UTF-8 can
/// never become a registered identity — keys and stored paths are
/// always lossless — so every boundary that meets one refuses it with
/// THIS one envelope: `E_VALIDATION` naming the path field, a lossy
/// rendering of the offending path echoed back (the only printable
/// form — at the session gate literally the caller's own input, at
/// the other seams the lossy spelling of the refused path; a
/// diagnostic, never stored, and paths may ride `params`, SEC-008),
/// and the honest reason naming the encoding and the fix.
/// `pub(crate)`
/// for the session gate's and the picker's reuse (the
/// [`rejected_field`] shape); no new §10 code — the registry has none
/// for this fact, and `E_VALIDATION`'s field/message family is
/// exactly the settings-shape seam.
pub(crate) fn rejected_non_utf8_path(path: &str) -> AppError {
    rejected_field(
        "path",
        path,
        "the folder path contains a name that is not valid UTF-8: the project cannot be \
         registered (rename the folder)",
    )
}

/// Maps the root-lookup failure onto §10 (the PRJ-003 half of the
/// flow): the negative lookup (`NoGitEntry` — every ancestor probed,
/// no `.git` anywhere) is `E_NOT_A_REPO`, §10's row for "a folder
/// without `.git` is being added"; a refused probe or a refused
/// canonicalization is `E_PROJECT_UNAVAILABLE` — "could not check" is
/// not "checked and absent" (the D-090 split; §10 keeps the two
/// apart, and the user actions differ: retry or fix access vs. pick
/// the repository root). The one exception inside the refusal bucket
/// is the D-086 addendum's UTF-8 fact: a canonical form that is not
/// valid UTF-8 is not an availability problem at all (nothing moved,
/// nothing is unreadable) — it is the same encoding rejection the
/// boundaries answer, so it gets the same typed envelope. The walked
/// path travels in `params` (SEC-008: paths may) and the OS refusal,
/// when there is one, in `details` (§10's first-class log fact).
/// `pub(crate)` for the scan commit (T-077), whose canonicalization
/// and root checks fail the same two ways.
pub(crate) fn repo_lookup_failed(source: &NotARepoError) -> AppError {
    let code = match source {
        NotARepoError::NoGitEntry { .. } => ErrorCode::NotARepo,
        NotARepoError::Canonicalize {
            source: eunomia_fs::paths::CanonicalizeError::NotUtf8 { .. },
        } => return rejected_non_utf8_path(source.path()),
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
/// `pub(crate)` for the scan commit's per-row probe (T-077).
pub(crate) fn probe_target_name(
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
/// re-checks before any write. `pub(crate)` for the scan commit's
/// per-row probe (T-077).
pub(crate) fn write_access_warning(source: &WriteAccessError) -> Option<String> {
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
/// nothing). While a sync holds the global `sync_running` flag, the
/// removal is refused up front with the typed `E_SYNC_RUNNING`
/// envelope instead (SYS-005, [`reject_during_sync`]) — nothing is
/// written, and the screen's retry after the run ends converges.
///
/// One transaction (D-037/D-051, §6.9 "the whole operation runs in
/// one transaction"): the by-id lookup through the repository's
/// ordered list (the store's public face has no by-id read, D-093;
/// the list is exactly the screen's own data, screen-sized), the
/// physical `DELETE`, and the journal entry — an `exclusion.removed`
/// record at info/user/ok whose entity columns name the removed row
/// (`EntityType::Exclusion` + its id) and whose `Details::File`
/// carries the removed path (SEC-008 permits paths, and it is exactly
/// the fact the screen shows). The member is the D-106 tracked
/// follow-up, landed by D-119: until it, this command journaled
/// `project.changed` with no entity — the frozen vocabulary's only
/// defensible shape — and those legacy rows keep reading unchanged
/// (both spellings are members of the same closed vocabulary; the
/// store's compatibility test pins the old shape beside the new).
/// `project_id` stays unset either way (no project row is involved;
/// `manual` exclusions have none). A failure between the writes rolls
/// both back: the journal never claims a removal that did not commit
/// (the D-051 in-transaction rule, pinned here in the settings
/// rollback test's shape).
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
    // SYS-005 (§5.4): a path exclusion is a project-changing write —
    // while a sync holds `sync_running`, the removal is refused with
    // `E_SYNC_RUNNING` before the database is even resolved (see
    // `reject_during_sync`).
    reject_during_sync(&app)?;
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
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
            // The journal entry inside the operation's own transaction
            // (D-051): `exclusion.removed` — the D-106 follow-up shape
            // (D-119): the entity columns name the removed row, the
            // File detail carries its path; no `project_id` (no
            // project row is involved).
            let mut entry = LogEntry::new(
                Action::ExclusionRemoved,
                Level::Info,
                Visibility::User,
                Outcome::Ok,
            );
            entry.entity = Some(Entity {
                kind: EntityType::Exclusion,
                id: exclusion.id.as_str(),
            });
            entry.details = Some(Details::File {
                path: PathBuf::from(&exclusion.path),
            });
            insert(transaction, &entry).map_err(activity_log_to_app_error)?;
            // The T-324/D-228 mirror contract: record inside the
            // transaction, flush only after the commit (info+user+ok
            // is the unmirrored shape today).
            mirror.record(&entry);
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
/// duplicate-race rule). While a sync holds the global `sync_running`
/// flag, the drag is refused before any of this with the typed
/// `E_SYNC_RUNNING` envelope (SYS-005, [`reject_during_sync`]) — the
/// table is untouched, and the UI's re-send after the run ends
/// converges on the then-current order.
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
    // SYS-005 (§5.4): the column order is project-changing state —
    // while a sync holds `sync_running`, the drag is refused with
    // `E_SYNC_RUNNING` before the database is even resolved (see
    // `reject_during_sync`); the UI re-sends it when the run ends.
    reject_during_sync(&app)?;
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
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
            // The T-324/D-228 mirror contract: record inside the
            // transaction, flush only after the commit (info+user+ok
            // is the unmirrored shape today).
            mirror.record(&entry);
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

// ---------------------------------------------------------------------------
// `library_get` (T-083, §7.10 Library module, §7.3 LIB-001/008/014,
// spec-table D-14, LIB-032; D-187)
// ---------------------------------------------------------------------------

/// One row of the `library_get` tree: an instruction or a section
/// heading, tagged `kind` with the §6.2 literals (`"instruction"`,
/// `"heading"` — the spellings the core domain type serializes to).
///
/// The tree is ONE flat vector in `position` order (§7.3: one
/// structure for all projects; OUT-001: the file order equals the
/// library order). The order is expressed as collection order and the
/// position NUMBER is deliberately not repeated on the wire — the
/// core `library` precedent: one fact, one place, nothing to drift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum LibraryTreeItem {
    /// An instruction row (§2 "Instruction").
    Instruction(LibraryInstructionEntry),
    /// A section heading row (§2 "Section heading").
    Heading(LibraryHeadingEntry),
}

/// An instruction's tree row — the LIB-001/008 field set plus the
/// inline variant list and the assignment labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct LibraryInstructionEntry {
    /// The row id (UUIDv7 TEXT, DB-002).
    pub(crate) id: String,
    /// The instruction's main name (§6.9, 1–200 after trimming).
    pub(crate) title: String,
    /// `"active"` or `"hidden"` — the §6.4 state the "Show hidden"
    /// filter (D-14) and the dashed chip (MAIN-013) read.
    pub(crate) state: String,
    /// The number of ACTIVE variants (DB-010's own scope; a
    /// soft-deleted variant is not counted) — the card's LIB-001
    /// field, derived in the same pass that builds
    /// [`Self::variants`].
    pub(crate) variant_count: u32,
    /// Creation time, DB-002 UTC — the row's own value.
    pub(crate) created_at: String,
    /// Last snapshot-visible write of the INSTRUCTION row, DB-002 UTC
    /// — a title rename or a hide/restore restamps it. A variant's
    /// own edits restamp the variant's row, not this one; a pure
    /// position move and a main-flag promotion restamp nothing (the
    /// store's §9.8/D-16 rule, `library.rs`'s module docs).
    pub(crate) updated_at: String,
    /// The instruction's ACTIVE variants, in the store's creation
    /// order (`created_at, id`). The main variant is identified by
    /// its `is_main` flag, not by its place: a `variant_set_main`
    /// promotion moves the flag without reordering the list (LIB-002's
    /// "main first" is the editor's own display rule).
    pub(crate) variants: Vec<LibraryVariantEntry>,
    /// The colored labels (LIB-001): exactly the ACTIVE projects with
    /// an assignment on this instruction, in the projects' own
    /// `sort_order`, each naming the assigned variant. An archived
    /// project's kept assignment rows (DB-014) never label — the
    /// labels sit on rows of a matrix whose columns are the active
    /// projects (MAIN-001, D-187).
    pub(crate) project_labels: Vec<ProjectLabelEntry>,
}

/// One variant of the served instruction — the tree's own view, not
/// the editor's: identity and the two flags the row UI reads, no
/// content (the variant-editor command family owns the text; D-187).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct LibraryVariantEntry {
    /// The variant's id (DB-002) — the identity `assignment_set` and
    /// the variant commands address.
    pub(crate) id: String,
    /// The variant's name (§6.9, unique among the instruction's
    /// active variants, NOCASE).
    pub(crate) name: String,
    /// The main-variant flag (DB-010): exactly one active variant of
    /// an instruction carries it — the click-assign default
    /// (MAIN-001) and the LIB-024 delete disable read it.
    pub(crate) is_main: bool,
    /// LIB-014: this variant is EMPTY in D-13's sense — "outputs no
    /// text" — decided by the build's own oracle
    /// (`normalize::variant_text`, ALG-002), so a whitespace-only
    /// stored body counts exactly as it counts in a built file. The
    /// empty-variant icon and the dimmed style read it; an empty
    /// variant is assignable (LIB-015, D-13) and the emptiness never
    /// blocks anything.
    pub(crate) is_empty: bool,
}

/// One project label of an instruction row (LIB-001): the chip's own
/// facts plus which variant the assignment names, or `None` for a
/// custom-text cell — the acceptance criterion's pair (labels match
/// assignments).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ProjectLabelEntry {
    /// The project's id — the identity the matrix column and the
    /// assignment commands share.
    pub(crate) project_id: String,
    /// The project's display name (the chip's tooltip text, the
    /// MAIN-28 pattern).
    pub(crate) name: String,
    /// The color-badge label, 1–6 characters (DB-042).
    pub(crate) label: String,
    /// The palette member (DB-040) the chip renders.
    pub(crate) color: String,
    /// The assigned variant's id, or `None` when this project carries
    /// per-project custom text (MAIN-007). D-02 still allows only one
    /// assignment target per project/instruction pair.
    pub(crate) variant_id: Option<String>,
}

/// A heading's tree row (§2 "Section heading", LIB-030).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct LibraryHeadingEntry {
    /// The row id (DB-002).
    pub(crate) id: String,
    /// The heading's title (§6.9, 1–200 after trimming).
    pub(crate) title: String,
    /// The heading level, 2–6 (`##`…`######`).
    pub(crate) level: u8,
    /// The LIB-032 skip flag: this heading ARRIVES more than one
    /// level deeper than the heading before it (`##` then `####`).
    /// Legal input marked with the warning icon — never a validation
    /// (D-072); computed by [`eunomia_core::gaps::heading_level_gaps`]
    /// over the served collection.
    pub(crate) level_skipped: bool,
    /// The skipped-from level (LIB-032/T-349, D-199): the pair's
    /// other half — the previous heading's level, 2–6 — exactly when
    /// this row IS the deep half of a gap (the same rows
    /// `level_skipped` is true for); `None` for every other heading.
    /// One report (`heading_level_gaps`) builds both facts.
    pub(crate) from_level: Option<u8>,
}

/// The `library_get` command's read, gathered by the one transaction
/// (D-037) and assembled outside it — the four lists the tree is
/// built from, all read inside the same snapshot.
struct LibraryRead {
    /// The items in `position` order (the served scope).
    items: Vec<eunomia_store::library::LibraryItem>,
    /// Every active variant of every instruction (the snapshot's own
    /// read, grouped per instruction by construction).
    variants: Vec<eunomia_store::library::Variant>,
    /// The active projects in `sort_order` — the label carriers.
    projects: Vec<eunomia_store::projects::Project>,
    /// The active projects' assignment rows, in the same order.
    assignments: Vec<eunomia_store::library::Assignment>,
}

/// The `library_get` answer (§7.10, D-200): the served tree plus the
/// D-14 number — the count of hidden instructions the default scope
/// does NOT serve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct LibraryGetResult {
    /// The flat position-ordered tree (D-187) — every row
    /// LIB-001/008 names under the asked scope.
    pub(crate) tree: Vec<LibraryTreeItem>,
    /// The number of hidden instructions (`state = 'hidden'`; the
    /// D-14 "Show hidden" toggle's number). Scope-independent: the
    /// same count under either scope, computed in the same read
    /// transaction — the default answer knows its hidden half without
    /// serving it. Saturated at `u32::MAX` (the `variant_count`
    /// precedent: specta forbids 64-bit integers on the wire).
    pub(crate) hidden_count: u32,
}

/// Serves the instruction library's tree (§7.10 Library module
/// `library_get`, §7.3): every row LIB-001/008 names — instructions
/// (name, number of variants, creation and change dates, state) and
/// headings (level, the LIB-032 skip flag) — in `position` order,
/// with the variants inline (LIB-014's emptiness flag, DB-010's main
/// flag) and the project labels of the assignments (D-187 for the
/// three readings this command fixed).
///
/// A pure read: one `Db::run` transaction gathers the four lists (the
/// items, the active variants, the active projects and their
/// assignments — one snapshot, D-037) plus the hidden-scope count
/// (`hidden_count`, D-200 — the answer's D-14 number), and the
/// assembly is linear
/// Rust over data already in memory (rule 6): the label join walks
/// the projects once, the variant grouping walks the variants once,
/// and the skip walk is `eunomia_core::gaps::heading_level_gaps` over
/// the served items. No SYS-005 gate (SYS-005 names the
/// project-*changing* commands), no journal (D-051: reads journal
/// nothing), no file I/O (nothing is probed — the availability flags
/// are `projects_list`'s own).
///
/// `include_hidden` follows the `projects_list` precedent: `null`
/// (omitted) and `false` serve the visible library, `true` widens to
/// the whole table (D-14's "Show hidden"). Headings are always active
/// (§6.2), so the flag only ever adds instruction rows.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_get<R: Runtime>(
    app: AppHandle<R>,
    include_hidden: Option<bool>,
) -> Result<LibraryGetResult, AppError> {
    library_get_impl(app, include_hidden)
        .await
        .inspect_err(logging::library_get_failed)
}

/// The body of [`library_get`], split the `projects_list` way so the
/// one logging edge covers every failure path: the missing database
/// and the ordered read.
async fn library_get_impl<R: Runtime>(
    app: AppHandle<R>,
    include_hidden: Option<bool>,
) -> Result<LibraryGetResult, AppError> {
    let database = managed_db(&app)?;
    let scope = if include_hidden == Some(true) {
        eunomia_store::library::ItemScope::All
    } else {
        eunomia_store::library::ItemScope::Active
    };
    let read = database
        .run(move |transaction| {
            let items = eunomia_store::library::list_items(transaction, scope)
                .map_err(library_store_to_app_error)?;
            let variants = eunomia_store::library::list_active_variants(transaction)
                .map_err(library_store_to_app_error)?;
            let projects = eunomia_store::projects::list(
                transaction,
                eunomia_store::projects::ListScope::Active,
            )
            .map_err(projects_store_to_app_error)?;
            let assignments =
                eunomia_store::library::list_assignments_for_active_projects(transaction)
                    .map_err(library_store_to_app_error)?;
            // D-200: the D-14 number rides the SAME read transaction —
            // the default answer knows its hidden half without serving
            // it.
            let hidden_count = eunomia_store::library::count_items(
                transaction,
                eunomia_store::library::ItemScope::Hidden,
            )
            .map_err(library_store_to_app_error)?;
            Ok((
                LibraryRead {
                    items,
                    variants,
                    projects,
                    assignments,
                },
                hidden_count,
            ))
        })
        .await?;
    let (read, hidden_count) = read;
    Ok(LibraryGetResult {
        tree: assemble_library_tree(read),
        // The wire bound's saturation (the `variant_count`
        // precedent): the COUNT is i64, the wire is u32.
        hidden_count: u32::try_from(hidden_count).unwrap_or(u32::MAX),
    })
}

/// Builds the wire tree from the one-transaction read — pure, linear
/// in the rows read (rule 6): one pass groups the variants per
/// instruction, one pass indexes the assignments per instruction (the
/// projects are already in `sort_order`, so the labels come out in
/// column order), and the skip flags come from one
/// [`eunomia_core::gaps::heading_level_gaps`] walk over the served
/// items (instructions are transparent to the pair walk, so the flag
/// set is identical under either scope; the indices address the
/// served collection — the list the UI renders).
///
/// D-200: the answer also carries `hidden_count` — the read's fifth
/// fact, the same transaction's hidden-scope count.
fn assemble_library_tree(read: LibraryRead) -> Vec<LibraryTreeItem> {
    // The active variants grouped per instruction — the read's own
    // order (`instruction_id, created_at, id`) already groups them,
    // and within one instruction it is the creation order.
    let mut variants_by_instruction: std::collections::HashMap<
        &str,
        Vec<&eunomia_store::library::Variant>,
    > = std::collections::HashMap::with_capacity(read.variants.len());
    for variant in &read.variants {
        variants_by_instruction
            .entry(variant.instruction_id.as_str())
            .or_default()
            .push(variant);
    }
    // The label carrier: the active projects by id, then the
    // assignments walked in the store's own order
    // (`p.sort_order, i.position`) — walking them in that order
    // leaves every instruction's labels in the projects' column
    // order.
    let projects_by_id: std::collections::HashMap<&str, &eunomia_store::projects::Project> = read
        .projects
        .iter()
        .map(|project| (project.id.as_str(), project))
        .collect();
    let mut labels_by_instruction: std::collections::HashMap<&str, Vec<ProjectLabelEntry>> =
        std::collections::HashMap::with_capacity(read.assignments.len());
    for assignment in &read.assignments {
        // Unreachable by construction: both reads — the projects list
        // and the assignments query — apply the same
        // `p.state = 'active'` predicate inside the one `Db::run`
        // snapshot, so every assignment's project is in the map (the
        // round-1 review verified this reachability before accepting
        // the panic; a reasoned panic, never a silent skip, if the
        // invariant ever breaks).
        let project = projects_by_id
            .get(assignment.project_id.as_str())
            .unwrap_or_else(|| {
                panic!("an assignment's project is missing from the same snapshot's active list")
            });
        labels_by_instruction
            .entry(assignment.instruction_id.as_str())
            .or_default()
            .push(ProjectLabelEntry {
                project_id: project.id.clone(),
                name: project.name.clone(),
                label: project.label.clone(),
                color: project.color.clone(),
                variant_id: match &assignment.target {
                    eunomia_store::library::AssignmentTarget::Variant(variant_id) => {
                        Some(variant_id.clone())
                    }
                    eunomia_store::library::AssignmentTarget::Custom(_) => None,
                },
            });
    }
    // The LIB-032 skip walk over the SERVED items: the store rows
    // become the core domain values the gaps module reads
    // (instructions with an empty variant vector — the walk reads
    // headings only), and the reports' `to_index` addresses this very
    // collection. Levels arrive decoded (`2..=6`, the §6.2 CHECK
    // matrix mirrored on the read), so the try_from cannot refuse a
    // real row — a reasoned panic instead of a lie (the house
    // precedent for broken invariants; round-1 P3-3's deeper
    // suggestion — returning `Result` from this pure assembly — is
    // declined: the branch is unreachable by construction, and an
    // untestable error arm is exactly the dead defense the project
    // does not add. The one decode this pass performs is reused by
    // the tree pass below through the zip).
    let core_items = read
        .items
        .iter()
        .map(|item| match item.kind {
            eunomia_store::library::ItemKind::Instruction => {
                CoreLibraryItem::Instruction(CoreInstruction {
                    id: ItemId::new(item.id.clone()),
                    title: item.title.clone(),
                    state: match item.state {
                        eunomia_store::library::ItemState::Active => InstructionState::Active,
                        eunomia_store::library::ItemState::Hidden => InstructionState::Hidden,
                    },
                    variants: Vec::new(),
                })
            }
            eunomia_store::library::ItemKind::Heading => CoreLibraryItem::Heading(CoreHeading {
                id: ItemId::new(item.id.clone()),
                level: HeadingLevel::try_from(item.heading_level.unwrap_or_else(|| {
                    panic!("a heading row carries its level (the §6.2 CHECK matrix, decoded on the read)")
                }))
                .unwrap_or_else(|error| {
                    panic!("a decoded heading level must be a core level: {error}")
                }),
                title: item.title.clone(),
            }),
        })
        .collect::<Vec<_>>();
    // One gaps pass, one fact container (D-199): the map is keyed by
    // the gap's `to_index`, so its key set IS the skip set —
    // `level_skipped` and `from_level` cannot disagree, because they
    // are two reads of the same map.
    let gaps = heading_level_gaps(&core_items);
    let from_levels: std::collections::HashMap<usize, u8> = gaps
        .iter()
        .map(|gap| (gap.to_index, u8::from(gap.from_level)))
        .collect();

    let mut tree = Vec::with_capacity(read.items.len());
    // The tree pass walks the store rows ZIPPED with the core vector:
    // the core half already carries the validated heading levels (the
    // single decode lives in the pass above), so the wire entry reads
    // its level from there instead of re-unwrapping the store cell —
    // one decode per row, not two (round-1 P3-3).
    for ((index, item), core) in read.items.iter().enumerate().zip(&core_items) {
        match core {
            CoreLibraryItem::Heading(heading) => {
                tree.push(LibraryTreeItem::Heading(LibraryHeadingEntry {
                    id: item.id.clone(),
                    title: item.title.clone(),
                    level: u8::from(heading.level),
                    level_skipped: from_levels.contains_key(&index),
                    from_level: from_levels.get(&index).copied(),
                }));
            }
            CoreLibraryItem::Instruction(_) => {
                let variants = variants_by_instruction
                    .remove(item.id.as_str())
                    .unwrap_or_default();
                let variant_count = variants.len();
                tree.push(LibraryTreeItem::Instruction(LibraryInstructionEntry {
                    id: item.id.clone(),
                    title: item.title.clone(),
                    state: item.state.as_str().to_owned(),
                    variant_count: u32::try_from(variant_count).unwrap_or(u32::MAX),
                    created_at: item.created_at.clone(),
                    updated_at: item.updated_at.clone(),
                    variants: variants
                        .into_iter()
                        .map(|variant| LibraryVariantEntry {
                            id: variant.id.clone(),
                            name: variant.name.clone(),
                            is_main: variant.is_main,
                            // D-13's empty variant: "Outputs no text".
                            // The build's own oracle decides it —
                            // `normalize::variant_text` (ALG-002) is
                            // what a project file receives, so a
                            // whitespace-only stored body is empty
                            // here exactly as it is empty in the
                            // build (round-1 P2-2: the raw
                            // `content.is_empty()` was narrower than
                            // the canon). Linear, one pass per
                            // variant.
                            is_empty: eunomia_core::normalize::variant_text(&variant.content)
                                .is_empty(),
                        })
                        .collect(),
                    project_labels: labels_by_instruction
                        .remove(item.id.as_str())
                        .unwrap_or_default(),
                }));
            }
        }
    }
    tree
}

/// Normalizes a [`eunomia_store::library::LibraryError`] into the
/// §7.10 envelope — the D-093 boundary's planned seam (the repository
/// returns its typed error directly until a task allowed to edit
/// `src-tauri` normalizes the shapes): `Sql` goes through the shared
/// SQLite base classifier, a corrupt vocabulary cell through the
/// D-037 fallback with a class-only diagnostic naming the column —
/// never the stored value (rule 8; a tampered cell is arbitrary
/// database text, and `params`/`os_message` are exactly the surfaces
/// the log line and the localized message read). The NOCASE-twin
/// title rejection is the typed `E_VALIDATION` refusal of the `name`
/// field WITHOUT a value — the context-free form (the title the
/// store names can be the row's own STORED title on the restore
/// path, and stored text never travels, SEC-008; the
/// create/update paths add the caller's own echo through
/// [`library_write_to_app_error`], T-328). The write-path variants
/// (`NotFound`, `AlreadyAssigned`, …) are unreachable from the reads
/// this mapper serves today and map to the family fallback — the
/// same belt every store mapper here wears.
pub(crate) fn library_store_to_app_error(error: eunomia_store::library::LibraryError) -> AppError {
    match error {
        eunomia_store::library::LibraryError::Sql(source) => db::sqlite_error_to_app_error(source),
        eunomia_store::library::LibraryError::Member { column, .. } => library_fallback(
            "LibraryCorrupt",
            &format!("a stored {column} cell is outside its vocabulary"),
        ),
        // The context-free duplicate-title arm (T-328): `field` +
        // `reason` only — the stored title never travels (SEC-008);
        // the create/update commands enrich it with the caller's own
        // echo through [`library_write_to_app_error`].
        eunomia_store::library::LibraryError::DuplicateInstructionTitle { .. } => {
            let mut app_error = AppError::new(ErrorCode::Validation);
            app_error
                .params
                .insert("field".to_owned(), "name".to_owned());
            app_error.params.insert(
                "reason".to_owned(),
                "a visible instruction already carries this name, case-insensitive (6.9)"
                    .to_owned(),
            );
            app_error
        }
        write_outcome => library_fallback(
            "LibraryError",
            match &write_outcome {
                eunomia_store::library::LibraryError::NotFound { .. } => {
                    "a by-id write outcome reached a command that does not map it"
                }
                eunomia_store::library::LibraryError::NotAnInstruction { .. } => {
                    "a heading-only outcome reached a command that does not map it"
                }
                eunomia_store::library::LibraryError::NotAHeading { .. } => {
                    "an instruction-only outcome reached a command that does not map it"
                }
                eunomia_store::library::LibraryError::DuplicateInstructionTitle { .. }
                | eunomia_store::library::LibraryError::DuplicateVariantName { .. } => {
                    "a name-conflict outcome reached a command that does not map it"
                }
                eunomia_store::library::LibraryError::MainVariantExists { .. }
                | eunomia_store::library::LibraryError::MainRequired { .. }
                | eunomia_store::library::LibraryError::NoActiveMain { .. } => {
                    "a main-variant outcome reached a command that does not map it"
                }
                eunomia_store::library::LibraryError::LastVariant { .. }
                | eunomia_store::library::LibraryError::ReplacementRequired { .. }
                | eunomia_store::library::LibraryError::ReplacementInvalid { .. } => {
                    "a delete-replacement outcome reached a command that does not map it"
                }
                eunomia_store::library::LibraryError::PositionOutOfRange { .. }
                | eunomia_store::library::LibraryError::InvalidItemOrder { .. }
                | eunomia_store::library::LibraryError::DuplicateOrderId { .. }
                | eunomia_store::library::LibraryError::UnknownOrderId { .. } => {
                    "an order/position outcome reached a command that does not map it"
                }
                eunomia_store::library::LibraryError::AlreadyAssigned { .. }
                | eunomia_store::library::LibraryError::VariantNotAssignable { .. }
                | eunomia_store::library::LibraryError::VariantNotActive { .. } => {
                    "an assignment outcome reached a command that does not map it"
                }
                eunomia_store::library::LibraryError::Member { .. }
                | eunomia_store::library::LibraryError::Sql(_) => {
                    "an unexpected library store outcome"
                }
            },
        ),
    }
}

/// Builds the D-037 fallback with a family label and a fixed,
/// class-only diagnostic — the shared shape of the two
/// [`library_store_to_app_error`] fallback arms (the
/// `projects_fallback` builder).
fn library_fallback(kind: &str, message: &str) -> AppError {
    let mut app_error = AppError::new(ErrorCode::DbLocked);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some(kind.to_owned()),
        os_message: Some(message.to_owned()),
    });
    app_error
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
/// in its canonical form (DB-030 — the form
/// `project_add_manual`/`scan_start` register and probe), or
/// `None` when the user cancelled, in which case nothing is
/// remembered.
///
/// SEC-001: the dialog plugin is invoked from this Rust command only
/// — the webview holds no `dialog:` permission
/// (`capabilities/default.json` grants only the event-subscription
/// pair `core:event:allow-listen`/`-unlisten`), so this command
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
/// [`picked_folder_vanished`]'s typed answer, and a pick whose
/// canonical form is not valid UTF-8 is the D-086-addendum encoding
/// rejection ([`rejected_non_utf8_path`]) — the dialog boundary the
/// owner's ruling places the refusal at, so no unrepresentable folder
/// ever reaches the allowlist or the wire answer. Nothing is
/// remembered for either refusal.
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
                .map_err(|source| {
                    // The pick bookkeeping's two refusals (the D-086
                    // addendum keeps them apart): a folder that cannot be
                    // RESOLVED is the vanish race's E_PROJECT_UNAVAILABLE;
                    // a folder whose canonical form is not valid UTF-8 is
                    // the encoding rejection — the dialog boundary the
                    // owner's ruling places it at. Neither is remembered.
                    match &source {
                        eunomia_fs::paths::CanonicalizeError::NotUtf8 { .. } => {
                            rejected_non_utf8_path(&raw.to_string_lossy())
                        }
                        eunomia_fs::paths::CanonicalizeError::Io { source, .. } => {
                            picked_folder_vanished(&raw, source)
                        }
                    }
                }),
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

/// The §10 mapping for a pick whose folder cannot be RESOLVED by the
/// OS: the dialog only returns existing folders, so this is the race
/// where the folder moved, was deleted or lost read access between
/// the pick and the bookkeeping — §10's `E_PROJECT_UNAVAILABLE` row
/// ("The project folder moved, was deleted, or cannot be read") is
/// exactly the fact. The picked path travels in `params` (SEC-008:
/// paths may) and the OS refusal in `details`
/// ([`AppError::with_os_error`], §10's first-class log fact). Nothing
/// is remembered for a vanished pick. Takes the io failure directly
/// (the caller has already destructured the `Io` variant apart from
/// the OTHER refusal the bookkeeping can meet — a canonical form that
/// is not valid UTF-8, which is [`rejected_non_utf8_path`]'s encoding
/// envelope instead, the D-086 addendum boundary).
fn picked_folder_vanished(picked: &Path, source: &std::io::Error) -> AppError {
    let mut app_error = AppError::new(ErrorCode::ProjectUnavailable);
    app_error
        .params
        .insert("path".to_owned(), picked.display().to_string());
    app_error.with_os_error(source)
}

// ---------------------------------------------------------------------------
// `instruction_create` / `instruction_update` / `instruction_hide` /
// `instruction_restore` (T-328, §7.10 Library module, §7.3 LIB-002,
// LIB-005…007/012, MAIN-012…014, §6.4, §6.9, DB-005/DB-010/DB-013/
// DB-016, §8 S3, §9.8 ALG-070…072, spec decision D-11; the five
// readings and the composition rules are recorded in DECISIONS.md
// D-188)
// ---------------------------------------------------------------------------

/// The `instruction_create` answer: the identities the created pair
/// carries plus the placement the row took — the ids are what every
/// later library command (`instruction_update`, `instruction_hide`,
/// the variant commands, `assignment_set`) addresses, and the
/// position pins where in the §6.4 sequence the new row landed. The
/// `library_get` read refreshes the tree; this DTO stays the minimal
/// create receipt (the `ScanCommitEntry` minimal-payload rule).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct InstructionCreated {
    /// The created instruction's row id (UUIDv7 TEXT, DB-002).
    pub(crate) instruction_id: String,
    /// The created main variant's row id — LIB-002's pair: the dialog
    /// always creates the instruction WITH its `Main` variant.
    pub(crate) variant_id: String,
    /// The stored `position` of the new instruction (DB-011), the
    /// placement the D-188 section rule chose — `i32` on the wire
    /// (specta forbids 64-bit integers; the table's row count always
    /// fits, the `AddedProject.sort_order` decision).
    pub(crate) position: i32,
}

/// The `instruction_create` command (§7.10 Library module, LIB-002,
/// §8 S3): the create dialog's save — one instruction, its main
/// variant named `Main`, placed in the picked section — validated
/// through the §6.9 field validators (DB-016's marker line included),
/// written with its two `create` versions and its journal entry in
/// ONE transaction.
#[tauri::command]
#[specta::specta]
pub(crate) async fn instruction_create<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    text: String,
    section: Option<String>,
) -> Result<InstructionCreated, AppError> {
    instruction_create_impl(app, name, text, section)
        .await
        .inspect_err(logging::instruction_create_failed)
}

/// The body of [`instruction_create`], split so the one logging edge
/// covers every failure path (the `activity_list` shape).
async fn instruction_create_impl<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    text: String,
    section: Option<String>,
) -> Result<InstructionCreated, AppError> {
    // §6.9 first, before any database is resolved (D-076's split: the
    // field rules are core's, the wire envelope is the command's).
    // The name validator returns the TRIMMED form — the stored form
    // (the D-075 contract). The text is validated unchanged: the store
    // owns the save-time CRLF fold, so the validator measures the
    // caller's own spelling. DB-016's marker line is its own §10 row
    // here (E_CONTENT_RESERVED), everything else the E_VALIDATION
    // field envelope.
    let sent_name = name;
    let name = validate_instruction_title(&sent_name)
        .map_err(|error| rejected_title_field(&sent_name, &error))?;
    validate_variant_text(&text).map_err(rejected_text_field)?;
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            // The D-188 section rule, computed over the ordered list
            // the store's own read serves — BEFORE the row exists: the
            // target is a position in `1..=M+1` (M = the current row
            // count).
            let target = section_target(transaction, section.as_deref())?;
            // The create pair (LIB-002): the instruction and its main
            // variant in this one transaction — DB-010's constructive
            // half is the composition itself (the store module docs).
            let item = eunomia_store::library::create_item(
                transaction,
                &NewLibraryItem::Instruction { title: &name },
            )
            .map_err(|error| library_write_to_app_error(error, &name))?;
            let variant = eunomia_store::library::create_variant(
                transaction,
                &NewVariant {
                    instruction_id: &item.id,
                    name: "Main",
                    content: &text,
                    is_main: true,
                },
            )
            .map_err(library_store_to_app_error)?;
            // The placement: `create_item` appended at the table end
            // (`M + 1`); a target short of that moves the row through
            // the store's own renumbering (DB-011 stays dense).
            let placed = if item.position != target {
                eunomia_store::library::move_item(transaction, &item.id, target)
                    .map_err(library_store_to_app_error)?
            } else {
                item
            };
            // The versions (ALG-070/071): one `create` row per entity —
            // the T-084 DoD's pair — with the §6.5 limit read in this
            // same transaction (ALG-072's caller contract, D-188).
            let max_revisions = history_max_revisions(transaction)?;
            record_instruction_version(transaction, &placed, InstructionOp::Create, max_revisions)?;
            record_variant_version(transaction, &variant, VariantOp::Create, max_revisions)?;
            // LOG-001…004 (D-051): the pair's birth is ONE user action
            // — one entry naming the instruction.
            journal_instruction(transaction, mirror, Action::InstructionCreated, &placed.id)?;
            Ok(InstructionCreated {
                instruction_id: placed.id,
                variant_id: variant.id,
                position: wire_position(placed.position),
            })
        })
        .await
}

/// The `instruction_update` command (§7.10 Library module, §7.3,
/// ALG-070/071): the rename — validated, written with its `update`
/// version and journal entry in one transaction, and the verified
/// no-op when the name did not change (D-188).
#[tauri::command]
#[specta::specta]
pub(crate) async fn instruction_update<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    name: String,
) -> Result<(), AppError> {
    instruction_update_impl(app, id, name)
        .await
        .inspect_err(logging::instruction_update_failed)
}

/// The body of [`instruction_update`], split so the one logging edge
/// covers every failure path (the `activity_list` shape).
async fn instruction_update_impl<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    name: String,
) -> Result<(), AppError> {
    // §6.9 first (D-076), the same envelope shape as the create's.
    let sent_name = name;
    let name = validate_instruction_title(&sent_name)
        .map_err(|error| rejected_title_field(&sent_name, &error))?;
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            // The rename's own by-id read: MAIN-015's identity split
            // (a heading's rename is `heading_update`, §7.10) and the
            // verified no-op's comparison input.
            let item = instruction_of(transaction, &id)?;
            // D-188's verified no-op: the validated name equals the
            // stored title — nothing changed, so nothing is written,
            // recorded or journaled (the D-115 precedent; the store
            // would restamp `updated_at` for a present field, and the
            // ALG-070 hash gate would only suppress the version row).
            if item.title == name {
                return Ok(());
            }
            let updated = eunomia_store::library::update_item(
                transaction,
                &id,
                &ItemUpdate {
                    title: Some(name.as_str()),
                    heading_level: None,
                },
            )
            .map_err(|error| library_write_to_app_error(error, &name))?;
            // The `update` version (ALG-070/071): the pre-check above
            // skipped the byte-equal title, and a different title is a
            // different snapshot hash — a real rename always records.
            let max_revisions = history_max_revisions(transaction)?;
            record_instruction_version(
                transaction,
                &updated,
                InstructionOp::Update,
                max_revisions,
            )?;
            journal_instruction(transaction, mirror, Action::InstructionChanged, &updated.id)?;
            Ok(())
        })
        .await
}

/// The `instruction_hide` command (§7.10 Library module, LIB-005,
/// §6.4, D-11/DB-013): the hide — variants and assignments stay, the
/// instruction leaves the build, the `hide` version and journal entry
/// land in one transaction, and a repeated hide is the verified
/// no-op (D-188).
#[tauri::command]
#[specta::specta]
pub(crate) async fn instruction_hide<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<(), AppError> {
    instruction_hide_impl(app, id)
        .await
        .inspect_err(logging::instruction_hide_failed)
}

/// The body of [`instruction_hide`], split so the one logging edge
/// covers every failure path (the `activity_list` shape).
async fn instruction_hide_impl<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), AppError> {
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            let item = instruction_of(transaction, &id)?;
            // D-188's verified no-op: an already-hidden instruction
            // hides nothing. The store's own idempotence would write
            // nothing anyway; the version and the journal skip the
            // re-run of a transition that did not happen (a
            // double-clicked eye button converges to one entry).
            if item.state == ItemState::Hidden {
                return Ok(());
            }
            let hidden = eunomia_store::library::hide_item(transaction, &id)
                .map_err(library_store_to_app_error)?;
            let max_revisions = history_max_revisions(transaction)?;
            record_instruction_version(transaction, &hidden, InstructionOp::Hide, max_revisions)?;
            journal_instruction(transaction, mirror, Action::InstructionHidden, &hidden.id)?;
            Ok(())
        })
        .await
}

/// The `instruction_restore` command (§7.10 Library module,
/// LIB-005…007/012): the restore — the instruction returns WITH its
/// assignments and its old position, the `restore` version and
/// journal entry land in one transaction, and a repeated restore is
/// the verified no-op (D-188).
#[tauri::command]
#[specta::specta]
pub(crate) async fn instruction_restore<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<(), AppError> {
    instruction_restore_impl(app, id)
        .await
        .inspect_err(logging::instruction_restore_failed)
}

/// The body of [`instruction_restore`], split so the one logging edge
/// covers every failure path (the `activity_list` shape).
async fn instruction_restore_impl<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<(), AppError> {
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            let item = instruction_of(transaction, &id)?;
            // The verified no-op twin: an active instruction restores
            // nothing (a heading is already refused above — it can
            // never be hidden).
            if item.state == ItemState::Active {
                return Ok(());
            }
            // §6.9's clause surfaces here (D-076): a visible NOCASE
            // twin may have taken the name while the row was hidden —
            // the store refuses the restore; the envelope stays
            // class-only (the stored title never travels, SEC-008's
            // rule for arbitrary stored text).
            let restored = eunomia_store::library::restore_item(transaction, &id)
                .map_err(library_store_to_app_error)?;
            let max_revisions = history_max_revisions(transaction)?;
            record_instruction_version(
                transaction,
                &restored,
                InstructionOp::Restore,
                max_revisions,
            )?;
            journal_instruction(
                transaction,
                mirror,
                Action::InstructionRestored,
                &restored.id,
            )?;
            Ok(())
        })
        .await
}

// ---- The shared helpers of the four instruction commands (T-328) ----

/// The D-115 bounded echo: caller input echoed into an error envelope
/// is capped at 64 code points, with the truncation named — a §6.9
/// VIOLATING value is unbounded by definition (a 300 000-character
/// text, a gigabyte "name"), and it must ride neither the wire
/// envelope nor the §10 log line. The mock layer mirrors the same cap.
/// `pub(crate)`: the id-addressed project commands outside this file
/// (`project_archive.rs`, T-108; `project_restore.rs`, T-109) refuse
/// an unknown row id with the same bounded echo — one shape for a
/// caller-supplied id on the rule-8 surfaces (the `exclusion_remove`
/// spelling stays as the older, short-id form).
pub(crate) fn bounded_echo(value: &str) -> String {
    const ECHO_CODE_POINTS: usize = 64;
    let total = value.chars().count();
    if total <= ECHO_CODE_POINTS {
        value.to_owned()
    } else {
        let bounded: String = value.chars().take(ECHO_CODE_POINTS).collect();
        format!("{bounded} (+{} more code points)", total - ECHO_CODE_POINTS)
    }
}

/// Maps the §6.9 title validator's rejection onto the wire (D-076):
/// the `rejected_field` shape — `E_VALIDATION`, the `name` field, the
/// caller's own input (bounded) as the value, and the validator's own
/// class-only reason (lengths and indices, never the content).
fn rejected_title_field(sent: &str, error: &LibraryFieldError) -> AppError {
    rejected_field("name", &bounded_echo(sent), &error.to_string())
}

/// Maps the §6.9 variant-text validator's rejection onto the wire.
/// DB-016's marker line is its own §10 row (`E_CONTENT_RESERVED`,
/// with the D-19 literal and the built form's 1-based line number);
/// every other arm is the `E_VALIDATION` field envelope — with NO
/// value echo: the instruction TEXT is the one thing SEC-008 keeps
/// out of logs and envelopes alike, and the validator's reason
/// already carries the length or the index.
fn rejected_text_field(error: LibraryFieldError) -> AppError {
    match error {
        LibraryFieldError::ReservedContent { line, marker } => {
            let mut app_error = AppError::new(ErrorCode::ContentReserved);
            app_error
                .params
                .insert("field".to_owned(), "text".to_owned());
            app_error.params.insert(
                "marker".to_owned(),
                match marker {
                    MarkerLine::Begin => BEGIN_MARKER,
                    MarkerLine::End => END_MARKER,
                }
                .to_owned(),
            );
            app_error.params.insert("line".to_owned(), line.to_string());
            app_error
        }
        other => {
            let mut app_error = AppError::new(ErrorCode::Validation);
            app_error
                .params
                .insert("field".to_owned(), "text".to_owned());
            app_error
                .params
                .insert("reason".to_owned(), other.to_string());
            app_error
        }
    }
}

/// The D-188 section rule's target position: the end of the picked
/// heading's ALG-060 range — the position of the first heading of
/// level `<=` its own that follows it, or the table end — or, for the
/// root pick (`None`), the end of the prefix before the FIRST heading
/// (ALG-021: the only place an instruction has no heading above it;
/// with no headings the whole table is root). The rejected
/// alternative, a raw `position` integer, is recorded in D-188.
///
/// The lookup rides the store's own ordered list (the `list_items`
/// read, position-ordered — the `exclusion_remove` by-id-through-
/// the-list precedent); the list walk is one linear pass, the same
/// cost class as the `move_item` renumbering the placement itself
/// pays (rule 6).
fn section_target(
    transaction: &rusqlite::Transaction<'_>,
    section: Option<&str>,
) -> Result<i64, AppError> {
    let items = eunomia_store::library::list_items(transaction, ItemScope::All)
        .map_err(library_store_to_app_error)?;
    // The position AFTER the current last row — where `create_item`
    // appends, and the target when no boundary follows.
    let table_end = items.len() as i64 + 1;
    let target = match section {
        Some(id) => {
            let heading = items.iter().find(|item| item.id == id).ok_or_else(|| {
                rejected_field("section", &bounded_echo(id), "no row with this id")
            })?;
            if heading.kind != ItemKind::Heading {
                return Err(rejected_field(
                    "section",
                    &bounded_echo(id),
                    "a section must be a heading row (MAIN-015)",
                ));
            }
            // The store's decoder guarantees a heading's level (the
            // §6.2 kind/level matrix); a `None` here is a decode
            // drift, not a data state.
            let level = heading
                .heading_level
                .expect("a heading row always carries its level");
            items
                .iter()
                .find(|item| {
                    item.kind == ItemKind::Heading
                        && item.position > heading.position
                        && item.heading_level.is_some_and(|next| next <= level)
                })
                .map(|next| next.position)
                .unwrap_or(table_end)
        }
        None => items
            .iter()
            .find(|item| item.kind == ItemKind::Heading)
            .map(|first| first.position)
            .unwrap_or(table_end),
    };
    Ok(target)
}

/// The by-id read the three id-addressed commands share: the row must
/// exist and be an instruction (MAIN-015's identity split — a heading
/// row answers the typed `E_VALIDATION` refusal of the `id` field,
/// never a store error or a silent rename/hide of the wrong kind).
fn instruction_of(
    transaction: &rusqlite::Transaction<'_>,
    id: &str,
) -> Result<LibraryItem, AppError> {
    let item = eunomia_store::library::get_item(transaction, id)
        .map_err(library_store_to_app_error)?
        .ok_or_else(|| rejected_field("id", &bounded_echo(id), "no instruction with this id"))?;
    if item.kind != ItemKind::Instruction {
        return Err(rejected_field(
            "id",
            &bounded_echo(id),
            "the row is a heading; headings have no instruction lifecycle (MAIN-015)",
        ));
    }
    Ok(item)
}

/// `history.max_revisions` read in the operation's own transaction
/// (ALG-072's caller contract, D-188): the §6.5-validated value the
/// prune runs on — the settings store owns the range (10–1000), this
/// seam only hands the number on. A corrupt stored cell surfaces
/// through the settings read mapper (class-only diagnostic, never
/// the stored value).
fn history_max_revisions(transaction: &rusqlite::Transaction<'_>) -> Result<i64, AppError> {
    match eunomia_store::settings::get(transaction, SettingKey::HistoryMaxRevisions)
        .map_err(settings_cell_to_app_error)?
    {
        SettingValue::Int(limit) => Ok(limit),
        // Unreachable: `get` answers the key's default or a §6.5-parsed
        // value, and the key's value kind is Int (the store's own
        // pairing rule) — the recorded fallback, never a guess.
        _ => Err(settings_cell_to_app_error(StorageError::Settings {
            key: SettingKey::HistoryMaxRevisions.as_str().to_owned(),
            source: SettingsError::Type {
                expected: "int",
                found: "the value the key's own reader produced",
            },
        })),
    }
}

/// Records the instruction's version (§9.8 ALG-070/071) for the row's
/// current title and state: the T-055 snapshot pair built over the
/// just-saved row, `op` naming the user action. The hash gate (a
/// byte-equal snapshot records nothing) and the ALG-072 prune are the
/// recording service's own.
fn record_instruction_version(
    transaction: &rusqlite::Transaction<'_>,
    item: &LibraryItem,
    op: InstructionOp,
    max_revisions: i64,
) -> Result<Recorded, AppError> {
    let state = match item.state {
        ItemState::Active => InstructionState::Active,
        ItemState::Hidden => InstructionState::Hidden,
    };
    let snapshot = instruction_snapshot(&item.title, state);
    let hash = instruction_snapshot_hash(&item.title, state);
    record_instruction(transaction, &item.id, op, &snapshot, &hash, max_revisions)
        .map_err(revision_store_to_app_error)
}

/// The variant twin of [`record_instruction_version`]: the snapshot
/// pair over the variant's name and text (the store's own folded
/// stored form — what DB-006 measures is what the snapshot carries).
fn record_variant_version(
    transaction: &rusqlite::Transaction<'_>,
    variant: &Variant,
    op: VariantOp,
    max_revisions: i64,
) -> Result<Recorded, AppError> {
    let snapshot = variant_snapshot(&variant.name, &variant.content);
    let hash = variant_snapshot_hash(&variant.name, &variant.content);
    record_variant(
        transaction,
        &variant.id,
        op,
        &snapshot,
        &hash,
        max_revisions,
    )
    .map_err(revision_store_to_app_error)
}

/// Writes one library lifecycle journal entry (LOG-001…004, D-051)
/// inside the operation's own transaction: the instruction action at
/// info/user/ok, the instruction's id as the entity — the identity
/// the Activity window's Library group filters on — and records it
/// into the transaction's mirror buffer (T-324/D-228: the flush rides
/// the commit). A refusal is the command's `Err`, never a journal
/// entry.
fn journal_instruction(
    transaction: &rusqlite::Transaction<'_>,
    mirror: &mut crate::logging::MirrorBuffer,
    action: Action,
    instruction_id: &str,
) -> Result<(), AppError> {
    let mut entry = LogEntry::new(action, Level::Info, Visibility::User, Outcome::Ok);
    entry.entity = Some(Entity {
        kind: EntityType::Instruction,
        id: instruction_id,
    });
    insert(transaction, &entry).map_err(activity_log_to_app_error)?;
    // The T-324/D-228 mirror contract: record inside the transaction,
    // flush only after the commit — a no-op for this info+user+ok
    // shape (the journal already holds it; `mirror_level` decides at
    // flush), and the single place a future level/visibility change
    // would pick up its mirroring.
    mirror.record(&entry);
    Ok(())
}

/// The `AddedProject.sort_order` decision's twin: the store's i64
/// position on the i32 wire (specta forbids 64-bit integers; the
/// library's row count always fits — set L is 1 000 items).
fn wire_position(position: i64) -> i32 {
    i32::try_from(position).expect("the library's row count fits the wire's i32")
}

/// Maps a library store failure of the four instruction commands'
/// WRITES onto the §7.10 wire, enriching one arm with the caller's
/// own name: the NOCASE-twin title rejection ([`LibraryError::DuplicateInstructionTitle`])
/// carries the SENT name as the envelope's value — the caller's own
/// input, bounded (D-115). Everything else is
/// [`library_store_to_app_error`].
fn library_write_to_app_error(error: LibraryError, sent_name: &str) -> AppError {
    match error {
        LibraryError::DuplicateInstructionTitle { .. } => rejected_field(
            "name",
            &bounded_echo(sent_name),
            "a visible instruction already carries this name, case-insensitive (6.9)",
        ),
        other => library_store_to_app_error(other),
    }
}

/// Maps a version-recording failure onto the wire (the
/// `library_store_to_app_error` shape, the revisions family): SQLite
/// failures through the shared base classifier, a corrupt vocabulary
/// cell through the fallback with a class-only diagnostic, and the
/// by-id outcomes — unreachable in these flows (the row was just
/// written inside the same transaction on the one connection,
/// D-037) — through the same fallback with the family label.
fn revision_store_to_app_error(error: RevisionError) -> AppError {
    match error {
        RevisionError::Sql(source) => db::sqlite_error_to_app_error(source),
        RevisionError::Member { column, .. } => library_fallback(
            "RevisionCorrupt",
            &format!("a stored {column} cell is outside its vocabulary"),
        ),
        other => library_fallback(
            "RevisionError",
            match &other {
                RevisionError::NotFound { .. } => {
                    "a by-id recording outcome reached a command that does not map it"
                }
                RevisionError::NotAnInstruction { .. } => {
                    "a heading row reached a version recording"
                }
                RevisionError::Member { .. } | RevisionError::Sql(_) => {
                    "an unexpected revision outcome"
                }
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    //! The seam tests of the pure mappings — the opener mapping and
    //! the `activity_list`/settings store-error mappings are pure, so
    //! they are tested directly; the opener call itself is the
    //! recorded untestable seam (module docs). The `sync://progress`
    //! reporter's throttle is pure over an injected clock, so its
    //! battery (the literal, the coalescing rule and the rate bound
    //! over simulated windows) lives here too — the scan
    //! `ProgressTracker` battery's shape (D-202).

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
        // does not exist — the only io shape the pick bookkeeping
        // fails with (the dialog returns existing folders; this is
        // the vanish race). The NotUtf8 refusal is deliberately NOT
        // this function's input: the caller routes it to the shared
        // encoding envelope (the D-086 addendum), pinned by its own
        // unit test.
        let vanished = Path::new("/definitely/not/here/eunomia-t073");
        let source = eunomia_fs::paths::canonicalize(vanished)
            .expect_err("a missing path must fail canonicalization");
        let io = source
            .io_error()
            .expect("an OS refusal carries its io error");

        // Act
        let app_error = picked_folder_vanished(vanished, io);

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
    #[allow(clippy::print_stderr)] // the root-skip notice below prints deliberately
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
                // A skip-notice: test-only stderr printing (the crate's
                // `deny(clippy::print_stderr)` covers production code;
                // the allow sits on the test fn below for the same
                // reason).
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
    fn a_non_utf8_root_lookup_failure_is_the_encoding_validation_envelope() {
        // The D-086 addendum inside the root-lookup mapping: a
        // canonicalize refusal that IS the UTF-8 fact is not an
        // availability error — nothing moved and nothing is
        // unreadable, the folder's name is simply unrepresentable as
        // a lossless identity — so it answers the shared encoding
        // envelope (E_VALIDATION naming the path field and the honest
        // reason), the same typed refusal the dialog and session
        // gates give. The lossy spelling rides only in the echoed
        // value, a diagnostic.
        let source = NotARepoError::Canonicalize {
            source: eunomia_fs::paths::CanonicalizeError::NotUtf8 {
                path: "/tmp/repo\u{fffd}".to_owned(),
            },
        };

        // Act
        let app_error = repo_lookup_failed(&source);

        // Assert: the encoding envelope, field/value/reason, no OS
        // details (a text fact, the D-090 split).
        assert_eq!(app_error.code, ErrorCode::Validation);
        assert_eq!(app_error.message_key, "errors.validation");
        assert_eq!(
            app_error.params.get("field").map(String::as_str),
            Some("path")
        );
        assert_eq!(
            app_error.params.get("value").map(String::as_str),
            Some("/tmp/repo\u{fffd}")
        );
        assert!(
            app_error
                .params
                .get("reason")
                .is_some_and(|reason| reason.contains("UTF-8")),
            "the reason names the encoding: {app_error:?}"
        );
        assert!(
            app_error.details.is_none(),
            "a text fact carries no OS details"
        );
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

    // ---- The shared new-project rules (T-077, D-179) ---------------------

    #[test]
    fn project_fields_carry_the_derived_name_label_and_the_root_identity() {
        // S1's "Fields" step in one value: the root's folder name, its
        // DB-042 label (the first six characters, upper case), the
        // empty validator-passed description, and the identity the
        // store keys and stores (DB-030: the key of the canonical text
        // — the platform's own rule, compared through `path_key` itself
        // so the test holds on every OS — and the verbatim path text).
        let root = Path::new("/home/user/eunomia-repo");
        let fields = ProjectFields::derive(root).expect("a plain repository root derives");

        assert_eq!(fields.name, "eunomia-repo");
        assert_eq!(fields.label, "EUNOMI", "DB-042: the first six upper-cased");
        assert_eq!(fields.description, "", "the add flows collect none (§7.1)");
        assert_eq!(
            fields.path_key,
            eunomia_fs::paths::path_key(root).expect("UTF-8 text keys"),
            "the key is DB-030's own key of the canonical text"
        );
        assert_eq!(
            fields.path, "/home/user/eunomia-repo",
            "the stored path is verbatim"
        );
    }

    #[test]
    fn an_unstorable_derived_label_is_rejected_naming_the_label() {
        // D-079's counterexample at the shared seam: a legal repository
        // name with a control-character prefix derives a label §6.9
        // cannot store ("\u{7}REPO") — the honest answer is the
        // E_VALIDATION envelope naming the LABEL field with the derived
        // spelling (the manual add's own edge, now the scan commit's
        // too). Pure over the path text, so no fixture is needed.
        let error = ProjectFields::derive(Path::new("/home/user/\u{7}repo"))
            .expect_err("an unstorable derived label must be rejected");

        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(
            error.params.get("field").map(String::as_str),
            Some("label"),
            "the failed field is the derived label, not the name"
        );
        assert_eq!(
            error.params.get("value").map(String::as_str),
            Some("\u{7}REPO"),
            "the offending value is the derivation itself"
        );
    }

    #[test]
    fn the_color_occupancy_picks_what_the_full_multiset_picks() {
        // The equivalence the reduced-multiset trick rests on (the
        // `ColorOccupancy` docs): D-29 reads only the counts RELATIVE
        // to the least-used member, so the occupancy's pick over its
        // counts must equal core's own `pick_color` over the FULL
        // multiset — the independent oracle, computed without the
        // reduction. Swept over deterministic pseudo-random occupancy
        // vectors (an LCG: no RNG crate, reproducible everywhere) and
        // several tie-break ids, including the all-equal and all-zero
        // shapes where the reduction empties the slice.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |bound: u64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % bound
        };
        let ids: Vec<ProjectId> = (0..7)
            .map(|n| ProjectId::new(format!("0193f1a0-0000-7000-8000-{n:012x}")))
            .collect();

        for round in 0..400u32 {
            let mut counts = [0usize; Color::ALL.len()];
            match round % 4 {
                0 => {}              // all zero: the first-ever project's shape
                1 => counts.fill(3), // all equal: the reduction empties
                _ => {
                    for count in &mut counts {
                        *count = (next(40)) as usize;
                    }
                }
            }
            let full: Vec<Color> = Color::ALL
                .iter()
                .zip(counts)
                .flat_map(|(&color, count)| std::iter::repeat_n(color, count))
                .collect();
            let occupancy = ColorOccupancy::from_colors(full.iter().copied());
            for id in &ids {
                assert_eq!(
                    occupancy.pick(id),
                    eunomia_core::color_pick::pick_color(&full, id),
                    "counts {counts:?}: the reduced pick must equal the full-multiset pick"
                );
            }
        }
    }

    #[test]
    fn a_batch_of_twelve_rows_takes_twelve_distinct_colors() {
        // PRJ-010's criterion at the shared seam, inside ONE batch: the
        // occupancy is updated per created row (`record`), so a
        // still-unused color always wins and twelve rows from an empty
        // library take twelve distinct colors; the thirteenth lands on
        // a used one (the tie-break's legal repeat, DB-041). The
        // rule's first half is what the criterion pins — not WHICH
        // color each row takes (that is D-29's hash, D-077).
        let mut occupancy = ColorOccupancy::from_colors(std::iter::empty());
        let mut taken: Vec<Color> = Vec::new();
        for n in 0..12 {
            let color =
                occupancy.pick(&ProjectId::new(format!("0193f1a0-0000-7000-8000-{n:012x}")));
            assert!(
                !taken.contains(&color),
                "row {n} must take a still-unused color; taken so far: {taken:?}"
            );
            taken.push(color);
            occupancy.record(color);
        }
        assert_eq!(
            taken.len(),
            Color::ALL.len(),
            "twelve rows, the whole palette"
        );
        let thirteenth = occupancy.pick(&ProjectId::new(
            "0193f1a0-6d6d-7cce-a3c0-1a2b3c4d5e6d".to_owned(),
        ));
        assert!(
            taken.contains(&thirteenth),
            "with the palette full the thirteenth row repeats a color (DB-041)"
        );
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

    // ---- The instruction commands' pure mapping seams (T-328) -------

    #[test]
    fn a_marker_line_rejection_is_its_own_section_10_code_with_the_line_facts() {
        // Arrange: the DB-016 arm of `validate_variant_text`, both D-19
        // literals — the only §10 row whose action text says "Remove
        // the reserved marker line".
        let cases = [
            (MarkerLine::Begin, BEGIN_MARKER),
            (MarkerLine::End, END_MARKER),
        ];

        for (marker, literal) in cases {
            // Act
            let app_error =
                rejected_text_field(LibraryFieldError::ReservedContent { line: 3, marker });

            // Assert: `E_CONTENT_RESERVED` with the field, the literal
            // (a fixed app constant, not user content) and the 1-based
            // line — no value echo (SEC-008 keeps the text out).
            assert_eq!(app_error.code, ErrorCode::ContentReserved);
            assert_eq!(app_error.message_key, "errors.content_reserved");
            assert_eq!(
                app_error.params.get("field").map(String::as_str),
                Some("text")
            );
            assert_eq!(
                app_error.params.get("marker").map(String::as_str),
                Some(literal)
            );
            assert_eq!(app_error.params.get("line").map(String::as_str), Some("3"));
            assert!(
                !app_error.params.contains_key("value"),
                "the instruction text never rides the envelope"
            );
        }
    }

    #[test]
    fn a_plain_text_rejection_is_the_validation_envelope_without_a_value() {
        // Arrange & Act: a non-marker §6.9 text arm (the DB-006 length
        // ceiling — the class of a 300 000-character save).
        let app_error =
            rejected_text_field(LibraryFieldError::VariantTextTooLong { length: 300_000 });

        // Assert: the E_VALIDATION field envelope, field and reason
        // only — the length is in the reason, the text itself never
        // travels (SEC-008).
        assert_eq!(app_error.code, ErrorCode::Validation);
        assert_eq!(app_error.message_key, "errors.validation");
        assert_eq!(
            app_error.params.get("field").map(String::as_str),
            Some("text")
        );
        assert!(
            app_error.params.contains_key("reason"),
            "the validator's own class-only reason travels"
        );
        assert!(
            !app_error.params.contains_key("value"),
            "the instruction text never rides the envelope"
        );
    }

    #[test]
    fn a_duplicate_title_from_the_store_maps_context_free_its_caller_enriched() {
        // Arrange: the store's NOCASE-twin rejection, which can carry
        // the row's own STORED title on the restore path — the mapper
        // must stay class-only, and only the caller's own input may be
        // echoed by the enriched write twin.
        let stored = LibraryError::DuplicateInstructionTitle {
            title: "a stored user title".to_owned(),
        };

        // Act & Assert (context-free): field + reason, never the title.
        let bare = library_store_to_app_error(stored);
        assert_eq!(bare.code, ErrorCode::Validation);
        assert_eq!(bare.params.get("field").map(String::as_str), Some("name"));
        assert!(
            !bare.params.contains_key("value"),
            "a stored title never travels (SEC-008)"
        );
        let reason = bare
            .params
            .get("reason")
            .map(|reason| reason.as_str().to_owned())
            .unwrap_or_default();
        assert!(
            !reason.contains("a stored user title"),
            "the reason is class-only, got: {reason}"
        );

        // Act & Assert (enriched): the caller's own sent name is the
        // value — bounded by the D-115 cap.
        let enriched = library_write_to_app_error(
            LibraryError::DuplicateInstructionTitle {
                title: "the sent name".to_owned(),
            },
            &"The Sent Name".repeat(80),
        );
        let value = enriched
            .params
            .get("value")
            .map(String::as_str)
            .unwrap_or_default();
        assert!(
            value.ends_with("more code points)"),
            "the echo is capped: {value}"
        );
        assert!(!value.contains("The Sent Name".repeat(80).as_str()));
    }

    #[test]
    fn corrupt_library_and_revision_cells_map_to_the_fallback_class_only() {
        // Arrange: the D-039 corrupt-cell arms of both store families —
        // a vocabulary cell outside its closed set.
        let library_member = LibraryError::Member {
            column: "state",
            found: "half-hidden".to_owned(),
        };
        let revision_member = RevisionError::Member {
            column: "kind",
            found: "subsection".to_owned(),
        };

        // Act
        let library_error = library_store_to_app_error(library_member);
        let revision_error = revision_store_to_app_error(revision_member);

        // Assert: each family's own fallback label, the column named in
        // the diagnostic, the stored value never present.
        for (app_error, kind, column) in [
            (library_error, "LibraryCorrupt", "state"),
            (revision_error, "RevisionCorrupt", "kind"),
        ] {
            assert_eq!(app_error.code, ErrorCode::DbLocked);
            assert!(app_error.params.is_empty());
            let details = app_error.details.expect("the details carry the cause");
            assert_eq!(details.os_kind.as_deref(), Some(kind));
            let message = details.os_message.clone().unwrap_or_default();
            assert!(
                message.contains(column),
                "the diagnostic names the column ({column}), got: {message}"
            );
            assert!(
                !message.contains("half-hidden") && !message.contains("subsection"),
                "the stored value never travels, got: {message}"
            );
        }
    }

    #[test]
    fn unreachable_library_write_outcomes_map_to_the_family_fallback() {
        // Arrange: the write-path outcomes an instruction command does
        // not itself pre-empt, each carrying a stored identifier.
        let outcomes = [
            LibraryError::NotFound {
                id: "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned(),
            },
            LibraryError::NotAnInstruction {
                id: "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned(),
            },
            LibraryError::AlreadyAssigned {
                project_id: "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned(),
                instruction_id: "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned(),
            },
        ];

        // Act
        for outcome in outcomes {
            let app_error = library_store_to_app_error(outcome);

            // Assert: the family fallback, class-only text — the
            // identifiers never reach `params` or `os_message`.
            assert_eq!(app_error.code, ErrorCode::DbLocked);
            assert!(app_error.params.is_empty());
            let details = app_error.details.expect("the details carry the cause");
            assert_eq!(details.os_kind.as_deref(), Some("LibraryError"));
            let message = details.os_message.unwrap_or_default();
            assert!(
                !message.contains("0193a2bb"),
                "no stored identifier travels in the diagnostic, got: {message}"
            );
        }
    }

    #[test]
    fn the_bounded_echo_caps_at_sixty_four_code_points_and_names_the_rest() {
        // The D-115 cap, directly: a short value rides whole; a long
        // one carries its first 64 code points and the count of the
        // rest — never the whole unbounded input.
        assert_eq!(bounded_echo("short"), "short");
        let long = "x".repeat(300);
        let echoed = bounded_echo(&long);
        assert!(echoed.starts_with(&"x".repeat(64)));
        assert!(echoed.ends_with("(+236 more code points)"), "got: {echoed}");
    }

    // -------------------------------------------------------------------
    // The `sync://progress` reporter (T-119, SYS-003, D-202): the
    // literals, the coalescing rule and the rate bound — over a
    // simulated clock, never wall seconds.
    // -------------------------------------------------------------------

    /// The simulated clock's epoch (the scan suite's `t0` shape:
    /// `Instant::now()` is only the anchor synthetic offsets are
    /// added to; every assertion compares offsets, so the anchor's
    /// real value never matters).
    fn t0() -> std::time::Instant {
        std::time::Instant::now()
    }

    /// A clock that answers the given instants in order, one per
    /// call — the coalescing tests' scripted timeline. The queue
    /// rides a `Mutex` so the closure stays `Fn` (the clock type's
    /// bound).
    fn scripted_clock(instants: Vec<std::time::Instant>) -> ProgressClock {
        let queue = std::sync::Mutex::new(std::collections::VecDeque::from(instants));
        Arc::new(move || {
            queue
                .lock()
                .expect("the scripted clock queue must not be poisoned")
                .pop_front()
                .expect("the scripted clock answers one instant per call")
        })
    }

    /// A clock that answers `start`, `start + 1 ms`,
    /// `start + 2 ms`, … — one simulated millisecond per call, the
    /// rate battery's timeline.
    fn millisecond_clock(start: std::time::Instant) -> ProgressClock {
        let tick = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let clock = Arc::clone(&tick);
        Arc::new(move || {
            let step = clock.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            start + std::time::Duration::from_millis(step)
        })
    }

    #[test]
    fn the_sync_progress_min_interval_is_the_spec_derived_literal() {
        // "No more than 10 times per second" (SYS-003) is one tenth
        // of a second of spacing — pinned against the literal the
        // way `dirty_recalc::DEBOUNCE` is.
        assert_eq!(
            SYNC_PROGRESS_MIN_INTERVAL,
            std::time::Duration::from_millis(100)
        );
    }

    #[test]
    fn the_sync_progress_event_name_is_the_spec_literal() {
        // The wire name is the frontend's whole contract; pinned
        // against the §7.10 literal exactly like `scan://progress`'s.
        assert_eq!(SyncProgressEvent::NAME, "sync://progress");
    }

    #[test]
    fn the_phase_words_serialize_as_the_spec_spellings() {
        // The wire vocabulary (round-1's P3): the generated bindings
        // type the phase as a string-literal union, so the serde
        // renames ARE the frontend's contract — each word pinned
        // against §9.4's own spelling, the payload whole beside it.
        for (phase, word) in [
            (SyncPhaseWire::Preflight, "preflight"),
            (SyncPhaseWire::Backups, "backups"),
            (SyncPhaseWire::Write, "write"),
            (SyncPhaseWire::Finish, "finish"),
        ] {
            let payload = serde_json::to_string(&SyncProgressEvent {
                phase,
                done: 1,
                total: 2,
            })
            .expect("the event payload must serialize");
            assert_eq!(
                payload,
                format!("{{\"phase\":\"{word}\",\"done\":1,\"total\":2}}"),
                "the {word} word rides the wire as §9.4 spells it"
            );
        }
    }

    #[test]
    fn the_first_sync_progress_report_emits_immediately_with_its_own_facts() {
        // MAIN-026's live region opens with the run: the very first
        // report reaches the wire at once, carrying its own phase,
        // k and N (a one-project run must not show a silent window
        // for a tenth of a second).
        let reporter = SyncProgressReporter::new(scripted_clock(vec![t0()]));
        let payload = reporter
            .report(SyncPhaseWire::Preflight, 0, 1)
            .expect("the first report emits");

        assert_eq!(
            (payload.phase, payload.done, payload.total),
            (SyncPhaseWire::Preflight, 0, 1)
        );
    }

    #[test]
    fn reports_inside_the_interval_are_suppressed_and_the_newest_state_lands() {
        // The coalescing rule (SYS-003's bound made of spacing):
        // reports inside the 100 ms quiet window are answered with
        // `None`, and the first report after the window carries ITS
        // OWN newest state — never the stale intermediate that opened
        // the window.
        let start = t0();
        let reporter = SyncProgressReporter::new(scripted_clock(vec![
            start,
            start + std::time::Duration::from_millis(10),
            start + std::time::Duration::from_millis(50),
            start + std::time::Duration::from_millis(99),
            start + SYNC_PROGRESS_MIN_INTERVAL,
        ]));

        reporter
            .report(SyncPhaseWire::Preflight, 0, 4)
            .expect("the first report emits");
        assert!(
            reporter.report(SyncPhaseWire::Preflight, 1, 4).is_none(),
            "10 ms after the last emission is inside the window"
        );
        assert!(
            reporter.report(SyncPhaseWire::Write, 2, 4).is_none(),
            "50 ms after the last emission is inside the window"
        );
        assert!(
            reporter.report(SyncPhaseWire::Write, 3, 4).is_none(),
            "99 ms after the last emission is still inside the window"
        );
        let payload = reporter
            .report(SyncPhaseWire::Write, 4, 4)
            .expect("a full interval after the last emission emits");
        assert_eq!(
            (payload.phase, payload.done, payload.total),
            (SyncPhaseWire::Write, 4, 4),
            "the emission after the window carries the newest state"
        );
    }

    #[test]
    fn a_simulated_run_never_exceeds_ten_sync_progress_emissions_per_second() {
        // The card's own bound, as SYS-003 words it: one report per
        // simulated millisecond over a ten-second run (10_001
        // reports — real runs report far faster, the bound must hold
        // for any rate), and EVERY sliding one-second window of the
        // emission timeline holds at most ten. The total is pinned
        // to the arithmetic the interval dictates (emissions at 0,
        // 100, …, 10_000 ms — 101), so a throttle that emitted one
        // extra time anywhere fails the count, and one that lost the
        // gate fails the window sweep by orders of magnitude.
        let start = t0();
        let reporter = SyncProgressReporter::new(millisecond_clock(start));
        let mut emission_times: Vec<std::time::Instant> = Vec::new();

        for step in 0..=10_000u64 {
            let done = u32::try_from(step).expect("the simulated step fits the wire's u32");
            if let Some(payload) = reporter.report(SyncPhaseWire::Write, done, 10_001) {
                assert_eq!((payload.done, payload.total), (done, 10_001));
                emission_times.push(start + std::time::Duration::from_millis(step));
            }
        }

        assert_eq!(
            emission_times.len(),
            101,
            "a 10-second run at one report per millisecond emits exactly at 0,100,…,10_000 ms"
        );
        for window in emission_times.windows(2) {
            let spacing = window[1].duration_since(window[0]);
            assert!(
                spacing >= SYNC_PROGRESS_MIN_INTERVAL,
                "consecutive emissions are at least {SYNC_PROGRESS_MIN_INTERVAL:?} apart; \
                 saw {spacing:?}"
            );
        }
        for from in &emission_times {
            let in_window = emission_times
                .iter()
                .filter(|time| {
                    let time = **time;
                    time >= *from && time.duration_since(*from) < std::time::Duration::from_secs(1)
                })
                .count();
            assert!(
                in_window <= 10,
                "a sliding one-second window holds at most 10 emissions; one starting at \
                 {:?} holds {in_window}",
                from.duration_since(start)
            );
        }
    }

    // -----------------------------------------------------------------------
    // The write-in-flight gate (T-123, D-219 point 5): the pure RAII and
    // the blocking wait, over the gate's own interface. The ipc-level
    // halves — phase 3's guard placement and the confirmed exit's
    // wait-then-exit ordering — live in `ipc_tests.rs` (the mock-app
    // harness with a real run).
    // -----------------------------------------------------------------------

    #[test]
    fn the_write_gate_counts_one_unit_per_guard() {
        // The RAII counting seam: enter counts, drop uncounts, and the
        // gate is quiet exactly when no guard lives. A panic's unwind
        // releases the same way (the SyncGuard reasoning).
        let gate = SyncWriteGate::new();
        assert!(gate.is_quiet(), "a fresh gate is quiet");

        let first = gate.enter();
        assert!(!gate.is_quiet(), "one unit is in flight");
        let second = gate.enter();
        assert!(
            !gate.is_quiet(),
            "two units are in flight (no interleave assumption)"
        );
        drop(second);
        assert!(!gate.is_quiet(), "the first unit is still in flight");
        drop(first);
        assert!(gate.is_quiet(), "the last guard's drop quieted the gate");
    }

    #[test]
    fn a_panicking_write_unit_releases_the_gate() {
        // The RAII half: the unwind drops the guard, so a panicking
        // write unit cannot wedge the confirmed exit's wait forever.
        let gate = SyncWriteGate::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _in_flight = gate.enter();
            panic!("the write unit died mid-flight");
        }));
        assert!(result.is_err(), "the scenario itself must have panicked");
        assert!(gate.is_quiet(), "the unwinding unit released the gate");
    }

    #[test]
    fn wait_quiet_blocks_until_the_in_flight_unit_completes() {
        // The DoD's mechanism, isolated: `wait_quiet` called on a HELD
        // gate returns only after the unit's guard drops. The oracle
        // is an ORDERED shared log, not a timing sample: "dropped" is
        // logged strictly BEFORE the guard's decrement, and
        // "returned" is logged strictly after `wait_quiet` returns —
        // which can only be after the count reaches zero — so a
        // correct wait can never log "returned" before "dropped",
        // and a wait that returns early (the mutation this test
        // exists for) logs it immediately after "waiting". No sleep,
        // no poll: the causal order is the assertion.
        let gate = SyncWriteGate::new();
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));
        let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let holder = {
            let gate = gate.clone();
            let log = std::sync::Arc::clone(&log);
            std::thread::spawn(move || {
                let in_flight = gate.enter();
                log.lock()
                    .expect("the log must not be poisoned")
                    .push("held");
                held_tx
                    .send(())
                    .expect("the test waits for the guard to be held");
                release_rx
                    .recv()
                    .expect("the test releases the holder before joining");
                // Strictly BEFORE the guard's decrement, so a correct
                // wait can never log "returned" ahead of "dropped".
                log.lock()
                    .expect("the log must not be poisoned")
                    .push("dropped");
                drop(in_flight);
            })
        };
        held_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the holder must signal its held guard");
        assert!(!gate.is_quiet(), "the unit is in flight");

        let (waiting_tx, waiting_rx) = std::sync::mpsc::channel::<()>();
        let waiter = {
            let gate = gate.clone();
            let log = std::sync::Arc::clone(&log);
            let waiting_tx = waiting_tx.clone();
            std::thread::spawn(move || {
                log.lock()
                    .expect("the log must not be poisoned")
                    .push("waiting");
                waiting_tx
                    .send(())
                    .expect("the test waits for the waiter to reach the wait");
                gate.wait_quiet();
                log.lock()
                    .expect("the log must not be poisoned")
                    .push("returned");
            })
        };
        drop(waiting_tx);
        waiting_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the waiter must reach its wait while the unit is held");
        // Only now — with the waiter inside `wait_quiet` over a held
        // gate — does the in-flight unit complete.
        release_tx
            .send(())
            .expect("the holder waits for this release");
        holder.join().expect("the holder must not panic");
        waiter.join().expect("the waiter must not panic");
        assert_eq!(
            *log.lock().expect("the log must not be poisoned"),
            ["held", "waiting", "dropped", "returned"],
            "the wait returned only after the in-flight unit completed"
        );
        assert!(gate.is_quiet(), "the wait observed the unit's completion");
    }
}
