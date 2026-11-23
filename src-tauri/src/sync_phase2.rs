//! The sync pipeline's phase 2 — backups (spec §9.4 phase 2, §7.6
//! BKP-001…011 via the fs crate, §10's `E_BACKUP_FAILED` row, OUT-006/
//! OUT-019; task T-115): for every project phase 1 said the run will
//! write AND whose target file exists, copy that file into the §5.5
//! backups tree through [`eunomia_fs::backup`] — which verifies the
//! copy's size and SHA-256 before it counts (BKP-010) — and record
//! each copy in `backup_records` through the new
//! [`eunomia_store::backup_records`] writer. Any error aborts the
//! whole run before a single target byte moves.
//!
//! **Who gets a copy (§9.4 phase 2 step 1's two conditions).** "For
//! each project that will be written and whose target file exists":
//! the first half is phase 1's `ready` verdict (§9.4's own words —
//! `unchanged` projects are not written, and `skipped` ones were
//! refused), the second is [`ReadyProject::existing_size`] being
//! `Some` (OUT-005's creation case: a project whose target does not
//! exist yet has nothing to back up — the write creates the file).
//! `unchanged`/`skipped` projects therefore never reach the copy
//! loop, and a run whose eligible list is EMPTY performs no copy, no
//! record and no abort-adjacent work at all — the run continues
//! (§9.4's pipeline has no step for "nothing to back up" because
//! zero copies cannot fail).
//!
//! **Dedup rides the store's lookup (BKP-010) — and writes no row.**
//! Before each copy the phase's one read transaction asks
//! [`backup_records::last_checked`] for the newest checked copy of
//! the "project and file" pair and hands its digest to the fs
//! operation as `last_backup_sha256_hex` — but only after the copy
//! loop has verified the recorded copy's FILE still exists on the
//! blocking pool: an `ok` row can outlive its file (D-122's recorded
//! window — a user-deleted backups tree, a quarantined copy), and a
//! dedup over a missing file would send phase 3 into its write with
//! no restore point, exactly what BKP-005's "no forced write without
//! a backup" forbids. A vanished copy therefore loses its dedup
//! claim: the phase hands the fs operation no digest and a fresh
//! copy is made (D-091's own docs place the existence question on
//! the caller). An equal digest then returns
//! [`BackupOutcome::Deduplicated`] — no file, no folders — and the
//! bookkeeping stays with the EXISTING row: the schema's UNIQUE
//! `backup_path` column is one row per copy, and a dedup decision
//! creates no copy. BKP-010's "the record points to the existing
//! one" is therefore read as "the pair's record-pointer resolves to
//! the existing copy" — the existing `ok` row already documents it
//! (its own run, its own stamp), and a second row over the same file
//! cannot exist. The outcome still carries the
//! [`BackupFact::Deduplicated`] fact (the referenced copy's path),
//! so the run's log and phase 4 see what happened.
//!
//! **The composition, D-130-shaped** (two transactions around a
//! blocking pass — the phase 0/1 pattern):
//!
//! 1. ONE read transaction: each eligible project's name, scheme and
//!    base name (the projects table — the D-131 "second home" ruling:
//!    the run holds `sync_running`, so the read cannot observe an
//!    app-driven change) and the pair's last checked digest. The
//!    `source_path` spelling stored in every record is the RESOLVED
//!    path's text — D-122's one-stable-spelling contract; a resolved
//!    path that is not UTF-8 is the class-only refusal (the phase-0
//!    `target_path_text` family), never a lossy copy.
//! 2. The copy loop on `spawn_blocking` AFTER the transaction
//!    (SYS-001, the D-096 ordering — three streaming passes per file
//!    over possibly dead volumes must never hold the one database
//!    lock), through the injected `backup` operation: production
//!    closes over [`eunomia_fs::backup::backup_target_file`] itself;
//!    tests substitute failing copies (the D-089 free-space seam
//!    pattern — the fault points of spec 15.1 T-F1 live behind the fs
//!    crate's own `fault-injection` feature, which release builds
//!    never enable, so the app layer's seam is the honest injector).
//!    The loop stops at the FIRST error and keeps what it made.
//! 3. ONE write transaction: every CREATED copy's record lands `ok`
//!    (§9.4 phase 2 step 1: "add a record to `backup_records`"; the
//!    BKP-010 check already passed inside the fs operation), with one
//!    `created_at` for the whole run (D-061: the run's instant is a
//!    parameter, resolved once into the `BackupTime` the copy names
//!    carry and the DB-002 stamp the records carry — record and name
//!    cannot disagree). Dedup decisions write no row (the module
//!    docs' UNIQUE ruling).
//!
//! **The abort path (§9.4 phase 2 step 2, BKP-005/009).** Any copy or
//! verify error:
//!
//! * the copies THIS run created are physically deleted first — on
//!   the blocking pool, `NotFound` already converged (D-091's
//!   `remove_copy` ruling), §9.5's transient-hold ladder applied
//!   (100/300/900 ms on a sharing violation or access denied — the
//!   same backoff the retention prunes ride), and the failed copy's
//!   own leftover partial bytes (`cleanup: Some`, the fs operation's
//!   retrier handshake) swept with them; a deletion the filesystem
//!   still refuses leaves that copy's record `ok` (a verified live
//!   copy is a fact, not debris — the same never-lying-mark rule
//!   D-122 fixed for `pruned`) and the refusal rides the run row's
//!   details as a class-only count;
//! * ONE write transaction then writes the whole verdict: the
//!   created-but-deleted copies' records land AS `discarded` (§6.4's
//!   history-of-a-gone-copy status — born terminal, because the
//!   alternative, insert-`ok`-then-mark, spends a transaction to
//!   reach the same cells), [`abort_run`] finishes the run `aborted`
//!   with `E_BACKUP_FAILED` and the ABORT's own instant as
//!   `finished_at` — the injected `abort_clock`, read once right
//!   before this transaction, never the run's backup instant (which
//!   would strike the whole copy pass and sweep out of the run's
//!   duration: a slow backup that aborts read as a run of zero
//!   duration, T-325) — and [`reset_projects_not_started`]
//!   returns every non-`skipped` verdict to `not_started`
//!   ("the last sync of every project stays unchanged" is OUT-020's
//!   `last_synced_at`/`last_synced_hash`, which no code path here
//!   touches — the reset keeps the run's OWN rows honest);
//! * the caller hears the typed `E_BACKUP_FAILED` envelope whose
//!   `details` carry BKP-005's field list: the project NAME (the one
//!   §10 message the spec explicitly demands it in — the ruling this
//!   module records against the class-only default), the source path,
//!   the backups folder, the OS code (`BackupError::os_code`) and
//!   error type, the cause (`BackupError`'s Display), and §10's
//!   recommendation ("Fix the cause from the error text and retry").
//!   The run ROW's `details` stay class-only (project id, error type,
//!   paths, OS facts — no stored name text; SEC-008).
//!
//! A failure of the record or abort TRANSACTION itself is not a
//! backup error: the store error propagates as-is (the D-131 posture
//! — every `Err` before the abort leaves the run row `running`, the
//! crash-recovery rule's case), and on the record path the phase
//! first deletes the copies it made (they would be unreferenced
//! debris otherwise; best-effort, the D-091 sweep posture). A process
//! death between the file deletions and the abort transaction leaves
//! deleted files, no records and a `running` row — the startup
//! `interrupted` recovery's case, the same window D-122 recorded for
//! its delete-first order.
//!
//! **After this phase succeeds, the run cannot be canceled** (§9.4
//! phase 2 step 3): phase 2's `Ok` is the pipeline's point of no
//! return — everything after it (the write phase) is §9.4's own
//! forward march. The cancellation policy itself belongs to the
//! `sync_run` command (T-118), exactly as D-130/D-131 recorded.
//!
//! **Idempotence by convergence.** A retried run after an abort
//! starts phase 0 anew: the aborted run's rows are history (its
//! records `discarded`, its files gone), the eligible list is
//! re-derived, and a leftover copy that survived a refused deletion
//! is simply the pair's newest checked copy — the retry's dedup
//! lookup points at it (BKP-010's own acceptance). Nothing in this
//! phase requires recognizing a previous attempt's state.

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use tauri::{AppHandle, Runtime};

