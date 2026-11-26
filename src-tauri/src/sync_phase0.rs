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
//!    rebuilding. The same transaction opens the run's journal
//!    bracket (T-124/D-223): one `sync.run_started` entry, info/
//!    user/ok, no details, riding the row's own insert — a run row
//!    that never committed never claimed a start, and a refused
//!    journal insert rolls the whole record back (the D-194/D-205
//!    one-transaction shape, trigger-proven in the behavior suite).
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
//! the `sync_run` command (T-118/D-138) owns the cancellation
//! policy; nothing here can do better without breaking the executor's
//! own contract.
//!
//! **The build mechanics are the shared core pass (T-265/D-145).**
//! Step 3's per-project content — the §9.2 body, ALG-030's managed
//! text around it, the §9.4 target path, the DB-020 hash — is
//! [`eunomia_core::content_build::build_projects`]: the chunked
//! mini-snapshot pass this module once mirrored from `dirty_recalc`
//! (D-121/D-130) now lives once in `eunomia-core`, with its verdict
//! semantics pinned by that crate's own suite. What stays here is the
//! phase's composition: the transactions, the run record, the
//! `PreparedProject` alias the later phases consume.
//!
//! **The set-L budget is the shared pass's.** `build_body` rescans
//! the snapshot's assignment list per call; called naively per
//! project, set L (500 × 100,000) is the measured ~725 ms quadratic.
//! The chunked mini-snapshots of the shared pass (25 projects per
//! chunk: the same items, only the chunk's assignments) hold the
//! flat-middle budget the dirty pass measured (D-121), and phase 0 —
//! which builds every project's content at every sync start —
//! inherits the constraint through it.
//!
//! **Recovery mode** (no database, the T-024 state) is refused the
//! honest way: step 1 still takes the flag (the state is managed in
//! every mode), then the read step answers the recorded
//! `NoDatabase` fallback through [`commands::managed_db`] — and the
//! guard's drop releases the flag on the way out, so a syncless app
//! cannot wedge the flag behind a refusal.
//!
//! The `sync_run` command (T-118/D-138) calls [`run`] as the
//! pipeline's first phase — after the cancel-token publication
//! (T-258's step 0) — so this module is a consumed seam, like the
//! flag (T-105) and the snapshot read (T-100) before it.

use tauri::{AppHandle, Runtime};

use eunomia_core::content_build::BuiltProject;
use eunomia_core::snapshot::BuildSnapshot;
use eunomia_store::build_snapshot::read as read_build_snapshot;
use eunomia_store::sync_runs::{NewSyncRunProject, SyncRun};

use crate::commands::{begin_sync, managed_db};
use crate::db::{join_failure_to_app_error, sync_runs_to_app_error};
use crate::dirty_recalc::build_snapshot_to_app_error;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::sync_running::SyncGuard;

/// One active project's phase-0 product: the content the pipeline
/// will write, where, and under which fingerprint — the shared core
/// pass's [`BuiltProject`] under the name this module's consumers
/// (the later phases, the tests) grew up with (T-265/D-145: the
/// struct this module once spelled itself dissolved into
/// `eunomia-core`'s).
///
/// The managed text is the markerless form (ALG-033's comparison
/// basis, the DB-020 hash input): the marker is a per-run wall-clock
/// line (ALG-032) phase 3 stamps once for all projects, so it
/// deliberately does not exist yet.
pub(crate) type PreparedProject = BuiltProject;

/// What phase 0 hands the pipeline: the flag lease, the run record,
/// and the built content, in the snapshot's `sort_order`.
///
/// The guard is a field, not a side effect, so the caller (the
/// `sync_run` command, T-118/D-138) cannot lose it between phases: it holds
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
/// [`crate::db::sync_runs_to_app_error`]); and the blocking-join
/// failure of the build step.
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
/// snapshot alone — [`eunomia_core::content_build::build_projects`],
/// the shared chunked pass (T-265/D-145), pure (no clock, no I/O, no
/// database), so the SYS-005 guarantee is structural: this function
/// *cannot* observe an edit that committed after the snapshot's
/// transaction, whatever connection committed it.
///
/// The projects come back in the snapshot's `sort_order` (the
/// pipeline's processing order, §9.4 phase 1's "in order"), batched
/// over scoped mini-snapshots of
/// [`MINI_SNAPSHOT_PROJECTS`](eunomia_core::content_build::MINI_SNAPSHOT_PROJECTS)
/// projects per chunk (D-121's set-L arithmetic — see the module
/// docs).
pub(crate) fn build_projects(snapshot: &BuildSnapshot) -> Vec<PreparedProject> {
    eunomia_core::content_build::build_projects(snapshot)
}

/// §9.4 phase 0's write half: the `running` run and its `not_started`
/// rows in **one** `Db::run` write transaction (`create_running`
/// joins it, D-037). `started_at` is the run's start — the value the
/// caller captured when the flag was taken (see [`run`]'s step 1);
/// `inputs` are the owned `(project_id, target_path, build_hash)`
/// triples — the executor's closure is `'static`, so the borrowed
/// store inputs are built inside it.
///
/// The run's journal bracket opener (T-124/D-223) rides the same
/// transaction: one `sync.run_started` entry — info/user/ok, the
/// entity naming the run row, no project attribution and no details
/// (the run row itself carries `started_at` and the counts;
/// LOG-001…004's time/operation/object/result quadruple is the whole
/// entry). A refused insert fails the transaction wholesale, so a
/// start entry cannot exist for a run row that does not.
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
            let run = eunomia_store::sync_runs::create_running(transaction, &started_at, &rows)
                .map_err(sync_runs_to_app_error)?;
            // The bracket opener, after the row it describes: the
            // D-051 call-once mirror fires inside the closure (the
            // phase-4 posture), and the info/user/ok shape is the
            // unmirrored one — no file-log line per run start.
            let mut entry = eunomia_store::action_log::LogEntry::new(
                eunomia_store::action_log::Action::SyncRunStarted,
                eunomia_store::action_log::Level::Info,
                eunomia_store::action_log::Visibility::User,
                eunomia_store::action_log::Outcome::Ok,
            );
            entry.entity = Some(eunomia_store::action_log::Entity {
                kind: eunomia_store::action_log::EntityType::SyncRun,
                id: run.id.as_str(),
            });
            entry.sync_run_id = Some(run.id.as_str());
            eunomia_store::action_log::insert(transaction, &entry)
                .map_err(crate::commands::activity_log_to_app_error)?;
            crate::logging::action_logged(&entry);
            Ok(run)
        })
        .await
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

#[cfg(test)]
mod tests {
    //! The module's own pure half — the target-path TEXT spelling
    //! (the managed-text seam moved to `eunomia_core::target_file`'s
    //! suite when this module's mirror dissolved, T-265/D-145; the
    //! wire mapper's arms are pinned at the one mapper's home,
    //! `db_tests.rs`, since T-264/D-149 consolidated the copies). The phase
    //! behavior (the card's criterion, the flag, the records, the
    //! chunk boundary) lives in `sync_phase0_tests.rs`, which owns
    //! its mock-app harness.

    use std::path::PathBuf;

    use super::target_path_text;

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
