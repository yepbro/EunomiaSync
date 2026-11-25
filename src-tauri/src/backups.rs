//! The Backups module's project-copy listing (spec §7.10, BKP-004,
//! T-125): `backups_list` — one project's whole `backup_records`
//! history for the project-settings "Backups" tab.
//!
//! The read joins the one `Db::run` transaction executor (D-037),
//! writes nothing and journals nothing. The answer is the project's
//! every row — all four §6.2 CHECK statuses, rows never deleted
//! (§6.4 keeps the history as status; D-193's ruling) — newest first
//! (`created_at DESC, id DESC`, the D-122 ordering), scoped by the
//! caller's project id. An unknown id is the typed `E_VALIDATION`
//! refusal (the `revisions_list` precedent: a caller bug, never
//! silently "no backups"); a known project without rows is the
//! honest empty list. The read is state-agnostic — an archived
//! project's history lists too (DB-014).

use eunomia_store::backup_records::{BackupRecord, list_for_project};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};

use crate::commands::managed_db;
use crate::db;
use crate::errors::{AppError, ErrorCode};
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
