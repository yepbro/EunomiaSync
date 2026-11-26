//! The sync pipeline's phase 3 — the write (spec §9.4 phase 3, §9.5
//! through `eunomia-fs`, §9.3's ALG-030/031/032 assembly, §10's four
//! write rows, DB-024/OUT-020; task T-116): every project phase 1
//! said `ready` gets its final bytes — the run's ONE creation marker
//! around the managed text phase 0 built — laid down through
//! [`eunomia_fs::atomic::write_target_file`]'s seven-step §9.5
//! pipeline, one project at a time, in `sort_order`.
//!
//! **The marker is taken once, here** (ALG-032: "the time is taken
//! once per run, at the start of the write phase, and is the same for
//! all projects" — the T-111 criterion). The phase captures one
//! [`MarkerClock`] through its injected `clock` seam before the first
//! write and formats ONE marker line every file of the run receives
//! byte for byte; preflight could not know this line (D-131's
//! markerless plan), and phase 4 never re-derives it. Production
//! closes the seam over
//! [`now_marker_clock`](eunomia_core::marker_clock::now_marker_clock)
//! (whose own guard degrades an out-of-domain system zone to the
//! honest `UTC+00:00` form); tests hand in a fixed instant.
//!
//! **The final bytes, per [`WritePlan`].** `WholeFile` (schemes A
//! and C) is ALG-030's exact tail around the plan's managed text —
//! `managed`, one empty line, the marker, one line break, or just the
//! marker and its break when the build is empty — spelled by
//! [`whole_file`], a deliberate mirror of `build_content`'s private
//! tail (the D-130 posture: the real builder consumes body and
//! reading lines, but the plan already owns the assembled managed
//! text, so the mirror is pinned against the real builder's output in
//! the unit tests). `Splice` (scheme B) calls the REAL
//! [`assembly_b::splice`] with the plan's read `content`, parsed
//! `span` and managed text: everything outside the marker pair
//! survives byte for byte, the managed block joins in the file's
//! dominant line-break style, and the marker rides inside the fence
//! (ALG-031, OUT-016) — no mirror exists for this half because the
//! real splice IS the operation.
//!
//! **One project at a time, each with its own commit** (§9.4 phase
//! 3's own shape — the D-130/D-131/D-132 "read tx, blocking pass,
//! write tx" composition deliberately does NOT apply):
//!
//! 1. ONE read transaction up front: every `ready` project's DB-020
//!    build hash from the run's own `sync_run_projects` rows — the
//!    fingerprint phase 0 froze (D-130: phases 1–4 and the DB-024
//!    writer read it instead of rebuilding), and the exact value
//!    DB-024 stores into `projects.last_synced_hash`. It is NOT the
//!    SHA-256 of the written bytes: those carry the marker, D-24 and
//!    ALG-033 compare the markerless form, and the stored hash must
//!    be what the next comparison recomputes against (the ruling this
//!    task's D-entry fixes).
//! 2. Per project, in order: the pure byte assembly, then the write
//!    on `spawn_blocking` (SYS-001 — §9.5 is five-plus syscalls over
//!    possibly dead volumes) through the injected `write` seam, then
//!    ONE short write transaction for the verdict:
//!     * success — [`mark_project_written`] plus
//!       [`projects::record_last_synced`] (the run's ONE DB-002
//!       instant as `last_synced_at`, the run's build hash as
//!       `last_synced_hash`), both in that one transaction so a
//!       `written` row can never commit without its stamp (§9.4
//!       phase 3 step 2's "right away in a separate short
//!       transaction" read as one commit per project — "separate"
//!       from the file write, which is not transactional, and from
//!       every other project's verdict);
//!     * write error — [`mark_project_failed`] with the §10 code and
//!       a class-only message, and the loop TAKES THE NEXT PROJECT:
//!       a failed write stops one project, never the run (§9.4 phase
//!       3 step 3). The backup stays (nothing here touches it —
//!       phase 2's copies and records are history the moment they
//!       committed), and the project's `last_synced_*` cells keep
//!       their previous values (OUT-020).
//!
//!    The whole per-project unit — the §9.5 write AND its verdict
//!    transaction — is announced on the managed
//!    [`crate::commands::SyncWriteGate`] (T-123, D-219 point 5): an
//!    RAII guard opens at the unit's top and drops at its end, so the
//!    §9.4 "Closing the window during a sync" flow's confirmed exit
//!    can wait for exactly this unit ("The app finishes the current
//!    atomic write and then exits"). The gate is read here through
//!    the panicking `app.state` — `setup` manages it beside the
//!    cancel slot in every mode, so a missing gate is an
//!    uninitialized-app bug that must be loud.
//!
//! **The §10 code table.** [`write_code`] maps the atomic pipeline's
//! typed verdicts arm by arm: `WriteFailed`/`NameFailed` →
//! `E_WRITE_FAILED` (the D-099 fold — a staging-name refusal is a
//! contract violation, not a §10 row of its own), `DiskFull` →
//! `E_DISK_FULL` (§10's write-time half: project `failed`, not the
//! phase-1 run abort), `FileLocked` → `E_FILE_LOCKED`, `VerifyFailed`
//! → `E_VERIFY_FAILED`. The failed row's message is the typed error's
//! `Display` — paths, steps and OS facts, never file content
//! (SEC-008) — and the outcome carries the structured OS details for
//! T-118's log line (the row has no column for them, the
//! [`PreflightVerdict::Skipped`] shape).
//!
//! **What the phase does NOT decide.** The run row stays `running`:
//! §9.4's `success`/`partial`/`failed` verdict and the counters are
//! phase 4's (T-117), and a `running` row left by a dead process is
//! the startup `interrupted` recovery's case. Cancellation is not
//! consulted here either: after phase 2 succeeds the run cannot be
//! canceled (§9.4 phase 2 step 3), so no token exists to check — the
//! `sync_running` guard (D-123) stays up in the caller's
//! [`Phase0Outcome`] through this phase and past phase 4's reset,
//! exactly as D-130 handed it over. And the D-25 confirmation gate
//! is the caller's precondition, not this phase's: the loop writes
//! EVERY `ready` project, including one whose empty build over a
//! non-empty file is the destructive case D-25 demands a confirmed
//! `confirm_empty` for — the `sync_run` command (T-118) resolves
//! [`Phase1Outcome::needs_confirmation`] before it gets here.
//!
//! **Store-failure postures** (the D-131/D-132 family). A failed
//! verdict transaction propagates the store error and stops the
//! phase: the run row stays `running`, the crash-recovery rule's
//! case. On the success path the FILE is already written when its
//! transaction refuses — the file stays (wholly the new content,
//! §9.5's old-or-new; the frozen pipeline never deletes a written
//! target — its `VerifyFailed` docs state the no-delete posture,
//! and a successful write's bytes are the content the run meant to
//! put there), the row stays `not_started`, the error propagates,
//! and the recorded residual window is the mirror of D-132's
//! orphaned copies: the next run's phase 1 sees the file's content
//! (ALG-033 ignores the marker line) and the startup recovery
//! recalculates hashes over what is on disk. Idempotence is by
//! convergence, the §9.5 way: a retried run is a new run whose
//! phase 1 finds the file already matching (`unchanged`) or writes
//! it whole again.

