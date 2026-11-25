//! The Projects module's `project_relink` command (spec §7.10, T-110;
//! D-09, DB-015, SYS-005, SYS-007; the contract's resolved readings
//! live in DECISIONS.md D-208 — the add flow's gate order over the
//! restore/update transaction shape).
//!
//! One id-addressed write over a picked folder, ONE `Db::run`
//! transaction (D-037): the store's [`eunomia_store::projects::relink`]
//! path transition — `path` and `path_key` move together (the key is
//! derived from the canonical root, DB-030; the caller's subfolder
//! pick resolves to the repository's root, the card's own
//! dependency) — and the single `project.changed` journal entry —
//! commit or roll back together (D-051). The assignments (DB-014),
//! `sort_order`, `added_via`, `state`, every §6.9/PRJ-012 field and
//! the OUT-020 sync cells stay exactly where they were (the family
//! posture): the command writes no hash of its own, and the DB-021
//! dirty verdict the moved target path implies (DB-020 covers it) is
//! the recalculation service's row-decidable answer.
//!
//! The gate order is the add flow's mirror (the only family member
//! that also receives a picked folder): SYS-005 first
//! ([`reject_during_sync`] — a check, never an acquire), then the
//! SEC-003 session gate ([`session_paths::validate_ui_path`] — the
//! Locate path is a UI-named path from `dialog_pick_folder`, so it
//! must be a registered project or a folder picked this session),
//! then the `.git` check ([`eunomia_fs::paths::find_git_root`] on the
//! blocking pool, BEFORE the transaction — the D-096 ordering: a dead
//! drive must not hold the one database lock), then the database and
//! the one transaction: the by-id row read, the converging
//! same-location arm, the DB-015 collision check, the transition and
//! the journal insert.
//!
//! No file WRITES at all (the D-194/D-204/D-205 posture): the old
//! folder is never probed (the common case is the folder MOVED, and a
//! copied repository relinks identically), the old target file is the
//! user's own, and the new target file is the next sync's. The one
//! file-system interaction is READ-only: the root probe above and the
//! watch pass a committed relink then awaits (OUT-011).
//!
//! The watch arm (SYS-007, T-279's LAST member, D-204's mirror): the
//! root is a watch input (`ProjectFact` derives the watched file from
//! it), so the committed arm — the row actually moved — awaits ONE
//! full watch pass through the bounded mutation seam
//! [`file_watch::refresh_after_mutation`] (D-156) strictly AFTER the
//! transaction, never-fatal, and the pass only READS target files. A
//! refused arm never reaches the refresh; the converging re-send runs
//! none. The SYS-005 gate runs as the first act, before everything:
//! while a sync holds the global `sync_running` flag, the command
//! answers the typed `E_SYNC_RUNNING` refusal and writes nothing.

use eunomia_store::action_log::{
    Action, Entity, EntityType, Level, LogEntry, Outcome, Visibility, insert,
};
use eunomia_store::projects::{self, ProjectState};
use tauri::{AppHandle, Runtime};

use crate::commands::{
    activity_log_to_app_error, bounded_echo, managed_db, projects_store_to_app_error,
    reject_during_sync, rejected_field, rejected_non_utf8_path, repo_lookup_failed,
};
use crate::db::join_failure_to_app_error;
use crate::errors::AppError;
use crate::file_watch;
use crate::logging;
use crate::session_paths;

