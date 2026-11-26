//! The sync pipeline's phase 1 — preflight (spec §9.4 phase 1, §10's
//! skip rows, D-09/D-24/D-25, OUT-008; task T-113): the per-project
//! gate run over [`Phase0Outcome`]'s built content that decides, for
//! every active project in `sort_order`, whether the run will write
//! it (`ready`), already owns its file (`unchanged`, D-24/ALG-033),
//! or must leave it alone (`skipped`, one §10 row per refusal) — and
//! the closing free-space rule that can stop the whole run
//! (`aborted`/`E_DISK_FULL`) before a single byte moves.
//!
//! **"Preflight (changes nothing)" reads on the target files, not on
//! the run's database.** No project file is written, no backup copy
//! is created, and the only bytes the phase ever puts into a target
//! folder are the write-access probe's one staging file — created and
//! removed again by [`eunomia_fs::write_access`] itself (D-094: the
//! probe IS §9.4 step 3's "whether a temporary file can be created"),
//! sweepable debris if the process dies between its two halves. The
//! phase's one deletion is step 7's sweep of app-stale tmp files
//! (D-088). The run's own bookkeeping — `skipped`/`unchanged` rows
//! and, on the space shortage, the `aborted` run row — is the
//! database half of the run (§9.4's pipeline records its progress),
//! not a target-file change.
//!
//! **The seven steps, in §9.4's own order** ([`preflight_one`]):
//!
//! 1. Availability — [`check_project_available`] (D-084); a refusal
//!    skips with `E_PROJECT_UNAVAILABLE` (D-09: "the sync skips such
//!    a project and does not stop").
//! 2. The target path — [`resolve_target_path`] (D-092): symlink
//!    chains resolve to the real file (the `CLAUDE.md` → `AGENTS.md`
//!    case), a dangling link / loop skips with
//!    `E_TARGET_BROKEN_LINK`, a real path outside the root with
//!    `E_TARGET_OUTSIDE_ROOT`, a folder with `E_TARGET_IS_DIR`.
//! 3. Rights — [`check_write_access`] (D-094): a read-only existing
//!    file or a folder that refuses the staging probe skips with
//!    `E_NO_WRITE_ACCESS`.
//! 4. The existing file — read once; scheme B demands UTF-8
//!    (`E_NOT_UTF8`) and exactly one marker pair in order
//!    (`E_MARKERS_INVALID`, SYS-009: a deleted scheme-B target is the
//!    no-markers skip, not a creation). The step also assembles each
//!    project's write plan — the bytes phase 3 will lay down — in its
//!    markerless form (below).
//! 5. `sync.skip_identical` (§6.5) on and the content matches by
//!    ALG-033 → `unchanged` ([`contents_match`]; the marker line is
//!    dropped from both sides — the timestamp never decides).
//! 6. Empty build over a non-empty file → the project joins the D-25
//!    confirmation list the outcome carries ([`Phase1Outcome::
//!     needs_confirmation`]); the confirm/decline gate itself is the
//!    `sync_run` command's (T-118).
//! 7. The stale-tmp sweep ([`remove_stale_tmp_files`], D-088) in the
//!    resolved target's folder, keyed on the resolved name — the one
//!    housekeeping deletion, best-effort: a refused listing never
//!    skips a project that already earned its write.
//!
//! A `skipped` project stops at its own step — steps after the
//! refusal (the sweep included) do not run for it — and never stops
//! the run: the loop takes the next project (the card's criterion 1).
//!
//! **The markerless write plan.** ALG-032 stamps the creation marker
//! "once per run, at the start of the write phase", which preflight
//! precedes — so the phase cannot know the marker line it would
//! write, and deliberately does not guess one. What it builds
//! instead is the markerless form of the write: schemes A and C take
//! ALG-030's shape minus the marker line (`managed` + one blank
//! line, or the empty string for an empty build — exactly
//! [`contents_match`]'s `new_without_marker_line` contract); scheme B
//! takes the ALG-031 splice minus the marker line (the preserved
//! outside around the converted managed block), spelled by
//! [`splice_without_marker`] — a deliberate byte-exact mirror of
//! `eunomia_core::assembly_b::splice`'s private block builder, pinned
//! against the real splice's own output the D-130 way (a mirror that
//! cannot drift because the real builder is its oracle). The plan
//! itself rides the outcome as a [`WritePlan`] — `WholeFile` (A/C)
//! carries `managed` and phase 3 rebuilds the whole file around the
//! run's marker; `Splice` (B) carries the read `content` and its
//! parsed `span` alongside `managed`, so phase 3 splices between the
//! markers WITHOUT re-reading the file (§9.4's reader is phase 1;
//! TOCTOU between phases is §9.4's own design — SYS-008's watcher
//! owns external edits, and the next sync converges). The variant IS
//! the scheme for phase 3's purposes: A and C share the whole-file
//! shape, so no third home for the scheme column is needed.
//!
//! **The regular-file gate.** `resolve_target_path` accepts any
//! non-directory, non-link entry by design (the write decides), but
//! a FIFO or device node named like a target would BLOCK this
//! phase's sequential read until a writer appears — hanging the run
//! and its flag for the process's life. The app layer (the fs crate
//! is a frozen zone for this task) refuses the entry kind between
//! steps 2 and 3 with the unavailability row's class-only message;
//! no §10 code names "wrong entry kind", and the ruling is recorded
//! in the task's D-entry.
//!
//! **The free-space tail.** After the per-project cycle, the phase
//! sizes its `ready` projects' existing files (phase 2's future
//! copies) and asks the injected free-space source — the D-089 seam,
//! production closes over `free_bytes(&data_dir)`, tests substitute a
//! number — whether the data-directory volume holds their sum plus
//! the spec-fixed 20% margin ([`check_backup_space`]); the probe is a
//! statvfs syscall, so it runs on the blocking pool like every other
//! syscall of the phase (SYS-001). A definitive
//! shortage aborts the whole run: the run row goes `aborted` with
//! `E_DISK_FULL` (criterion 2), per-project rows keep the verdicts
//! phase 1 already wrote (nothing claims a write that did not
//! happen), and the phase answers the typed `E_DISK_FULL` envelope
//! with the free/required numbers in its details. The abort
//! transaction also writes the run's `sync.run_aborted` journal
//! entry (T-124/D-223): error/error, the `E_DISK_FULL` literal in
//! `Details::Failure` and nothing else — no path, project or OS
//! code, because a space comparison is about the volume, not one
//! project, and no syscall produced it (§10's fields ride when they
//! exist). A **failed probe**
//! is not a shortage: like `atomic`'s write-time check, it does not
//! block — phase 2's own backup failure (`E_BACKUP_FAILED`) is the
//! honest reporter of a volume that cannot take the copies. An empty
//! copy list skips the probe outright: zero required bytes cannot
//! come up short.
//!
//! **The step-5/step-6 order, and the idempotence edge.** §9.4 runs
//! the steps in order, so step 5 short-circuits step 6: a build that
//! matches by ALG-033 is `unchanged` even when the build is empty and
//! the file holds only the marker line — the file the next write
//! would produce byte for byte, which is D-24's own acceptance ("a
//! second sync with no edits changes no files"), not a D-25 wipe.
//! Step 6 therefore fires only on the `ready` path: an empty build
//! over an existing file with more than zero bytes. `skip_identical`
//! off moves that marker-only file to the confirmation list (the
//! file is "not empty" in the raw, size-based reading of D-25) — the
//! spelling recorded with this task's D-entry.
//!
//! **The scheme's second home.** `Phase0Outcome` carries content,
//! hash and target path per project but not the scheme, and
//! `sync_phase0.rs` is a frozen zone for this task — so the phase
//! reads each project's scheme from the `projects` table in its one
//! input transaction. The read cannot observe an app-driven change:
//! the run holds `sync_running`, and D-123's SYS-005 gate rejects
//! the project-mutation commands while it is up, so the scheme the
//! check branches on is the scheme the snapshot built with (only an
//! out-of-band writer on the database file could race it — the same
//! residual T-100 window every post-snapshot read shares).
//!
//! **Composed like phase 0** (D-130): one read transaction (the
//! setting + the schemes), the per-project probes on the blocking
//! pool after it (SYS-001, the D-096 ordering — four-plus syscalls
//! per project over possibly dead drives must never hold the one
//! database lock), one write transaction applying every verdict, and
//! — only on the space shortage — a second write transaction for the
//! abort. Every `Err` before the abort leaves the run row exactly as
//! phase 0 wrote it (`running`): a failed preflight is the
//! crash-recovery rule's business, not a half-verdict.