use std::collections::HashMap;
use std::path::Path;

use jiff::Timestamp;
use tauri::{AppHandle, Manager, Runtime};

use eunomia_core::assembly_b::splice;
use eunomia_core::marker_clock::MarkerClock;
use eunomia_fs::atomic::{AtomicWriteError, WrittenFile};
use eunomia_store::projects;
use eunomia_store::sync_runs::{list_projects, mark_project_failed, mark_project_written};

use crate::commands::{managed_db, projects_store_to_app_error};
use crate::db::{join_failure_to_app_error, sync_runs_to_app_error};
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::sync_phase1::{Phase1Outcome, PreflightVerdict, WritePlan};

/// The phase's product for one written or failed project.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WriteOutcome {
    /// §9.4 phase 3 step 2: the file on disk holds the final bytes
    /// and passed §9.5 step 6's re-read; the row is `written` and
    /// the project's last-sync facts are stamped.
    Written {
        /// The written content's size in bytes (marker included).
        size: u64,
        /// The written content's SHA-256 — the digest §9.5 step 6
        /// confirmed against the disk, 64 lowercase hex characters.
        /// NOT the DB-020 build hash (that is the markerless form's
        /// fingerprint, the one `last_synced_hash` stores).
        sha256_hex: String,
    },
    /// §9.4 phase 3 step 3: the write refused; the row is `failed`
    /// with the §10 code and message, the backup stays, and the run
    /// took the next project.
    Failed {
        /// The §10 code of the refusal (stored as its `"E_*"`
        /// literal).
        code: ErrorCode,
        /// The refusal's Display text (paths, steps and OS codes
        /// only, never file content — SEC-008).
        message: String,
        /// The structured OS facts behind the refusal, when an OS
        /// call produced it (`io_error()` → `from_io_error`) — §10's
        /// first-class log facts, read by T-118's log line off the
        /// outcome (the store row has no column for them).
        details: Option<ErrorDetails>,
    },
}

