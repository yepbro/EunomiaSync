//! The Backups module (spec §7.10, BKP-004/BKP-012, T-125/T-353):
//! `backups_list` — one project's whole `backup_records` history for
//! the project-settings "Backups" tab — and `backup_save` — the
//! per-row "Save to a chosen location" export of one stored copy.
//!
//! `backups_list` joins the one `Db::run` transaction executor
//! (D-037), writes nothing and journals nothing. The answer is the
//! project's every row — all four §6.2 CHECK statuses, rows never
//! deleted (§6.4 keeps the history as status; D-193's ruling) —
//! newest first (`created_at DESC, id DESC`, the D-122 ordering),
//! scoped by the caller's project id. An unknown id is the typed
//! `E_VALIDATION` refusal (the `revisions_list` precedent: a caller
//! bug, never silently "no backups"); a known project without rows is
//! the honest empty list. The read is state-agnostic — an archived
//! project's history lists too (DB-014).
//!
//! `backup_save` (BKP-012, D-203 (2)) is the export, NOT the restore
//! (BKP-007 stays P1, D-17): it takes the row's `id` (the D-193 list
//! form), opens the OS save dialog from Rust (the SEC-001 dialog
//! pattern — `dialog_pick_folder`'s sibling, the plugin invoked from
//! this command only), and writes the copy's bytes to the chosen path
//! through the §9.5 atomic write, verified against the record's own
//! `size_bytes`/`sha256` columns (BKP-010's stored oracle — no new
//! digest policy). It journals nothing (an export of stored data, not
//! a recorded action) and runs no SYS-005 gate (it writes outside
//! every managed area and changes no sync state). The refusals: the
//! typed `E_VALIDATION` for an unknown id; `E_RESTORE_SOURCE_MISSING`
//! when the copy's file is gone from disk — a `pruned`/`missing`/
//! `discarded` row (BKP-012: naming the record's status), a row whose
//! `ok` file vanished anyway, or one the OS refuses to read (the
//! record keeps its stored status in every case, §10's row); and
//! `E_VERIFY_FAILED`/`E_WRITE_FAILED` (§9.5 semantics, D-203 (2)) for
//! a copy whose bytes do not match the record's columns or a refused
//! write — the write's own `E_DISK_FULL`/`E_FILE_LOCKED` arms keep
//! their codes (the `write_code` table). The user cancelling the
//! dialog answers `Ok(None)` — `dialog_pick_folder`'s "nothing on
//! cancel" shape — and a successful export answers the path actually
//! written (the user may edit the suggested name in the dialog).

use std::path::{Path, PathBuf};

use eunomia_core::backup_name::export_suggested_name;
use eunomia_core::hash::Sha256Hex;
use eunomia_store::backup_records::{BackupRecord, BackupRecordStatus, get, list_for_project};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};

use crate::commands::managed_db;
use crate::db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::logging;

/// One Backups-tab row: BKP-004's "the date, file, size, and sync run
/// number" plus the card's status — every `backup_records` column the
/// tab can reach except `project_id` (the scope the caller named, not
/// per-row data; D-193's ruling).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct BackupCopyRecord {
    /// The record row's UUIDv7 TEXT id.
    pub(crate) id: String,
    /// The id of the run that created the copy — BKP-004's "sync run
    /// number" (the schema's run identifier; D-193's ruling). `None`
    /// when the run was swept by §6.4's run cleanup (`ON DELETE SET
    /// NULL`) or a future non-run producer wrote the row.
    pub(crate) sync_run_id: Option<String>,
    /// The copied file's resolved path text — BKP-004's "file" (the
    /// §9.4 phase-1 resolved spelling, one stable form per file).
    pub(crate) source_path: String,
    /// The copy's §5.5 relative layout path
    /// (`backups/<project_id>__<slug>/<file>.<UTC>.<hash8>.bak`) —
    /// the copy's own name and the future restore flow's pick handle.
    pub(crate) backup_path: String,
    /// The copy's size in bytes — the size the BKP-010 check passed,
    /// saturated at `u32::MAX` (specta forbids 64-bit integers on the
    /// TS wire, D-049's `DbBackupInfo` decision).
    pub(crate) size_bytes: u32,
    /// `ok`, `pruned`, `missing` or `discarded` (the §6.2 CHECK's
    /// stored literal — the row's lifecycle place, §6.4).
    pub(crate) status: String,
    /// The copy's creation time, DB-002 UTC ISO-8601 — BKP-004's
    /// "date" (the run's one backup instant, the same stamp the copy
    /// name carries).
    pub(crate) created_at: String,
}

