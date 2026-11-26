//! The Sync module's two READ commands (spec §7.10 Sync row's
//! `sync_status` and `sync_last_result`, MAIN-024…027/031, LOG-007,
//! OUT-009, §9.4 phase 4 step 4; T-121; the contract's resolved
//! readings live in DECISIONS.md D-217).
//!
//! Both commands serve MAIN-024's sync panel between runs — the
//! cold-start facts the panel renders (`sync_status`) and the last
//! attempt's summary the "Sync summary" dialog re-opens over
//! (`sync_last_result`). Neither writes anything: no SYS-005 gate
//! (SYS-005's list is the project-*changing* commands — reads are
//! never gated, the `library_get`/`projects_list` posture), no
//! journal (reading is not an event), and recovery mode answers the
//! honest `NoDatabase` refusal through the shared
//! [`crate::commands::managed_db`] edge every database command takes
//! (the recovery screen has no sync panel to serve, and the flag
//! knowledge would be `false` there anyway — phase 0 refuses before
//! any flag semantics could matter).
//!
//! **`sync_status`'s four members** (each a resolved reading,
//! D-217 point 3):
//!
//! - `is_running` — the [`crate::sync_running::SyncRunning`] flag's
//!   plain load, SYS-004's OWN surface ("the global `sync_running`
//!   flag"): the check-never-acquire posture of
//!   [`crate::commands::reject_during_sync`] — a poll is a
//!   moment-of-read fact, and a run boundary between two members'
//!   reads can mix moments (inherent to polls, the `projects_list`
//!   verdict-read posture: the next read settles it).
//! - `last_success_finished_at` — MAIN-027's DERIVED time through
//!   its single owner [`eunomia_store::sync_runs::
//!   last_success_finished_at`] (D-134: the `finished_at` of the
//!   newest `success` run; a `partial`/`failed`/`aborted`/
//!   `interrupted` finish moves nothing — §9.4 phase 4 step 4's own
//!   rule). `None` over a database with no success run yet.
//! - `last_attempt_partial` — §9.4 phase 4 step 4's "the UI shows
//!   'Last attempt: partial'": true iff the newest TERMINAL run
//!   row's verdict is the `partial` literal. Every terminal verdict
//!   counts as an attempt (an aborted or interrupted run was an
//!   attempt whose outcome is known); a live run is not one yet —
//!   while it runs, the panel shows the live k-of-N region, and the
//!   run's own answer (`sync_run`'s summary) is the outcome its
//!   consumer reads.
//! - `dirty_projects` — the COUNT of projects with unsynced changes
//!   (MAIN-024's counter pill, MAIN-026/S9's "the counter matches
//!   the number of dirty projects"): aggregated from the SAME two
//!   halves `projects_list`'s column serves — the managed
//!   [`crate::dirty_recalc::DirtyState`] cache's last recorded
//!   verdicts (the recalculation service's own state seam; absent ≡
//!   the clean baseline) plus the row-decidable DB-014 archived half
//!   — through the shared [`crate::commands::dirty_verdict`] helper,
//!   so the count, the column and the `state://dirty` event can
//!   never disagree about which projects are dirty.
//!
//! **`sync_last_result`'s answer** (D-217 point 4): the newest
//! TERMINAL run row's [`crate::commands::SyncRunSummary`] — D-138's
//! shape reused VERBATIM over [`crate::commands::summary_of`] (the
//! committed rows read back; one DTO, one read shape) — plus the ONE
//! member the cold-start contract adds: the RUN row's `error_code`
//! (a stopped or failed attempt's §10 code, the UI-011 what-to-do
//! hint's input), read through the ONE seam
//! [`crate::commands::run_error_code_or_none`] — the pinned T-282
//! path (D-147's half 3): this command is the cell's first
//! production reader, collapsing both no-code spellings (`NULL` and
//! `""`) to "no code", and every later reader reuses the seam. The
//! PER-PROJECT rows' codes stay the summary's own contract (each
//! row's own column — a `skipped`/`failed` row carries a real §10
//! literal by definition, never the empty spelling). With no
//! terminal run yet the answer is the `noRuns` state — a state,
//! never an `Err` (D-138's answer-shape discipline). While a run is
//! LIVE the command answers the PREVIOUS terminal attempt: the live
//! run's summary is its own answer, and a mid-run read of a
//! half-written row set would lie.
//!
//! The read is two transactions over a FIXED run id
//! ([`eunomia_store::sync_runs::last_attempt`], then
//! [`crate::commands::summary_of`] by id): a terminal row is never
//! rewritten (the pinned invariant the abort wrapper's no-op arm
//! proves), so the pair is consistent by construction.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};

use eunomia_store::projects::ListScope;
use eunomia_store::sync_runs::SyncRunStatus;

