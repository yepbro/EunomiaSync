//! The sync pipeline's phase 0 (spec §9.4 phase 0, §5.4 SYS-004/
//! SYS-005, ALG-001; task T-112): the flag-guarded start that turns
//! "the user pressed Run sync" into the two facts every later phase
//! works from — the run record in the database, and the built
//! content of every active project.
//!
//! **The composition, in §9.4's own order.**
//!
//! 1. [`commands::begin_sync`] — §9.4 phase 0 step 1: atomically take
//!    `sync_running`; a second start while one runs gets the typed
//!    `E_SYNC_RUNNING` §7.10 envelope (SYS-004, D-123). `Ok` carries
//!    the RAII [`SyncGuard`] this module holds for the whole run and
//!    hands to its caller inside [`Phase0Outcome`]. The run's start
//!    time is captured **here**, the moment the flag is taken — the
//!    value `sync_runs.started_at` records (§9.4 phase 0 step 2's
//!    "record the start time"), so the column names the run's start,
//!    not the record's later insert.
//! 2. One **read** transaction ([`Db::run`]) reads the ALG-001 build
//!    snapshot (T-100/D-105). One transaction is not a style choice:
//!    WAL pins the transaction's view at its first read, so the
//!    snapshot is a single moment of the database — SYS-005's "a sync
//!    first builds all texts from a DB snapshot (one read
//!    transaction)" — and the serialization claim in
//!    `sync_running.rs`'s module docs (D-123) rests on this exact
//!    spelling.
//! 3. The content build — CPU over the whole library, so
//!    `spawn_blocking` **after** the transaction (SYS-001, the D-096
//!    ordering, the `dirty_recalc` precedent): per project the §9.2
//!    body, ALG-030's managed text around it (scheme A's reading
//!    lines spliced in), the §9.4 phase-1-step-2 target path, and the
//!    DB-020 build hash of it all. Built **from the snapshot value
//!    only** — that is the whole SYS-005 guarantee: an edit that
//!    commits after step 2, whatever connection commits it, cannot
//!    reach this content; it stays "dirty" for the next run
//!    (DB-021's world, D-121's service).
//! 4. One **write** transaction ([`Db::run`]) records the run:
//!    `eunomia_store::sync_runs::create_running` writes the
//!    `running` `sync_runs` row (with step 1's start time) and one
//!    `not_started` `sync_run_projects` row per project, each
//!    carrying the target path and build hash of step 3 — the
//!    fingerprint phases 1–4 and the DB-024 writer read instead of
//!    rebuilding.
//!
//! **The exit is a hand-off, not a finish.** Phase 0 writes no final
//! status: the run row stays `running`, its counters at its zeros,
//! because §9.4's verdicts (`success`/`partial`/`failed`/…) are
//! phase 4's to write (T-113+) and the crash-recovery rule turns a
//! `running` row left by a dead process into `interrupted`. What
//! phase 0 DOES guarantee on every exit — success, error, panic, a
//! cancelled task — is the flag: the [`SyncGuard`] travels inside
//! [`Phase0Outcome`] and its RAII `Drop` is §9.4 phase 4 step 5's
//! "Reset `sync_running`" whenever the pipeline stops holding it
//! (D-123). An error before the outcome exists releases the guard on
//! the way out: a dev/test panic unwinds through the same drop, and
//! in a release build (`panic = "abort"`) the abort ends the process
//! — the flag lives in process memory and dies with it, so either
//! way no stale "running" survives behind a dead pipeline (the same
//! caveat `db.rs` records for its own panic path). A failed start
//! leaves the database exactly as it was (the write transaction
//! rolled back); a process death **before** step 4 leaves no run row
//! at all — the crash-recovery rule has nothing to act on, which is
//! the honest reading of "the run never started": the row's existence
//! is the run's existence, recorded once its content is built
//! (D-130). One documented asymmetry remains (the `Db::run` contract,
//! `db.rs`'s "not cancellable" note): a **cancelled caller** — a
//! dropped `run` future — releases the guard at the drop but cannot
//! stop an in-flight write transaction; the record may still commit,
//! leaving a `running` row behind a run nobody drives. That row is
//! exactly what the startup `interrupted` recovery exists for, and
//! the future `sync_run` command (T-118) owns the cancellation
//! policy; nothing here can do better without breaking the executor's
//! own contract.
//!
//! **The build mechanics are a deliberate mirror of `dirty_recalc`
//! (D-121/D-065).** The dirty service computes the same per-project
//! managed text and hash over the same snapshot; its `managed_text`
//! and its chunked mini-snapshot batching are private, and that
//! module is a frozen zone for this task (T-102's window), so this
//! module carries its own spelling of both — pinned by the same
//! oracles (`build_content`'s real output for schemes A and C, the
//! body for B; the crossing-wires and hash pins live in
//! `sync_phase0_tests.rs`), so the two spellings cannot drift from
//! the truth even while they exist twice. Consolidating them behind
//! one shared builder is a recorded follow-up for the first task
//! allowed to touch `dirty_recalc.rs`.
//!
//! **The set-L budget mirrors D-121 too.** `build_body` rescans the
//! snapshot's assignment list per call; called naively per project,
//! set L (500 × 100,000) is the measured ~725 ms quadratic. The
//! chunked mini-snapshots (25 projects per chunk: the same items,
//! only the chunk's assignments) hold the same flat-middle budget
//! the dirty pass measured, and phase 0 — which builds every
//! project's content at every sync start — inherits the constraint.
//!
//! **Recovery mode** (no database, the T-024 state) is refused the
//! honest way: step 1 still takes the flag (the state is managed in
//! every mode), then the read step answers the recorded
//! `NoDatabase` fallback through [`commands::managed_db`] — and the
//! guard's drop releases the flag on the way out, so a syncless app
//! cannot wedge the flag behind a refusal.
//!
//! No `sync_run` command exists yet (T-118 will add it, calling
//! [`run`] as its body's first act): this module is the
//! tested-today/consumed-later seam the flag (T-105) and the
//! snapshot read (T-100) already keep.