use eunomia_core::backup_name::{BackupTime, backup_dir_name};
use eunomia_core::project::{BaseName, ProjectId, Scheme};
use eunomia_fs::backup::{BackupCopy, BackupError, BackupOutcome, BackupTarget};
use eunomia_store::backup_records::{
    BackupRecordStatus, CheckedCopy, NewBackupRecord, insert as insert_record, last_checked,
};
use eunomia_store::sync_runs::{abort_run, reset_projects_not_started};

use crate::commands::{managed_db, projects_store_to_app_error};
use crate::db::{backup_records_to_app_error, join_failure_to_app_error, sync_runs_to_app_error};
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::sync_phase1::{Phase1Outcome, PreflightVerdict};

/// The §5.5 `backups` root — the folder half of BKP-005's message
/// when the project folder's own name cannot be spelled (an
/// unreachable shape for DB ids, kept for totality).
const BACKUPS_DIR: &str = "backups";

/// The phase's product for one backed-up project.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BackupFact {
    /// A new copy exists under the §5.5 layout and passed the
    /// BKP-010 check; its record is `ok`.
    Created(BackupCopy),
    /// BKP-010's dedup: the source digest equals the pair's last
    /// checked copy — no file was made and no record is written (the
    /// schema's UNIQUE `backup_path` keeps one row per copy; the
    /// existing row already documents it).
    Deduplicated {
        /// The source file's size in bytes.
        size: u64,
        /// The source's (and the existing copy's) full SHA-256.
        sha256_hex: String,
        /// The §5.5 relative path of the existing copy the pair's
        /// record already names.
        backup_path: String,
    },
}

/// What phase 2 hands the pipeline: the run's id and one fact per
/// backed-up project, in the snapshot's `sort_order`.
///
/// The fact list has no production reader yet — the run's summary
/// reads the committed rows back, and the future consumers (the P1
/// `backups_list` screen, run diagnostics) are not this card's; the
/// behavior suite reads it today (the D-130 tested-today/consumed-
/// later posture, updated by T-118: the sync-run command composes
/// the phase but not this outcome).
#[allow(dead_code)] // the fact list's consumers are future cards; tested today
#[derive(Debug)]
pub(crate) struct Phase2Outcome {
    /// The run these backups belong to (the `sync_runs` id).
    pub(crate) run_id: String,
    /// One `(project_id, fact)` pair per eligible project, in
    /// `sort_order`; empty when the run had nothing to back up.
    pub(crate) backups: Vec<(String, BackupFact)>,
}

