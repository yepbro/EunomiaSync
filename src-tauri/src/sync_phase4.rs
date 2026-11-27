//! The sync pipeline's phase 4 — the finish (spec §9.4 phase 4, task
//! T-117): the five steps that turn a walked run into a finished
//! one — the `unchanged` stamps (step 1), the BKP-003 backup
//! retention (step 2), the counted `success`/`partial`/`failed`
//! verdict with `finished_at` (step 3, MAIN-027's derived update),
//! the run journal (§7.7 LOG-001…004), and the after-commit
//! `state://dirty` recalculation (step 5's send half).
//!
//! **One transaction, then one seam.** Everything step 1–3 and the
//! journal claim rides ONE [`Db::run`] write transaction (the
//! D-037/D-051 joined shape): the rows are read (the run's own
//! `sync_run_projects` — the committed truth D-130 froze the build
//! hash into, which is why this phase needs [`Phase3Outcome`] only
//! for the run id), every `unchanged` row's project is stamped
//! through [`projects::record_last_synced`] with the run's one
//! DB-002 instant and the row's own fingerprint (the D-133 ruling:
//! `last_synced_hash` is the markerless DB-020 value the next
//! comparison recomputes against — the same FORM phase 3 stamped the
//! `written` projects with; the VALUES differ by design, the written
//! carrying the write instant and the unchanged the finish instant,
//! each its own §9.4 step), the retention sweep
//! runs over the caller's `data_dir`, [`sync_runs::finish_run`]
//! counts and writes the verdict, and the journal entries insert.
//! The commit makes the finished run and everything its summary
//! claims one set: a run row that says `success` without its stamps,
//! its journal or its pruned history cannot exist — with one
//! documented residue: the retention sweep's FILE deletions are
//! filesystem effects a rollback cannot undo, so a finish that
//! refuses after the sweep leaves the already-removed copies removed
//! (their records still `ok`, converged by the next run — D-122's
//! own crash-window posture, spelled for this composition). AFTER the
//! commit — and only after — the injected `dirty` seam runs (§9.4
//! phase 4 step 5's "send `state://dirty`"): production closes it
//! over [`dirty_recalc::recalc_and_emit`] (D-121's after-sync
//! recalculation point, the same pass the commit seam schedules),
//! which must observe the committed stamps, not the in-flight ones.
//!
//! One bound worth naming (the round-1 review's note): the retention
//! sweep's filesystem work — `remove_file` with the §9.5
//! transient-hold retries — runs INSIDE this transaction, so it
//! holds the app's one connection (SYS-002) for its duration; a
//! handful of held copies can queue the UI's read commands behind it
//! for the retries' worst case. The one-transaction ruling (the
//! finished run and its pruned history commit as one set) is worth
//! that bounded hold, and D-122's convergence keeps a
//! partially-removed set honest if the process dies inside it.
//!
//! **The flag is not this phase's.** §9.4 phase 4 step 5's "Reset
//! `sync_running`" is the RAII drop of the [`SyncGuard`] living in
//! the caller's [`Phase0Outcome`](crate::sync_phase0::Phase0Outcome)
//! (D-123): phases 1–4 hold it BY holding that outcome, and whatever
//! path the pipeline takes out of phase 4, the guard's owner — the
//! `sync_run` command (T-118) — releases the flag by dropping it.
//! Nothing here takes, checks or drops the guard; the contract is
//! documented, not re-implemented.
//!
//! **Retention and journal errors never fail the run** (BKP-003,
//! §7.7): the retention sweep is housekeeping "after the run
//! finishes", not a step of the run's verdict — its filesystem
//! failures are reported-not-raised by the service itself (D-122),
//! and this phase extends the same posture to the service's own
//! refusals (a failed mark, an unreadable `backup.retention` cell):
//! the reason lands in
//! [`RetentionReport::failures`] for the caller's log, the verdict
//! commits anyway, and the next run converges (D-122's crash-window
//! argument — files already removed stay removed, records still `ok`
//! are retried). The journal is the same sentence over §7.7: a
//! journal insert that refuses costs the Activity window its entry,
//! not the sync its verdict — the refusal lands in
//! [`JournalReport::failures`]. Both reports are the outcome's facts
//! for T-118's log lines, never `Err` — with ONE carve-out each
//! swallow point carries (the round-1 P2-1 containment): a FATAL
//! SQLite class (I/O, full disk, out of memory, interrupt) performs
//! an implicit rollback, and continuing past it would let the
//! verdict and journal commit individually in autocommit behind a
//! failed `COMMIT` — so every swallow point probes the transaction's
//! liveness ([`transaction_alive`]) and refuses honestly while the
//! rollback still covers everything this phase wrote. What ELSE is
//! `Err`: the store failures of the rows read, a stamp, or the
//! finish, and the broken-chain invariants below — every one leaves
//! the run row `running` (the crash-recovery rule's case, the
//! D-131/D-133 posture).
//!
//! **The verdict is counted, not remembered.** [`finish_run`] reads
//! the per-project rows back and derives §9.4 step 3's rule from
//! what committed — this phase adds no verdict of its own and could
//! not disagree with the counters without refusing first. MAIN-027
//! (§9.4 step 4, LOG-007) is DERIVED by the same honesty: no column
//! or table stores "the overall time of the last successful sync" —
//! it IS the `finished_at` of the last `success` run, so the one
//! UPDATE that writes verdict and stamp together is the whole update
//! mechanism, `partial` and `failed` move nothing, and
//! [`sync_runs::last_success_finished_at`] is the single read. The
//! empty run ("no active projects") is `success` with the §9.4
//! message in the row's `details` cell — the one free-text home the
//! schema offers, spelled by the store.
//!
//! **The journal dictionary** (D-051, widened by T-124/D-223):
//! LOG-001…004's "sync runs and their results per project" is THREE
//! members — phase 0 opens the run's bracket with one
//! `sync.run_started` entry (info/user/ok, in its own record
//! transaction), one `sync.run_finished` summary (entity `sync_run`,
//! result by verdict) and one `sync.project_result` per project (the
//! entry's own `project_id`/`sync_run_id` columns, result by the
//! row's status) close it. The level/result mapping is this module's
//! ruling over the closed vocabularies: `written`/`unchanged` →
//! info/ok (a written project carries its target path in
//! [`Details::File`](action_log::Details::File), LOG-001…004's "for
//! file writes, the project and path"); `skipped` → warn/skipped
//! with the §10 code in [`Details::Failure`] and — the T-124 reading
//! — the row's own frozen target path; `failed` → error/error with
//! the code and the refused path; the run summary follows its
//! verdict — success info/ok, partial warn/skipped (the nearest
//! member of the closed `result` vocabulary for "completed, not
//! fully good" — the per-project entries carry the precise
//! outcomes), failed error/error. §10's fourth field rides both
//! error shapes: the `os_code` behind a `skipped`/`failed` verdict
//! comes from the OUTCOME that produced it (phase 1's `Skipped`
//! details, phase 3's `Failed` details — the structured facts the
//! committed rows have no column for, handed in through [`run`]'s
//! `phase1`/`phase3` parameters), absent from the JSON when no
//! syscall backs the refusal. Every entry is
//! [`Visibility::User`] (§7.7's Activity window groups Sync), and
//! each successfully inserted entry is recorded into the
//! transaction's [`crate::logging::MirrorBuffer`] — the executor
//! flushes the mirror lines only after the verdict transaction
//! commits (T-324/D-228, D-051's call-once rule made
//! commit-aware).
//!
//! **Recovery mode** (no database) answers the honest `NoDatabase`
//! fallback through [`managed_db`], like every phase — nothing runs.

