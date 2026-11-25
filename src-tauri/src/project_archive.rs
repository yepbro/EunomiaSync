//! The Projects module's `project_archive` command (spec §7.10, T-108;
//! PRJ-013…018, §6.4, DB-014, D-10, SYS-005; the contract's five
//! resolved readings live in DECISIONS.md D-194).
//!
//! One id-addressed write, ONE `Db::run` transaction (D-037): the row
//! transition the store's [`eunomia_store::projects::archive`] owns,
//! the PRJ-017 exclusion row for a **scan-born** project (converging:
//! an already-excluded path stays as it is), and the single
//! `project.archived` journal entry — commit or roll back together
//! (D-051). Assignments (DB-014), `path_key` (DB-015: an archived row
//! still holds its uniqueness), `sort_order` (the store's recorded
//! §6.4 reading: archive keeps the position) and `last_synced_hash`
//! (D-194 point 3: the restore's own facts stay) are untouched;
//! "not counted as dirty" is
//! decided by the read paths that already count it (D-128's
//! row-decidable arm) and the recalc service's silent prune, so this
//! command writes no dirty fact of its own.
//!
//! No file I/O at all (D-194): §6.4's "the project folder is not
//! touched" is total — no availability probe, no folder check, and a
//! project whose folder died archives fine (`projects_list` keeps
//! "Archive" an available action on an unavailable column). The
//! SYS-005 gate runs as the first act, before the database is
//! resolved: while a sync holds the global `sync_running` flag, the
//! command answers the typed `E_SYNC_RUNNING` refusal and writes
//! nothing.

use eunomia_store::action_log::{
    Action, Entity, EntityType, Level, LogEntry, Outcome, Visibility, insert,
};
use eunomia_store::projects::{self, AddedVia, ExclusionReason, NewExclusion, ProjectState};
use tauri::{AppHandle, Runtime};

use crate::commands::{
    activity_log_to_app_error, managed_db, projects_store_to_app_error, reject_during_sync,
    rejected_field,
};
use crate::errors::AppError;
use crate::logging;

/// Archives the project with this id (§6.4: `state = archived`): the
/// folder is not touched, the assignments are kept (DB-014), and a
/// **scan-born** project's path goes to `project_exclusions`
/// (PRJ-017) — all in one transaction that also journals the single
/// `project.archived` entry (LOG-001…004, D-051).
///
/// Id-addressed like `exclusion_remove` (the id comes from
/// `projects_list`'s rows; no UI-typed path can become a store fact
/// through this command), and the answer is `()` — the D-10 screens
/// re-read through the widened `projects_list` and `exclusions_list`
/// that already exist. An id no row matches is the typed
/// `E_VALIDATION` refusal naming `project_id` (a stale screen's
/// re-send); nothing was written.
///
/// Idempotent (the make-operations-idempotent rule, D-194): a re-send
/// on an already-archived row converges with `Ok(())` — no journal
/// entry, `archived_at` not restamped (the store's own rule), no
/// exclusion churn. A pre-existing exclusion row with the same
/// `path_key` (whatever its reason — a manual exclusion, or a prior
/// archive's row a later restore will remove) is left untouched:
/// PRJ-017's end state is already true, and the existing row's own
/// reason and date are facts of their own.
///
/// While a sync holds the global `sync_running` flag, the command
/// answers the typed `E_SYNC_RUNNING` envelope instead (SYS-005, the
/// card's own line; [`reject_during_sync`] — a check, never an
/// acquire), and the screen's retry after the run ends converges.
/// Recovery mode answers the honest `NoDatabase` fallback: the flag
/// is down there by construction, so the answer comes from
/// [`managed_db`].
///
/// Failures reach the log with their §10 code
/// ([`logging::project_archive_failed`]); a successful archive is the
/// journal entry itself (the D-051 mirror policy — info+user+ok is
/// not mirrored to the file log).
#[tauri::command]
#[specta::specta]
pub(crate) async fn project_archive<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
) -> Result<(), AppError> {
    project_archive_impl(app, project_id)
        .await
        .inspect_err(logging::project_archive_failed)
}

/// The body of [`project_archive`], split so the one logging edge
/// covers every failure path (the `activity_list` shape) — including
/// the SYS-005 refusal, which fires before anything else.
async fn project_archive_impl<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
) -> Result<(), AppError> {
    // SYS-005 (§5.4): the mode gate runs before everything — while a
    // sync holds `sync_running`, the archive is refused with
    // `E_SYNC_RUNNING` whatever the input is (see `reject_during_sync`).
    reject_during_sync(&app)?;
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            // The row first: the by-id read decides the idempotence
            // branch and carries the origin the exclusion half needs.
            // The refusal echoes the caller's id bounded (D-115's
            // cap: a webview string is unbounded by definition, and
            // it rides the envelope and the §10 log line otherwise).
            let project = projects::get(transaction, &project_id)
                .map_err(projects_store_to_app_error)?
                .ok_or_else(|| {
                    rejected_field(
                        "project_id",
                        &crate::commands::bounded_echo(&project_id),
                        "no project has this id",
                    )
                })?;
            // D-194: the re-send converges — an already-archived row
            // answers Ok with nothing written (no journal entry, no
            // `archived_at` restamp, no exclusion churn even if the
            // exclusion row was since removed by hand).
            if project.state == ProjectState::Archived {
                return Ok(());
            }
            // §6.4's row transition — the store owns the state, the
            // stamp and the position-keeping.
            let archived =
                projects::archive(transaction, &project_id).map_err(projects_store_to_app_error)?;
            // PRJ-017: a scan-born project's path goes to the
            // exclusions so a later scan cannot resurface it. Only
            // scan-born rows: §6.4's own "if the project was found by
            // a scan" is the row's origin column, and a manual-born
            // project's way back is PRJ-001's picker. Converging: the
            // two tables are separate namespaces, so an exclusion with
            // this `path_key` may already exist (a manual exclusion of
            // the same path, or a prior archive's row a restore will
            // remove) — it stays as it is, and the typed duplicate
            // outcome never fires at this boundary.
            if project.added_via == AddedVia::Scan
                && projects::find_exclusion_by_path_key(transaction, &archived.path_key)
                    .map_err(projects_store_to_app_error)?
                    .is_none()
            {
                projects::create_exclusion(
                    transaction,
                    &NewExclusion {
                        path: &archived.path,
                        path_key: &archived.path_key,
                        reason: ExclusionReason::RemovedScanned,
                    },
                )
                .map_err(projects_store_to_app_error)?;
            }
            // LOG-001…004 ("archiving … projects") inside the
            // operation's own transaction (D-051): ONE attributable
            // entry — entity and project carry the archived row's id;
            // no details (the `register_project` shape: the row itself
            // holds the path). The exclusion write is not journaled
            // separately: the closed vocabulary has no exclusion-add
            // member (D-106 added only the removal's), and this entry,
            // unlike that command's old shape, names its object.
            let mut entry = LogEntry::new(
                Action::ProjectArchived,
                Level::Info,
                Visibility::User,
                Outcome::Ok,
            );
            entry.entity = Some(Entity {
                kind: EntityType::Project,
                id: archived.id.as_str(),
            });
            entry.project_id = Some(archived.id.as_str());
            insert(transaction, &entry).map_err(activity_log_to_app_error)?;
            // The D-051 mirror contract: one call per written entry
            // (info+user+ok is the unmirrored shape today).
            logging::action_logged(&entry);
            Ok(())
        })
        .await
}