impl BackupCopyRecord {
    /// Shapes a store row into its wire form: the columns travel
    /// verbatim except the size's saturating narrowing (D-049).
    fn from_store(record: BackupRecord) -> Self {
        Self {
            id: record.id,
            sync_run_id: record.sync_run_id,
            source_path: record.source_path,
            backup_path: record.backup_path,
            size_bytes: u32::try_from(record.size_bytes).unwrap_or(u32::MAX),
            status: record.status.as_str().to_owned(),
            created_at: record.created_at,
        }
    }
}

/// The typed refusal for a project id no `projects` row backs (the
/// `revisions_list` shape: field + reason, never the id itself).
fn unknown_project_refusal() -> AppError {
    let mut refusal = AppError::new(ErrorCode::Validation);
    refusal
        .params
        .insert("field".to_owned(), "project_id".to_owned());
    refusal
        .params
        .insert("reason".to_owned(), "no project has this id".to_owned());
    refusal
}

/// Lists the project's backup copies for the "Backups" tab (BKP-004):
/// date, file, size, sync run and status of every record, newest
/// first. One `Db::run` read transaction; nothing is written or
/// journaled.
#[tauri::command]
#[specta::specta]
pub(crate) async fn backups_list<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
) -> Result<Vec<BackupCopyRecord>, AppError> {
    backups_list_impl(app, project_id)
        .await
        .inspect_err(logging::backups_list_failed)
}

async fn backups_list_impl<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
) -> Result<Vec<BackupCopyRecord>, AppError> {
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            // The scope check first — a projects fact, decided at the
            // command boundary (D-193): the store read stays a pure
            // per-project keying. Either state lists (DB-014).
            eunomia_store::projects::get(transaction, &project_id)
                .map_err(crate::commands::projects_store_to_app_error)?
                .ok_or_else(unknown_project_refusal)?;
            list_for_project(transaction, &project_id)
                .map(|rows| rows.into_iter().map(BackupCopyRecord::from_store).collect())
                .map_err(db::backup_records_to_app_error)
        })
        .await
}

// ---------------------------------------------------------------------------
// `backup_save` (T-353, §7.10 Backups module, BKP-012, §5.6 SEC-001,
// D-203 (2))
// ---------------------------------------------------------------------------

/// Exports one stored copy to a location the OS save dialog chooses
/// (BKP-012): the per-row "Save to a chosen location" action of the
/// Backups tab — a plain verified copy of the stored file, not the P1
/// restore (BKP-007).
///
/// The dialog opens with the suggested name
/// `<stem>.backup-<stamp>.<ext>` (the copy's file stem and creation
/// stamp, derived from the record's §5.5 `backup_path` through the
/// one grammar that module owns); the answer is the path actually
/// written, or `None` when the user cancelled the dialog
/// (`dialog_pick_folder`'s "nothing on cancel" shape — the user may
/// edit the suggested name, so the written path is the honest answer,
/// not an echo of the suggestion). A save path whose text is not
/// valid UTF-8 is refused before anything is written (the D-086
/// dialog-boundary posture, `dialog_pick_folder`'s twin).
///
/// SEC-001: the dialog plugin is invoked from this Rust command only
/// (the webview holds no `dialog:` permission), exactly the
/// `dialog_pick_folder` posture. SEC-003 does NOT apply: the session
/// allowlist gates paths the UI SENDS to commands, and this command
/// takes only `id` — the destination is the OS dialog's own answer,
/// produced inside Rust, and nothing later consumes a UI-named
/// export path, so there is nothing to remember and no gate to feed.
/// The answered path is the dialog's spelling verbatim (a fresh save
/// path does not resolve yet — canonicalizing it would fail), and the
/// write targets exactly what the dialog named.
///
/// No journal entry, no SYS-005 gate, no row change: §10's
/// `E_RESTORE_SOURCE_MISSING` row keeps the record at its stored
/// status even when the copy is gone.
#[tauri::command]
#[specta::specta]
pub(crate) async fn backup_save<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<Option<String>, AppError> {
    backup_save_with(app, id, plugin_save_file::<R>)
        .await
        .inspect_err(logging::backup_save_failed)
}