use std::path::PathBuf;
use std::time::SystemTime;

use tauri::{AppHandle, Runtime};

use eunomia_core::compare::contents_match;
use eunomia_core::markers::{MarkerSpan, parse as parse_markers};
use eunomia_fs::free_space::{self, FreeSpaceError};
use eunomia_fs::paths::{
    TargetPath, TargetPathError, check_project_available, resolve_target_path,
};
use eunomia_fs::temp_files::remove_stale_tmp_files;
use eunomia_fs::write_access::{WriteAccessError, check_write_access};
use eunomia_store::settings::{SettingKey, SettingValue};
use eunomia_store::sync_runs::{abort_run, mark_project_skipped, mark_project_unchanged};
use eunomia_store::{projects, settings};

use crate::commands::{managed_db, projects_store_to_app_error, settings_cell_to_app_error};
use crate::db::{join_failure_to_app_error, sync_runs_to_app_error};
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::sync_phase0::{Phase0Outcome, PreparedProject};

/// The phase's product for one project: what §9.4 phase 1 decided and
/// everything phases 2–3 need to act on it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PreflightVerdict {
    /// §9.4 phase 1 refused the project: the §10 code of the refusal
    /// and a short, class-only message for the run's row.
    Skipped {
        /// The §10 code (stored as its `"E_*"` literal).
        code: ErrorCode,
        /// The refusal's Display text (paths and OS codes only, never
        /// file content — SEC-008).
        message: String,
        /// The structured OS facts behind the refusal, when an OS
        /// call produced it (`io_error()` → `from_io_error`): §10
        /// names the OS code a first-class fact of every error log
        /// line, and the store's row has no column for it — the
        /// outcome is where T-118's log line reads it from.
        details: Option<ErrorDetails>,
    },
    /// Step 5's D-24 verdict: the file already matches the build by
    /// ALG-033; phase 4 updates the project's sync stamp.
    Unchanged,
    /// The run will write the project in phase 3; [`ReadyProject`]
    /// carries where, what, and how large the phase-2 backup input
    /// is.
    Ready(ReadyProject),
}