use std::future::Future;
use std::path::Path;

use jiff::Timestamp;
use tauri::{AppHandle, Runtime};

use eunomia_store::action_log;
use eunomia_store::action_log::{
    Action, Details, Entity, EntityType, FailureCode, Level, LogEntry, Outcome, Visibility,
};
use eunomia_store::backup_retention;
use eunomia_store::projects;
use eunomia_store::settings;
use eunomia_store::settings::{SettingKey, SettingValue};
use eunomia_store::sync_runs::{SyncRun, SyncRunProjectStatus, finish_run};

use crate::commands::{managed_db, projects_store_to_app_error};
use crate::db::sync_runs_to_app_error;
use crate::dirty_recalc::ProjectDirtyChange;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::sync_phase1::{Phase1Outcome, PreflightVerdict};
use crate::sync_phase3::{Phase3Outcome, WriteOutcome};

/// What phase 4 hands the pipeline: the finished run row and the
/// three never-fatal reports (retention, journal, dirty) T-118's log
/// lines read. The flag is deliberately absent — the guard stays
/// with the caller's `Phase0Outcome` (the module docs' contract).
#[derive(Debug)]
pub(crate) struct Phase4Outcome {
    /// The finished run row: the §9.4 verdict, `finished_at`, the
    /// counted per-status counters, and the empty-run message in
    /// `details` when the run had no active projects. No production
    /// reader: the sync summary and the `sync_last_result` command
    /// (T-121) read the COMMITTED row back (`commands::summary_of`,
    /// D-217's ruling), so this copy rides the outcome for the tests
    /// alone.
    #[allow(dead_code)] // both production readers read the committed row; tested today
    pub(crate) run: SyncRun,
    /// The BKP-003 retention sweep's report (never fatal — the
    /// module docs' ruling).
    pub(crate) retention: RetentionReport,
    /// The run journal's report: how many entries landed, and every
    /// refusal as a class-only line (never fatal).
    pub(crate) journal: JournalReport,
    /// The after-commit `state://dirty` seam's outcome: the diff the
    /// pass emitted, or the class-only reason it refused (at-least-
    /// once, D-121 — the next trigger re-runs it).
    pub(crate) dirty: DirtyReport,
}