/// What phase 3 hands the pipeline: the run's id and one outcome per
/// `ready` project, in the snapshot's `sort_order` (§9.4 phase 3's
/// "projects are processed one by one, in order").
#[derive(Debug)]
pub(crate) struct Phase3Outcome {
    /// The run these outcomes belong to (the `sync_runs` id).
    pub(crate) run_id: String,
    /// One `(project_id, outcome)` pair per `ready` project of the
    /// run, in `sort_order`; empty when phase 1 readied nothing (the
    /// run continues straight to phase 4).
    pub(crate) results: Vec<(String, WriteOutcome)>,
}

/// Runs §9.4 phase 3 end to end over phase 1's outcome (phase 2 is
/// assumed to have succeeded — §9.4 phase 2 step 3's point of no
/// return): capture the run's one marker clock, read the run's
/// build-hash fingerprints in one transaction, then per `ready`
/// project in order — final bytes, atomic write on the blocking pool,
/// one short verdict transaction.
///
/// `write` is the §9.5 operation (the D-089/phase-2 seam):
/// production passes [`production_write`] (which closes over
/// [`eunomia_fs::atomic::write_target_file`] with the real free-space
/// probe); the tests inject the typed refusals of §10's write rows.
/// `clock` is the marker's one capture — production passes
/// [`now_marker_clock`](eunomia_core::marker_clock::now_marker_clock);
/// tests fix the instant (and an empty `ready` list never captures
/// one at all). `now` is the run's write instant (D-061): the ONE
/// [`Timestamp`] every written project's `last_synced_at` derives
/// from, in the DB-002 spelling.
///
/// # Errors
///
/// - The honest `NoDatabase` fallback in recovery mode, and the
///   classified store errors of the fingerprint read or of any
///   verdict transaction — a failed transaction leaves the run row
///   `running` (the crash-recovery rule's case; on the success path
///   the already-written FILE stays, wholly the new content).
/// - NEVER a per-project write refusal: §9.4 phase 3 step 3 makes
///   those `failed` rows the run walks past, carried in the outcome.
pub(crate) async fn run<R, W, C>(
    app: &AppHandle<R>,
    phase1: &Phase1Outcome,
    write: W,
    clock: C,
    now: Timestamp,
) -> Result<Phase3Outcome, AppError>
where
    R: Runtime,
    W: Fn(&Path, &[u8]) -> Result<WrittenFile, AtomicWriteError> + Send + Sync + 'static,
    C: FnOnce() -> MarkerClock + Send + 'static,
{
    let database = managed_db(app)?;

    // Step A — the write list: the `ready` verdicts, in `sort_order`.
    // `skipped`/`unchanged` projects never reach this phase's loop
    // (their verdicts are already rows); an empty list writes nothing
    // and captures no marker — §9.4 has no step for "nothing to
    // write", and zero writes cannot fail (the phase-2 ruling).
    let ready = phase1
        .projects
        .iter()
        .filter_map(|(project_id, verdict)| match verdict {
            PreflightVerdict::Ready(ready) => Some((project_id.as_str(), ready)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if ready.is_empty() {
        return Ok(Phase3Outcome {
            run_id: phase1.run_id.clone(),
            results: Vec::new(),
        });
    }

    // Step B — ONE read transaction: the run's own build-hash
    // fingerprint per `ready` project (D-130's "read the fingerprint
    // instead of rebuilding"). The join answers in `sort_order`; the
    // map keeps the lookup honest when `skipped`/`unchanged` rows
    // interleave.
    let expected_projects = ready.len();
    let run_id = phase1.run_id.clone();
    let fingerprints = database
        .run(move |transaction| {
            let mut hashes = HashMap::with_capacity(expected_projects);
            for row in list_projects(transaction, &run_id).map_err(sync_runs_to_app_error)? {
                hashes.insert(row.project_id, row.build_hash);
            }
            Ok(hashes)
        })
        .await?;

    // The precondition pass (the round-1 P3-2 ordering): every
    // `ready` project's fingerprint must exist BEFORE the first byte
    // moves — a broken phase-0 chain is a precondition of the whole
    // write list, not of whichever project trips over it mid-loop,
    // and refusing here keeps "zero writes when the phase cannot
    // proceed" (the empty-list posture) instead of a half-processed
    // run. The in-loop lookup then rides the reasoned-panic posture
    // (D-061): the pre-pass proved the fact.
    if ready
        .iter()
        .any(|(project_id, _)| fingerprints.get(*project_id).is_none_or(Option::is_none))
    {
        return Err(fingerprint_missing_refusal());
    }

    // Step C — the run's ONE marker (ALG-032/T-111): captured at the
    // start of the write phase, formatted once, identical in every
    // file this run writes.
    let marker_line = clock().marker_line();
    let last_synced_at = db002_stamp(now);

    // Step D — the per-project loop. One project, one write, one
    // short transaction; a refusal is a row, never a stop. The write
    // operation rides an `Arc`: every project's `spawn_blocking`
    // closure is `'static`, and the operation must survive them all.
    // The write-in-flight gate's guard (T-123, D-219 point 5) opens
    // HERE, at the unit's top: its region is the WHOLE per-project
    // unit — the blocking-pool §9.5 write AND the verdict/stamp
    // transaction — the region §9.4's closing paragraph protects
    // ("The app finishes the current atomic write and then exits";
    // Rule 7: the guard lives across every `.await` of the unit, and
    // its drop at the iteration's end is what the confirmed exit's
    // `SyncWriteGate::wait_quiet` waits for). The gate is managed by
    // `setup` beside the cancel slot (the panicking lookup: a wiring
    // bug must be loud).
    let write_gate = app.state::<crate::commands::SyncWriteGate>();
    let write = std::sync::Arc::new(write);
    let mut results = Vec::with_capacity(ready.len());
    for (project_id, ready_project) in ready {
        let _write_in_flight = write_gate.enter();
        // The fingerprint this project's stamp will store — the run's
        // own DB-020 value, not a recomputation. The pre-pass above
        // proved it exists; the expect is the reasoned-panic posture
        // over that proof (D-061).
        let build_hash = fingerprints
            .get(project_id)
            .cloned()
            .flatten()
            .expect("the pre-pass validated every ready project's fingerprint");

        // The final bytes (pure): ALG-030's tail or ALG-031's splice
        // around the ONE marker line.
        let bytes = final_bytes(&ready_project.plan, &marker_line).into_bytes();

        // The write on the blocking pool (SYS-001: §9.5 is a run of
        // syscalls; nothing here may hold the database lock).
        let target = ready_project.resolved_path.clone();
        let project_write = std::sync::Arc::clone(&write);
        let write_handle =
            tauri::async_runtime::spawn_blocking(move || (*project_write)(&target, &bytes));
        let written = write_handle
            .await
            .map_err(join_failure_to_app_error)?
            .map_err(|error| failure_of(&error));

        match written {
            Ok(file) => {
                // §9.4 phase 3 step 2: `written` plus the project's
                // last-sync facts, one short transaction, right away.
                // The owned inputs ride the `'static` executor
                // closure (the phase-0 `record_run` shape).
                let tx_run_id = phase1.run_id.clone();
                let tx_project_id = project_id.to_owned();
                let tx_at = last_synced_at.clone();
                database
                    .run(move |transaction| {
                        mark_project_written(transaction, &tx_run_id, &tx_project_id)
                            .map_err(sync_runs_to_app_error)?;
                        projects::record_last_synced(
                            transaction,
                            &tx_project_id,
                            &tx_at,
                            &build_hash,
                        )
                        .map_err(projects_store_to_app_error)?;
                        Ok(())
                    })
                    .await?;
                results.push((
                    project_id.to_owned(),
                    WriteOutcome::Written {
                        size: file.size,
                        sha256_hex: file.sha256_hex,
                    },
                ));
            }
            Err(failure) => {
                // §9.4 phase 3 step 3: `failed` with the code and
                // message; the loop takes the next project. The
                // backup stays (phase 2's committed history) and the
                // project's `last_synced_*` keeps its previous value
                // (OUT-020) — neither is this arm's to touch.
                let tx_run_id = phase1.run_id.clone();
                let tx_project_id = project_id.to_owned();
                let tx_code = failure.code.as_str().to_owned();
                let tx_message = failure.message.clone();
                database
                    .run(move |transaction| {
                        mark_project_failed(
                            transaction,
                            &tx_run_id,
                            &tx_project_id,
                            &tx_code,
                            &tx_message,
                        )
                        .map_err(sync_runs_to_app_error)?;
                        Ok(())
                    })
                    .await?;
                results.push((
                    project_id.to_owned(),
                    WriteOutcome::Failed {
                        code: failure.code,
                        message: failure.message,
                        details: failure.details,
                    },
                ));
            }
        }
    }

    Ok(Phase3Outcome {
        run_id: phase1.run_id.clone(),
        results,
    })
}

/// The production §9.5 operation (the `write` seam's caller-side
/// half): [`write_target_file`] itself with the real free-space probe
/// of the target's folder — the D-089 closure shape `atomic`'s docs
/// prescribe. `pub(crate)`: the `sync_run` command (T-118) passes it.
pub(crate) fn production_write(
    target: &Path,
    contents: &[u8],
) -> Result<WrittenFile, AtomicWriteError> {
    // One code path, no dead arm (the round-1 P3-6): the frozen
    // pipeline refuses a parent-less target BEFORE it consults the
    // probe (its own no-parent check), so the `unwrap_or(target)`
    // fallback's only job is handing the closure a path — never
    // stat'ed for a target the pipeline refuses anyway (and
    // unreachable for the resolved targets of D-092, which are
    // absolute).
    let folder = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(target)
        .to_owned();
    eunomia_fs::atomic::write_target_file(target, contents, || {
        eunomia_fs::free_space::free_bytes(&folder)
    })
}

/// The final bytes of one write plan around the run's marker line:
/// ALG-030's whole-file tail for schemes A and C, the REAL ALG-031
/// splice for scheme B (the module docs' split — no mirror for the
/// half the real builder owns).
fn final_bytes(plan: &WritePlan, marker_line: &str) -> String {
    match plan {
        WritePlan::WholeFile { managed } => whole_file(managed, marker_line),
        WritePlan::Splice {
            managed,
            content,
            span,
        } => splice(content, span, managed, marker_line),
    }
}

/// ALG-030's whole-file form: `managed`, one empty line, the marker,
/// one line break — or, when the build is empty, just the marker and
/// its break. A deliberate mirror of `build_content`'s private tail
/// (the D-130 posture): the plan already owns the assembled managed
/// text, so only the tail is spelled here — pinned against the real
/// builder's own output in the unit tests so the two cannot drift.
fn whole_file(managed: &str, marker_line: &str) -> String {
    if managed.is_empty() {
        format!("{marker_line}\n")
    } else {
        format!("{managed}\n\n{marker_line}\n")
    }
}

/// The §10 code of one refused atomic write (the D-099 table):
/// `WriteFailed`/`NameFailed` fold into `E_WRITE_FAILED`, and the
/// three typed verdicts of their own rows — `E_DISK_FULL` (the
/// write-time half), `E_FILE_LOCKED`, `E_VERIFY_FAILED`.
fn write_code(error: &AtomicWriteError) -> ErrorCode {
    match error {
        AtomicWriteError::NameFailed { .. } | AtomicWriteError::WriteFailed(_) => {
            ErrorCode::WriteFailed
        }
        AtomicWriteError::DiskFull(_) => ErrorCode::DiskFull,
        AtomicWriteError::FileLocked(_) => ErrorCode::FileLocked,
        AtomicWriteError::VerifyFailed(_) => ErrorCode::VerifyFailed,
    }
}

/// One refused write's row payload: the code, the class-only Display
/// message (paths, steps, OS facts — SEC-008), and the structured OS
/// details for the outcome's log consumer.
struct WriteFailure {
    code: ErrorCode,
    message: String,
    details: Option<ErrorDetails>,
}

/// Shapes a typed atomic refusal into its row payload (the
/// [`PreflightVerdict::Skipped`] three-fact shape, spelled for the
/// write phase).
fn failure_of(error: &AtomicWriteError) -> WriteFailure {
    WriteFailure {
        code: write_code(error),
        message: error.to_string(),
        details: error.io_error().map(ErrorDetails::from_io_error),
    }
}

/// The DB-002 UTC millisecond stamp of the run's write instant — the
/// exact `values::now_utc` spelling (`%Y-%m-%dT%H:%M:%S%.3fZ`,
/// truncating), mirrored here because the store helper reads the real
/// clock and this module's contract is the injected instant (D-061;
/// the phase-2 `db002_stamp` twin, pinned the same way).
fn db002_stamp(now: Timestamp) -> String {
    now.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// The class-only refusal for a `ready` project whose run row holds
/// no DB-020 fingerprint: phase 0's writer records one per project
/// and the FK keeps the row alive, so a miss names a broken invariant
/// of that chain (the phase-1 `project_missing_refusal` family).
fn fingerprint_missing_refusal() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("Phase3FingerprintMissing".to_owned()),
        os_message: Some(
            "a project readied by the run has no build hash in its sync_run_projects row"
                .to_owned(),
        ),
    });
    error
}

#[cfg(test)]
mod tests {
    //! The pure halves — the ALG-030 tail mirror against the real
    //! builder, the §10 code table, and the DB-002 stamp. The phase
    //! behavior (the marker pin, the per-project loop, the verdict
    //! rows, the stamps) lives in `sync_phase3_tests.rs`, which owns
    //! its mock app harness.

    use eunomia_core::extra_files::{ExtraFileLines, extra_file_lines};
    use eunomia_core::project::{BaseName, ExtraFile, ExtraFilePosition, Scheme};
    use eunomia_core::target_file::build_content;

    use super::*;

    /// The ALG-032 example marker (the suite-wide fixture shape).
    const MARKER: &str = "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";

    /// The no-lines value of a project with both options disabled.
    fn no_lines() -> ExtraFileLines {
        extra_file_lines(
            BaseName::AgentsMd,
            ExtraFile {
                enabled: false,
                position: ExtraFilePosition::End,
            },
            ExtraFile {
                enabled: false,
                position: ExtraFilePosition::End,
            },
        )
    }

    /// The mirror's proof shape (the D-065/D-130 pin): for the same
    /// managed text the real whole-file builder produces, the mirror
    /// must produce byte for byte the same file — non-empty and empty
    /// builds both, so the empty-build arm ("just the marker") is the
    /// real builder's own output, not a second opinion.
    #[test]
    fn the_whole_file_tail_is_byte_exact_against_the_real_builder() {
        for (scheme, body, lines) in [
            (Scheme::A, "Be brief.", no_lines()),
            (Scheme::A, "", no_lines()),
            (Scheme::C, "Be brief.", no_lines()),
            (Scheme::C, "", no_lines()),
        ] {
            let content = build_content(scheme, body, &lines, MARKER);
            // The managed text is the real builder's own input half —
            // read back from its output by dropping the tail, so the
            // pin compares the mirror against the builder over the
            // builder's own assembly, not a reimplementation of it.
            let managed = content
                .strip_suffix(&format!("\n\n{MARKER}\n"))
                .or_else(|| content.strip_suffix(&format!("{MARKER}\n")))
                .unwrap_or_else(|| panic!("the builder's tail must be present: {content:?}"));
            assert_eq!(
                whole_file(managed, MARKER),
                content,
                "scheme={scheme:?} body={body:?}"
            );
        }
    }

    /// The §10 table, arm by arm, with synthetic errors: each typed
    /// atomic verdict must reach its §10 code — the oracle is §10's
    /// own table, spelled here as literals.
    #[test]
    fn every_atomic_error_maps_to_its_code() {
        let io = || std::io::Error::from_raw_os_error(28);
        let write_failed = |step| {
            AtomicWriteError::WriteFailed(eunomia_fs::atomic::WriteFailed {
                step,
                path: std::path::PathBuf::from("/r/AGENTS.md"),
                source: io(),
                cleanup: None,
            })
        };
        for (error, expected) in [
            (
                write_failed(eunomia_fs::atomic::WriteStep::TempCreate),
                ErrorCode::WriteFailed,
            ),
            (
                write_failed(eunomia_fs::atomic::WriteStep::Replace),
                ErrorCode::WriteFailed,
            ),
            (
                AtomicWriteError::NameFailed {
                    source: eunomia_fs::temp_files::TmpNameError::InvalidTargetName,
                },
                // The D-099 fold: a staging-name refusal is a contract
                // violation without a §10 row of its own.
                ErrorCode::WriteFailed,
            ),
            (
                AtomicWriteError::DiskFull(eunomia_fs::atomic::DiskFullFailure {
                    path: std::path::PathBuf::from("/r/AGENTS.md"),
                    step: None,
                    free: Some(2),
                    required: 3,
                    source: None,
                    cleanup: None,
                }),
                ErrorCode::DiskFull,
            ),
            (
                AtomicWriteError::FileLocked(eunomia_fs::atomic::FileLockedFailure {
                    step: eunomia_fs::atomic::WriteStep::Replace,
                    path: std::path::PathBuf::from("/r/AGENTS.md"),
                    retries: 3,
                    source: io(),
                    cleanup: None,
                }),
                ErrorCode::FileLocked,
            ),
            (
                AtomicWriteError::VerifyFailed(eunomia_fs::atomic::VerifyFailed {
                    path: std::path::PathBuf::from("/r/AGENTS.md"),
                    expected_sha256_hex: "0".repeat(64),
                    mismatch: eunomia_fs::atomic::VerifyMismatch::Digest {
                        actual_sha256_hex: "f".repeat(64),
                    },
                }),
                ErrorCode::VerifyFailed,
            ),
        ] {
            assert_eq!(write_code(&error), expected, "error={error:?}");
        }
    }

    /// The DB-002 stamp: the exact 24-character UTC millisecond
    /// spelling, truncated (not rounded) — the `values::now_utc`
    /// contract mirrored (the phase-2 pin, spelled for this module's
    /// own twin).
    #[test]
    fn the_db002_stamp_is_the_values_spelling() {
        let now: Timestamp = "2026-11-21T10:30:00.123Z"
            .parse()
            .expect("the fixed instant must parse");
        assert_eq!(db002_stamp(now), "2026-11-21T10:30:00.123Z");
        // Truncation, not rounding: a fourth fractional digit drops.
        let extra: Timestamp = "2026-11-21T10:30:00.1239Z"
            .parse()
            .expect("the four-digit instant must parse");
        assert_eq!(db002_stamp(extra), "2026-11-21T10:30:00.123Z");
    }

    /// The invariant refusals are class-only (SEC-008's shape check):
    /// the label and a fixed sentence, no bytes of any path.
    #[test]
    fn the_invariant_refusal_is_class_only() {
        let error = fingerprint_missing_refusal();
        assert_eq!(error.code, ErrorCode::DbLocked);
        let details = error.details.expect("the refusal carries details");
        assert_eq!(details.os_code, None);
        assert_eq!(details.os_kind.as_deref(), Some("Phase3FingerprintMissing"));
        assert!(details.os_message.is_some());
    }
}