/// Phase 3's write recipe for one `ready` project: everything the
/// write phase needs to lay down the final bytes once the run's
/// marker line exists (ALG-032 stamps it at the write phase's start).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WritePlan {
    /// Schemes A and C — the whole-file rebuild: the final bytes are
    /// ALG-030's shape around `managed` and the marker
    /// (`managed` + a blank line + marker + one break, or just
    /// `marker` + the break when `managed` is empty). The read
    /// content never matters: the write replaces the whole file.
    WholeFile {
        /// The markerless managed text phase 0 built.
        managed: String,
    },
    /// Scheme B — the ALG-031 splice: everything outside the marker
    /// pair of the file phase 1 read survives byte for byte, so the
    /// read `content` and its parsed `span` travel here — §9.4's
    /// reader is phase 1, and phase 3 splices `managed` and the
    /// marker between the markers WITHOUT re-reading the file.
    Splice {
        /// The markerless managed text phase 0 built.
        managed: String,
        /// The existing file's whole text, as read in step 4.
        content: String,
        /// The one marker pair's span inside `content` (D-19).
        span: MarkerSpan,
    },
}

/// One `ready` project's preflight product (§9.4 phase 1 step 4's
/// "build the final bytes", in the markerless form the module docs
/// explain).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadyProject {
    /// The resolved write path (D-092): the real file behind any
    /// symlink, the path the atomic write renames over.
    pub(crate) resolved_path: PathBuf,
    /// The resolved target's file name (lossy text) — the staging
    /// grammar's and the step-7 sweep's key (D-088).
    pub(crate) resolved_name: String,
    /// The markerless form of the bytes phase 3 will write: the
    /// ALG-033 comparison basis this phase decided on (the write plan
    /// plus the run's marker line reproduces it exactly).
    pub(crate) markerless: String,
    /// The phase-3 write recipe (the scheme rides the variant:
    /// A and C share the whole-file shape, B splices).
    pub(crate) plan: WritePlan,
    /// The existing target file's size in bytes: `None` is the
    /// creation case (OUT-005 — a missing A/C target is written, not
    /// skipped); `Some(size)` feeds phase 2's backup list and this
    /// phase's free-space check.
    pub(crate) existing_size: Option<u64>,
    /// D-25: the build is empty and the existing file is not — the
    /// `sync_run` command (T-118) refuses to proceed past this list
    /// without `confirm_empty`.
    pub(crate) needs_confirmation: bool,
}

/// What phase 1 hands the pipeline: the run's id and every project's
/// verdict, in the snapshot's `sort_order` (§9.4 phase 1's "for each
/// active project, in order").
#[derive(Debug)]
pub(crate) struct Phase1Outcome {
    /// The run these verdicts belong to (the `sync_runs` id phase 0
    /// recorded).
    pub(crate) run_id: String,
    /// One `(project_id, verdict)` pair per active project of the
    /// run, in `sort_order`.
    pub(crate) projects: Vec<(String, PreflightVerdict)>,
}

impl Phase1Outcome {
    /// The D-25 confirmation list: every `ready` project whose build
    /// is empty over a non-empty existing file, in `sort_order` —
    /// the list §9.4 phase 1 step 6 says the `sync_run` command
    /// returns with `NeedsConfirmation` (T-118's gate).
    pub(crate) fn needs_confirmation(&self) -> Vec<&str> {
        self.projects
            .iter()
            .filter(|(_, verdict)| {
                matches!(
                    verdict,
                    PreflightVerdict::Ready(ReadyProject {
                        needs_confirmation: true,
                        ..
                    })
                )
            })
            .map(|(project_id, _)| project_id.as_str())
            .collect()
    }
}

/// Runs §9.4 phase 1 end to end over phase 0's outcome: one read
/// transaction for the setting and the schemes, the seven-step
/// preflight per project on the blocking pool, one write transaction
/// applying the `skipped`/`unchanged` rows, and the free-space tail.
///
/// `free_space` is the D-089 seam — the free bytes of the
/// data-directory volume, consulted at most once, only when some
/// `ready` project has an existing file to back up (production closes
/// over `free_space::free_bytes(&data_dir)`; tests substitute a
/// number). `now` is the step-7 sweep's clock (production passes
/// [`SystemTime::now`]; the tests shift it to age fixtures).
///
/// # Errors
///
/// - The typed `E_DISK_FULL` envelope after the run row went
///   `aborted` — the §9.4 tail rule (criterion 2); `details` carries
///   the free and required byte counts, numbers only.
/// - The honest `NoDatabase` fallback in recovery mode, and the
///   classified store errors of the transactions — a failed
///   preflight leaves the run row `running`, the crash-recovery
///   rule's case.
pub(crate) async fn run<R, S>(
    app: &AppHandle<R>,
    phase0: &Phase0Outcome,
    free_space: S,
    now: SystemTime,
) -> Result<Phase1Outcome, AppError>
where
    R: Runtime,
    S: FnOnce() -> Result<u64, FreeSpaceError> + Send + 'static,
{
    run_with_abort_clock(app, phase0, free_space, now, eunomia_store::values::now_utc).await
}