/// The retention sweep's never-fatal report (BKP-003/§7.7; D-122's
/// report-not-raise posture, extended to the service's own
/// refusals).
#[derive(Debug, Default)]
pub(crate) struct RetentionReport {
    /// Records marked `pruned` by this run (their files removed now
    /// or already gone — converged).
    pub(crate) pruned: u64,
    /// Why the sweep did not fully apply: per-record filesystem
    /// refusals (the service's own reports, Display strings) and the
    /// class-only lines for a refused settings read or a refused
    /// mark. The next run retries everything still `ok`.
    pub(crate) failures: Vec<String>,
}

/// The run journal's never-fatal report (§7.7).
#[derive(Debug, Default)]
pub(crate) struct JournalReport {
    /// How many entries (per-project plus the run summary) landed.
    pub(crate) written: u32,
    /// A class-only line per refused insert: the Activity window
    /// misses the entry, the run keeps its verdict.
    pub(crate) failures: Vec<String>,
}

/// The after-commit dirty seam's outcome (§9.4 phase 4 step 5's send
/// half).
#[derive(Debug, Default)]
pub(crate) struct DirtyReport {
    /// The changes the pass emitted (empty — nothing to tell the UI;
    /// a pass that emitted nothing is not a failure). The command
    /// logs only the refusal below; the changes themselves rode the
    /// `state://dirty` event and have no further reader — future
    /// cards (diagnostics, the unsynced dialog's after-sync refresh)
    /// are the consumers.
    #[allow(dead_code)] // the event carried the payload; consumers are future cards
    pub(crate) changes: Vec<ProjectDirtyChange>,
    /// The class-only reason the seam refused, when it did: the
    /// at-least-once contract (D-121) leaves the recalculation to
    /// the next trigger.
    pub(crate) failure: Option<String>,
}

