//! The Projects module's `project_restore` command (spec §7.10,
//! T-109; PRJ-013…018, §6.4, D-10, DB-015, SYS-005, SYS-007; the
//! contract's resolved readings live in DECISIONS.md D-205 — the
//! D-194 mirror).
//!
//! One id-addressed write, ONE `Db::run` transaction (D-037): the row
//! transition the store's [`eunomia_store::projects::restore`] owns,
//! the exclusion deletion keyed by the row's [`path_key`] (DB-030's
//! identity — reason-AGNOSTIC, the pinned owner ruling: the
//! archive-born `removed_scanned` row and a pre-existing `manual`
//! exclusion the archive deliberately left in place both go; no row
//! at all is the converging half, the end state already true), and
//! the single `project.restored` journal entry — commit or roll back
//! together (D-051). Assignments (DB-014), `path_key` (DB-015),
//! `sort_order` (the store's recorded §6.4 position-keeping) and
//! `last_synced_hash` (D-194 point 3's deliberate keep: the stored
//! hash covers the PRE-archive build configuration) are untouched —
//! the restore writes no hash of its own, and the DB-021 dirty
//! verdict is the recalculation service's over the now-active row
//! (the D-194/D-204 row-decidable posture).
//!
//! No file WRITES at all (D-204 point 6's posture, mirrored): §6.4's
//! folder guarantee holds through the restore exactly as through the
//! archive, and a project whose folder died restores fine — no
//! availability probe, no folder check. The one deliberate exception
//! to the command's otherwise complete lack of file-system
//! interaction is the watch refresh (D-205 point 5, SYS-007's
//! reverse membership arm): the committed transition returns the row
//! to the watched set — the ADD arm's shape (D-144/D-204 point 7) —
//! so it awaits ONE full watch pass through the bounded mutation
//! seam [`file_watch::refresh_after_mutation`] (D-156) strictly AFTER
//! the transaction, never-fatal, and the pass only READS target files
//! (OUT-011). A converging re-send runs no refresh. The SYS-005 gate
//! runs as the first act, before the database is resolved: while a
//! sync holds the global `sync_running` flag, the command answers
//! the typed `E_SYNC_RUNNING` refusal and writes nothing.
//!
//! [`path_key`]: eunomia_store::projects::Project::path_key

use eunomia_store::action_log::{
    Action, Entity, EntityType, Level, LogEntry, Outcome, Visibility, insert,
};
use eunomia_store::projects::{self, ProjectState};
use tauri::{AppHandle, Runtime};

use crate::commands::{
    activity_log_to_app_error, managed_db, projects_store_to_app_error, reject_during_sync,
    rejected_field,
};
use crate::errors::AppError;
use crate::file_watch;
use crate::logging;

/// Restores the archived project with this id (§6.4 / PRJ-013…018:
/// `state` returns to `active`): the project comes back with its
/// assignments (DB-014) and its old `sort_order` position, and its
/// path is removed from the exclusions — all in one transaction that
/// also journals the single `project.restored` entry (LOG-001…004,
/// D-051).
///
/// Id-addressed like `project_archive` (the id comes from
/// `projects_list`'s widened read, the D-10 "Archived projects"
/// screen's own rows; the two DB-015 offer carriers — the manual
/// add's `archived_duplicate` arm and `scan_commit`'s `"archived"`
/// outcome — name the same row through their `existing` answer), and
/// the answer is `()` — the screens re-read through the already-landed
/// `projects_list` and `exclusions_list`. An id no row matches is the
/// typed `E_VALIDATION` refusal naming `project_id` (a stale screen's
/// re-send); nothing was written.
///
/// The exclusion deletion is keyed by the row's `path_key` and is
/// reason-agnostic: the archive-born row AND a pre-existing manual
/// exclusion the archive deliberately left in place are both deleted
/// (the pinned owner ruling, D-205 point 1) — PRJ-013…018's end state
/// names the path, not a row class. A row whose exclusion was since
/// hand-removed restores fine (the converging no-existence arm): the
/// command never re-adds what the user removed.
///
/// Idempotent (the make-operations-idempotent rule, D-205 point 3 —
/// the D-194 point-2 mirror): a re-send on an already-active row
/// converges with `Ok(())` — no journal entry, no `updated_at`
/// restamp (the store's own rule), no exclusion churn even if an
/// exclusion row with the same `path_key` has since appeared: the
/// re-send overrides no fact it did not own.
///
/// While a sync holds the global `sync_running` flag, the command
/// answers the typed `E_SYNC_RUNNING` envelope instead (SYS-005,
/// [`reject_during_sync`] — a check, never an acquire), and the
/// screen's retry after the run ends converges. Recovery mode
/// answers the honest `NoDatabase` fallback: the flag is down there
/// by construction, so the answer comes from [`managed_db`].
///
/// The committed restore awaits ONE full watch pass through the
/// bounded mutation seam after the transaction (SYS-007's reverse
/// membership arm — the row returns to the watched set;
/// [`file_watch::refresh_after_mutation`], never-fatal aftermath: a
/// refused send, a failed pass or an expired budget degrades with the
/// class-only warn and the restore stands, the next watch anchor
/// re-derives the set without loss). The converging re-send runs no
/// refresh.
///
/// Failures reach the log with their §10 code
/// ([`logging::project_restore_failed`]); a successful restore is the
/// journal entry itself (the D-051 mirror policy — info+user+ok is
/// not mirrored to the file log).
#[tauri::command]
#[specta::specta]
pub(crate) async fn project_restore<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
) -> Result<(), AppError> {
    project_restore_impl(app, project_id)
        .await
        .inspect_err(logging::project_restore_failed)
}