/// [`run`] with the abort's clock injected — the seam T-346 added,
/// the [`sync_phase2::run_with_abort_clock`] shape over phase 1's own
/// stamp type.
///
/// `abort_clock` answers the ABORT's instant, in the DB-002 spelling
/// (phase 1's abort writes `values::now_utc()` directly — the string
/// is the phase's native stamp type, no jiff conversion anywhere in
/// it). It is consulted at most once, on the free-space abort path
/// only, right before the verdict transaction — the moment the row
/// goes terminal (the phase-2 abort's and the command's `stop_run`'s
/// posture, D-185's semantics). The read point is documented, not
/// pinned: the abort has no side effect before the stamp for a
/// second clock observation to discriminate (phase 2's read-point pin
/// uses the swept copy — phase 1 has no sweep), and a `FnOnce` clock
/// answers once.
///
/// # Errors
///
/// [`run`]'s.
pub(crate) async fn run_with_abort_clock<R, S>(
    app: &AppHandle<R>,
    phase0: &Phase0Outcome,
    free_space: S,
    now: SystemTime,
    abort_clock: impl FnOnce() -> String,
) -> Result<Phase1Outcome, AppError>
where
    R: Runtime,
    S: FnOnce() -> Result<u64, FreeSpaceError> + Send + 'static,
{
    let database = managed_db(app)?;

    // Step A — one read transaction: `sync.skip_identical` and each
    // project's scheme (the module docs' second-home ruling: the
    // E_SYNC_RUNNING gate makes the table the snapshot's equal).
    let project_ids = phase0
        .projects
        .iter()
        .map(|project| project.project_id.clone())
        .collect::<Vec<_>>();
    let (skip_identical, schemes) = database
        .run(move |transaction| {
            let value = settings::get(transaction, SettingKey::SyncSkipIdentical)
                .map_err(settings_cell_to_app_error)?;
            let skip_identical = match value {
                SettingValue::Bool(enabled) => enabled,
                // `get` applies the key's own default and validates
                // stored cells against the key's contract, so any
                // other variant names a broken invariant of that
                // chain — the class-only refusal, never a guess.
                _ => return Err(setting_shape_refusal()),
            };
            let mut schemes = Vec::with_capacity(project_ids.len());
            for id in &project_ids {
                let project = projects::get(transaction, id)
                    .map_err(projects_store_to_app_error)?
                    // The sync_run_projects rows reference this id
                    // with ON DELETE RESTRICT, so a missing row is a
                    // broken invariant, not a data state.
                    .ok_or_else(project_missing_refusal)?;
                schemes.push(project.scheme);
            }
            Ok((skip_identical, schemes))
        })
        .await?;

    // Step B — the per-project preflight on the blocking pool, over
    // owned copies of phase 0's built content (SYS-001: the probes
    // are file I/O; nothing here may hold the database lock).
    let prepared = phase0.projects.clone();
    let verdicts = {
        let build_handle = tauri::async_runtime::spawn_blocking(move || {
            prepared
                .iter()
                .zip(schemes)
                .map(|(project, scheme)| {
                    let verdict = preflight_one(project, scheme, skip_identical, now);
                    (project.project_id.clone(), verdict)
                })
                .collect::<Vec<_>>()
        });
        build_handle.await.map_err(join_failure_to_app_error)?
    };

    // Step C — one write transaction: every `skipped` and `unchanged`
    // verdict lands in its row; `ready` rows stay `not_started`
    // (phase 3 owns their transition).
    let run_id = phase0.run.id.clone();
    let skipped_rows = verdicts
        .iter()
        .filter_map(|(project_id, verdict)| match verdict {
            PreflightVerdict::Skipped { code, message, .. } => Some((
                project_id.clone(),
                code.as_str().to_owned(),
                message.clone(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    let unchanged_ids = verdicts
        .iter()
        .filter(|(_, verdict)| matches!(verdict, PreflightVerdict::Unchanged))
        .map(|(project_id, _)| project_id.clone())
        .collect::<Vec<_>>();
    database
        .run(move |transaction| {
            for (project_id, code, message) in &skipped_rows {
                mark_project_skipped(transaction, &run_id, project_id, code, message)
                    .map_err(sync_runs_to_app_error)?;
            }
            for project_id in &unchanged_ids {
                mark_project_unchanged(transaction, &run_id, project_id)
                    .map_err(sync_runs_to_app_error)?;
            }
            Ok(())
        })
        .await?;

    // Step D — the free-space tail (§9.4's closing rule): the copies
    // are the `ready` projects' existing files — what phase 2 would
    // back up. An empty list skips the probe: zero bytes cannot come
    // up short (and a failed probe must not abort a backupless run).
    // The probe itself is a statvfs/GetDiskFreeSpaceExW syscall —
    // production closes over `free_bytes(&data_dir)` — so it runs on
    // the blocking pool like every other syscall of the phase
    // (SYS-001; a hung network volume must never hold the runtime).
    let copies = verdicts
        .iter()
        .filter_map(|(_, verdict)| match verdict {
            PreflightVerdict::Ready(ready) => ready.existing_size,
            _ => None,
        })
        .collect::<Vec<_>>();
    let probe = if copies.is_empty() {
        Ok(())
    } else {
        let probe_handle = tauri::async_runtime::spawn_blocking(move || {
            free_space::check_backup_space(&copies, free_space)
        });
        probe_handle
            .await
            .map_err(join_failure_to_app_error)?
            .map(|_| ())
    };
    if let Err(FreeSpaceError::NotEnoughSpace { free, required }) = probe {
        // The whole run aborts (criterion 2): the row records the
        // verdict with class-only numbers, then the caller hears the
        // typed envelope. A failed probe (`ProbeFailed`, Windows'
        // `ProbeUnavailable`) is deliberately NOT this arm — the
        // module docs' proceed ruling. The stamp is the ABORT's own
        // instant (D-185's semantics, T-346's seam): read right
        // before the verdict transaction, never derived from `now`.
        let numbers = format!("free {free} bytes, required {required} bytes");
        let finished_at = abort_clock();
        let abort_details = numbers.clone();
        let abort_run_id = phase0.run.id.clone();
        database
            .run(move |transaction| {
                abort_run(
                    transaction,
                    &abort_run_id,
                    &finished_at,
                    ErrorCode::DiskFull.as_str(),
                    &abort_details,
                )
                .map_err(sync_runs_to_app_error)?;
                // The journal's cancel entry (T-124/D-223, reading 4a):
                // the same transaction that aborts the row claims the
                // abort — error/error, the §10 literal in details, and
                // the honest partial field list (a space comparison
                // offers no path, project or OS code). The D-051
                // call-once mirror fires inside the closure.
                let mut entry = eunomia_store::action_log::LogEntry::new(
                    eunomia_store::action_log::Action::SyncRunAborted,
                    eunomia_store::action_log::Level::Error,
                    eunomia_store::action_log::Visibility::User,
                    eunomia_store::action_log::Outcome::Error,
                );
                entry.entity = Some(eunomia_store::action_log::Entity {
                    kind: eunomia_store::action_log::EntityType::SyncRun,
                    id: abort_run_id.as_str(),
                });
                entry.sync_run_id = Some(abort_run_id.as_str());
                // The degraded `.ok()` posture the wrapper's and
                // phase 4's entries document: a literal outside the
                // §10 registry shape (only tampering — the literal is
                // compile-time and registry-pinned by the errors
                // suite) keeps the entry at the cost of the details.
                entry.details =
                    eunomia_store::action_log::FailureCode::new(ErrorCode::DiskFull.as_str())
                        .ok()
                        .map(|code| eunomia_store::action_log::Details::Failure {
                            code,
                            path: None,
                            os_code: None,
                        });
                eunomia_store::action_log::insert(transaction, &entry)
                    .map_err(crate::commands::activity_log_to_app_error)?;
                crate::logging::action_logged(&entry);
                Ok(())
            })
            .await?;
        let mut error = AppError::new(ErrorCode::DiskFull);
        error.details = Some(ErrorDetails {
            os_code: None,
            os_kind: Some("NotEnoughSpace".to_owned()),
            os_message: Some(numbers),
        });
        return Err(error);
    }

    Ok(Phase1Outcome {
        run_id: phase0.run.id.clone(),
        projects: verdicts,
    })
}

/// §9.4 phase 1's per-project core: the seven steps, in order, over
/// one prepared project. Synchronous and self-contained — the probes
/// are the `eunomia-fs` dependency calls (read-only reuse), the
/// decision is local, and a skip at any step exits before every later
/// step (a refused project has no sweep to run).
fn preflight_one(
    project: &PreparedProject,
    scheme: settings::Scheme,
    skip_identical: bool,
    now: SystemTime,
) -> PreflightVerdict {
    // The built target path decomposes into §9.4 step 2's inputs: the
    // root (`<root>`) and the file name — the same spelling phase 0
    // joined (the stored root is canonical, the name one of four
    // static ASCII literals, so both are UTF-8 by construction; a
    // miss is the class-only broken-invariant refusal).
    let Some(root) = project.target_path.parent() else {
        return skip_unavailable("the built target path has no project root");
    };
    let Some(file_name) = project
        .target_path
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return skip_unavailable("the built target path has no UTF-8 file name");
    };

    // Step 1 — availability (D-084/D-09).
    if let Err(error) = check_project_available(root) {
        return skip_verdict(
            ErrorCode::ProjectUnavailable,
            &error.to_string(),
            error.io_error(),
        );
    }

    // Step 2 — the resolved target path (D-092): symlink chains,
    // containment, the folder verdict.
    let target = match resolve_target_path(root, file_name) {
        Ok(target) => target,
        Err(error) => return skip_target_refusal(&error),
    };

    // The regular-file gate: `resolve_target_path` accepts any
    // non-directory, non-link entry by design (the write decides),
    // but a FIFO or device node named like a target would BLOCK this
    // phase's read until a writer appears — the whole preflight runs
    // sequentially on one blocking task, so one such entry hangs the
    // run for the process's life. The fs crate is a frozen zone, so
    // the app layer refuses the entry kind here: the class-only skip
    // under §10's unavailability row (no §10 code names "wrong entry
    // kind"; the ruling is recorded in the task's D-entry).
    match std::fs::metadata(target.path()) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return skip_verdict(
                ErrorCode::ProjectUnavailable,
                "the target entry is not a regular file",
                None,
            );
        }
        // Absent here is a mid-run race (steps 1–3 saw it): the read
        // below answers the creation case, and any other refusal has
        // its own arm there.
        Err(_) => {}
    }

    // Step 3 — write rights (D-094): the resolved path is the input.
    if let Err(error) = check_write_access(target.path()) {
        return skip_access_refusal(&error);
    }

    // Step 4 — read the existing file and build the write plan.
    let existing = match std::fs::read(target.path()) {
        Ok(bytes) => Some(bytes),
        // The creation case (OUT-005): a missing A/C target is
        // written; scheme B answers it as the no-markers skip below
        // (SYS-009).
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        // A refusal is not a verdict (D-090): the file exists but
        // cannot be read — §10 files that under E_PROJECT_UNAVAILABLE
        // ("no read rights"), never fabricated as a write-access
        // fact. Unreachable through mode bits (an unreadable file is
        // read-only-flagged and skips at step 3); the arm is the
        // total-function posture for ACL-shaped refusals.
        Err(source) => {
            let message = format!("the existing target cannot be read: {source}");
            return skip_verdict(ErrorCode::ProjectUnavailable, &message, Some(&source));
        }
    };

    let (markerless, plan, existing_size) = match scheme {
        settings::Scheme::B => {
            // The scheme-B gates: UTF-8 first (upstream of the marker
            // parser, whose input is `&str`), then exactly one marker
            // pair in order (D-19/§7.5).
            let content = match existing.as_deref() {
                Some(bytes) => match std::str::from_utf8(bytes) {
                    Ok(text) => text.to_owned(),
                    Err(_) => {
                        return skip_verdict(
                            ErrorCode::NotUtf8,
                            "the scheme-B file is not valid UTF-8",
                            None,
                        );
                    }
                },
                // SYS-009: a deleted scheme-B target skips as usual —
                // the empty text has no markers, the honest refusal.
                None => String::new(),
            };
            let span = match parse_markers(&content) {
                Ok(span) => span,
                Err(error) => {
                    return skip_verdict(ErrorCode::MarkersInvalid, &error.to_string(), None);
                }
            };
            (
                splice_without_marker(&content, &span, &project.managed),
                WritePlan::Splice {
                    managed: project.managed.clone(),
                    content,
                    span,
                },
                existing.as_ref().map(|bytes| bytes.len() as u64),
            )
        }
        settings::Scheme::A | settings::Scheme::C => {
            // ALG-030's shape minus the marker line — the comparison
            // basis compare.rs itself documents for the whole-file
            // schemes (both schemes rebuild the same shape; their
            // target names already differ, and the scheme rides the
            // plan's variant, not this form).
            let markerless = if project.managed.is_empty() {
                String::new()
            } else {
                let mut form = project.managed.clone();
                form.push_str("\n\n");
                form
            };
            (
                markerless,
                WritePlan::WholeFile {
                    managed: project.managed.clone(),
                },
                existing.as_ref().map(|bytes| bytes.len() as u64),
            )
        }
    };

    // Step 5 — D-24/ALG-033: the marker line dropped from both sides,
    // the timestamp never decides.
    if skip_identical
        && let Some(bytes) = existing.as_deref()
        && contents_match(bytes, &markerless)
    {
        sweep_stale(&target, now);
        return PreflightVerdict::Unchanged;
    }

    // Step 6 — D-25 on the ready path: an empty build over a file
    // with more than zero bytes (the module docs' order ruling).
    let needs_confirmation =
        project.managed.is_empty() && existing_size.is_some_and(|size| size > 0);

    // Step 7 — the stale-tmp sweep (D-088) in the resolved folder,
    // best-effort; then the ready product.
    sweep_stale(&target, now);
    PreflightVerdict::Ready(ReadyProject {
        resolved_path: target.path().to_owned(),
        resolved_name: target.file_name().to_owned(),
        markerless,
        plan,
        existing_size,
        needs_confirmation,
    })
}

/// Step 7 for one target that reached it: delete the app's stale
/// staging files (older than an hour) keyed on the resolved name.
/// Best-effort by §9.4's own shape — the sweep is hygiene, not a
/// precondition — so neither a refused listing nor per-file failures
/// change the project's verdict. The report and its per-file
/// failures are not reported by this phase: the next sync's sweep
/// simply re-attempts whatever is left (the D-088 convergence rule);
/// a diagnostic line for them belongs to the run's logging task.
fn sweep_stale(target: &TargetPath, now: SystemTime) {
    if let Some(folder) = target.path().parent() {
        let _ = remove_stale_tmp_files(folder, target.file_name(), now);
    }
}

/// Shapes a skip from its three §10 facts: the code, the class-only
/// message (the row's `error_message`), and the structured OS
/// details when an OS call produced the refusal (§10's first-class
/// log facts, read by T-118's log line off the outcome — the store
/// row has no column for them).
fn skip_verdict(
    code: ErrorCode,
    message: &str,
    io_error: Option<&std::io::Error>,
) -> PreflightVerdict {
    PreflightVerdict::Skipped {
        code,
        message: message.to_owned(),
        details: io_error.map(ErrorDetails::from_io_error),
    }
}

/// Step 2's refusal table (D-092): each typed verdict maps to its §10
/// row. A refused probe (`NotReadable`) is not one of §9.4's own
/// verdicts — the D-090 split — so it files under
/// `E_PROJECT_UNAVAILABLE`'s "folder cannot be inspected" situation
/// with the OS facts riding in the details; the ruling is recorded
/// with this task's D-entry.
fn skip_target_refusal(error: &TargetPathError) -> PreflightVerdict {
    let code = match error {
        TargetPathError::BrokenLink { .. } => ErrorCode::TargetBrokenLink,
        TargetPathError::OutsideRoot { .. } => ErrorCode::TargetOutsideRoot,
        TargetPathError::IsDir { .. } => ErrorCode::TargetIsDir,
        TargetPathError::NotReadable { .. } => ErrorCode::ProjectUnavailable,
    };
    skip_verdict(code, &error.to_string(), error.io_error())
}

/// Step 3's refusal table (D-094): both rights verdicts are the §10
/// `E_NO_WRITE_ACCESS` fact; a probe failure ("could not check") is
/// not a rights verdict, and files under `E_PROJECT_UNAVAILABLE`'s
/// read-rights situation — most probe failures are a vanished or
/// dropped folder, which is that row's own list.
fn skip_access_refusal(error: &WriteAccessError) -> PreflightVerdict {
    let code = match error {
        WriteAccessError::FolderNotWritable { .. } | WriteAccessError::FileReadOnly { .. } => {
            ErrorCode::NoWriteAccess
        }
        WriteAccessError::ProbeFailed { .. } => ErrorCode::ProjectUnavailable,
    };
    skip_verdict(code, &error.to_string(), error.io_error())
}

/// Step 1's shape for the built-path broken invariants: the §10 row
/// whose situation ("folder moved or deleted") is nearest to a target
/// path that cannot even be asked about, with the class-only message.
fn skip_unavailable(message: &str) -> PreflightVerdict {
    skip_verdict(ErrorCode::ProjectUnavailable, message, None)
}

/// The scheme-B markerless splice (ALG-031 minus the marker line): the
/// preserved outside around the managed block in the file's dominant
/// line-break style — the marker-independent form of the very bytes
/// phase 3 will write, and ALG-033's new-side input for the scheme.
///
/// A deliberate byte-exact mirror of `eunomia_core::assembly_b`'s
/// private `build_block`/`dominant_terminator` pair (the D-130
/// `managed_text` posture: core's seam is private, so this module
/// carries its own spelling pinned against the real builder's output
/// — see the unit tests). The style counts over the preserved outside
/// only, CRLF only when strictly more frequent (ALG-031's "LF on a
/// tie"); the managed text's lines rejoin in that style with one
/// trailing break absorbed; an empty managed block leaves the markers
/// adjacent.
fn splice_without_marker(content: &str, span: &MarkerSpan, managed: &str) -> String {
    let head = &content[..span.managed_start];
    let tail = &content[span.managed_end..];
    let terminator = dominant_terminator(head, tail);
    let mut written = String::with_capacity(content.len() + managed.len());
    written.push_str(head);
    if !managed.is_empty() {
        // `lines()` semantics absorb exactly one trailing break; the
        // rejoin restores the inner ones in the file's style, and the
        // two trailing terminators are ALG-030's managed-end break
        // plus the blank line before the (here absent) marker.
        let mut lines = managed.lines();
        if let Some(first) = lines.next() {
            written.push_str(first);
            for line in lines {
                written.push_str(terminator);
                written.push_str(line);
            }
            written.push_str(terminator);
            written.push_str(terminator);
        }
    }
    written.push_str(tail);
    written
}

/// The line-break style of a scheme-B file, counted over the splice's
/// preserved outside (ALG-031; the `assembly_b` mirror): every
/// `\r\n` once as CRLF, every bare `\n` once as LF, a lone `\r` is
/// content; CRLF only when strictly ahead.
fn dominant_terminator(head: &str, tail: &str) -> &'static str {
    let (head_crlf, head_lf) = count_breaks(head.as_bytes());
    let (tail_crlf, tail_lf) = count_breaks(tail.as_bytes());
    if head_crlf + tail_crlf > head_lf + tail_lf {
        "\r\n"
    } else {
        "\n"
    }
}

/// Counts a text's line terminators (the `assembly_b::count_breaks`
/// mirror): CRLF pairs once as CRLF, bare LFs once as LF.
fn count_breaks(bytes: &[u8]) -> (usize, usize) {
    let mut crlf = 0usize;
    let mut lf = 0usize;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'\n' => {
                lf += 1;
                at += 1;
            }
            b'\r' if bytes.get(at + 1) == Some(&b'\n') => {
                crlf += 1;
                at += 2;
            }
            _ => at += 1,
        }
    }
    (crlf, lf)
}