use std::collections::HashMap;
use std::path::PathBuf;

use tauri::{AppHandle, Runtime};

use eunomia_core::backup_name::target_file_name;
use eunomia_core::body::build_body;
use eunomia_core::extra_files::{ExtraFileLines, extra_file_lines};
use eunomia_core::hash::build_hash;
use eunomia_core::project::Scheme;
use eunomia_core::snapshot::BuildSnapshot;
use eunomia_store::build_snapshot::read as read_build_snapshot;
use eunomia_store::sync_runs::{NewSyncRunProject, SyncRun, SyncRunsError};

use crate::commands::{begin_sync, managed_db};
use crate::db::{join_failure_to_app_error, sqlite_error_to_app_error};
use crate::dirty_recalc::build_snapshot_to_app_error;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::sync_running::SyncGuard;

/// One active project's phase-0 product: the content the pipeline
/// will write, where, and under which fingerprint.
///
/// The managed text is the markerless form (ALG-033's comparison
/// basis, the DB-020 hash input): the marker is a per-run wall-clock
/// line (ALG-032) phase 3 stamps once for all projects, so it
/// deliberately does not exist yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedProject {
    /// The project row id (UUIDv7 TEXT, DB-002), from the snapshot.
    pub(crate) project_id: String,
    /// The markerless managed text (ALG-030's `managed`: scheme A's
    /// reading lines spliced around the §9.2 body, B and C the body
    /// alone).
    pub(crate) managed: String,
    /// The target file path — §9.4 phase 1 step 2's spelling:
    /// `<root>/<base_name>` (A, B) or `<root>/<STEM>.project.md` (C).
    /// Phase 1 resolves symlinks on it; the stored
    /// `sync_run_projects.target_path` keeps this snapshot-derived
    /// spelling.
    pub(crate) target_path: PathBuf,
    /// The DB-020 build hash of `managed` over this project's write
    /// settings — the value DB-024 stores into
    /// `projects.last_synced_hash` after `written`/`unchanged`.
    pub(crate) build_hash: String,
}

/// What phase 0 hands the pipeline: the flag lease, the run record,
/// and the built content, in the snapshot's `sort_order`.
///
/// The guard is a field, not a side effect, so the caller (the future
/// `sync_run` command, T-118) cannot lose it between phases: it holds
/// the flag for phases 1–4 by holding this value, and whatever path
/// the pipeline takes to its end, the guard's `Drop` performs §9.4
/// phase 4 step 5.
#[derive(Debug)]
pub(crate) struct Phase0Outcome {
    /// The RAII lease on `sync_running` — §9.4 phase 4 step 5 happens
    /// when the pipeline drops it.
    pub(crate) guard: SyncGuard,
    /// The recorded run row (`running`, fresh UUIDv7 id, DB-002
    /// start, the project count, the counters at their zeros).
    pub(crate) run: SyncRun,
    /// Every active project's built content, in `sort_order`.
    pub(crate) projects: Vec<PreparedProject>,
}