/// Re-points the project with this id at a newly picked folder (D-09's
/// "Locate…": the action an unavailable column offers beside
/// "Archive"): the picked folder must pass the SEC-003 session gate
/// and the `.git` check — it resolves to its repository's CANONICAL
/// root (nearest root wins, D-06; the card's own dependency sentence)
/// — and the row's `path` and `path_key` move there TOGETHER in one
/// transaction that also journals the single `project.changed` entry
/// (LOG-001…004/D-051, D-208 point 3).
///
/// Id-addressed like `project_update` (the id comes from
/// `projects_list`'s active columns — the Locate action rides one,
/// D-09/MAIN-028), and the answer is `()`: the screen re-reads
/// through the already-landed `projects_list`. An id no ACTIVE row
/// matches — unknown, or archived since the column rendered (the
/// archived screen's own action is `project_restore`) — is the typed
/// `E_VALIDATION` refusal naming `project_id` with the bounded echo
/// (D-115's cap); nothing was written.
///
/// The transition keeps everything it does not own (D-208 point 4):
/// the assignments (DB-014, keyed by the unmoved id), the position,
/// the origin, every settings field and the OUT-020 sync cells — the
/// stored hash covers the PRE-relink target, and the dirty verdict is
/// the recalculation service's row-decidable answer over the new one
/// (DB-021 over DB-020).
///
/// DB-015 (checked inside the transaction, the add flow's own
/// serialization rule): a resolved key another project already holds
/// refuses the whole operation with the typed `E_VALIDATION` envelope
/// naming `path` (the caller's path echoed bounded, a class-only
/// reason). The ACTIVE arm is the plain refusal; the ARCHIVED arm —
/// the DB-015 offer shape adapted to this command's `Err`-only wire —
/// additionally carries `params.existing_project_id`, the archived
/// row's id, exactly what `project_restore` (T-109) consumes: the UI
/// cannot find the row itself (`projects_list` serves no path
/// column), so the offer's id is the refusal's own fact to carry.
///
/// Idempotent (the make-operations-idempotent rule, D-208 point 3):
/// a re-send whose resolved root equals the row's stored `path` AND
/// `path_key` converges with `Ok(())` — nothing written, no
/// `updated_at` restamp, no journal entry, no watch refresh (the
/// `project_update` verified no-op). A screen's retry after any
/// interruption converges.
///
/// While a sync holds the global `sync_running` flag, the command
/// answers the typed `E_SYNC_RUNNING` envelope instead (SYS-005,
/// [`reject_during_sync`] — a check, never an acquire), and the
/// screen's retry after the run ends converges. Recovery mode answers
/// the honest `NoDatabase` fallback: the picked path passes the
/// SEC-003 session half first (it needs no database), so the answer
/// is the database's own.
///
/// A folder outside any repository is §10's `E_NOT_A_REPO`
/// ("pick the repository root"); an unresolvable or never-picked path
/// is refused by the SEC-003 gate before any probe (D-100's recorded
/// cost), and a probe the OS refused between the two canonicalizations
/// is `E_PROJECT_UNAVAILABLE` with the OS facts ("could not check" is
/// not "not a repo", D-090); a root whose canonical text is not valid
/// UTF-8 is the D-086 addendum's encoding rejection.
///
/// The committed relink awaits ONE full watch pass through the
/// bounded mutation seam after the transaction (SYS-007 — the root is
/// a watch input; [`file_watch::refresh_after_mutation`], never-fatal
/// aftermath: a refused send, a failed pass or an expired budget
/// degrades with the class-only warn and the relink stands, the next
/// watch anchor re-derives the set without loss). The converging
/// re-send runs no refresh.
///
/// Failures reach the log with their §10 code
/// ([`logging::project_relink_failed`]); a successful relink is the
/// journal entry itself (the D-051 mirror policy — info+user+ok is
/// not mirrored to the file log).
#[tauri::command]
#[specta::specta]
pub(crate) async fn project_relink<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
    path: String,
) -> Result<(), AppError> {
    project_relink_impl(app, project_id, path)
        .await
        .inspect_err(logging::project_relink_failed)
}