use crate::commands::{
    SyncRunSummary, dirty_verdict, managed_db, managed_sync_flag, run_error_code_or_none,
    summary_of,
};
use crate::db::sync_runs_to_app_error;
use crate::dirty_recalc::DirtyState;
use crate::errors::AppError;
use crate::logging;

/// The `sync_status` command's answer (§7.10 Sync module, MAIN-024…
/// 027): the four facts the sync panel renders between runs. Every
/// member is a fact, never an error state — an empty database
/// answers `false`/`None`/`false`/`0` honestly (the pre-first-sync
/// panel), and a missing database is the command's `Err` (the
/// recovery-mode `NoDatabase` every database command shares).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct SyncStatus {
    /// Whether a sync holds the SYS-004 flag right now — the panel's
    /// disabled "Run sync" button and live region. A poll read: the
    /// moment-of-load fact (a run boundary between this and another
    /// member's read can mix moments; the next read settles it).
    pub(crate) is_running: bool,
    /// MAIN-027's overall time of the last successful sync (LOG-007):
    /// the `finished_at` of the newest `success` run (DB-002 UTC
    /// ISO-8601), DERIVED, never stored; `None` before the first
    /// successful sync. A `partial` attempt moves it never (§9.4
    /// phase 4 step 4).
    pub(crate) last_success_finished_at: Option<String>,
    /// §9.4 phase 4 step 4's "Last attempt: partial" fact: the newest
    /// TERMINAL run row's verdict is `partial`. A live run is not an
    /// attempt yet (its own answer is the outcome its consumer
    /// reads); every terminal verdict counts — an aborted or
    /// interrupted run was an attempt whose outcome is known.
    pub(crate) last_attempt_partial: bool,
    /// The count of active projects with unsynced changes (MAIN-024's
    /// counter pill, DB-021): aggregated from the same
    /// [`DirtyState`] verdicts `projects_list`'s column serves, so
    /// the pill, the column dots and the `state://dirty` event agree.
    /// `i32` (specta forbids BigInt-style types on the wire; a
    /// projects-table row count cannot reach `i32::MAX`).
    pub(crate) dirty_projects: i32,
}

/// The `sync_last_result` command's answer (§7.10 Sync module,
/// OUT-009/MAIN-031): the last attempt's summary, or the no-runs
/// state — serde-tagged the [`crate::commands::SyncRunOutcome`]
/// family way (`{"outcome": …}`).
///
/// The `completed` arm's `summary` is D-138's
/// [`crate::commands::SyncRunSummary`] verbatim — the committed rows
/// read back — so the dialog re-opened at cold start renders exactly
/// what the run's own answer rendered; the ONE member the cold-start
/// read adds is `error_code`, the RUN row's §10 code through the
/// [`crate::commands::run_error_code_or_none`] seam (the pinned
/// T-282 path: both no-code spellings collapse to "no code" — a
/// finished run answers `None`, an error abort its §10 literal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub(crate) enum SyncLastResult {
    /// No terminal run row exists: a fresh database, or one where
    /// only a run is live (a live run is not an attempt yet — the
    /// command answers the PREVIOUS terminal attempt, never a
    /// half-written row set). A state, never an `Err`: the panel
    /// simply has no last result to render.
    NoRuns,
    /// The newest terminal attempt's summary: the verdict, MAIN-024's
    /// counter chips, the §9.4 details cell, and every project's row
    /// (status, target path, §10 code, class-only message — the
    /// reasons OUT-009/MAIN-031 demand), plus the run row's own §10
    /// code through the one seam.
    Completed {
        /// The last attempt's committed rows, read back the
        /// `sync_run` answer's way (D-138's shape).
        summary: SyncRunSummary,
        /// The RUN row's §10 code of a non-success outcome, read
        /// through [`crate::commands::run_error_code_or_none`] (the
        /// T-282 seam: `NULL` and `""` both answer "no code"). The
        /// PER-PROJECT codes ride the summary's own rows — this
        /// member is the run-level refusal's code alone.
        error_code: Option<String>,
    },
}

/// Serves the sync panel's cold-start facts (§7.10 Sync module
/// `sync_status`, MAIN-024…027): whether a run is live, the overall
/// time of the last successful sync, whether the last attempt was
/// `partial`, and the count of projects with unsynced changes.
///
/// One read transaction carries the three database facts (MAIN-027's
/// derived time, the last attempt row, the active projects list the
/// dirty count aggregates over); the flag is read before it (the
/// cheapest act, the `reject_during_sync` check posture) and the
/// dirty verdicts after it (the cache read the
/// `projects_list` staleness rule orders after its own blocking
/// probes — this command probes nothing, so the read follows the
/// transaction directly). Failures reach the log with their §10 code
/// ([`logging::sync_status_failed`]); a successful read logs nothing.
#[tauri::command]
#[specta::specta]
pub(crate) async fn sync_status<R: Runtime>(app: AppHandle<R>) -> Result<SyncStatus, AppError> {
    sync_status_impl(app)
        .await
        .inspect_err(logging::sync_status_failed)
}