/// The class-only refusal for a `sync.skip_identical` cell the
/// settings read cannot shape (the D-037 fallback family).
fn setting_shape_refusal() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("PreflightSettingShape".to_owned()),
        os_message: Some(
            "the sync.skip_identical setting did not read back as a boolean".to_owned(),
        ),
    });
    error
}

/// The class-only refusal for a project row phase 0 recorded that the
/// projects table no longer backs (structurally unreachable through
/// the FK; the reasoned-posture spelling as an error).
fn project_missing_refusal() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("PreflightProjectMissing".to_owned()),
        os_message: Some(
            "a project recorded by the run is missing from the projects table".to_owned(),
        ),
    });
    error
}

#[cfg(test)]
mod tests {
    //! The pure halves: the scheme-B markerless mirror against the
    //! real splice (the D-130 pin posture) and the refusal tables'
    //! every arm. The seven steps, the criteria and the database
    //! behavior live in `sync_phase1_tests.rs`, which owns its mock
    //! app harness.

    use eunomia_core::assembly_b::splice;
    use eunomia_core::markers;

    use super::*;

    /// The D-130 pin, scheme-B edition: for every fixture, splicing
    /// the real marker line and deleting that line's span must equal
    /// the mirror byte for byte — the mirror cannot drift from
    /// `assembly_b`'s private builder while this holds.
    #[test]
    fn the_markerless_mirror_is_byte_exact_against_the_real_splice() {
        const MARKER: &str =
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";
        for content in [
            // LF file, text on both sides of the block.
            "Intro.\n<!-- EUNOMIASYNC:BEGIN -->\nOld.\n<!-- EUNOMIASYNC:END -->\nTail.\n",
            // CRLF-dominant outside: the block must follow the style.
            "Intro.\r\n<!-- EUNOMIASYNC:BEGIN -->\r\nOld.\r\n<!-- EUNOMIASYNC:END -->\r\nTail.\r\n",
            // A tie (one of each): LF wins (ALG-031).
            "A\r\nB\n<!-- EUNOMIASYNC:BEGIN -->\nOld.\n<!-- EUNOMIASYNC:END -->\nC\n",
            // BOM and blank lines around the markers survive.
            "\u{feff}# H\n\n<!-- EUNOMIASYNC:BEGIN -->\nOld.\n<!-- EUNOMIASYNC:END -->\n\nBye.\n",
            // Adjacent marker lines (an empty current block).
            "Intro.\n<!-- EUNOMIASYNC:BEGIN -->\n<!-- EUNOMIASYNC:END -->\n",
        ] {
            for managed in ["New body.", "", "Line one.\nLine two.", "Trailing break.\n"] {
                let span = markers::parse(content)
                    .unwrap_or_else(|error| panic!("fixture must parse: {error}"));
                let real = splice(content, &span, managed, MARKER);
                // Delete the marker line's full span — the terminator
                // after it is the style the splice itself chose,
                // read back from the real output's own bytes.
                let marker_at = real.rfind(MARKER).expect("the marker must be spliced in");
                let after_marker = marker_at + MARKER.len();
                let terminator_len = if real[after_marker..].starts_with("\r\n") {
                    2
                } else {
                    1
                };
                let expected = format!(
                    "{}{}",
                    &real[..marker_at],
                    &real[after_marker + terminator_len..]
                );
                assert_eq!(
                    splice_without_marker(content, &span, managed),
                    expected,
                    "content={content:?} managed={managed:?}"
                );
            }
        }
    }