/// The body of [`project_restore`], split so the one logging edge
/// covers every failure path (the `activity_list` shape) — including
/// the SYS-005 refusal, which fires before anything else.
async fn project_restore_impl<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
) -> Result<(), AppError> {
    // SYS-005 (§5.4): the mode gate runs before everything — while a
    // sync holds `sync_running`, the restore is refused with
    // `E_SYNC_RUNNING` whatever the input is (see `reject_during_sync`).
    reject_during_sync(&app)?;
    let database = managed_db(&app)?;
    let committed = database
        .run_with_mirror(move |transaction, mirror| {
            // The row first: the by-id read decides the idempotence
            // branch and carries the identity key the exclusion half
            // needs. The refusal echoes the caller's id bounded
            // (D-115's cap: a webview string is unbounded by
            // definition, and it rides the envelope and the §10 log
            // line otherwise).
            let project = projects::get(transaction, &project_id)
                .map_err(projects_store_to_app_error)?
                .ok_or_else(|| {
                    rejected_field(
                        "project_id",
                        &crate::commands::bounded_echo(&project_id),
                        "no project has this id",
                    )
                })?;
            // D-205 point 3: the re-send converges — an already-active
            // row answers Ok with nothing written (no journal entry,
            // no `updated_at` restamp, no exclusion churn even if an
            // exclusion row with the same `path_key` has since
            // appeared — the re-send overrides no fact it did not
            // own, D-194 point 2's own posture).
            if project.state == ProjectState::Active {
                return Ok(false);
            }
            // §6.4's row transition — the store owns the state, the
            // stamp-clearing and the position-keeping.
            let restored =
                projects::restore(transaction, &project_id).map_err(projects_store_to_app_error)?;
            // PRJ-013…018's second half: "removes its path from the
            // exclusions" — the deletion is keyed by the row's
            // identity key (DB-030, the same key the archive wrote)
            // and is reason-AGNOSTIC (D-205 point 1): the archive-born
            // `removed_scanned` row AND a pre-existing manual
            // exclusion the archive deliberately left in place are
            // both deleted — the end state names the path, not a row
            // class, and a surviving row would keep hiding the
            // restored path from the scan. No row at all is the
            // converging half (D-205 point 2): the end state is
            // already true, and nothing is ever re-added.
            if let Some(exclusion) =
                projects::find_exclusion_by_path_key(transaction, &restored.path_key)
                    .map_err(projects_store_to_app_error)?
            {
                projects::delete_exclusion(transaction, &exclusion.id)
                    .map_err(projects_store_to_app_error)?;
            }
            // LOG-001…004 ("restoring … projects") inside the
            // operation's own transaction (D-051): ONE attributable
            // entry — entity and project carry the restored row's id;
            // no details (the `project.archived` shape: the row itself
            // holds the path). The exclusion deletion is not journaled
            // separately (D-205 point 7, D-194 point 4's mirror): this
            // entry, unlike `exclusion_remove`'s, names its object,
            // and the D-10 "Excluded paths" screen re-reads the table
            // through `exclusions_list` — the row's disappearance IS
            // the screen's data.
            let mut entry = LogEntry::new(
                Action::ProjectRestored,
                Level::Info,
                Visibility::User,
                Outcome::Ok,
            );
            entry.entity = Some(Entity {
                kind: EntityType::Project,
                id: restored.id.as_str(),
            });
            entry.project_id = Some(restored.id.as_str());
            insert(transaction, &entry).map_err(activity_log_to_app_error)?;
            // The T-324/D-228 mirror contract: record inside the
            // transaction, flush only after the commit (info+user+ok
            // is the unmirrored shape today).
            mirror.record(&entry);
            Ok(true)
        })
        .await?;
    // SYS-007/D-144, T-279's arm (D-205 point 5): the committed
    // transition — the row actually returned to the active set, the
    // ADD arm's shape — awaits ONE full watch pass through the
    // bounded mutation seam: the same awaited pass (command, ack,
    // service pass) `project_add_manual`'s committed arm and T-107's
    // arm run, with the interactive wait budget D-156/T-280 added.
    // NEVER-FATAL aftermath: the restore stands committed whatever
    // the pass answers — a refused send (a watcher backend that
    // never started), a failed pass or an expired budget degrades
    // with the class-only warn, and the next watch anchor re-derives
    // the set without loss (D-144's convergence). Strictly AFTER the
    // transaction — the journal insert stays inside it, the refresh
    // never is. The converging re-send (committed == false) runs no
    // refresh: a pass over a row that did not move is file reads with
    // no observable change (D-201 point 7's own boundary).
    if committed && let Err(refused) = file_watch::refresh_after_mutation(&app).await {
        logging::file_watch_mutation_refresh_degraded(&refused);
    }
    Ok(())
}