/// The body of [`backup_save`], split the `dialog_pick_folder_with`
/// way so the one logging edge covers every failure path and so the
/// tests drive the whole flow with their own `pick` closure (the
/// plugin seam, `dialog_pick_folder`'s precedent — the real dialog is
/// the one untestable step; everything around it, including the
/// write, runs for real under the tests' chosen paths).
///
/// The order is the refusal order: the row is read first (unknown id,
/// then the status), and the file work — existence, read, the record
/// check, the dialog, the write — runs on the blocking pool (SYS-001:
/// a run of syscalls and a dialog that parks its thread, never an
/// async worker), one closure so the record the row named and the
/// bytes the disk served stay the same facts through the dialog.
/// The record's columns are checked BEFORE the dialog opens and
/// before any destination byte moves: a copy that is not the record's
/// bytes refuses without ever asking the user where to put it, and a
/// refused export never touches the destination (§9.5's staging keeps
/// the user's pre-existing file, if any, whole).
pub(crate) async fn backup_save_with<R, F>(
    app: AppHandle<R>,
    id: String,
    pick: F,
) -> Result<Option<String>, AppError>
where
    R: Runtime,
    F: FnOnce(&AppHandle<R>, &str) -> Result<Option<PathBuf>, AppError> + Send + 'static,
{
    let database = managed_db(&app)?;
    // One `Db::run` read transaction — the row's every column, the
    // `backups_list` executor shape. Writes nothing, journals nothing.
    let record = database
        .run(move |transaction| get(transaction, &id).map_err(db::backup_records_to_app_error))
        .await?;
    let Some(record) = record else {
        // BKP-012: an unknown id is the typed `E_VALIDATION` — a
        // caller bug, never a silent export of some other row.
        return Err(unknown_backup_refusal());
    };
    if record.status != BackupRecordStatus::Ok {
        // BKP-012: a row whose file is gone (the three terminal
        // statuses of D-193's four) is `E_RESTORE_SOURCE_MISSING`,
        // naming the record's status; the record keeps it.
        return Err(source_missing_refusal(record.status));
    }
    let data_dir = crate::commands::data_root(&app)?;
    let app_handle = app.clone();
    let exported = tauri::async_runtime::spawn_blocking(move || {
        export_one(&app_handle, data_dir, record, pick)
    })
    .await
    .map_err(db::join_failure_to_app_error)??;
    Ok(exported)
}