/// Runs §9.4 phase 4 end to end over phase 1's and phase 3's
/// outcomes: ONE write transaction (unchanged stamps, retention, the
/// counted verdict, the journal), then the `state://dirty` seam after
/// the commit.
///
/// `phase1` is the run's preflight outcome (T-124/D-223): the command
/// holds it through phase 3's call, and this phase reads exactly one
/// fact from it — the `skipped` verdicts' structured OS facts, the
/// one §10 field the committed rows cannot store (the store row has
/// no `os_code` column; D-131's round-1 P3-5 added the details to the
/// outcomes for exactly this consumption class). `phase3` is the
/// write phase's outcome, supplying the `failed` outcomes' OS facts
/// the same way. The committed row stays the authority for the code,
/// the path and the project (D-134's read-the-rows posture).
///
/// `data_dir` is the data-directory root the retention sweep joins
/// its §5.5 layout paths under (the phase-2 shape; production passes
/// the managed `DataRoot`'s path). `now` is the run's finish instant
/// (D-061): the ONE [`Timestamp`] the row's `finished_at` and every
/// `unchanged` project's `last_synced_at` derive from, in the DB-002
/// spelling. `dirty` is the after-commit seam — production passes
/// [`production_dirty`]; tests inject a counting closure (the
/// phase-2/3 injector pattern over an async seam).
///
/// # Errors
///
/// - The honest `NoDatabase` fallback in recovery mode, and the
///   classified store errors of the rows read, a stamp, or the
///   finish — every one leaves the run row `running` (the
///   crash-recovery rule's case).
/// - The broken-chain invariant refusals (class-only): a
///   `not_started` row at finish time (`Phase4RowUnsettled`), an
///   `unchanged` row whose fingerprint is missing
///   (`Phase4FingerprintMissing`, the phase-3 family), a run row no
///   longer `running` (`Phase4RunNotRunning`), and a finish
///   transaction a fatal SQLite class killed mid-phase
///   (`Phase4TransactionLost`).
/// - NEVER a statement-class retention or journal refusal, and never
///   the dirty seam's: those are the outcome's reports (the module
///   docs' ruling, with the fatal-class carve-out above).
pub(crate) async fn run<R, D, F>(
    app: &AppHandle<R>,
    phase1: &Phase1Outcome,
    phase3: &Phase3Outcome,
    data_dir: &Path,
    now: Timestamp,
    dirty: D,
) -> Result<Phase4Outcome, AppError>
where
    R: Runtime,
    D: FnOnce(&AppHandle<R>) -> F,
    F: Future<Output = Result<Vec<ProjectDirtyChange>, AppError>> + Send,
{
    let database = managed_db(app)?;
    let finished_at = db002_stamp(now);

    // §10's fourth field, harvested before the transaction (the
    // executor closure is `'static`): the OS code behind every
    // `skipped` (phase 1) and `failed` (phase 3) verdict, keyed by
    // project — `None` for refusals no syscall backs (a marker check,
    // a shape validation), and absent for a project neither phase
    // reported. The map is the outcomes' OWN facts, never a guess: a
    // row whose project has no entry journals no OS code.
    let mut os_codes = std::collections::HashMap::new();
    for (project_id, verdict) in &phase1.projects {
        if let PreflightVerdict::Skipped {
            details: Some(facts),
            ..
        } = verdict
        {
            os_codes.insert(project_id.clone(), facts.os_code);
        }
    }
    for (project_id, outcome) in &phase3.results {
        if let WriteOutcome::Failed {
            details: Some(facts),
            ..
        } = outcome
        {
            os_codes.insert(project_id.clone(), facts.os_code);
        }
    }

    // Steps 1–4 — one transaction. The owned inputs ride the
    // `'static` executor closure (the phase-0 `record_run` shape).
    let run_id = phase3.run_id.clone();
    let data_dir = data_dir.to_path_buf();
    let (run, retention, journal) = database
        .run_with_mirror(move |tx, mirror| {
            // Step A — the committed truth: the run's own rows, in
            // `sort_order`. Everything this phase writes derives
            // from them (the counted verdict reads the same rows
            // inside `finish_run`), so a broken chain refuses HERE,
            // before a stamp or a prune moves.
            let rows = eunomia_store::sync_runs::list_projects(tx, &run_id)
                .map_err(sync_runs_to_app_error)?;
            if rows
                .iter()
                .any(|row| row.status == SyncRunProjectStatus::NotStarted)
            {
                // Phases 1–3 own every transition; a `not_started`
                // row at finish time names a broken chain, and a
                // verdict over half a run is not a verdict (the
                // store's own refusal, spelled class-only here for
                // the honest diagnostic).
                return Err(unsettled_row_refusal());
            }
            // The same containment for the run row itself: phase 4
            // owns only the `running → finished` edge, and the
            // stamps/prunes below must not run over a run that is
            // already finished, aborted or interrupted — a second
            // finish of the same run (the caller's bug) would leave
            // the verdict committed once and the files pruned twice
            // before the store's own zero-row refusal rolled the
            // database half back (the round-1 P3-4 finding).
            let run_row = eunomia_store::sync_runs::get(tx, &run_id)
                .map_err(sync_runs_to_app_error)?
                .ok_or_else(run_not_running_refusal)?;
            if run_row.status != eunomia_store::sync_runs::SyncRunStatus::Running {
                return Err(run_not_running_refusal());
            }

            // Step B — §9.4 phase 4 step 1: the `unchanged`
            // projects' stamps. The run's one DB-002 instant and the
            // row's own DB-020 fingerprint (D-130: read the frozen
            // value; D-133: the markerless form `last_synced_hash`
            // stores) — the exact shape phase 3 stamped the written
            // projects with, so both verdicts of one run agree on
            // what "synced at" means.
            for row in &rows {
                if row.status == SyncRunProjectStatus::Unchanged {
                    let Some(build_hash) = row.build_hash.as_deref() else {
                        return Err(fingerprint_missing_refusal());
                    };
                    projects::record_last_synced(tx, &row.project_id, &finished_at, build_hash)
                        .map_err(projects_store_to_app_error)?;
                }
            }

            // Step C — §9.4 phase 4 step 2 (BKP-003): the retention
            // sweep, inside the same transaction so the finished run
            // and its pruned history commit as one set. The limit is
            // read in the sweep's own transaction (the D-054
            // precedent), and every refusal — the settings read, a
            // refused mark — is REPORTED, never raised (the module
            // docs' ruling): the files the sweep already removed stay
            // removed, the records still `ok` retry next run.
            let mut retention = RetentionReport::default();
            let keep = settings::get(tx, SettingKey::BackupRetention)
                .ok()
                .and_then(|value| match value {
                    SettingValue::Int(keep) => Some(keep),
                    // `get` validates the cell against the key's own
                    // contract, so another variant is a broken
                    // invariant of that chain — class-only, never the
                    // stored value (SEC-008).
                    _ => None,
                });
            match keep {
                Some(keep) => match backup_retention::apply(tx, &data_dir, keep) {
                    Ok(outcome) => {
                        retention.pruned = outcome.pruned;
                        retention
                            .failures
                            .extend(outcome.failures.iter().map(|failure| failure.to_string()));
                    }
                    Err(error) => retention.failures.push(format!(
                        "the retention sweep refused and was skipped: {error}"
                    )),
                },
                None => retention.failures.push(
                    "the backup.retention setting could not be read; the limit was not \
                     applied this run"
                        .to_owned(),
                ),
            }
            // The round-1 P2-1 containment: a swallowed store error
            // of SQLite's FATAL classes (I/O, full disk, out of
            // memory, interrupt) performs an implicit ROLLBACK — the
            // connection silently returns to autocommit, and every
            // later statement of this closure would then commit
            // INDIVIDUALLY (the verdict, the journal) before the
            // executor's final COMMIT fails over a dead transaction.
            // One liveness probe after the swallow point turns that
            // into the honest `Err` — the run row stays `running`,
            // the crash-recovery rule's case — instead of a
            // half-finished finish. Statement-class failures (the
            // trigger ABORTs, constraint refusals) leave the
            // transaction alive and keep the swallow posture.
            transaction_alive(tx)?;

            // Step D — §9.4 phase 4 step 3 (+ step 4's MAIN-027, the
            // same UPDATE): the counted verdict, `finished_at`, the
            // counters, and the empty-run message.
            let run = finish_run(tx, &run_id, &finished_at).map_err(sync_runs_to_app_error)?;

            // The run journal (LOG-001…004, D-051): one
            // `sync.project_result` per project in `sort_order`,
            // then the `sync.run_finished` summary. Every insert
            // joins this transaction (D-051's in-transaction rule —
            // a rolled-back verdict takes its journal with it), and
            // a refused insert is REPORTED, never raised (§7.7).
            let mut journal = JournalReport::default();
            for row in &rows {
                let (level, result) = project_entry_shape(row.status);
                let mut entry =
                    LogEntry::new(Action::SyncProjectResult, level, Visibility::User, result);
                entry.entity = Some(Entity {
                    kind: EntityType::Project,
                    id: row.project_id.as_str(),
                });
                entry.project_id = Some(row.project_id.as_str());
                entry.sync_run_id = Some(run_id.as_str());
                entry.details = project_entry_details(
                    row,
                    os_codes.get(row.project_id.as_str()).copied().flatten(),
                );
                match action_log::insert(tx, &entry) {
                    Ok(_) => {
                        journal.written += 1;
                        // The T-324/D-228 mirror records inside the
                        // closure; the executor flushes it only after
                        // the verdict transaction commits (the facade
                        // decides whether the line lands — a
                        // rolled-back verdict takes its mirror with
                        // it, D-051's rule on the file-log half).
                        mirror.record(&entry);
                    }
                    Err(error) => {
                        journal.failures.push(format!(
                            "the project result of a run row was not journaled: {error}"
                        ));
                        // The P2-1 liveness probe, spelled for this
                        // swallow point: a fatal-class insert failure
                        // rolled the transaction back — refuse now,
                        // while everything this phase wrote is still
                        // rolled back WITH it.
                        transaction_alive(tx)?;
                    }
                }
            }
            let (level, result) = run_entry_shape(run.status);
            let mut entry = LogEntry::new(Action::SyncRunFinished, level, Visibility::User, result);
            entry.entity = Some(Entity {
                kind: EntityType::SyncRun,
                id: run_id.as_str(),
            });
            entry.sync_run_id = Some(run_id.as_str());
            match action_log::insert(tx, &entry) {
                Ok(_) => {
                    journal.written += 1;
                    // The T-324/D-228 mirror records inside the
                    // closure; the flush rides the commit (the
                    // facade decides whether the line lands).
                    mirror.record(&entry);
                }
                Err(error) => {
                    journal
                        .failures
                        .push(format!("the run summary was not journaled: {error}"));
                    transaction_alive(tx)?;
                }
            }

            Ok((run, retention, journal))
        })
        .await?;

    // Step E — §9.4 phase 4 step 5's send half, strictly AFTER the
    // commit: the recalculation must read the stamps that landed.
    // A refused pass is reported, never raised — D-121's
    // at-least-once contract leaves it to the next trigger (the
    // commit seam already fired one for this transaction).
    let dirty = match dirty(app).await {
        Ok(changes) => DirtyReport {
            changes,
            failure: None,
        },
        Err(error) => DirtyReport {
            changes: Vec::new(),
            // Class-only (SEC-008): the §10 code literal, no
            // interpolated params.
            failure: Some(format!(
                "the after-sync recalculation refused: {}",
                error.code.as_str()
            )),
        },
    };

    Ok(Phase4Outcome {
        run,
        retention,
        journal,
        dirty,
    })
}