/// The body of [`sync_status`], split the `projects_list` way so the
/// one logging edge covers every failure path.
async fn sync_status_impl<R: Runtime>(app: AppHandle<R>) -> Result<SyncStatus, AppError> {
    // The flag first: a plain load (a check, never an acquire — only
    // the pipeline holds the guard, `sync_running.rs`'s own rule),
    // through the same fallback-carrying lookup the SYS-005 gate
    // uses, so a missing state answers the honest diagnostic here
    // too.
    let is_running = managed_sync_flag(&app)?.is_running();
    // The three database facts in ONE read transaction (the D-037
    // executor: a run's writes serialize against this read on the
    // one connection, so the members describe one committed moment).
    let (last_success, last_attempt, projects) = managed_db(&app)?
        .run(move |tx| {
            let last_success = eunomia_store::sync_runs::last_success_finished_at(tx)
                .map_err(sync_runs_to_app_error)?;
            let last_attempt =
                eunomia_store::sync_runs::last_attempt(tx).map_err(sync_runs_to_app_error)?;
            let projects = eunomia_store::projects::list(tx, ListScope::Active)
                .map_err(crate::commands::projects_store_to_app_error)?;
            Ok((last_success, last_attempt, projects))
        })
        .await?;
    // The dirty count over the same verdicts `projects_list`'s column
    // serves: the managed cache read after the transaction (the
    // `projects_list` staleness rule — this command owns no blocking
    // probes, so the read follows the transaction directly), the
    // shared helper deciding both halves (the cache's verdict, the
    // DB-014 archived arm). A missing state degrades to the clean
    // baseline (the helper's own posture — an uninitialized-app bug
    // must not fail a display read).
    let dirty_cache = app.try_state::<DirtyState>();
    let dirty = projects
        .iter()
        .filter(|project| dirty_verdict(dirty_cache.as_deref(), project))
        .count();
    Ok(SyncStatus {
        is_running,
        last_success_finished_at: last_success,
        last_attempt_partial: last_attempt.is_some_and(|run| run.status == SyncRunStatus::Partial),
        // The wire's i32 (the sync chips' posture, D-138): a count of
        // projects, never near the bound.
        dirty_projects: i32::try_from(dirty).expect("the active project count fits the wire's i32"),
    })
}

/// Serves the last attempt's summary (§7.10 Sync module
/// `sync_last_result`, OUT-009/MAIN-031): the newest TERMINAL run
/// row's committed rows read back the `sync_run` answer's way, plus
/// the run row's §10 code through the one seam — or the `noRuns`
/// state when no terminal run exists.
///
/// Failures reach the log with their §10 code
/// ([`logging::sync_last_result_failed`]); both ANSWERS are states
/// and log nothing (reading is not an event).
#[tauri::command]
#[specta::specta]
pub(crate) async fn sync_last_result<R: Runtime>(
    app: AppHandle<R>,
) -> Result<SyncLastResult, AppError> {
    sync_last_result_impl(app)
        .await
        .inspect_err(logging::sync_last_result_failed)
}

/// The body of [`sync_last_result`], split the same way so the one
/// logging edge covers every failure path.
async fn sync_last_result_impl<R: Runtime>(app: AppHandle<R>) -> Result<SyncLastResult, AppError> {
    // The last attempt: the newest TERMINAL row. `None` answers a
    // database with no terminal run yet (fresh, or only a live run —
    // a live run is not an attempt and its half-written rows must
    // not leak into a summary).
    let last = managed_db(&app)?
        .run(move |tx| eunomia_store::sync_runs::last_attempt(tx).map_err(sync_runs_to_app_error))
        .await?;
    let Some(run) = last else {
        return Ok(SyncLastResult::NoRuns);
    };
    // THE seam (the pinned T-282 path, D-147's half 3): the RUN row's
    // `error_code`, both no-code spellings collapsed — this command
    // is the cell's first production reader, and the spelling is
    // never re-decided here. The per-project rows' codes ride the
    // summary below, each from its own column.
    let error_code = run_error_code_or_none(run.error_code.as_deref()).map(str::to_owned);
    // The summary over the SAME row, by its fixed id: two
    // transactions, one consistent answer — a terminal row is never
    // rewritten, so the pair cannot drift.
    let summary = summary_of(&app, &run.id).await?;
    Ok(SyncLastResult::Completed {
        summary,
        error_code,
    })
}
