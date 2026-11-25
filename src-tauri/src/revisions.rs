//! The History module's two read commands (spec §7.10, T-331):
//! `revisions_list` and `revision_get` (LIB-026/027).
//!
//! Both reads join the one `Db::run` transaction executor (D-037), write
//! nothing and journal nothing. The list is scoped to one instruction plus
//! all of its variants, including soft-deleted ones, and is newest-first;
//! it carries summaries only, while the single read loads one full revision
//! (including its snapshot) by the revision row's own id.

use eunomia_store::library_versions::{
    Revision, RevisionError, RevisionSummary, get_revision, list_revision_summaries,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};

use crate::commands::managed_db;
use crate::db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::logging;

/// One History-list row: time, operation and entity are explicit, while the
/// potentially large snapshot is fetched only by `revision_get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct RevisionSummaryRecord {
    /// The revision row's UUIDv7 TEXT id.
    pub(crate) id: String,
    /// `instruction` or `variant` (§6.2's closed vocabulary).
    pub(crate) entity_type: String,
    /// The versioned instruction/variant id.
    pub(crate) entity_id: String,
    /// The per-entity revision number as a lossless decimal string.
    pub(crate) rev_no: String,
    /// `create`, `update`, `hide`, `restore`, `delete` or `revert`.
    pub(crate) op: String,
    /// The snapshot's SHA-256, stored verbatim.
    pub(crate) snapshot_hash: String,
    /// The source revision of a P1 revert, when present.
    pub(crate) reverted_from: Option<String>,
    /// Creation time, DB-002 UTC ISO-8601.
    pub(crate) created_at: String,
}

impl RevisionSummaryRecord {
    fn from_store(summary: RevisionSummary) -> Self {
        Self {
            id: summary.id,
            entity_type: summary.entity_type.as_str().to_owned(),
            entity_id: summary.entity_id,
            rev_no: summary.rev_no.to_string(),
            op: summary.op.as_str().to_owned(),
            snapshot_hash: summary.snapshot_hash,
            reverted_from: summary.reverted_from.map(|value| value.to_string()),
            created_at: summary.created_at,
        }
    }
}

/// One full revision row on the History wire: the canonical snapshot is
/// available to the version view and P1 diff/restore work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct RevisionRecord {
    /// The revision row's UUIDv7 TEXT id.
    pub(crate) id: String,
    /// `instruction` or `variant` (§6.2's closed vocabulary).
    pub(crate) entity_type: String,
    /// The versioned instruction/variant id.
    pub(crate) entity_id: String,
    /// The per-entity monotonically increasing number (ALG-071), as a
    /// decimal string so tauri-specta never narrows SQLite's i64 to an
    /// imprecise JavaScript number.
    pub(crate) rev_no: String,
    /// `create`, `update`, `hide`, `restore`, `delete` or `revert`.
    pub(crate) op: String,
    /// Canonical snapshot JSON, stored verbatim.
    pub(crate) snapshot: String,
    /// The snapshot's SHA-256, stored verbatim.
    pub(crate) snapshot_hash: String,
    /// The source revision of a P1 revert, when present.
    pub(crate) reverted_from: Option<String>,
    /// Creation time, DB-002 UTC ISO-8601.
    pub(crate) created_at: String,
}

impl RevisionRecord {
    fn from_store(revision: Revision) -> Self {
        Self {
            id: revision.id,
            entity_type: revision.entity_type.as_str().to_owned(),
            entity_id: revision.entity_id,
            rev_no: revision.rev_no.to_string(),
            op: revision.op.as_str().to_owned(),
            snapshot: revision.snapshot,
            snapshot_hash: revision.snapshot_hash,
            reverted_from: revision.reverted_from.map(|value| value.to_string()),
            created_at: revision.created_at,
        }
    }
}

fn revision_error(error: RevisionError, field: &str) -> AppError {
    match error {
        RevisionError::Sql(source) => db::sqlite_error_to_app_error(source),
        RevisionError::NotFound { .. } => {
            let mut refusal = AppError::new(ErrorCode::Validation);
            refusal.params.insert("field".to_owned(), field.to_owned());
            refusal.params.insert(
                "reason".to_owned(),
                if field == "instruction_id" {
                    "no instruction has this id"
                } else {
                    "no revision has this id"
                }
                .to_owned(),
            );
            refusal
        }
        RevisionError::NotAnInstruction { .. } => {
            let mut refusal = AppError::new(ErrorCode::Validation);
            refusal
                .params
                .insert("field".to_owned(), "instruction_id".to_owned());
            refusal.params.insert(
                "reason".to_owned(),
                "a heading has no version history (LIB-026/6.2)".to_owned(),
            );
            refusal
        }
        RevisionError::Member { column, .. } => {
            let mut fallback = AppError::new(ErrorCode::DbLocked);
            fallback.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("RevisionCorrupt".to_owned()),
                os_message: Some(format!("a stored {column} cell is outside its vocabulary")),
            });
            fallback
        }
    }
}

/// Lists the instruction's versions and every variant's versions,
/// including soft-deleted variants, newest first (`created_at DESC`,
/// `rev_no DESC`, `id DESC`).
#[tauri::command]
#[specta::specta]
pub(crate) async fn revisions_list<R: Runtime>(
    app: AppHandle<R>,
    instruction_id: String,
) -> Result<Vec<RevisionSummaryRecord>, AppError> {
    revisions_list_impl(app, instruction_id)
        .await
        .inspect_err(logging::revisions_list_failed)
}

async fn revisions_list_impl<R: Runtime>(
    app: AppHandle<R>,
    instruction_id: String,
) -> Result<Vec<RevisionSummaryRecord>, AppError> {
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            list_revision_summaries(transaction, &instruction_id)
                .map(|rows| {
                    rows.into_iter()
                        .map(RevisionSummaryRecord::from_store)
                        .collect()
                })
                .map_err(|error| revision_error(error, "instruction_id"))
        })
        .await
}

/// Reads one full revision by its row id.
#[tauri::command]
#[specta::specta]
pub(crate) async fn revision_get<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<RevisionRecord, AppError> {
    revision_get_impl(app, id)
        .await
        .inspect_err(logging::revision_get_failed)
}

async fn revision_get_impl<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<RevisionRecord, AppError> {
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            get_revision(transaction, &id)
                .map_err(|error| revision_error(error, "id"))?
                .map(RevisionRecord::from_store)
                .ok_or_else(|| {
                    let mut refusal = AppError::new(ErrorCode::Validation);
                    refusal.params.insert("field".to_owned(), "id".to_owned());
                    refusal
                        .params
                        .insert("reason".to_owned(), "no revision has this id".to_owned());
                    refusal
                })
        })
        .await
}