/// The production `state://dirty` seam (the `dirty` parameter's
/// caller-side half): D-121's after-sync recalculation point — the
/// same pass the commit seam schedules, awaited directly so §9.4
/// phase 4 step 5's "send `state://dirty`" and the 150 ms-debounced
/// trigger share every line of the computation. `pub(crate)`: the
/// `sync_run` command (T-118) passes it.
pub(crate) async fn production_dirty<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ProjectDirtyChange>, AppError> {
    crate::dirty_recalc::recalc_and_emit(app).await
}

/// The journal shape of one project's entry (the module docs'
/// ruling): `written`/`unchanged` are the routine good row,
/// `skipped` is the deliberate no-op at warn, `failed` is the §10
/// failure at error (D-051's documented error-level pairing).
fn project_entry_shape(status: SyncRunProjectStatus) -> (Level, Outcome) {
    match status {
        SyncRunProjectStatus::Written | SyncRunProjectStatus::Unchanged => {
            (Level::Info, Outcome::Ok)
        }
        SyncRunProjectStatus::Skipped => (Level::Warn, Outcome::Skipped),
        SyncRunProjectStatus::Failed => (Level::Error, Outcome::Error),
        // Phase 4 refuses the run before the journal when a row is
        // unsettled, so the member has no entry of its own — the
        // match stays exhaustive for the vocabulary, honest if a
        // future caller reaches it.
        SyncRunProjectStatus::NotStarted => (Level::Warn, Outcome::Skipped),
    }
}