/// One eligible project's copy inputs — everything the blocking copy
/// loop needs, owned (the loop moves onto the pool, SYS-001).
struct EligibleBackup {
    /// The project row id (also the `ProjectId` the name builders
    /// gate; DB UUIDv7s always pass, D-080).
    project_id: ProjectId,
    /// The raw project name — the builders clean it into the slug.
    project_name: String,
    /// The project's scheme, mapped onto core's enum (the backup
    /// file name's first input, SEC-006).
    scheme: Scheme,
    /// The target's base name (the enum file name's other half).
    base_name: BaseName,
    /// The resolved target file — the copy's source (§9.4 phase 1
    /// step 2 resolved it; the fs operation opens what it is given).
    source: PathBuf,
    /// The resolved path's text — the record's `source_path`
    /// (D-122's one stable spelling per file).
    source_text: String,
    /// The pair's newest checked copy, from the phase's read
    /// transaction (BKP-010's dedup input).
    last_checked: Option<CheckedCopy>,
}

/// What the blocking copy pass came to.
enum CopyPass {
    /// Every eligible project's copy was made (or deduped).
    Done(Vec<(String, BackupFact)>, Vec<NewRecordInput>),
    /// One copy failed: the typed error, the failing project's facts
    /// (BKP-005's message fields — boxed to keep the pass enum's
    /// other arm small), and everything made so far.
    Failed {
        error: BackupError,
        failed: Box<EligibleBackup>,
        made: Vec<NewRecordInput>,
    },
}

/// One record-shaped fact the pass produced — the input both write
/// transactions share, with the status decided by the path that
/// writes it (`ok` on success, `discarded`/`ok` by the abort sweep's
/// deletion outcome).
struct NewRecordInput {
    project_id: String,
    source_text: String,
    backup_path: String,
    /// The run's one DB-002 stamp (D-061: resolved once, shared by
    /// every record and every copy name of the run).
    created_at: String,
    size_bytes: i64,
    sha256_hex: String,
    /// The on-disk copy a `Created` fact must be able to delete on
    /// the abort path; `None` for dedup claims (the referenced copy
    /// is another run's — §9.4's "the copies of this run").
    copy_path: Option<PathBuf>,
}