/// The blocking half of one export: resolve the copy, read it, check
/// it against the record's columns, open the dialog, write. Runs only
/// on the blocking pool (the caller's `spawn_blocking`); takes the
/// row's facts as data so the closure is one `'static` unit.
fn export_one<R, F>(
    app: &AppHandle<R>,
    data_dir: PathBuf,
    record: BackupRecord,
    pick: F,
) -> Result<Option<String>, AppError>
where
    R: Runtime,
    F: FnOnce(&AppHandle<R>, &str) -> Result<Option<PathBuf>, AppError>,
{
    // The suggested name FIRST — its derivation validates the stored
    // `backup_path` against the §5.5 grammar (the core builders' own
    // shape), so the join below only ever sees the validated
    // three-component relative path and a corrupt cell is refused
    // before any file is touched.
    let suggested = export_suggested_name(&record.backup_path).map_err(backup_name_to_app_error)?;

    // The copy's file. A path that cannot be read at all — gone, or
    // refused by the OS — is the source-missing refusal with the OS
    // facts attached (§10's row: the export is refused, the record
    // keeps its stored status; `ok` is the honest cell while §10's
    // future detection owns flipping it).
    let copy_path = data_dir.join(&record.backup_path);
    let bytes = std::fs::read(&copy_path)
        .map_err(|source| source_read_refusal(&copy_path, &source, record.status))?;

    // BKP-010's stored oracle: the bytes this export will write must
    // BE the record's `size_bytes`/`sha256` — measured here, before
    // the dialog and before the destination exists, so a copy that is
    // not what its record claims never reaches a user-chosen path.
    // The measured size is the file's real length (the read bounded
    // the memory already); the expected side takes the stored i64
    // through the same widening, so a corrupt negative cell compares
    // unequal and fails here, not in a panic.
    let measured_size = bytes.len() as u64;
    let mut hasher = Sha256Hex::new();
    hasher.update(&bytes);
    let measured_sha256 = hasher.finalize_hex();
    if measured_size != u64::try_from(record.size_bytes).unwrap_or(u64::MAX)
        || measured_sha256 != record.sha256
    {
        return Err(export_verify_refusal(&record));
    }

    // The dialog. `None` is the user's cancel — nothing was written,
    // and the honest answer is "nothing was saved".
    let Some(destination) = pick(app, &suggested)? else {
        return Ok(None);
    };

    // The D-086 dialog-boundary posture (`dialog_pick_folder`'s twin,
    // the round-1 P2-3 fix): a save path the wire cannot carry — a
    // `FilePath::into_path()` path whose text is not valid UTF-8 — is
    // refused BEFORE anything is written, so no file lands at a
    // location the answer could only misname (`display()` would
    // silently replace the offending bytes).
    let Some(path_text) = destination.to_str() else {
        return Err(crate::commands::rejected_field(
            "path",
            &destination.to_string_lossy(),
            "the chosen save path contains a name that is not valid UTF-8: the \
             export cannot answer it (rename the folder or pick another location)",
        ));
    };

    // The §9.5 write: stage beside the destination, verify the
    // settled bytes, replace in one rename — the same operation the
    // sync pipeline uses (`production_write`), so the export is
    // atomic and a refused write leaves whatever the destination held
    // before whole (§9.5 step 7).
    crate::sync_phase3::production_write(&destination, &bytes)
        .map_err(|error| atomic_write_refusal(&record.id, &destination, &error))?;
    Ok(Some(path_text.to_owned()))
}

/// The production save-dialog seam: the dialog plugin's Rust-side API
/// (SEC-001 — never the webview's) with the suggested file name set,
/// `blocking_save_file`, mapped onto the command's internal
/// `Option<PathBuf>` answer. The plugin requires being registered
/// (`tauri_plugin_dialog::init()` in `lib.rs`, pinned by
/// `config_tests.rs`); `set_file_name` and `blocking_save_file` are
/// not desktop-gated in the installed 2.8.4 (unlike the folder
/// picker's `blocking_pick_folder`, lib.rs:471/783) — no non-desktop
/// twin is needed, and this seam is never driven in tests (the
/// `plugin_pick_folder` posture).
fn plugin_save_file<R: Runtime>(
    app: &AppHandle<R>,
    suggested_name: &str,
) -> Result<Option<PathBuf>, AppError> {
    use tauri_plugin_dialog::DialogExt;
    match app
        .dialog()
        .file()
        .set_file_name(suggested_name)
        .blocking_save_file()
    {
        None => Ok(None),
        Some(picked) => picked
            .into_path()
            .map(Some)
            .map_err(|error| crate::commands::dialog_error_to_app_error(&error.to_string())),
    }
}