/// How many projects share one scoped mini-snapshot in
/// [`build_projects`] — the batching knob of D-121's set-L
/// arithmetic, mirrored here (the flat middle its measurements
/// found; see the module docs).
const MINI_SNAPSHOT_PROJECTS: usize = 25;

/// Runs §9.4 phase 0 end to end: take the flag, read the snapshot in
/// one transaction, build every active project's content on the
/// blocking pool, record the `running` run in one transaction, and
/// return the whole state to the pipeline.
///
/// Every failure path is total: the flag is released by the guard's
/// drop (RAII, D-123), the read transaction wrote nothing, and a
/// failed write transaction rolled its inserts back — a refused or
/// failed start leaves no run row and no held flag behind
/// (idempotence by construction: nothing a retry must recognize).
///
/// # Errors
///
/// Returns the `E_SYNC_RUNNING` §7.10 envelope when a sync already
/// holds the flag ([`begin_sync`]); the honest `NoDatabase` fallback
/// in recovery mode; the classified store errors of the snapshot
/// read and the run insert ([`build_snapshot_to_app_error`],
/// [`sync_runs_to_app_error`]); and the blocking-join failure of the
/// build step.
pub(crate) async fn run<R: Runtime>(app: &AppHandle<R>) -> Result<Phase0Outcome, AppError> {
    // Step 1 — §9.4 phase 0 step 1. The guard local is the run's
    // lease from here on: every `?` below releases it on the way
    // out, and on success it moves into the outcome (never dropped
    // early, never forgotten by the caller — the field is the
    // contract). The start time is captured the moment the flag is
    // taken: `sync_runs.started_at` names the run's start, not the
    // later insert (§9.4 phase 0 step 2's "record the start time").
    let guard = begin_sync(app)?;
    let started_at = eunomia_store::values::now_utc();

    // Step 2 — the one-transaction snapshot read (SYS-005/D-105; the
    // transaction is `Db::run`'s, one `BEGIN DEFERRED` around the
    // four SELECTs).
    let snapshot = read_snapshot(app).await?;

    // Step 3 — the content build on the blocking pool, over the
    // immutable snapshot value only. `spawn_blocking` moves the value
    // in; nothing here can see the database again until step 4.
    let build = {
        let build_handle = tauri::async_runtime::spawn_blocking(move || build_projects(&snapshot));
        build_handle.await.map_err(join_failure_to_app_error)?
    };

    // Step 4 — the one-transaction record. The owned triples ride
    // the `'static` executor closure; the store's borrowed inputs are
    // built inside it.
    let inputs = build
        .iter()
        .map(|project| {
            target_path_text(&project.target_path).map(|target| {
                (
                    project.project_id.clone(),
                    target,
                    project.build_hash.clone(),
                )
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let run = record_run(app, started_at, inputs).await?;

    Ok(Phase0Outcome {
        guard,
        run,
        projects: build,
    })
}

/// §9.4 phase 0's read half, as its own step for the pipeline's (and
/// the tests') deterministic interleaving: the ALG-001 snapshot in
/// **one** `Db::run` read transaction — the SYS-005 shape; see the
/// module docs before changing the transaction boundary.
///
/// `pub(crate)`: the criterion test drives the steps around a
/// mid-run edit; production reaches it through [`run`].
pub(crate) async fn read_snapshot<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<BuildSnapshot, AppError> {
    let database = managed_db(app)?;
    database
        .run(move |transaction| {
            read_build_snapshot(transaction).map_err(build_snapshot_to_app_error)
        })
        .await
}

/// §9.4 phase 0's build half: every active project's content from the
/// snapshot alone — pure (no clock, no I/O, no database), so the
/// SYS-005 guarantee is structural: this function *cannot* observe an
/// edit that committed after the snapshot's transaction, whatever
/// connection committed it.
///
/// The projects come back in the snapshot's `sort_order` (the
/// pipeline's processing order, §9.4 phase 1's "in order"), batched
/// over scoped mini-snapshots of [`MINI_SNAPSHOT_PROJECTS`] projects
/// (D-121's set-L arithmetic, mirrored — see the module docs).
pub(crate) fn build_projects(snapshot: &BuildSnapshot) -> Vec<PreparedProject> {
    let grouped = group_assignments(snapshot);
    let mut prepared: Vec<PreparedProject> = Vec::with_capacity(snapshot.projects().len());
    for start in (0..snapshot.projects().len()).step_by(MINI_SNAPSHOT_PROJECTS) {
        prepared.extend(chunk_projects(snapshot, &grouped, start));
    }
    prepared
}

/// §9.4 phase 0's write half: the `running` run and its `not_started`
/// rows in **one** `Db::run` write transaction (`create_running`
/// joins it, D-037). `started_at` is the run's start — the value the
/// caller captured when the flag was taken (see [`run`]'s step 1);
/// `inputs` are the owned `(project_id, target_path, build_hash)`
/// triples — the executor's closure is `'static`, so the borrowed
/// store inputs are built inside it.
pub(crate) async fn record_run<R: Runtime>(
    app: &AppHandle<R>,
    started_at: String,
    inputs: Vec<(String, String, String)>,
) -> Result<SyncRun, AppError> {
    let database = managed_db(app)?;
    database
        .run(move |transaction| {
            let rows = inputs
                .iter()
                .map(|(project_id, target_path, build_hash)| NewSyncRunProject {
                    project_id,
                    target_path,
                    build_hash,
                })
                .collect::<Vec<_>>();
            eunomia_store::sync_runs::create_running(transaction, &started_at, &rows)
                .map_err(sync_runs_to_app_error)
        })
        .await
}

/// The assignment half of the batching: row indices into the
/// snapshot's assignment list, grouped by project id — one pass over
/// the assignments instead of a rescan per chunk (D-121's map,
/// mirrored verbatim).
type AssignmentsByProject = HashMap<String, Vec<usize>>;

/// Groups the snapshot's assignment row indices by project id
/// (D-121's `group_assignments`, mirrored).
fn group_assignments(snapshot: &BuildSnapshot) -> AssignmentsByProject {
    let mut grouped: AssignmentsByProject = HashMap::with_capacity(snapshot.projects().len());
    for (index, assignment) in snapshot.assignments().iter().enumerate() {
        grouped
            .entry(assignment.project_id.as_str().to_owned())
            .or_default()
            .push(index);
    }
    grouped
}

/// Builds the projects of chunk `start..` (up to
/// [`MINI_SNAPSHOT_PROJECTS`] of them): a fresh scoped mini-snapshot
/// of the chunk, then the §9.2 body, the ALG-030 managed text, the
/// §9.4 target path and the DB-020 hash per project, in chunk
/// (=`sort_order`) order (D-121's `chunk_verdicts`, mirrored with the
/// content as the product instead of the verdict).
fn chunk_projects(
    snapshot: &BuildSnapshot,
    grouped: &AssignmentsByProject,
    start: usize,
) -> Vec<PreparedProject> {
    // The last chunk may be shorter — clamp the range end, or `get`
    // would answer None and silently drop the chunk's projects.
    let end = (start + MINI_SNAPSHOT_PROJECTS).min(snapshot.projects().len());
    let Some(projects) = snapshot.projects().get(start..end) else {
        // `start` is always a chunk boundary of the sequential loop.
        return Vec::new();
    };
    // This chunk's assignments, scoped once from the grouped indices.
    let mut scoped: Vec<eunomia_core::assignment::Assignment> = Vec::new();
    for project in projects {
        if let Some(indices) = grouped.get(project.id.as_str()) {
            scoped.extend(
                indices
                    .iter()
                    .map(|&index| snapshot.assignments()[index].clone()),
            );
        }
    }
    let mini = BuildSnapshot::new(projects.to_vec(), snapshot.items().to_vec(), scoped).expect(
        "the full snapshot passed the same validation: a subset of its \
         assignments and projects over the same items cannot fail it",
    );
    projects
        .iter()
        .map(|project| {
            // The mini holds this chunk's projects — `build_body`'s
            // None is a broken invariant of this composition, the
            // reasoned-panic posture (D-061), never a silent default.
            let body = build_body(&mini, &project.id)
                .expect("the mini-snapshot holds every project of its chunk");
            let settings = &project.settings;
            // D-064's seam (the `dirty_recalc::managed_text` mirror —
            // see the module docs): the assembled managed block,
            // marker excluded, scheme A's reading lines included. The
            // argument order is project-then-local — §7.5's tie
            // order, the reverse of the settings struct's field
            // order.
            let lines = extra_file_lines(
                settings.base_name,
                settings.extra_project,
                settings.extra_local,
            );
            let managed = managed_text(settings.scheme, &body, &lines);
            // The DB-020 target path: §9.4 phase 1 step 2's spelling
            // through the one public file-name owner.
            let target_path = project
                .root
                .join(target_file_name(settings.scheme, settings.base_name));
            let build_hash = build_hash(
                settings.scheme,
                &target_path,
                settings.extra_local,
                settings.extra_project,
                &managed,
            );
            PreparedProject {
                project_id: project.id.as_str().to_owned(),
                managed,
                build_hash,
                target_path,
            }
        })
        .collect()
}

/// The markerless managed text of one project — D-064's seam, the
/// deliberate byte-exact mirror of `target_file`'s private
/// `managed_block` (D-065) and of `dirty_recalc::managed_text`
/// (D-121, a frozen zone for this task): ALG-030's parts joined with
/// one empty line between neighbours, scheme A splicing the reading
/// lines around the body, schemes B and C taking the body alone.
/// Pinned against `build_content`'s real output (the file minus its
/// marker tail) for A and C, and against the body for B, so neither
/// mirror can drift from the truth.
fn managed_text(scheme: Scheme, body: &str, lines: &ExtraFileLines) -> String {
    /// One reading-lines position as a single owned block: a
    /// position's lines are ONE part (adjacent lines joined with
    /// single breaks), and only neighbouring parts get the empty line
    /// — the same two-level split `managed_block` performs.
    fn lines_part(block: &[String]) -> Option<String> {
        (!block.is_empty()).then(|| block.join("\n"))
    }

    let mut parts: Vec<String> = Vec::with_capacity(3);
    match scheme {
        Scheme::A => {
            if let Some(start) = lines_part(&lines.start) {
                parts.push(start);
            }
            if !body.is_empty() {
                parts.push(body.to_owned());
            }
            if let Some(end) = lines_part(&lines.end) {
                parts.push(end);
            }
        }
        Scheme::B | Scheme::C => {
            if !body.is_empty() {
                parts.push(body.to_owned());
            }
        }
    }
    parts.join("\n\n")
}

/// The TEXT spelling of a built target path for the
/// `sync_run_projects.target_path` column.
///
/// Total by construction, not a hopeful conversion: the snapshot's
/// root comes from `projects.path` — TEXT, therefore UTF-8, and
/// validated UTF-8 at its write boundary (`project_add_manual`
/// rejects a non-UTF-8 path before the row exists) — and the file
/// name joined onto it is one of four static ASCII literals, so the
/// joined path is valid UTF-8 on every platform this app builds for.
/// A `None` here names a broken invariant of that chain, the
/// reasoned-panic posture (D-061) — never a lossy `to_string_lossy`
/// that would silently rewrite the stored path.
fn target_path_text(path: &std::path::Path) -> Result<String, AppError> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        // Class-only diagnostics (SEC-008): the bytes of a broken
        // path never enter the message.
        let mut app_error = AppError::new(ErrorCode::DbLocked);
        app_error.details = Some(ErrorDetails {
            os_code: None,
            os_kind: Some("TargetPathNotUtf8".to_owned()),
            os_message: Some(
                "a built target path is not valid UTF-8 — the stored \
                 project path and the file name are UTF-8 by construction"
                    .to_owned(),
            ),
        });
        app_error
    })
}

/// Maps the run repository's failures onto the §7.10 wire — the
/// D-093/D-096 boundary in the `projects_store_to_app_error` shape
/// (this module owns its mapper because `commands.rs` is a frozen
/// zone for this task; the shape is the same): SQLite failures
/// through the shared base classifier, a corrupt vocabulary cell as a
/// class-only fallback (never the stored value — SEC-008).
///
/// The `Member` arm serves the repository's **reads** (`get`,
/// `list_projects`, which phases 1–4 will route through this same
/// mapper); `create_running` — the only store call phase 0 makes —
/// cannot produce it today (it writes the literals itself), so the
/// arm is a forward seam, kept because dropping it would invite the
/// reads' future caller to hand-roll a worse one.
fn sync_runs_to_app_error(error: SyncRunsError) -> AppError {
    match error {
        SyncRunsError::Sql(source) => sqlite_error_to_app_error(source),
        SyncRunsError::Member { column, .. } => sync_fallback(
            "SyncRunsCorrupt",
            &format!("a stored {column} cell is outside its vocabulary"),
        ),
    }
}

/// Builds the D-037 fallback with a family label and a fixed,
/// class-only diagnostic — the `projects_fallback`/`dirty_fallback`
/// shape.
fn sync_fallback(kind: &str, message: &str) -> AppError {
    let mut app_error = AppError::new(ErrorCode::DbLocked);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some(kind.to_owned()),
        os_message: Some(message.to_owned()),
    });
    app_error
}