/// Runs §9.4 phase 2 end to end over phase 1's outcome — the
/// production entry: [`run_with_abort_clock`] over the real clock.
///
/// The five parameters are the `sync_run` command's wiring and they
/// do not change: the abort's clock is not the command's to hand over
/// (production's is the real clock and nothing else), so the seam
/// sits BEHIND this signature and the tests drive
/// [`run_with_abort_clock`] directly — the command layer's own
/// `sync_run` / `sync_run_with` shape.
///
/// # Errors
///
/// [`run_with_abort_clock`]'s.
pub(crate) async fn run<R, B>(
    app: &AppHandle<R>,
    phase1: &Phase1Outcome,
    data_dir: &Path,
    backup: B,
    now: Timestamp,
) -> Result<Phase2Outcome, AppError>
where
    R: Runtime,
    B: Fn(&BackupTarget<'_>) -> Result<BackupOutcome, BackupError> + Send + 'static,
{
    run_with_abort_clock(app, phase1, data_dir, backup, now, Timestamp::now).await
}

/// [`run`] with the abort's clock injected — the seam T-325 added.
///
/// One read transaction (project facts + dedup digests), the copy
/// loop on the blocking pool through the injected `backup`
/// operation, and one write transaction for the records — or, on any
/// copy error, the §9.4 phase 2 step 2 abort (delete this run's
/// copies, one transaction writing the `discarded` history, the
/// aborted run and the per-project reset, then the typed
/// `E_BACKUP_FAILED` envelope).
///
/// `data_dir` is the app's data directory (the §5.5 tree's root —
/// production passes the managed `DataRoot`'s path; the tests point
/// it at a temp root). `backup` is the copy operation — production
/// passes [`backup_target_file`] itself; the tests inject the spec
/// 15.1 failure points. `now` is the run's backup instant (D-061):
/// one [`Timestamp`] resolves into the `BackupTime` of every copy
/// name and the DB-002 `created_at` of every record.
///
/// `abort_clock` answers the ABORT's instant. It is consulted at most
/// once, on the abort path only — after the sweep, right before the
/// verdict transaction, the moment the row goes terminal (the phase-1
/// abort's and the command's `stop_run`'s posture) — and its answer,
/// in the same DB-002 spelling, is the aborted run's `finished_at`.
/// It is deliberately a SECOND instant of the run, not one derived
/// from `now`: the copy pass and the sweep are the run's time too, so
/// a `finished_at` equal to the backup instant strikes them out of
/// the run's duration — a slow backup that aborts read as a run of
/// zero duration (the 2026-11-23 project review's P3-2, T-325; the
/// same point D-132's round-1 P3-1 had declined, reversed).
/// [`FnOnce`], because one run aborts at most once. The read point is
/// pinned: the suite's clock answers one instant while a swept copy
/// is still on disk and a later one once it is gone.
///
/// # Errors
///
/// - The typed `E_BACKUP_FAILED` envelope (BKP-005's fields in
///   `details`) after the run row went `aborted` and the rows were
///   reset — §9.4 phase 2 step 2.
/// - The honest `NoDatabase` fallback in recovery mode, and the
///   classified store errors of the transactions (a failed
///   transaction leaves the run row `running` — the crash-recovery
///   rule's case; the copies such a failure orphaned are deleted
///   best-effort on the way out).
pub(crate) async fn run_with_abort_clock<R, B>(
    app: &AppHandle<R>,
    phase1: &Phase1Outcome,
    data_dir: &Path,
    backup: B,
    now: Timestamp,
    abort_clock: impl FnOnce() -> Timestamp,
) -> Result<Phase2Outcome, AppError>
where
    R: Runtime,
    B: Fn(&BackupTarget<'_>) -> Result<BackupOutcome, BackupError> + Send + 'static,
{
    let database = managed_db(app)?;

    // Step A — eligibility (§9.4 phase 2 step 1's two conditions),
    // from the verdicts phase 1 already produced.
    let eligible = phase1
        .projects
        .iter()
        .filter_map(|(project_id, verdict)| match verdict {
            PreflightVerdict::Ready(ready) if ready.existing_size.is_some() => {
                Some((project_id.clone(), ready))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        // Zero copies cannot fail (§9.4 has no step for it): the run
        // continues, and the outcome is the empty fact list.
        return Ok(Phase2Outcome {
            run_id: phase1.run_id.clone(),
            backups: Vec::new(),
        });
    }

    // The run's one backup instant (D-061): the copy names' stamp and
    // the records' `created_at` both derive from it.
    let created_at = db002_stamp(now);
    let backup_time = backup_time_of(now);

    // Step B — ONE read transaction: the project facts (the D-131
    // second-home ruling) and the dedup digests. The inputs are owned
    // (`Db::run`'s executor closure is `'static`): the id and the
    // resolved source path per eligible project.
    let prepared = eligible
        .iter()
        .map(|(project_id, ready)| (project_id.clone(), ready.resolved_path.clone()))
        .collect::<Vec<_>>();
    let facts = database
        .run(move |transaction| {
            let mut facts = Vec::with_capacity(prepared.len());
            for (project_id, resolved) in &prepared {
                let project = eunomia_store::projects::get(transaction, project_id)
                    .map_err(projects_store_to_app_error)?
                    // The sync_run_projects rows reference this id
                    // with ON DELETE RESTRICT — the phase-1 reasoning
                    // verbatim: a missing row is a broken invariant.
                    .ok_or_else(project_missing_refusal)?;
                let source_text = path_text(resolved)?;
                let last = last_checked(transaction, project_id, &source_text)
                    .map_err(backup_records_to_app_error)?;
                facts.push(EligibleBackup {
                    project_id: ProjectId::new(project_id.clone()),
                    project_name: project.name,
                    scheme: core_scheme(project.scheme),
                    base_name: core_base_name(project.base_name),
                    source: resolved.clone(),
                    source_text,
                    last_checked: last,
                });
            }
            Ok(facts)
        })
        .await?;

    // Step C — the copy loop on the blocking pool (SYS-001: three
    // streaming passes per file; nothing here may hold the lock).
    let pass = {
        let data_dir = data_dir.to_owned();
        // The run's backup stamp moves onto the pool with the loop:
        // every record it makes carries it. The abort path below reads
        // its OWN instant (T-325) — the backup stamp is not the run's
        // end.
        let pass_handle = tauri::async_runtime::spawn_blocking(move || {
            copy_pass(facts, &data_dir, backup_time, created_at, backup)
        });
        pass_handle.await.map_err(join_failure_to_app_error)?
    };

    match pass {
        CopyPass::Done(facts, records) => {
            // Step D — ONE write transaction: every CREATED copy's
            // record lands `ok` (the BKP-010 check passed inside the
            // fs operation; dedup decisions write no row — the module
            // docs' UNIQUE ruling). The orphan list is taken before
            // the move: a failed transaction must still be able to
            // clean the copies up.
            let orphan_copies = records
                .iter()
                .filter_map(|record| record.copy_path.clone())
                .collect::<Vec<_>>();
            let run_id = phase1.run_id.clone();
            let written = database
                .run(move |transaction| {
                    for record in &records {
                        insert_record(
                            transaction,
                            &record_input(record, &run_id, BackupRecordStatus::Ok),
                        )
                        .map_err(backup_records_to_app_error)?;
                    }
                    Ok(())
                })
                .await;
            if let Err(error) = written {
                // The transaction rolled back: no records exist, and
                // the copies must not linger unreferenced — the
                // best-effort D-091 sweep posture. The store error is
                // the honest answer; the run row stays `running`
                // (the crash-recovery rule's case).
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    for copy_path in &orphan_copies {
                        let _ = remove_copy(copy_path);
                    }
                })
                .await;
                return Err(error);
            }
            Ok(Phase2Outcome {
                run_id: phase1.run_id.clone(),
                backups: facts,
            })
        }
        CopyPass::Failed {
            error,
            failed,
            made,
        } => {
            // Step E — §9.4 phase 2 step 2. The physical deletions
            // first (a `discarded` record over a live file is the
            // lying mark D-122's order exists to avoid); a refused
            // deletion keeps that copy's record `ok` (a verified live
            // copy) and rides the run row's details. The sweep covers
            // both debris kinds: this run's verified copies, and the
            // FAILED copy's partial bytes the fs operation could not
            // remove itself (`cleanup: Some` — its docs name this
            // sweep as the retrier, D-091/BKP-005).
            let lingering = lingering_copy_path(&error).map(Path::to_owned);
            let deletions = {
                let mut copies = made
                    .iter()
                    .filter_map(|record| record.copy_path.clone())
                    .collect::<Vec<_>>();
                copies.extend(lingering);
                let delete_handle = tauri::async_runtime::spawn_blocking(move || {
                    copies
                        .into_iter()
                        .map(|copy_path| (copy_path.clone(), remove_copy(&copy_path).is_ok()))
                        .collect::<Vec<_>>()
                });
                delete_handle.await.map_err(join_failure_to_app_error)?
            };

            // ONE write transaction: the discarded history (and the
            // `ok` rows of copies the filesystem refused to delete),
            // the aborted run, and the per-project reset — one
            // verdict, atomically. A refused deletion is a class-only
            // count in the details, so an operator reading the row
            // understands the `ok` record inside an aborted run.
            let refused = deletions
                .iter()
                .filter(|(path, gone)| {
                    !gone
                        && made
                            .iter()
                            .any(|record| record.copy_path.as_deref() == Some(path.as_path()))
                })
                .count();
            let run_id = phase1.run_id.clone();
            let mut abort_details = format!("project {}: {error}", failed.project_id);
            if refused > 0 {
                abort_details.push_str(&format!(
                    "; {refused} copy(ies) of this run could not be deleted"
                ));
            }
            // The abort's own instant (T-325), read ONCE here: after
            // the sweep, right before the verdict transaction — the
            // moment the row goes terminal. Never the backup instant
            // `now`: the copy pass and the sweep are the run's time
            // too, so a sweep riding §9.5's ladder (up to 1.3 s per
            // copy) is inside the stamp by the same rule.
            let finished_at = db002_stamp(abort_clock());
            let aborted = database
                .run(move |transaction| {
                    for record in &made {
                        let deleted = record.copy_path.as_deref().is_some_and(|path| {
                            deletions
                                .iter()
                                .any(|(deleted_path, gone)| *gone && deleted_path.as_path() == path)
                        });
                        let status = if deleted {
                            BackupRecordStatus::Discarded
                        } else {
                            // The copy is still on disk and verified —
                            // `ok` is the only honest status; the next
                            // run's dedup may even claim it.
                            BackupRecordStatus::Ok
                        };
                        insert_record(transaction, &record_input(record, &run_id, status))
                            .map_err(backup_records_to_app_error)?;
                    }
                    abort_run(
                        transaction,
                        &run_id,
                        &finished_at,
                        ErrorCode::BackupFailed.as_str(),
                        &abort_details,
                    )
                    .map_err(sync_runs_to_app_error)?;
                    reset_projects_not_started(transaction, &run_id)
                        .map_err(sync_runs_to_app_error)?;
                    Ok(())
                })
                .await;
            // The files are already gone and the records never landed
            // on this path; a failed abort transaction leaves the run
            // row `running` — the honest crash-window posture the
            // module docs record.
            aborted?;

            Err(backup_failed_envelope(&failed, data_dir, &error))
        }
    }
}

/// The copy loop: one `backup` call per eligible project, in
/// `sort_order`, stopping at the first error and keeping what it
/// made — the input of whichever write transaction follows.
///
/// The dedup input is honest about the disk, not just the record
/// (BKP-005's "no forced write without a backup"): a pair whose
/// newest checked copy's FILE is gone (the user deleted the backups
/// tree, an antivirus quarantined a copy — D-122's recorded window
/// where an `ok` row outlives its file) does not dedup — the phase
/// drops the digest, the fs operation makes a fresh copy, and the
/// write always has a restore point behind it. The check runs BOTH
/// before the call (the digest is handed over only over an existing
/// file) and AFTER a dedup decision (the recorded copy can vanish
/// inside the source-hash window — the round-2 residual TOCTOU; a
/// vanished copy re-copies with no digest). `eunomia-fs`'s own docs
/// place exactly this question on the caller ("whether the existing
/// copy is still on disk is the store's record question", D-091).
fn copy_pass<B>(
    facts: Vec<EligibleBackup>,
    data_dir: &Path,
    time: BackupTime,
    created_at: String,
    backup: B,
) -> CopyPass
where
    B: Fn(&BackupTarget<'_>) -> Result<BackupOutcome, BackupError>,
{
    let mut made = Vec::new();
    let mut done = Vec::new();
    for fact in facts {
        // A recorded copy that no longer exists is not a dedup
        // target: hand the fs operation no digest and let it make a
        // fresh copy (the module docs' no-forced-write ruling).
        let dedup_input = fact
            .last_checked
            .as_ref()
            .filter(|checked| data_dir.join(&checked.backup_path).is_file());
        let outcome = backup(&BackupTarget {
            source: &fact.source,
            data_dir,
            project_id: &fact.project_id,
            project_name: &fact.project_name,
            scheme: fact.scheme,
            base_name: fact.base_name,
            time,
            last_backup_sha256_hex: dedup_input.map(|checked| checked.sha256_hex.as_str()),
        });
        match outcome {
            Ok(BackupOutcome::Created(copy)) => {
                made.push(NewRecordInput {
                    project_id: fact.project_id.to_string(),
                    source_text: fact.source_text.clone(),
                    backup_path: copy.relative_path.clone(),
                    created_at: created_at.clone(),
                    size_bytes: size_bytes(copy.size),
                    sha256_hex: copy.sha256_hex.clone(),
                    copy_path: Some(copy.path.clone()),
                });
                done.push((fact.project_id.to_string(), BackupFact::Created(copy)));
            }
            Ok(BackupOutcome::Deduplicated { size, sha256_hex }) => {
                // The dedup decision is only as good as the copy it
                // points at: re-check the file AFTER the decision —
                // the recorded copy can vanish between the pre-call
                // gate and the digest comparison (the round-2
                // residual TOCTOU: a quarantine or cleanup inside
                // the source-hash window), and a dedup over a
                // missing copy would send phase 3 into its write
                // with no restore point. Vanished → drop the digest
                // and copy for real.
                let checked =
                    dedup_input.expect("a dedup outcome implies the digest the lookup supplied");
                if data_dir.join(&checked.backup_path).is_file() {
                    // The dedup fact for the outcome: the existing
                    // copy's path (the lookup named it — the outcome
                    // is the run's report of WHERE the pair's backup
                    // lives). No `NewRecordInput`: the schema's
                    // UNIQUE `backup_path` keeps one row per copy,
                    // and the existing row already documents this
                    // one (the module docs' ruling).
                    done.push((
                        fact.project_id.to_string(),
                        BackupFact::Deduplicated {
                            size,
                            sha256_hex,
                            backup_path: checked.backup_path.clone(),
                        },
                    ));
                } else {
                    match rebackup_without_digest(&backup, &fact, data_dir, time) {
                        Ok(copy) => {
                            made.push(NewRecordInput {
                                project_id: fact.project_id.to_string(),
                                source_text: fact.source_text.clone(),
                                backup_path: copy.relative_path.clone(),
                                created_at: created_at.clone(),
                                size_bytes: size_bytes(copy.size),
                                sha256_hex: copy.sha256_hex.clone(),
                                copy_path: Some(copy.path.clone()),
                            });
                            done.push((fact.project_id.to_string(), BackupFact::Created(copy)));
                        }
                        Err(error) => {
                            return CopyPass::Failed {
                                error,
                                failed: Box::new(fact),
                                made,
                            };
                        }
                    }
                }
            }
            Err(error) => {
                return CopyPass::Failed {
                    error,
                    failed: Box::new(fact),
                    made,
                };
            }
        }
    }
    CopyPass::Done(done, made)
}

/// The vanished-dedup fallback: one more `backup` call with NO
/// digest, which can only produce a fresh copy — a `Deduplicated`
/// answer without a supplied digest names the injected operation's
/// contract (production's `backup_target_file` dedups only against
/// the digest it was handed), the P3-5 posture again.
fn rebackup_without_digest<B>(
    backup: &B,
    fact: &EligibleBackup,
    data_dir: &Path,
    time: BackupTime,
) -> Result<BackupCopy, BackupError>
where
    B: Fn(&BackupTarget<'_>) -> Result<BackupOutcome, BackupError>,
{
    match backup(&BackupTarget {
        source: &fact.source,
        data_dir,
        project_id: &fact.project_id,
        project_name: &fact.project_name,
        scheme: fact.scheme,
        base_name: fact.base_name,
        time,
        last_backup_sha256_hex: None,
    })? {
        BackupOutcome::Created(copy) => Ok(copy),
        BackupOutcome::Deduplicated { .. } => {
            unreachable!("no digest supplied: the operation has nothing to dedup against")
        }
    }
}

/// Shapes one store record from a pass fact and the status the
/// calling path decided.
fn record_input<'a>(
    record: &'a NewRecordInput,
    run_id: &'a str,
    status: BackupRecordStatus,
) -> NewBackupRecord<'a> {
    NewBackupRecord {
        project_id: &record.project_id,
        sync_run_id: Some(run_id),
        source_path: &record.source_text,
        backup_path: &record.backup_path,
        size_bytes: record.size_bytes,
        sha256_hex: &record.sha256_hex,
        status,
        created_at: &record.created_at,
    }
}

/// Removes one copy file, `NotFound` counting as converged (D-091's
/// `remove_copy` ruling) and everything else retried through §9.5's
/// transient-hold ladder (100/300/900 ms on a sharing violation or
/// access denied — the same backoff `storage::retry_transient_hold`
/// gives the retention prunes; a freshly written copy is exactly
/// what an antivirus scanner holds, and `eunomia-store`'s helper is
/// `pub(crate)`, so the ladder is mirrored here with the same gate).
fn remove_copy(copy_path: &Path) -> std::io::Result<()> {
    // The closure, not the bare fn item: `std::fs::remove_file` is
    // generic over the argument's lifetime and a bare fn item does
    // not implement the higher-ranked `Fn(&Path)` bound.
    remove_copy_with(|path| std::fs::remove_file(path), copy_path)
}

/// The ladder over an injectable single-attempt removal — the seam
/// the unit tests drive (a closure counting calls and answering
/// transient holds), so the retry semantics are pinned without
/// staging real antivirus interference.
fn remove_copy_with(
    remove: impl Fn(&Path) -> std::io::Result<()>,
    copy_path: &Path,
) -> std::io::Result<()> {
    const PAUSES_MS: [u64; 3] = [100, 300, 900];
    let mut attempt = 0;
    loop {
        match remove(copy_path) {
            Ok(()) => return Ok(()),
            // Raced away: converged, not a failure (D-091).
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source)
                if matches!(source.raw_os_error(), Some(32) | Some(5))
                    && attempt < PAUSES_MS.len() =>
            {
                std::thread::sleep(std::time::Duration::from_millis(PAUSES_MS[attempt]));
                attempt += 1;
            }
            Err(source) => return Err(source),
        }
    }
}

/// The partial bytes a FAILED copy left behind and could not remove
/// itself: the fs operation's `cleanup: Some` handshake (D-091 — its
/// docs name the run's discarded-copies sweep, this module, as the
/// retrier). `None` when the failed operation cleaned up after
/// itself (or never created anything).
fn lingering_copy_path(error: &BackupError) -> Option<&Path> {
    match error {
        BackupError::CopyFailed {
            path,
            cleanup: Some(_),
            ..
        } => Some(path),
        BackupError::VerifyFailed(failed) if failed.cleanup.is_some() => Some(&failed.path),
        _ => None,
    }
}

/// A file size as the store's INTEGER column binds it: `i64`. A real
/// file's length always fits (no filesystem sizes a volume beyond
/// 2^63 bytes); the arm is the reasoned-invariant posture the store
/// repositories keep, not a clamp that would write a lying size.
fn size_bytes(size: u64) -> i64 {
    i64::try_from(size).expect("a filesystem length cannot exceed i64::MAX")
}

/// The DB-002 UTC millisecond stamp of a run instant — the exact
/// `values::now_utc` spelling (`%Y-%m-%dT%H:%M:%S%.3fZ`, truncating),
/// mirrored here because the store helper reads the real clock and
/// this module's contract is the injected instant (D-061). Pinned by
/// the phase tests against a fixed timestamp.
fn db002_stamp(now: Timestamp) -> String {
    now.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// The BKP-001 name stamp of a run instant: the UTC civil fields the
/// core builders format (`<UTC YYYYMMDDTHHMMSSmmmZ>`). The casts are
/// the D-061 posture — the fields arrive from a time library's civil
/// accessors, which never produce out-of-range values.
fn backup_time_of(now: Timestamp) -> BackupTime {
    let utc = now.to_zoned_datetime(jiff::tz::TimeZone::UTC);
    BackupTime {
        year: utc.year(),
        month: utc.month() as u8,
        day: utc.day() as u8,
        hour: utc.hour() as u8,
        minute: utc.minute() as u8,
        second: utc.second() as u8,
        millisecond: utc.millisecond() as u16,
    }
}

/// The resolved path's text (D-122's one stable spelling), or the
/// class-only refusal for a path that is not UTF-8 — the phase-0
/// `target_path_text` family: the stored root is canonical and the
/// resolution adds static ASCII names, so a miss names broken bytes
/// the record must never guess around.
fn path_text(path: &Path) -> Result<String, AppError> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        let mut error = AppError::new(ErrorCode::DbLocked);
        error.details = Some(ErrorDetails {
            os_code: None,
            os_kind: Some("ResolvedPathNotUtf8".to_owned()),
            os_message: Some(
                "a resolved target path is not valid UTF-8 — the stored \
                 project path and the file name are UTF-8 by construction"
                    .to_owned(),
            ),
        });
        error
    })
}