/// The journal details of one project's entry (SEC-008's closed
/// shapes only): a `written` project is LOG-001…004's "file write"
/// — the target path rides [`Details::File`]; a `skipped`/`failed`
/// row carries its §10 code in [`Details::Failure`] (the refused
/// path along a failure; a skip touched no file). An `unchanged`
/// verdict needs no payload, and a code cell that fails the §10
/// shape check (only tampering produces one) keeps its entry at the
/// cost of the details, never the entry.
///
/// `os_code` is §10's fourth field (T-124/D-223), supplied by the
/// caller from the verdict's own outcome — phase 1's `Skipped`
/// details or phase 3's `Failed` details — because the committed row
/// has no column for it. It rides whatever row it was given, absent
/// from the JSON when `None`, so pre-field rows stay one shape. The
/// `skipped` path is this task's reading: phase 0 froze the target
/// path for EVERY project, and it is the honest "path" of an error
/// about that project's target (D-134's `path: None` was the pre-§10
/// reading).
fn project_entry_details(
    row: &eunomia_store::sync_runs::SyncRunProject,
    os_code: Option<i32>,
) -> Option<Details> {
    match row.status {
        SyncRunProjectStatus::Written => row.target_path.as_deref().map(|path| Details::File {
            path: std::path::PathBuf::from(path),
        }),
        SyncRunProjectStatus::Unchanged => None,
        SyncRunProjectStatus::Skipped => row
            .error_code
            .as_deref()
            .and_then(|code| FailureCode::new(code).ok())
            .map(|code| Details::Failure {
                code,
                path: row.target_path.as_deref().map(std::path::PathBuf::from),
                os_code,
            }),
        SyncRunProjectStatus::Failed => {
            let code = row
                .error_code
                .as_deref()
                .and_then(|code| FailureCode::new(code).ok())?;
            let path = row.target_path.as_deref().map(std::path::PathBuf::from);
            Some(Details::Failure {
                code,
                path,
                os_code,
            })
        }
        SyncRunProjectStatus::NotStarted => None,
    }
}

/// The journal shape of the run summary entry: the level/result pair
/// of §9.4's verdict — success is the routine good run, partial is
/// the degraded one (warn, and the closed `result` vocabulary's
/// nearest member: the run completed deliberately short of full
/// success, its per-project entries carry the precise outcomes),
/// failed is the §10-adjacent failure at error.
fn run_entry_shape(status: eunomia_store::sync_runs::SyncRunStatus) -> (Level, Outcome) {
    use eunomia_store::sync_runs::SyncRunStatus;
    match status {
        SyncRunStatus::Success => (Level::Info, Outcome::Ok),
        SyncRunStatus::Partial => (Level::Warn, Outcome::Skipped),
        SyncRunStatus::Failed => (Level::Error, Outcome::Error),
        // `running`/`aborted`/`interrupted` runs never reach this
        // journal (their owners write no phase-4 summary); the arms
        // keep the match exhaustive over the vocabulary.
        SyncRunStatus::Running | SyncRunStatus::Aborted | SyncRunStatus::Interrupted => {
            (Level::Warn, Outcome::Skipped)
        }
    }
}

/// The DB-002 UTC millisecond stamp of the run's finish instant —
/// the exact `values::now_utc` spelling (`%Y-%m-%dT%H:%M:%S%.3fZ`,
/// truncating), mirrored here because the store helper reads the
/// real clock and this module's contract is the injected instant
/// (D-061; the phase-2/3 `db002_stamp` twins, pinned the same way).
fn db002_stamp(now: Timestamp) -> String {
    now.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// The liveness probe of every swallowed store error (the round-1
/// P2-1 containment): SQLite's FATAL error classes perform an
/// implicit ROLLBACK and return the connection to autocommit, so a
/// swallowed fatal error would let every later statement of the
/// finish transaction commit INDIVIDUALLY before the executor's
/// final `COMMIT` fails over a dead transaction — a run row finished
/// behind an `Err` the caller reads as a failure. [`Connection::
/// is_autocommit`] answers exactly "did that happen": the executor's
/// `BEGIN DEFERRED` turned autocommit off before the first
/// statement, so `true` here names a lost transaction, and the
/// honest `Err` — with everything this phase wrote rolled back with
/// it — beats a half-finished finish. Statement-class failures (a
/// trigger `ABORT`, a constraint refusal) leave the transaction
/// alive and keep the swallow posture.
///
/// [`Connection::is_autocommit`]: rusqlite::Connection::is_autocommit
fn transaction_alive(tx: &rusqlite::Connection) -> Result<(), AppError> {
    if tx.is_autocommit() {
        return Err(transaction_lost_refusal());
    }
    Ok(())
}

/// The class-only refusal for a finish transaction a fatal SQLite
/// error killed mid-phase (the implicit-rollback case the liveness
/// probe catches): the run row stays `running` with everything this
/// phase wrote, and the crash-recovery rule owns the corpse.
fn transaction_lost_refusal() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("Phase4TransactionLost".to_owned()),
        os_message: Some(
            "a fatal database error ended the finish transaction mid-phase; everything \
             the phase wrote is rolled back and the run stays running"
                .to_owned(),
        ),
    });
    error
}