    /// The refusal tables' every arm, with synthetic errors: each
    /// typed fs verdict must reach its §10 code — the mapping's
    /// oracle is the §10 table itself, spelled here as literals.
    #[test]
    fn the_refusal_tables_map_every_typed_verdict_to_its_code() {
        let io = || std::io::Error::from_raw_os_error(13);
        for (error, expected) in [
            (
                TargetPathError::BrokenLink {
                    path: "/r/l".to_owned(),
                    source: io(),
                },
                ErrorCode::TargetBrokenLink,
            ),
            (
                TargetPathError::OutsideRoot {
                    root: "/r".to_owned(),
                    path: "/elsewhere/x.md".to_owned(),
                },
                ErrorCode::TargetOutsideRoot,
            ),
            (
                TargetPathError::IsDir {
                    path: "/r/AGENTS.md".to_owned(),
                },
                ErrorCode::TargetIsDir,
            ),
            (
                TargetPathError::NotReadable {
                    path: "/r/AGENTS.md".to_owned(),
                    source: io(),
                },
                // The D-090 ruling: a refusal files under the
                // unavailability row, not a target verdict.
                ErrorCode::ProjectUnavailable,
            ),
        ] {
            let PreflightVerdict::Skipped { code, .. } = skip_target_refusal(&error) else {
                panic!("a step-2 refusal must skip: {error:?}");
            };
            assert_eq!(code, expected, "error={error:?}");
        }

        for (error, expected) in [
            (
                WriteAccessError::FolderNotWritable {
                    path: "/r/AGENTS.md".to_owned(),
                    source: io(),
                },
                ErrorCode::NoWriteAccess,
            ),
            (
                WriteAccessError::FileReadOnly {
                    path: "/r/AGENTS.md".to_owned(),
                },
                ErrorCode::NoWriteAccess,
            ),
            (
                WriteAccessError::ProbeFailed {
                    path: "/r/AGENTS.md".to_owned(),
                    source: io(),
                },
                // The D-094 ruling: "could not check" is not a rights
                // verdict — the unavailability row carries it.
                ErrorCode::ProjectUnavailable,
            ),
        ] {
            let PreflightVerdict::Skipped { code, .. } = skip_access_refusal(&error) else {
                panic!("a step-3 refusal must skip: {error:?}");
            };
            assert_eq!(code, expected, "error={error:?}");
        }
    }

    /// The class-only refusals carry labels and fixed sentences,
    /// never user content (SEC-008's shape check).
    #[test]
    fn the_invariant_refusals_are_class_only() {
        for error in [setting_shape_refusal(), project_missing_refusal()] {
            assert_eq!(error.code, ErrorCode::DbLocked);
            let details = error.details.expect("the refusal carries details");
            assert_eq!(details.os_code, None);
            assert!(details.os_message.is_some());
        }
    }
}