/// The store's scheme onto core's (the backup name's enum input):
/// two spellings of the same three members, mapped arm by arm so a
/// fourth member on either side is a compile error, not a silent
/// default.
fn core_scheme(scheme: eunomia_store::settings::Scheme) -> Scheme {
    match scheme {
        eunomia_store::settings::Scheme::A => Scheme::A,
        eunomia_store::settings::Scheme::B => Scheme::B,
        eunomia_store::settings::Scheme::C => Scheme::C,
    }
}

/// The store's base name onto core's, the same arm-by-arm mapping.
fn core_base_name(base_name: eunomia_store::settings::BaseName) -> BaseName {
    match base_name {
        eunomia_store::settings::BaseName::AgentsMd => BaseName::AgentsMd,
        eunomia_store::settings::BaseName::ClaudeMd => BaseName::ClaudeMd,
    }
}

/// The class-only refusal for a project row phase 1 recorded that the
/// projects table no longer backs (the phase-1 refusal family, each
/// phase naming its own refusal).
fn project_missing_refusal() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("Phase2ProjectMissing".to_owned()),
        os_message: Some(
            "a project recorded by the run is missing from the projects table".to_owned(),
        ),
    });
    error
}

/// The typed `E_BACKUP_FAILED` envelope with BKP-005's field list,
/// split across the two surfaces the frozen error UI renders: the
/// user-visible half is `params` (`project`, `path` — UI-011's
/// "Where" row interpolates exactly these), the diagnostic half is
/// `details` (the backups folder, OS code and error type, the cause,
/// §10's recommendation — one composed line the collapsed Details
/// section shows). The project NAME rides the envelope (not the run
/// row) — the spec explicitly demands it in THIS message, the one
/// ruling that outweighs the class-only default (SEC-008's own
/// scope: diagnostics, not wire answers).
fn backup_failed_envelope(
    failed: &EligibleBackup,
    data_dir: &Path,
    error: &BackupError,
) -> AppError {
    // The backups folder: the project's own §5.5 folder, spelled by
    // the pure builder (the path exists whether or not the copy ever
    // created it); the root is the total-function fallback for the
    // unreachable name refusal.
    let folder = backup_dir_name(&failed.project_id, &failed.project_name)
        .map(|dir| data_dir.join(BACKUPS_DIR).join(dir))
        .unwrap_or_else(|_| data_dir.join(BACKUPS_DIR));
    let message = format!(
        "project {}: backing up {} into {} failed: {error}. \
         Fix the cause from the error text and run the sync again.",
        failed.project_name,
        failed.source.display(),
        folder.display(),
    );
    let mut envelope = AppError::new(ErrorCode::BackupFailed);
    envelope
        .params
        .insert("project".to_owned(), failed.project_name.clone());
    envelope
        .params
        .insert("path".to_owned(), failed.source.display().to_string());
    envelope.details = Some(ErrorDetails {
        os_code: error.os_code(),
        os_kind: Some(error_kind(error).to_owned()),
        os_message: Some(message),
    });
    envelope
}