/// The body of [`project_relink`], split so the one logging edge
/// covers every failure path (the `activity_list` shape) — including
/// the SYS-005 refusal, which fires before anything else. The watch
/// refresh's never-fatal degrade is NOT a failure path: it logs its
/// own class and the answer stays `Ok`.
async fn project_relink_impl<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
    path: String,
) -> Result<(), AppError> {
    // SYS-005 (§5.4): the mode gate runs before everything — while a
    // sync holds `sync_running`, the relink is refused with
    // `E_SYNC_RUNNING` whatever the input is (see `reject_during_sync`).
    reject_during_sync(&app)?;
    // SEC-003 (§5.6): the input gate, the add flow's own order — the
    // Locate path is a UI-named path (the picker's answer), so it must
    // be a registered project or a folder picked in this session
    // before anything else runs: a foreign folder never reaches the
    // `.git` check, the database, or the disk probes. The canonical
    // form comes back from the check.
    let picked = session_paths::validate_ui_path(&app, &path).await?;
    // D-09's `.git` check (the card's dependency: find the git
    // repository ROOT for the selected folder): the walk-up over live
    // folders is blocking I/O (SYS-001) — a dead mount must not stall
    // an async worker — and it runs BEFORE the transaction (the
    // D-096 probe-order rule: a dead drive delays only this response,
    // never the one database lock). A subfolder resolves to its
    // repository's canonical root (nearest root wins, D-06); a folder
    // outside any repository is `E_NOT_A_REPO`; a refused probe is
    // `E_PROJECT_UNAVAILABLE` with the OS facts (D-090's split).
    let walked = picked;
    let root =
        tauri::async_runtime::spawn_blocking(move || eunomia_fs::paths::find_git_root(&walked))
            .await
            .map_err(join_failure_to_app_error)?
            .map_err(|source| repo_lookup_failed(&source))?;
    // The identity half of the add flow's `ProjectFields::derive` (the
    // row keeps its own name and every other field): the canonical
    // root's verbatim UTF-8 text and its DB-030 key.
    let new_path = root
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| rejected_non_utf8_path(&root.to_string_lossy()))?;
    let new_path_key = eunomia_fs::paths::path_key(&root)
        .map_err(|source| rejected_non_utf8_path(source.path()))?;
    let database = managed_db(&app)?;
    let committed = database
        .run(move |transaction| {
            // The row first: the by-id read decides the stale-screen
            // branch and carries the stored identity the convergence
            // and collision arms compare against. The refusal echoes
            // the caller's id bounded (D-115's cap: a webview string
            // is unbounded by definition, and it rides the envelope
            // and the §10 log line otherwise).
            let project = projects::get(transaction, &project_id)
                .map_err(projects_store_to_app_error)?
                .filter(|project| project.state == ProjectState::Active)
                .ok_or_else(|| {
                    rejected_field(
                        "project_id",
                        &bounded_echo(&project_id),
                        "no active project has this id",
                    )
                })?;
            // D-208 point 3, the idempotence rule: a re-send whose
            // resolved root equals the row's stored location pair
            // converges — Ok with nothing written (no `updated_at`
            // restamp, no journal entry; the refresh half is the
            // caller's `committed == false` below), the
            // `project_update` verified no-op.
            if project.path == new_path && project.path_key == new_path_key {
                return Ok(false);
            }
            // DB-015 inside the operation's own transaction (the add
            // flow's serialization rule: the check and the write share
            // the one connection, so the write cannot race past it):
            // the resolved key must be free in EITHER state, the row
            // itself excluded (a divergent stored pair — a tampered
            // row — relinks onto its own key's root rather than
            // colliding with itself; unreachable through the flows,
            // which keep path and key consistent).
            if let Some(existing) = projects::find_by_path_key(transaction, &new_path_key)
                .map_err(projects_store_to_app_error)?
                .filter(|existing| existing.id != project.id)
            {
                let mut refusal = rejected_field(
                    "path",
                    &bounded_echo(&path),
                    if existing.state == ProjectState::Active {
                        "an active project already has this path (DB-015)"
                    } else {
                        "an archived project already has this path (DB-015)"
                    },
                );
                if existing.state == ProjectState::Archived {
                    // The DB-015 offer shape adapted to this command's
                    // `Err`-only wire (D-208 point 5): the archived
                    // row's id — a UUIDv7 TEXT, machine-generated
                    // identity rather than user content, so Rule 8
                    // does not keep it off the envelope — is exactly
                    // what `project_restore` (T-109) consumes, and the
                    // UI cannot find the row itself.
                    refusal
                        .params
                        .insert("existing_project_id".to_owned(), existing.id);
                }
                return Err(refusal);
            }
            // The transition (D-208 point 4): `path` and `path_key`
            // move together, `updated_at` restamps, and NOTHING else
            // — the assignments, the position, the origin, the sync
            // cells and every settings field stay verbatim. The
            // store's own WHERE guard keeps the call idempotent.
            let relinked = projects::relink(transaction, &project_id, &new_path, &new_path_key)
                .map_err(projects_store_to_app_error)?;
            // LOG-001…004 ("changing … projects") inside the
            // operation's own transaction (D-051): ONE attributable
            // entry — the closed vocabulary's own member for the
            // relink ("A project's row changed (update, relink,
            // reorder)"); entity and project carry the row's id; no
            // details (the `project_update` shape: the row itself
            // holds every fact this command writes).
            let mut entry = LogEntry::new(
                Action::ProjectChanged,
                Level::Info,
                Visibility::User,
                Outcome::Ok,
            );
            entry.entity = Some(Entity {
                kind: EntityType::Project,
                id: relinked.id.as_str(),
            });
            entry.project_id = Some(relinked.id.as_str());
            insert(transaction, &entry).map_err(activity_log_to_app_error)?;
            // The D-051 mirror contract: one call per written entry
            // (info+user+ok is the unmirrored shape today).
            logging::action_logged(&entry);
            Ok(true)
        })
        .await?;
    // SYS-007/D-144, T-279's LAST arm (D-208 point 6): the committed
    // relink — the row's root actually moved, and the root is a watch
    // input (`ProjectFact` derives the watched file from it) — awaits
    // ONE full watch pass through the bounded mutation seam: the same
    // awaited pass (command, ack, service pass) `project_add_manual`'s
    // committed arm, T-107's arm and T-109's arm run, with the
    // interactive wait budget D-156/T-280 added. NEVER-FATAL
    // aftermath: the relink stands committed whatever the pass
    // answers — a refused send (a watcher backend that never
    // started), a failed pass or an expired budget degrades with the
    // class-only warn, and the next watch anchor re-derives the set
    // without loss (D-144's convergence). Strictly AFTER the
    // transaction — the journal insert stays inside it, the refresh
    // never is. The converging re-send (committed == false) runs no
    // refresh: a pass over a row that did not move is file reads with
    // no observable change (D-201 point 7's own boundary).
    if committed && let Err(refused) = file_watch::refresh_after_mutation(&app).await {
        logging::file_watch_mutation_refresh_degraded(&refused);
    }
    Ok(())
}