#[cfg(test)]
mod tests {
    //! The pure halves — the managed-text mirror against the real
    //! builders, and the wire mapper's arms. The database and
    //! flag behavior (the card's criterion included) lives in
    //! `sync_phase0_tests.rs`, which owns its mock-app harness.

    use eunomia_core::project::{BaseName, ExtraFile, ExtraFilePosition};
    use eunomia_core::target_file::build_content;

    use super::*;

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

    /// The D-21 reading lines for the AGENTS stem, both positions on.
    fn both_lines() -> ExtraFileLines {
        extra_file_lines(
            BaseName::AgentsMd,
            ExtraFile {
                enabled: true,
                position: ExtraFilePosition::Start,
            },
            ExtraFile {
                enabled: true,
                position: ExtraFilePosition::End,
            },
        )
    }

    const MARKER: &str = "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";

    /// The mirror's proof shape for the whole-file schemes:
    /// `build_content`'s real output must be exactly the mirror's
    /// string with the marker tail attached — drift in the join fails
    /// against the real builder (the D-065 pin, mirrored from
    /// `dirty_recalc` so the two spellings are pinned independently).
    #[test]
    fn the_managed_mirror_is_byte_exact_for_schemes_a_and_c() {
        for (scheme, lines, body) in [
            (Scheme::A, both_lines(), "Be brief."),
            (Scheme::A, both_lines(), ""),
            (Scheme::A, no_lines(), "Be brief."),
            (Scheme::C, both_lines(), "Be brief."),
            (Scheme::C, both_lines(), ""),
        ] {
            let content = build_content(scheme, body, &lines, MARKER);
            let managed = managed_text(scheme, body, &lines);
            let expected = if managed.is_empty() {
                format!("{MARKER}\n")
            } else {
                format!("{managed}\n\n{MARKER}\n")
            };
            assert_eq!(content, expected, "scheme={scheme:?} body={body:?}");
        }
    }