/// The error type behind a backup failure — BKP-005's "OS code or
/// error type" half, from the typed variant (the OS code rides
/// `details.os_code`).
fn error_kind(error: &BackupError) -> &'static str {
    match error {
        BackupError::NameFailed { .. } => "NameFailed",
        BackupError::FolderCreateFailed { .. } => "FolderCreateFailed",
        BackupError::SourceFailed { .. } => "SourceFailed",
        BackupError::CopyFailed { .. } => "CopyFailed",
        BackupError::VerifyFailed { .. } => "VerifyFailed",
    }
}

#[cfg(test)]
mod tests {
    //! The pure halves: the run-instant stamps (pinned against
    //! literal oracles — the DB-002 spelling and the BKP-001 civil
    //! fields), the error-kind table, and the size conversion. The
    //! phase behavior (eligibility, records, the abort verdict, the
    //! envelope fields) lives in `sync_phase2_tests.rs`, which owns
    //! its mock app harness.

    use super::*;

    /// A fixed run instant — 2026-11-21T10:30:00.123Z.
    fn fixed_now() -> Timestamp {
        "2026-11-21T10:30:00.123Z"
            .parse()
            .expect("the fixed instant must parse")
    }

    /// The DB-002 stamp: the exact 24-character UTC millisecond
    /// spelling, truncated (not rounded) — the `values::now_utc`
    /// contract mirrored.
    #[test]
    fn the_db002_stamp_is_the_values_spelling() {
        assert_eq!(db002_stamp(fixed_now()), "2026-11-21T10:30:00.123Z");
        // Truncation, not rounding: a fourth fractional digit drops.
        let extra = "2026-11-21T10:30:00.1239Z"
            .parse::<Timestamp>()
            .expect("the four-digit instant must parse");
        assert_eq!(db002_stamp(extra), "2026-11-21T10:30:00.123Z");
    }