/// The class-only refusal for a run row phase 4 may not finish: the
/// row is absent (the `list_projects` read above makes that a broken
/// invariant) or no longer `running` — already finished, aborted or
/// interrupted. The stamps and prunes of this phase must not run
/// over such a run (the round-1 P3-4 containment): the store's own
/// zero-row refusal would come only after them.
fn run_not_running_refusal() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("Phase4RunNotRunning".to_owned()),
        os_message: Some(
            "the run phase 4 was asked to finish is not in the running state — a \
             second finish of one run is a caller bug"
                .to_owned(),
        ),
    });
    error
}

/// The class-only refusal for a run with an unsettled row at finish
/// time: phases 1–3 own every `sync_run_projects` transition, so a
/// `not_started` row here names a broken phase chain (the phase-1
/// `project_missing_refusal` family).
fn unsettled_row_refusal() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("Phase4RowUnsettled".to_owned()),
        os_message: Some(
            "a project row of the run is still not_started at the finish — a phase of the \
             pipeline did not settle its verdict"
                .to_owned(),
        ),
    });
    error
}

/// The class-only refusal for an `unchanged` row whose run row holds
/// no DB-020 fingerprint: phase 0's writer records one per project
/// and the FK keeps the row alive, so a miss names a broken
/// invariant of that chain (the phase-3 `fingerprint_missing_refusal`
/// twin).
fn fingerprint_missing_refusal() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("Phase4FingerprintMissing".to_owned()),
        os_message: Some(
            "an unchanged project of the run has no build hash in its sync_run_projects \
             row"
            .to_owned(),
        ),
    });
    error
}

#[cfg(test)]
mod tests {
    //! The pure halves — the journal shape tables, the DB-002 stamp,
    //! and the class-only refusals. The phase behavior (the stamps,
    //! the retention, the verdict, the journal rows, the dirty seam)
    //! lives in `sync_phase4_tests.rs`, which owns its mock-app
    //! harness.

    use eunomia_store::sync_runs::{SyncRunProject, SyncRunStatus};

    use super::*;

    /// The journal shape table for one project's entry: the level
    /// and result members per status — the Activity window's ok/
    /// skipped/ error chips (§7.7 UI row) are these literals.
    #[test]
    fn the_project_journal_shape_table() {
        for (status, level, result) in [
            (SyncRunProjectStatus::Written, Level::Info, Outcome::Ok),
            (SyncRunProjectStatus::Unchanged, Level::Info, Outcome::Ok),
            (SyncRunProjectStatus::Skipped, Level::Warn, Outcome::Skipped),
            (SyncRunProjectStatus::Failed, Level::Error, Outcome::Error),
        ] {
            assert_eq!(
                project_entry_shape(status),
                (level, result),
                "the shape of {status:?}"
            );
        }
    }

    /// The journal shape table for the run summary: success info/ok,
    /// partial warn/skipped, failed error/error — the module docs'
    /// ruling, pinned member by member.
    #[test]
    fn the_run_journal_shape_table() {
        assert_eq!(
            run_entry_shape(SyncRunStatus::Success),
            (Level::Info, Outcome::Ok)
        );
        assert_eq!(
            run_entry_shape(SyncRunStatus::Partial),
            (Level::Warn, Outcome::Skipped)
        );
        assert_eq!(
            run_entry_shape(SyncRunStatus::Failed),
            (Level::Error, Outcome::Error)
        );
    }