/// The typed refusal for a record id no `backup_records` row backs
/// (BKP-012; the `revisions_list` shape: field + reason, never the id
/// itself).
fn unknown_backup_refusal() -> AppError {
    let mut refusal = AppError::new(ErrorCode::Validation);
    refusal.params.insert("field".to_owned(), "id".to_owned());
    refusal.params.insert(
        "reason".to_owned(),
        "no backup record has this id".to_owned(),
    );
    refusal
}

/// The `E_RESTORE_SOURCE_MISSING` refusal (§10's row, BKP-012):
/// the export is refused and the record keeps its stored status —
/// `params.status` names it (BKP-012's "naming the record's status").
fn source_missing_refusal(status: BackupRecordStatus) -> AppError {
    let mut refusal = AppError::new(ErrorCode::RestoreSourceMissing);
    refusal
        .params
        .insert("status".to_owned(), status.as_str().to_owned());
    refusal
}

/// The source-missing refusal for a copy whose file cannot be read at
/// all — gone from disk while the row still says `ok`, or refused by
/// the OS (the vanish race `dialog_pick_folder` models for folders):
/// the record keeps its stored status, the path travels in `params`
/// (SEC-008: paths may), and the OS refusal rides `details` (§10's
/// first-class log fact).
fn source_read_refusal(
    copy_path: &Path,
    source: &std::io::Error,
    status: BackupRecordStatus,
) -> AppError {
    let mut refusal = source_missing_refusal(status);
    refusal
        .params
        .insert("path".to_owned(), copy_path.display().to_string());
    refusal.with_os_error(source)
}

/// The `E_VERIFY_FAILED` refusal (§9.5 semantics, D-203 (2)): the
/// copy's bytes are not the record's stored size and digest — a
/// corrupt or substituted copy. Class-only diagnostics (SEC-008: the
/// record id is an id; no stored text is interpolated); nothing was
/// written.
fn export_verify_refusal(record: &BackupRecord) -> AppError {
    let mut refusal = AppError::new(ErrorCode::VerifyFailed);
    refusal.params.insert("id".to_owned(), record.id.clone());
    refusal.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("BackupExportVerifyFailed".to_owned()),
        os_message: Some(
            "the stored copy's bytes do not match the record's size/sha256 columns".to_owned(),
        ),
    });
    refusal
}

/// The corrupt-cell envelope for a stored `backup_path` that does not
/// spell a §5.5 backup name (the core grammar's typed refusal):
/// the `backup_records` Member posture — the D-037 fallback code with
/// a class-only kind label, never the stored value in the wire
/// envelope (SEC-008; the `backup_records_to_app_error` twin).
fn backup_name_to_app_error(_error: eunomia_core::backup_name::BackupNameError) -> AppError {
    let mut app_error = AppError::new(ErrorCode::DbLocked);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("BackupRecordsCorrupt".to_owned()),
        os_message: Some("a stored backup_path cell does not spell a §5.5 backup name".to_owned()),
    });
    app_error
}

/// Shapes a refused §9.5 export write onto the wire: the code comes
/// from the one `write_code` table the sync pipeline shares
/// (`E_WRITE_FAILED`/`E_DISK_FULL`/`E_FILE_LOCKED`/`E_VERIFY_FAILED`),
/// the record id and the destination path travel in `params` (paths
/// may, SEC-008), and the OS facts ride `details`.
fn atomic_write_refusal(id: &str, destination: &Path, error: &AtomicWriteError) -> AppError {
    let mut refusal = AppError::new(crate::sync_phase3::write_code(error));
    refusal.params.insert("id".to_owned(), id.to_owned());
    refusal
        .params
        .insert("path".to_owned(), destination.display().to_string());
    if let Some(source) = error.io_error() {
        refusal = refusal.with_os_error(source);
    }
    refusal
}

/// The typed §9.5 write error the export maps through the shared
/// `write_code` table — named here so the signature says what
/// [`atomic_write_refusal`] consumes.
type AtomicWriteError = eunomia_fs::atomic::AtomicWriteError;