    #[test]
    fn the_managed_mirror_takes_the_body_alone_for_scheme_b() {
        // B has no whole-file builder to pin against
        // (`build_content` panics on B by design, D-065); the managed
        // block is the body alone (ALG-030's "(A only)" gate).
        let lines = both_lines();
        assert_eq!(managed_text(Scheme::B, "Body.", &lines), "Body.");
        assert_eq!(managed_text(Scheme::B, "", &lines), "");
    }

    /// The wire mapper's arms: SQLite through the shared classifier,
    /// the corrupt cell class-only (the stored value never enters the
    /// diagnostics, SEC-008).
    #[test]
    fn the_store_mapper_shapes_the_wire_arms() {
        let sql = sync_runs_to_app_error(SyncRunsError::Sql(rusqlite::Error::InvalidQuery));
        assert_eq!(sql.code, ErrorCode::DbLocked);

        let member = sync_runs_to_app_error(SyncRunsError::Member {
            column: "status",
            found: "paused<script>".to_owned(),
        });
        let details = member.details.expect("the Member arm carries details");
        assert_eq!(details.os_kind.as_deref(), Some("SyncRunsCorrupt"));
        assert_eq!(
            details.os_message,
            Some("a stored status cell is outside its vocabulary".to_owned()),
            "the stored value must not enter the class-only diagnostic"
        );
    }

    /// The UTF-8 spelling is the path verbatim; the broken-invariant
    /// arm is class-only (the broken bytes never enter the message).
    #[test]
    fn target_path_text_is_verbatim_and_the_refusal_is_class_only() {
        let path = PathBuf::from("/repos/demo/AGENTS.md");
        assert_eq!(
            target_path_text(&path),
            Ok("/repos/demo/AGENTS.md".to_owned())
        );
        // A non-UTF-8 path on Unix: the honest refusal, no bytes in
        // the diagnostic.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let broken = PathBuf::from(std::ffi::OsStr::from_bytes(b"/repos/\xff\xfe.md"));
            let error = target_path_text(&broken).expect_err("the broken path must be refused");
            assert_eq!(
                error.details.expect("details").os_kind.as_deref(),
                Some("TargetPathNotUtf8")
            );
        }
    }
}