    /// The details table: a written row carries its target path, a
    /// skipped row its §10 code and (T-124/D-223) its own frozen
    /// target path plus the caller-supplied OS code, a failed row
    /// code, path and OS code, an unchanged row nothing — and a code
    /// outside the §10 shape (tampering) keeps the entry at the cost
    /// of the payload, and a written row without a path (no writer
    /// produces one) the same.
    #[test]
    fn the_project_journal_details_table() {
        fn row(
            status: SyncRunProjectStatus,
            target_path: Option<&str>,
            error_code: Option<&str>,
        ) -> SyncRunProject {
            SyncRunProject {
                sync_run_id: "run".to_owned(),
                project_id: "project".to_owned(),
                status,
                target_path: target_path.map(str::to_owned),
                build_hash: Some("f".repeat(64)),
                error_code: error_code.map(str::to_owned),
                error_message: None,
            }
        }

        assert_eq!(
            project_entry_details(
                &row(
                    SyncRunProjectStatus::Written,
                    Some("/repos/r/AGENTS.md"),
                    None
                ),
                None
            ),
            Some(Details::File {
                path: std::path::PathBuf::from("/repos/r/AGENTS.md")
            })
        );
        assert_eq!(
            project_entry_details(
                &row(SyncRunProjectStatus::Unchanged, Some("/r/AGENTS.md"), None),
                None
            ),
            None
        );
        // A skip: §10's code, the row's own frozen target path (the
        // T-124 reading — an error about the project's target), and
        // the OS code riding only when the refusal's outcome carried
        // one (a marker check backs E_MARKERS_INVALID with no
        // syscall: the member is absent, not null).
        assert_eq!(
            project_entry_details(
                &row(
                    SyncRunProjectStatus::Skipped,
                    Some("/r/AGENTS.md"),
                    Some("E_PROJECT_UNAVAILABLE")
                ),
                None
            ),
            Some(Details::Failure {
                code: FailureCode::new("E_PROJECT_UNAVAILABLE")
                    .expect("the §10 literal must parse"),
                path: Some(std::path::PathBuf::from("/r/AGENTS.md")),
                os_code: None,
            })
        );
        assert_eq!(
            project_entry_details(
                &row(
                    SyncRunProjectStatus::Skipped,
                    Some("/r/AGENTS.md"),
                    Some("E_NO_WRITE_ACCESS")
                ),
                Some(13)
            ),
            Some(Details::Failure {
                code: FailureCode::new("E_NO_WRITE_ACCESS").expect("the §10 literal must parse"),
                path: Some(std::path::PathBuf::from("/r/AGENTS.md")),
                os_code: Some(13),
            })
        );
        let failed = project_entry_details(
            &row(
                SyncRunProjectStatus::Failed,
                Some("/r/AGENTS.md"),
                Some("E_FILE_LOCKED"),
            ),
            Some(32),
        );
        assert_eq!(
            failed,
            Some(Details::Failure {
                code: FailureCode::new("E_FILE_LOCKED").expect("the §10 literal must parse"),
                path: Some(std::path::PathBuf::from("/r/AGENTS.md")),
                os_code: Some(32),
            })
        );
        // The degraded shapes: a non-§10 code and a missing path keep
        // the entry alive without the payload.
        assert_eq!(
            project_entry_details(
                &row(
                    SyncRunProjectStatus::Failed,
                    Some("/r/AGENTS.md"),
                    Some("not a code")
                ),
                None
            ),
            None
        );
        assert_eq!(
            project_entry_details(&row(SyncRunProjectStatus::Written, None, None), None),
            None
        );
    }

    /// The DB-002 stamp: the exact 24-character UTC millisecond
    /// spelling, truncated (not rounded) — the `values::now_utc`
    /// contract mirrored (the phase-2/3 pins, spelled for this
    /// module's own twin).
    #[test]
    fn the_db002_stamp_is_the_values_spelling() {
        let now: Timestamp = "2026-11-21T11:45:00.456Z"
            .parse()
            .expect("the fixed instant must parse");
        assert_eq!(db002_stamp(now), "2026-11-21T11:45:00.456Z");
        let extra: Timestamp = "2026-11-21T11:45:00.4569Z"
            .parse()
            .expect("the four-digit instant must parse");
        assert_eq!(db002_stamp(extra), "2026-11-21T11:45:00.456Z");
    }

    /// The invariant refusals are class-only (SEC-008's shape
    /// check): the label and a fixed sentence, no bytes of any path
    /// or stored value.
    #[test]
    fn the_invariant_refusals_are_class_only() {
        for (error, kind) in [
            (unsettled_row_refusal(), "Phase4RowUnsettled"),
            (fingerprint_missing_refusal(), "Phase4FingerprintMissing"),
            (run_not_running_refusal(), "Phase4RunNotRunning"),
            (transaction_lost_refusal(), "Phase4TransactionLost"),
        ] {
            assert_eq!(error.code, ErrorCode::DbLocked);
            let details = error.details.expect("the refusal carries details");
            assert_eq!(details.os_code, None);
            assert_eq!(details.os_kind.as_deref(), Some(kind));
            assert!(details.os_message.is_some());
        }
    }

    /// The liveness probe: an autocommit connection (a fatal class's
    /// implicit rollback) refuses; a live transaction passes. Driven
    /// over a real connection so both branches are behavior, not
    /// type-checking.
    #[test]
    fn the_liveness_probe_discriminates_a_lost_transaction() {
        let mut connection =
            rusqlite::Connection::open_in_memory().expect("an in-memory connection must open");
        let transaction = connection
            .transaction()
            .expect("opening a transaction must work");
        assert_eq!(transaction_alive(&transaction), Ok(()));
        drop(transaction);
        assert!(
            transaction_alive(&connection).is_err(),
            "an autocommit connection names a lost transaction"
        );
        let _ = &mut connection;
    }
}