    /// The BKP-001 stamp: the UTC civil fields the core builders
    /// format, with the millisecond from the timestamp's subsecond.
    #[test]
    fn the_backup_time_is_the_utc_civil_fields() {
        assert_eq!(
            backup_time_of(fixed_now()),
            BackupTime {
                year: 2026,
                month: 11,
                day: 21,
                hour: 10,
                minute: 30,
                second: 0,
                millisecond: 123,
            }
        );
    }

    /// The transient-hold ladder over the injectable seam (§9.5):
    /// a transient hold (os 32/5) is retried through the pauses, a
    /// non-transient refusal is answered after one attempt, and a
    /// raced-away copy (NotFound) is converged success.
    #[test]
    fn the_removal_ladder_retries_transient_holds_only() {
        use std::cell::Cell;
        let path = Path::new("/data/backups/p/x.bak");
        // Two transient holds, then success: three attempts.
        let transient_calls = Cell::new(0);
        let outcome = remove_copy_with(
            |_| {
                if transient_calls.get() < 2 {
                    transient_calls.set(transient_calls.get() + 1);
                    Err(std::io::Error::from_raw_os_error(32))
                } else {
                    Ok(())
                }
            },
            path,
        );
        assert!(outcome.is_ok());
        assert_eq!(
            transient_calls.get() + 1,
            3,
            "two holds answered, third attempt succeeds"
        );

        // A non-transient refusal: one attempt, the error passes
        // through untouched (its code survives).
        let refusal = remove_copy_with(|_| Err(std::io::Error::from_raw_os_error(28)), path)
            .expect_err("a permanent refusal must surface");
        assert_eq!(refusal.raw_os_error(), Some(28));

        // Raced away: converged (D-091), not a failure.
        assert!(
            remove_copy_with(
                |_| Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "raced away"
                )),
                path,
            )
            .is_ok()
        );
    }

    /// The transient-hold ladder's bounded tail: holds past the
    /// third pause surface the refusal instead of retrying forever.
    #[test]
    fn the_removal_ladder_is_bounded() {
        let path = Path::new("/data/backups/p/x.bak");
        let refusal = remove_copy_with(|_| Err(std::io::Error::from_raw_os_error(5)), path)
            .expect_err("an eternal hold must surface after the ladder");
        assert_eq!(refusal.raw_os_error(), Some(5));
    }

    /// Every typed backup failure carries its error kind — BKP-005's
    /// "error type" half, mapped arm by arm.
    #[test]
    fn every_backup_error_names_its_kind() {
        let io = || std::io::Error::from_raw_os_error(28);
        let verify = BackupError::VerifyFailed(eunomia_fs::backup::VerifyFailed {
            path: PathBuf::from("/data/backups/p/x.bak"),
            expected_size: 3,
            expected_sha256_hex: "0".repeat(64),
            mismatch: eunomia_fs::backup::VerifyMismatch::Size { actual_size: 2 },
            cleanup: None,
        });
        for (error, kind) in [
            (
                BackupError::NameFailed {
                    source: eunomia_core::backup_name::BackupNameError::UnsafeProjectId {
                        value: "../etc".to_owned(),
                    },
                },
                "NameFailed",
            ),
            (
                BackupError::FolderCreateFailed {
                    path: PathBuf::from("/data/backups"),
                    source: io(),
                },
                "FolderCreateFailed",
            ),
            (
                BackupError::SourceFailed {
                    path: PathBuf::from("/r/AGENTS.md"),
                    source: io(),
                },
                "SourceFailed",
            ),
            (
                BackupError::CopyFailed {
                    path: PathBuf::from("/data/backups/p/x.bak"),
                    source: io(),
                    cleanup: None,
                },
                "CopyFailed",
            ),
            (verify, "VerifyFailed"),
        ] {
            assert_eq!(error_kind(&error), kind, "error={error:?}");
        }
    }
}
