//! Behavior tests for the `sync_runs` repositories (spec §6.2 DDL,
//! §9.4 phase 0 step 2 and phase 1's transitions; tasks T-112/T-113).
//!
//! The tests run against a real database opened through `open_db` and
//! migrated with `run_migrations` — the production configuration
//! (DB-001 pragmas, foreign keys included, D-030) — seeded through
//! the repositories' public API and read back through this module's
//! own reads:
//!
//! * the phase-0 row shape: `create_running` writes the `running`
//!   run (fresh UUIDv7 id, DB-002 `started_at`, the project count in
//!   `projects_total`, the counters at their §6.2 zeros, no
//!   `finished_at`/`error_code`/`details`) and one `not_started`
//!   per-project row per input, each carrying its target path and
//!   build hash verbatim;
//! * the joined-transaction contract (D-037): wrapped in one
//!   transaction, a per-project row the `projects` foreign key
//!   refuses rolls the already-inserted run row back with it — zero
//!   rows survive, the atomicity §9.4's pipeline depends on;
//! * the reads future phases consume: `get` round-trips the row
//!   (and answers `None` for an unknown id), `list_projects` returns
//!   the run's rows in the projects' `sort_order`;
//! * phase 1's transitions (T-113): `mark_project_skipped` writes
//!   the §10 code and message over an intact fingerprint,
//!   `mark_project_unchanged` clears the error columns with the
//!   status, `abort_run` finishes the run `aborted` without touching
//!   the counters or the per-project rows — and an update that
//!   matches no row is the typed refusal, never a silent success;
//! * phase 2's abort reset (T-115): `reset_projects_not_started`
//!   returns every verdict but `skipped` to `not_started` with the
//!   error columns cleared and the fingerprints intact — a bulk
//!   sweep whose zero-row answer is `Ok(0)`, not the per-project
//!   writers' refusal;
//! * phase 4's finisher (T-117): `finish_run` counts the rows,
//!   derives §9.4's verdict, writes the counters and `finished_at`,
//!   and clears/sets the error columns by its own rule — `success`
//!   (all `written`/`unchanged`, including the empty run with its
//!   message), `partial` (a mix), `failed` (nobody) — while
//!   MAIN-027's derived read (`last_success_finished_at`) moves on
//!   `success` only, and an unsettled row or a finished run is the
//!   typed refusal, never a verdict over half a run;
//! * the "last attempt" read (T-121): `last_attempt` answers the
//!   newest TERMINAL row by the runs order — a live `running` row is
//!   not an attempt yet, every terminal verdict (an `interrupted`
//!   corpse included) is one, and a fresh database answers `None`;
//! * the corrupt-cell policy (D-039): a `status` cell tampered
//!   outside the CHECK vocabulary surfaces as the typed `Member`
//!   refusal at the read, never as a guess.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::projects::{AddedVia, NewProject};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use eunomia_store::storage::{open_db, run_migrations};
use eunomia_store::sync_runs::{
    NewSyncRunProject, SyncRunProjectStatus, SyncRunStatus, SyncRunsError, abort_run,
    create_running, finish_run, get, last_attempt, last_success_finished_at, list_projects,
    mark_interrupted_runs, mark_project_failed, mark_project_skipped, mark_project_unchanged,
    mark_project_written, reset_projects_not_started,
};
use rusqlite::Connection;

/// A temporary database directory removed when the guard is dropped —
/// the same teardown every sibling suite does explicitly, made
/// impossible to forget at a call site. Best-effort: a directory that
/// cannot be removed is left behind rather than failing a green test.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates a unique temporary directory holding a migrated database
/// (opened through `open_db`, so the DB-001 pragma set — WAL and
/// `foreign_keys=ON` included — applies) and returns the connection
/// plus the directory guard (the `tests/build_snapshot.rs` pattern).
fn runs_db(label: &str) -> (Connection, TempDir) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-sync-runs-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    let mut conn =
        open_db(&dir.join("db").join("eunomiasync.sqlite3")).expect("open_db should succeed");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to a fresh database");
    (conn, TempDir(dir))
}

/// A default-shape project input; every call gets a distinct
/// `path_key`, so a set of these can be created together (the
/// `tests/build_snapshot.rs` `project_sample` shape).
fn project_sample(n: usize) -> NewProject<'static> {
    NewProject {
        path: Box::leak(format!("/repos/repo-{n}").into_boxed_str()),
        path_key: Box::leak(format!("/repos/repo-{n}").into_boxed_str()),
        name: Box::leak(format!("Project {n}").into_boxed_str()),
        label: Box::leak(format!("P{n}").into_boxed_str()),
        color: "blue",
        description: "",
        added_via: AddedVia::Manual,
        scheme: Scheme::A,
        base_name: BaseName::AgentsMd,
        extra_local: ExtraFile {
            enabled: false,
            position: Position::End,
        },
        extra_project: ExtraFile {
            enabled: false,
            position: Position::End,
        },
    }
}

/// Creates `project_sample(n)` and panics on failure (project rows
/// are the link table's FK targets, not these tests' subject).
fn seed_project(conn: &Connection, n: usize) -> String {
    eunomia_store::projects::create(conn, &project_sample(n))
        .unwrap_or_else(|err| panic!("seed project {n} should create, got {err}"))
        .id
}

/// One per-project input for `create_running`, with distinct literals
/// per project so a swapped column binding fails the verbatim
/// assertions.
fn run_project(project_id: &str, n: usize) -> NewSyncRunProject<'_> {
    let digit =
        char::from_digit(u32::try_from(n).expect("a single digit"), 10).expect("a decimal digit");
    NewSyncRunProject {
        project_id,
        target_path: Box::leak(format!("/repos/repo-{n}/AGENTS.md").into_boxed_str()),
        build_hash: Box::leak(digit.to_string().repeat(64).into_boxed_str()),
    }
}

/// The caller's run-start timestamp for these tests — an explicit
/// DB-002 literal (an oracle, not `now_utc` output), so the row's
/// `started_at` is pinned verbatim: the value the caller passed is
/// the value stored, byte for byte.
const STARTED_AT: &str = "2026-11-21T12:00:00.000Z";

/// Asserts the canonical UUIDv7 TEXT shape on an id — the DB-002
/// spelling `new_id` produces, pinned here by structure (36 chars,
/// hyphens at the fixed offsets, lowercase hex elsewhere, version
/// nibble `7`, variant nibble `8`..=`b`), the same oracle
/// `values.rs`'s own suite uses.
fn assert_uuidv7_text(id: &str) {
    let bytes = id.as_bytes();
    assert_eq!(bytes.len(), 36, "id must be 36 characters: {id}");
    for index in [8, 13, 18, 23] {
        assert_eq!(bytes[index], b'-', "byte {index} of {id}");
    }
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            continue;
        }
        assert!(
            byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase(),
            "byte {index} of {id} must be a lowercase hex digit"
        );
    }
    assert_eq!(bytes[14], b'7', "version nibble of {id}");
    assert!(
        matches!(bytes[19], b'8' | b'9' | b'a' | b'b'),
        "variant nibble of {id}"
    );
}

// ---- the phase-0 row shape ------------------------------------------

#[test]
fn create_running_writes_the_running_run_and_its_not_started_rows() {
    let (conn, _guard) = runs_db("shape");
    let first = seed_project(&conn, 1);
    let second = seed_project(&conn, 2);

    let run = create_running(
        &conn,
        STARTED_AT,
        &[run_project(&first, 1), run_project(&second, 2)],
    )
    .expect("creating the running record should work");

    // The run row: fresh UUIDv7 id, the caller's start timestamp
    // verbatim, the running status, the project count, the §6.2
    // counter zeros, and nothing claimed about a finish.
    assert_uuidv7_text(&run.id);
    assert_eq!(run.started_at, STARTED_AT);
    assert_eq!(run.status, SyncRunStatus::Running);
    assert_eq!(run.projects_total, 2);
    assert_eq!(run.projects_written, 0);
    assert_eq!(run.projects_unchanged, 0);
    assert_eq!(run.projects_skipped, 0);
    assert_eq!(run.projects_failed, 0);
    assert_eq!(run.finished_at, None);
    assert_eq!(run.error_code, None);
    assert_eq!(run.details, None);

    // The per-project rows: `not_started`, the inputs verbatim (a
    // swapped column binding fails the distinct literals).
    let rows = list_projects(&conn, &run.id).expect("reading the run's rows should work");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].sync_run_id, run.id);
    assert_eq!(rows[0].project_id, first);
    assert_eq!(rows[0].status, SyncRunProjectStatus::NotStarted);
    assert_eq!(
        rows[0].target_path.as_deref(),
        Some("/repos/repo-1/AGENTS.md")
    );
    assert_eq!(rows[0].build_hash, Some("1".repeat(64)));
    assert_eq!(rows[0].error_code, None);
    assert_eq!(rows[0].error_message, None);
    assert_eq!(rows[1].project_id, second);
    assert_eq!(
        rows[1].target_path.as_deref(),
        Some("/repos/repo-2/AGENTS.md")
    );
    assert_eq!(rows[1].build_hash, Some("2".repeat(64)));
}

#[test]
fn create_running_with_no_projects_records_the_empty_run() {
    // §9.4 phase 4 step 3's empty case starts here: a run over zero
    // active projects is still a run — phase 0 records it, and the
    // "success with a message" verdict is phase 4's to write.
    let (conn, _guard) = runs_db("empty");
    let run = create_running(&conn, STARTED_AT, &[]).expect("an empty run should create");
    assert_eq!(run.status, SyncRunStatus::Running);
    assert_eq!(run.projects_total, 0);
    let rows = list_projects(&conn, &run.id).expect("reading the empty run should work");
    assert_eq!(rows, Vec::new());
}

#[test]
fn a_new_run_does_not_touch_the_previous_one() {
    // Two runs in sequence (a sync, then the next): each is its own
    // row set — the id is fresh, the rows belong to their own run,
    // and retention's newest-500 window (T-029) keeps both here.
    let (conn, _guard) = runs_db("two-runs");
    let project = seed_project(&conn, 1);
    let first = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("the first run should create");
    let second = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("the second run should create");
    assert_ne!(first.id, second.id, "each run gets a fresh id");
    let first_rows = list_projects(&conn, &first.id).expect("reading run 1");
    let second_rows = list_projects(&conn, &second.id).expect("reading run 2");
    assert_eq!(first_rows.len(), 1);
    assert_eq!(second_rows.len(), 1);
    assert_eq!(first_rows[0].sync_run_id, first.id);
    assert_eq!(second_rows[0].sync_run_id, second.id);
}

// ---- the joined-transaction contract (D-037) -----------------------

#[test]
fn a_refused_project_row_rolls_the_whole_record_back() {
    // The atomicity the pipeline depends on: wrapped in one
    // transaction (the production shape is `Db::run`), a per-project
    // row whose `project_id` no `projects` row backs — the foreign
    // key's `ON DELETE RESTRICT` half, live because `open_db` sets
    // `foreign_keys=ON` — fails the insert, and the run row the call
    // already wrote goes with it: zero rows in either table.
    let (conn, _guard) = runs_db("rollback");
    seed_project(&conn, 1);

    let transaction = conn
        .unchecked_transaction()
        .expect("opening the caller's transaction should work");
    let outcome = create_running(
        &transaction,
        STARTED_AT,
        &[run_project("no-such-project", 9)],
    );
    assert!(
        matches!(outcome, Err(SyncRunsError::Sql(_))),
        "the foreign-key refusal must surface as the Sql arm, got {outcome:?}"
    );
    // The caller's rollback — what `Db::run` does with an `Err`
    // operation — drops the half-written record.
    drop(transaction);

    let runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM sync_runs", [], |row| row.get(0))
        .expect("counting runs should work");
    assert_eq!(runs, 0, "no run row may survive the refused record");
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM sync_run_projects", [], |row| {
            row.get(0)
        })
        .expect("counting run rows should work");
    assert_eq!(rows, 0, "no per-project row may survive either");
}

#[test]
fn a_duplicate_project_in_one_input_is_the_schema_s_refusal() {
    // The composite PRIMARY KEY (sync_run_id, project_id) is the
    // floor for a caller passing the same project twice; the
    // pipeline cannot (its ids come from the snapshot's validated
    // projects), and the honest answer for any other caller is the
    // typed Sql arm, not a silent skip.
    let (conn, _guard) = runs_db("duplicate");
    let project = seed_project(&conn, 1);

    let transaction = conn
        .unchecked_transaction()
        .expect("opening the caller's transaction should work");
    let outcome = create_running(
        &transaction,
        STARTED_AT,
        &[run_project(&project, 1), run_project(&project, 1)],
    );
    assert!(
        matches!(outcome, Err(SyncRunsError::Sql(_))),
        "the duplicate must surface as the Sql arm, got {outcome:?}"
    );
    drop(transaction);
    let runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM sync_runs", [], |row| row.get(0))
        .expect("counting runs should work");
    assert_eq!(runs, 0);
}

// ---- the reads ------------------------------------------------------

#[test]
fn get_round_trips_the_created_row_and_answers_none_for_unknown() {
    let (conn, _guard) = runs_db("get");
    let project = seed_project(&conn, 1);
    let created = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("creating the run should work");

    let read = get(&conn, &created.id)
        .expect("reading the run by id should work")
        .expect("the just-created run must be readable");
    assert_eq!(read, created);

    assert_eq!(
        get(&conn, "no-such-run").expect("reading an unknown id should not fail"),
        None
    );
}

#[test]
fn list_projects_follows_the_sort_order_not_the_insert_order() {
    // The pipeline processes projects "in order" (§9.4 phase 1) —
    // the snapshot's `sort_order`. The read answers that order even
    // when the rows were inserted in the reverse one, so no consumer
    // re-derives it.
    let (conn, _guard) = runs_db("order");
    let first = seed_project(&conn, 1);
    let second = seed_project(&conn, 2);

    let run = create_running(
        &conn,
        STARTED_AT,
        // The reverse order on purpose: the insert order must not
        // leak into the read.
        &[run_project(&second, 2), run_project(&first, 1)],
    )
    .expect("creating the run should work");

    let rows = list_projects(&conn, &run.id).expect("reading the run's rows should work");
    assert_eq!(
        rows.iter()
            .map(|row| row.project_id.as_str())
            .collect::<Vec<_>>(),
        vec![first.as_str(), second.as_str()],
        "the rows must come back in sort_order"
    );
}

// ---- phase 1's transitions (T-113) -----------------------------------

#[test]
fn mark_project_skipped_writes_the_code_and_keeps_the_fingerprint() {
    // §9.4 phase 1's per-project refusal: the row goes `skipped`
    // carrying the §10 code and the message — and the `target_path`/
    // `build_hash` phase 0 froze stay verbatim (the skip explains a
    // build that already happened, it does not erase it).
    let (conn, _guard) = runs_db("skip");
    let project = seed_project(&conn, 1);
    let run = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("creating the run should work");

    mark_project_skipped(
        &conn,
        &run.id,
        &project,
        "E_PROJECT_UNAVAILABLE",
        "folder is gone",
    )
    .expect("marking the skip should work");

    let rows = list_projects(&conn, &run.id).expect("reading the rows should work");
    assert_eq!(rows[0].status, SyncRunProjectStatus::Skipped);
    assert_eq!(rows[0].error_code.as_deref(), Some("E_PROJECT_UNAVAILABLE"));
    assert_eq!(rows[0].error_message.as_deref(), Some("folder is gone"));
    assert_eq!(
        rows[0].target_path.as_deref(),
        Some("/repos/repo-1/AGENTS.md")
    );
    assert_eq!(rows[0].build_hash, Some("1".repeat(64)));
}

#[test]
fn mark_project_unchanged_clears_the_error_columns_with_the_status() {
    // §9.4 phase 1 step 5's verdict: `unchanged` — and a row that
    // reaches this transition carries no refusal, so a stale error
    // cell from an earlier spelling of the run must not survive into
    // the summary. The stale cells are planted by direct SQL (free
    // TEXT columns, no CHECK to bypass) because the writers' state
    // scoping makes the old skip-then-unchanged sequence unreachable
    // — exactly the reordering the scoping exists to refuse.
    let (conn, _guard) = runs_db("unchanged");
    let project = seed_project(&conn, 1);
    let run = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("creating the run should work");
    conn.execute(
        "UPDATE sync_run_projects SET error_code = 'E_NOT_UTF8', error_message = 'stale refusal' \
         WHERE sync_run_id = ?1 AND project_id = ?2",
        rusqlite::params![run.id, project],
    )
    .expect("planting the stale error cells should work");

    mark_project_unchanged(&conn, &run.id, &project).expect("marking the verdict should work");

    let rows = list_projects(&conn, &run.id).expect("reading the rows should work");
    assert_eq!(rows[0].status, SyncRunProjectStatus::Unchanged);
    assert_eq!(rows[0].error_code, None);
    assert_eq!(rows[0].error_message, None);
}

#[test]
fn mark_project_written_clears_the_error_columns_with_the_status() {
    // §9.4 phase 3 step 2's verdict: `written` — a row that reaches
    // this transition carries no refusal, so a stale error cell must
    // not survive into the summary (the unchanged clearing rule,
    // spelled for this edge; the stale cells are planted by direct
    // SQL for the same reason — the state scoping makes the natural
    // sequence unreachable). The fingerprint stays verbatim.
    let (conn, _guard) = runs_db("written");
    let project = seed_project(&conn, 1);
    let run = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("creating the run should work");
    conn.execute(
        "UPDATE sync_run_projects SET error_code = 'E_WRITE_FAILED', error_message = 'stale refusal' \
         WHERE sync_run_id = ?1 AND project_id = ?2",
        rusqlite::params![run.id, project],
    )
    .expect("planting the stale error cells should work");

    mark_project_written(&conn, &run.id, &project).expect("marking the verdict should work");

    let rows = list_projects(&conn, &run.id).expect("reading the rows should work");
    assert_eq!(rows[0].status, SyncRunProjectStatus::Written);
    assert_eq!(rows[0].error_code, None);
    assert_eq!(rows[0].error_message, None);
    assert_eq!(
        rows[0].build_hash,
        Some("1".repeat(64)),
        "the fingerprint phase 0 froze stays"
    );
}

#[test]
fn mark_project_failed_writes_the_code_and_message() {
    // §9.4 phase 3 step 3's verdict: `failed` carrying the §10 code
    // and the class-only message — with the fingerprint intact, the
    // failure explains a build that happened, it does not erase it.
    let (conn, _guard) = runs_db("failed");
    let project = seed_project(&conn, 1);
    let run = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("creating the run should work");

    mark_project_failed(
        &conn,
        &run.id,
        &project,
        "E_VERIFY_FAILED",
        "the written file failed the check",
    )
    .expect("marking the failure should work");

    let rows = list_projects(&conn, &run.id).expect("reading the rows should work");
    assert_eq!(rows[0].status, SyncRunProjectStatus::Failed);
    assert_eq!(rows[0].error_code.as_deref(), Some("E_VERIFY_FAILED"));
    assert_eq!(
        rows[0].error_message.as_deref(),
        Some("the written file failed the check")
    );
    assert_eq!(rows[0].build_hash, Some("1".repeat(64)));
}

#[test]
fn abort_run_finishes_the_run_without_touching_counters_or_rows() {
    // §9.4's tail rules (phase 1's E_DISK_FULL here): the run row
    // goes `aborted` with the caller's finish timestamp verbatim, the
    // code and the details — and neither the per-status counters
    // (phase 4's, §9.4 phase 4) nor any per-project row moves: the
    // verdicts each phase wrote are exactly what the aborted run
    // keeps.
    let (conn, _guard) = runs_db("abort");
    let project = seed_project(&conn, 1);
    let run = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("creating the run should work");
    mark_project_skipped(
        &conn,
        &run.id,
        &project,
        "E_PROJECT_UNAVAILABLE",
        "folder is gone",
    )
    .expect("the pre-abort verdict should work");
    const FINISHED_AT: &str = "2026-11-21T12:05:00.000Z";

    abort_run(
        &conn,
        &run.id,
        FINISHED_AT,
        "E_DISK_FULL",
        "free 119 bytes, required 120 bytes",
    )
    .expect("aborting the run should work");

    let row = get(&conn, &run.id)
        .expect("reading the run should work")
        .expect("the run must persist");
    assert_eq!(row.status, SyncRunStatus::Aborted);
    assert_eq!(row.finished_at.as_deref(), Some(FINISHED_AT));
    assert_eq!(row.error_code.as_deref(), Some("E_DISK_FULL"));
    assert_eq!(
        row.details.as_deref(),
        Some("free 119 bytes, required 120 bytes")
    );
    // Phase 4's counters stay at their zeros (the ownership rule).
    assert_eq!(row.projects_written, 0);
    assert_eq!(row.projects_unchanged, 0);
    assert_eq!(row.projects_skipped, 0);
    assert_eq!(row.projects_failed, 0);
    // The per-project verdicts survive the abort untouched.
    let rows = list_projects(&conn, &run.id).expect("reading the rows should work");
    assert_eq!(rows[0].status, SyncRunProjectStatus::Skipped);
    assert_eq!(rows[0].error_code.as_deref(), Some("E_PROJECT_UNAVAILABLE"));
}

#[test]
fn updates_addressing_no_row_or_a_settled_row_are_the_typed_refusal() {
    // An UPDATE that matched nothing is a caller bug the store
    // refuses to swallow as a silent success — both per-project
    // writers and the run-level abort (the `create_running`
    // duplicate-input posture) — and the state scoping extends that
    // to a row a later spelling of the run already settled: driving a
    // transition twice, or against a finished run, must never rewrite
    // the run's audit record.
    let (conn, _guard) = runs_db("miss");
    let project = seed_project(&conn, 1);
    let run = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("creating the run should work");

    // Unknown ids.
    for outcome in [
        mark_project_skipped(&conn, &run.id, "no-such-project", "E_NOT_UTF8", "m"),
        mark_project_unchanged(&conn, &run.id, "no-such-project"),
        mark_project_written(&conn, &run.id, "no-such-project"),
        mark_project_failed(&conn, &run.id, "no-such-project", "E_WRITE_FAILED", "m"),
        abort_run(&conn, "no-such-run", STARTED_AT, "E_DISK_FULL", "d"),
    ] {
        assert!(
            matches!(outcome, Err(SyncRunsError::Sql(_))),
            "a zero-row update must be the typed Sql refusal"
        );
    }

    // A settled row: the skip landed once, and no later transition may
    // move it again — the second skip, the unchanged and both phase-3
    // verdicts refuse.
    mark_project_skipped(
        &conn,
        &run.id,
        &project,
        "E_PROJECT_UNAVAILABLE",
        "folder is gone",
    )
    .expect("the first skip should work");
    assert!(
        matches!(
            mark_project_skipped(&conn, &run.id, &project, "E_NOT_UTF8", "rewrite attempt"),
            Err(SyncRunsError::Sql(_))
        ),
        "a second skip must refuse, not rewrite the code"
    );
    assert!(
        matches!(
            mark_project_unchanged(&conn, &run.id, &project),
            Err(SyncRunsError::Sql(_))
        ),
        "unchanged over a settled row must refuse"
    );
    assert!(
        matches!(
            mark_project_written(&conn, &run.id, &project),
            Err(SyncRunsError::Sql(_))
        ),
        "written over a settled row must refuse"
    );
    assert!(
        matches!(
            mark_project_failed(
                &conn,
                &run.id,
                &project,
                "E_WRITE_FAILED",
                "rewrite attempt"
            ),
            Err(SyncRunsError::Sql(_))
        ),
        "failed over a settled row must refuse"
    );
    let rows = list_projects(&conn, &run.id).expect("reading the rows should work");
    assert_eq!(rows[0].status, SyncRunProjectStatus::Skipped);
    assert_eq!(rows[0].error_code.as_deref(), Some("E_PROJECT_UNAVAILABLE"));

    // A finished run: the abort landed once, and a re-abort refuses
    // instead of overwriting the finish stamp.
    abort_run(&conn, &run.id, STARTED_AT, "E_DISK_FULL", "free 0 bytes")
        .expect("the first abort should work");
    assert!(
        matches!(
            abort_run(
                &conn,
                &run.id,
                STARTED_AT,
                "E_BACKUP_FAILED",
                "rewrite attempt"
            ),
            Err(SyncRunsError::Sql(_))
        ),
        "a re-abort must refuse, not rewrite the verdict"
    );
    let row = get(&conn, &run.id)
        .expect("reading the run should work")
        .expect("the run must persist");
    assert_eq!(row.error_code.as_deref(), Some("E_DISK_FULL"));

    // The untouched run: an abort of a `running` run still works.
    let other = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("the second run should create");
    abort_run(&conn, &other.id, STARTED_AT, "E_DISK_FULL", "free 0 bytes")
        .expect("aborting a running run should work");
}

#[test]
fn a_tampered_status_cell_is_the_typed_member_refusal() {
    // D-039: a cell outside the CHECK vocabulary (only tampering or a
    // newer app writes one) surfaces as the typed refusal with the
    // column named — both tables' reads, since two CHECKs guard two
    // vocabularies. The schema's own CHECK refuses the tampering
    // write, so the cell is planted with `ignore_check_constraints`
    // for the duration of the statement (the `build_snapshot` suite's
    // corrupt-cell pattern) — the read under test never bypasses
    // anything.
    let (conn, _guard) = runs_db("corrupt");
    let project = seed_project(&conn, 1);
    let run = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("creating the run should work");

    conn.pragma_update(None, "ignore_check_constraints", true)
        .expect("relaxing the CHECK for the tampering write should work");
    conn.execute(
        "UPDATE sync_runs SET status = 'paused' WHERE id = ?1",
        [&run.id],
    )
    .expect("tampering with the status cell should work");
    assert_eq!(
        get(&conn, &run.id),
        Err(SyncRunsError::Member {
            column: "status",
            found: "paused".to_owned(),
        })
    );

    conn.execute(
        "UPDATE sync_runs SET status = 'running' WHERE id = ?1",
        [&run.id],
    )
    .expect("restoring the run status should work");
    conn.execute(
        "UPDATE sync_run_projects SET status = 'queued' WHERE sync_run_id = ?1",
        [&run.id],
    )
    .expect("tampering with the project status cell should work");
    conn.pragma_update(None, "ignore_check_constraints", false)
        .expect("restoring the CHECK should work");
    assert_eq!(
        list_projects(&conn, &run.id),
        Err(SyncRunsError::Member {
            column: "status",
            found: "queued".to_owned(),
        })
    );
}

// ---- phase 2's abort reset (T-115) -----------------------------------

#[test]
fn reset_projects_not_started_returns_every_verdict_except_skipped() {
    // §9.4 phase 2 step 2 (the E_BACKUP_FAILED abort): the rows a
    // run earned before its backups failed go back to `not_started`
    // with their error columns cleared, while `skipped` keeps its
    // refusal (the spec's own "except `skipped`") and every row's
    // phase-0 fingerprint (target_path/build_hash) survives. The
    // `failed` row is planted with plain SQL — its store writer is
    // phase 3's (T-116+); the CHECK accepts the member, so no
    // constraint is bypassed, and the reset's sweep must cover the
    // whole vocabulary, not just the states today's writers produce.
    let (conn, _guard) = runs_db("reset");
    let skipped = seed_project(&conn, 1);
    let unchanged = seed_project(&conn, 2);
    let failed = seed_project(&conn, 3);
    let untouched = seed_project(&conn, 4);
    let run = create_running(
        &conn,
        STARTED_AT,
        &[
            run_project(&skipped, 1),
            run_project(&unchanged, 2),
            run_project(&failed, 3),
            run_project(&untouched, 4),
        ],
    )
    .expect("creating the run should work");
    mark_project_skipped(
        &conn,
        &run.id,
        &skipped,
        "E_PROJECT_UNAVAILABLE",
        "folder is gone",
    )
    .expect("the skip verdict should work");
    mark_project_unchanged(&conn, &run.id, &unchanged).expect("the unchanged verdict should work");
    conn.execute(
        "UPDATE sync_run_projects SET status = 'failed', error_code = 'E_WRITE_FAILED', \
         error_message = 'disk fell off' \
         WHERE sync_run_id = ?1 AND project_id = ?2",
        rusqlite::params![run.id, failed],
    )
    .expect("planting the phase-3 verdict should work");

    let changed = reset_projects_not_started(&conn, &run.id).expect("the reset should work");

    assert_eq!(changed, 2, "exactly the unchanged and failed rows reset");
    let rows = list_projects(&conn, &run.id).expect("reading the rows should work");
    // `skipped` survives verbatim — refusal and all.
    assert_eq!(rows[0].status, SyncRunProjectStatus::Skipped);
    assert_eq!(rows[0].error_code.as_deref(), Some("E_PROJECT_UNAVAILABLE"));
    assert_eq!(rows[0].error_message.as_deref(), Some("folder is gone"));
    // The reset rows: back to `not_started`, error columns cleared,
    // fingerprint intact (the row still names what phase 0 built).
    for row in &rows[1..3] {
        assert_eq!(row.status, SyncRunProjectStatus::NotStarted);
        assert_eq!(row.error_code, None);
        assert_eq!(row.error_message, None);
        assert!(row.target_path.is_some());
        assert!(row.build_hash.is_some());
    }
    // A row that was already `not_started` never matched the sweep.
    assert_eq!(rows[3].status, SyncRunProjectStatus::NotStarted);

    // Idempotent: a second sweep finds nothing left to reset.
    assert_eq!(
        reset_projects_not_started(&conn, &run.id).expect("the second sweep should work"),
        0
    );
}

#[test]
fn reset_projects_not_started_answers_zero_where_nothing_matches() {
    // The bulk-sweep ruling: a run whose every row is `not_started`
    // or `skipped` (an all-creation-case backup failure) has nothing
    // to reset, and that is `Ok(0)` — never the per-project writers'
    // zero-row refusal, because the abort fires whatever the run's
    // verdict mix is. An unknown run id is the same honest zero.
    let (conn, _guard) = runs_db("reset-zero");
    let project = seed_project(&conn, 1);
    let run = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("creating the run should work");

    assert_eq!(
        reset_projects_not_started(&conn, &run.id).expect("the sweep should work"),
        0
    );
    assert_eq!(
        reset_projects_not_started(&conn, "no-such-run").expect("the sweep should work"),
        0
    );
    // The run row is the sweep's own business only through the
    // caller's `abort_run` — the reset never finishes a run.
    let row = get(&conn, &run.id)
        .expect("reading the run should work")
        .expect("the run must persist");
    assert_eq!(row.status, SyncRunStatus::Running);
    assert_eq!(row.finished_at, None);
}

// ---- phase 4's finisher (T-117) --------------------------------------

/// The caller's finish timestamp for these tests — an explicit DB-002
/// literal (the `STARTED_AT` oracle shape), pinned verbatim in the
/// row exactly the way the start time is.
const FINISHED_AT: &str = "2026-11-21T12:30:00.000Z";

#[test]
fn finish_run_counts_success_and_moves_the_last_success_time() {
    // §9.4 phase 4 step 3's success arm: every row `written` or
    // `unchanged` (one of each here) → `success`, the counted
    // counters, `finished_at` verbatim, no error code, no details —
    // and step 4 (MAIN-027): the derived last-success time IS this
    // run's `finished_at`, because the verdict and the stamp are one
    // UPDATE.
    let (conn, _guard) = runs_db("finish-success");
    let written = seed_project(&conn, 1);
    let unchanged = seed_project(&conn, 2);
    let run = create_running(
        &conn,
        STARTED_AT,
        &[run_project(&written, 1), run_project(&unchanged, 2)],
    )
    .expect("creating the run should work");
    mark_project_written(&conn, &run.id, &written).expect("the write verdict should work");
    mark_project_unchanged(&conn, &run.id, &unchanged).expect("the match verdict should work");
    // Stale outcome cells planted by direct SQL (free TEXT columns;
    // no writer produces them on a `running` row, so the planting is
    // the only route): the finish must clear both — a non-empty
    // success claims no error code and no details.
    conn.execute(
        "UPDATE sync_runs SET error_code = 'E_DISK_FULL', details = 'stale abort text' \
         WHERE id = ?1",
        [&run.id],
    )
    .expect("planting the stale outcome cells should work");

    let finished = finish_run(&conn, &run.id, FINISHED_AT).expect("finishing the run should work");

    assert_eq!(finished.status, SyncRunStatus::Success);
    assert_eq!(finished.finished_at.as_deref(), Some(FINISHED_AT));
    assert_eq!(finished.projects_total, 2);
    assert_eq!(finished.projects_written, 1);
    assert_eq!(finished.projects_unchanged, 1);
    assert_eq!(finished.projects_skipped, 0);
    assert_eq!(finished.projects_failed, 0);
    assert_eq!(finished.error_code, None);
    assert_eq!(finished.details, None);
    assert_eq!(
        last_success_finished_at(&conn)
            .expect("the MAIN-027 read should work")
            .as_deref(),
        Some(FINISHED_AT),
        "a success run's finished_at is the overall last-success time"
    );
}

#[test]
fn finish_run_counts_partial_and_failed_by_the_spec_rule() {
    // §9.4 phase 4 step 3's other two arms over one database: a mix
    // of `skipped`/`failed` and `written`/`unchanged` is `partial`;
    // nobody written or unchanged is `failed`. Neither moves
    // MAIN-027's answer (step 4's "For `partial`, the overall time is
    // not updated" — the failed arm is the same sentence read over no
    // success at all).
    let (conn, _guard) = runs_db("finish-partial-failed");
    let skipped = seed_project(&conn, 1);
    let written = seed_project(&conn, 2);
    let failed = seed_project(&conn, 3);
    let other_skipped = seed_project(&conn, 4);

    // The partial run: one skipped, one written, one failed.
    let partial = create_running(
        &conn,
        STARTED_AT,
        &[
            run_project(&skipped, 1),
            run_project(&written, 2),
            run_project(&failed, 3),
        ],
    )
    .expect("creating the partial run should work");
    mark_project_skipped(
        &conn,
        &partial.id,
        &skipped,
        "E_PROJECT_UNAVAILABLE",
        "folder is gone",
    )
    .expect("the skip verdict should work");
    mark_project_written(&conn, &partial.id, &written).expect("the write verdict should work");
    mark_project_failed(
        &conn,
        &partial.id,
        &failed,
        "E_FILE_LOCKED",
        "held by another process",
    )
    .expect("the failure verdict should work");
    let finished =
        finish_run(&conn, &partial.id, FINISHED_AT).expect("finishing the partial run should work");
    assert_eq!(finished.status, SyncRunStatus::Partial);
    assert_eq!(finished.projects_written, 1);
    assert_eq!(finished.projects_unchanged, 0);
    assert_eq!(finished.projects_skipped, 1);
    assert_eq!(finished.projects_failed, 1);
    assert_eq!(finished.error_code, None);
    assert_eq!(finished.details, None);
    assert_eq!(
        last_success_finished_at(&conn).expect("the MAIN-027 read should work"),
        None,
        "no success run exists — the overall time stays unknown"
    );

    // The failed run: skipped and failed only, nobody good.
    let failed_run = create_running(
        &conn,
        STARTED_AT,
        &[run_project(&skipped, 1), run_project(&other_skipped, 4)],
    )
    .expect("creating the failed run should work");
    mark_project_skipped(
        &conn,
        &failed_run.id,
        &skipped,
        "E_PROJECT_UNAVAILABLE",
        "folder is gone",
    )
    .expect("the skip verdict should work");
    mark_project_failed(
        &conn,
        &failed_run.id,
        &other_skipped,
        "E_WRITE_FAILED",
        "the write refused",
    )
    .expect("the failure verdict should work");
    let finished = finish_run(&conn, &failed_run.id, FINISHED_AT)
        .expect("finishing the failed run should work");
    assert_eq!(finished.status, SyncRunStatus::Failed);
    assert_eq!(finished.projects_skipped, 1);
    assert_eq!(finished.projects_failed, 1);
    assert_eq!(finished.details, None);
    assert_eq!(
        last_success_finished_at(&conn).expect("the MAIN-027 read should work"),
        None,
        "still no success run — a failed finish moves nothing"
    );

    // The success that finally lands is the answer from then on, and
    // a later partial does not displace it (MAIN-027's pin).
    let success_run = create_running(&conn, STARTED_AT, &[run_project(&written, 2)])
        .expect("creating the success run should work");
    mark_project_written(&conn, &success_run.id, &written).expect("the write verdict should work");
    const SUCCESS_AT: &str = "2026-11-21T13:00:00.000Z";
    finish_run(&conn, &success_run.id, SUCCESS_AT).expect("finishing the success run should work");
    let later_partial = create_running(
        &conn,
        STARTED_AT,
        &[run_project(&written, 2), run_project(&skipped, 1)],
    )
    .expect("creating the later partial run should work");
    mark_project_written(&conn, &later_partial.id, &written)
        .expect("the write verdict should work");
    mark_project_skipped(
        &conn,
        &later_partial.id,
        &skipped,
        "E_PROJECT_UNAVAILABLE",
        "folder is gone",
    )
    .expect("the skip verdict should work");
    finish_run(&conn, &later_partial.id, FINISHED_AT)
        .expect("finishing the later partial run should work");
    assert_eq!(
        last_success_finished_at(&conn)
            .expect("the MAIN-027 read should work")
            .as_deref(),
        Some(SUCCESS_AT),
        "a partial after the success does not move the overall time"
    );
}

#[test]
fn finish_run_succeeds_the_empty_run_with_the_message() {
    // §9.4 phase 4 step 3's empty case: a run over zero active
    // projects is `success` — every (zero) rows are `written` or
    // `unchanged` — with the message in `details`, and MAIN-027 takes
    // its `finished_at` like any success.
    let (conn, _guard) = runs_db("finish-empty");
    let run = create_running(&conn, STARTED_AT, &[]).expect("the empty run should create");

    let finished =
        finish_run(&conn, &run.id, FINISHED_AT).expect("finishing the empty run should work");

    assert_eq!(finished.status, SyncRunStatus::Success);
    assert_eq!(finished.finished_at.as_deref(), Some(FINISHED_AT));
    assert_eq!(finished.projects_total, 0);
    assert_eq!(finished.projects_written, 0);
    assert_eq!(finished.projects_unchanged, 0);
    assert_eq!(finished.projects_skipped, 0);
    assert_eq!(finished.projects_failed, 0);
    assert_eq!(
        finished.details.as_deref(),
        Some("no active projects to synchronize"),
        "the §9.4 message is the row's details cell"
    );
    assert_eq!(
        last_success_finished_at(&conn)
            .expect("the MAIN-027 read should work")
            .as_deref(),
        Some(FINISHED_AT),
        "the empty success is a success for the overall time too"
    );
}

#[test]
fn finish_run_refuses_an_unsettled_row_and_a_finished_run() {
    // The two refusals: a row still `not_started` names a broken
    // phase chain (phases 1–3 own every transition), and the verdict
    // must not be written over half a run — the run row stays
    // `running`, the crash-recovery rule's case. And the finisher is
    // scoped to `running`: finishing twice, an unknown id, or a run
    // the abort already settled is the module's typed zero-row
    // refusal, never a history rewrite.
    let (conn, _guard) = runs_db("finish-refusals");
    let settled = seed_project(&conn, 1);
    let unsettled = seed_project(&conn, 2);
    let run = create_running(
        &conn,
        STARTED_AT,
        &[run_project(&settled, 1), run_project(&unsettled, 2)],
    )
    .expect("creating the run should work");
    mark_project_written(&conn, &run.id, &settled).expect("the write verdict should work");

    assert!(
        matches!(
            finish_run(&conn, &run.id, FINISHED_AT),
            Err(SyncRunsError::Sql(_))
        ),
        "an unsettled row must refuse the verdict"
    );
    let row = get(&conn, &run.id)
        .expect("reading the run should work")
        .expect("the run must persist");
    assert_eq!(row.status, SyncRunStatus::Running, "the run stays running");
    assert_eq!(row.finished_at, None);

    // Settle the row and finish once — then every re-finish refuses.
    mark_project_failed(&conn, &run.id, &unsettled, "E_WRITE_FAILED", "late refusal")
        .expect("the late failure verdict should work");
    finish_run(&conn, &run.id, FINISHED_AT).expect("finishing the run should work");
    for outcome in [
        finish_run(&conn, &run.id, FINISHED_AT),
        finish_run(&conn, "no-such-run", FINISHED_AT),
    ] {
        assert!(
            matches!(outcome, Err(SyncRunsError::Sql(_))),
            "a finished or unknown run must refuse, not rewrite"
        );
    }
    // An aborted run refuses too: the abort owns that row's verdict.
    let aborted = create_running(&conn, STARTED_AT, &[run_project(&settled, 1)])
        .expect("creating the aborting run should work");
    abort_run(
        &conn,
        &aborted.id,
        FINISHED_AT,
        "E_DISK_FULL",
        "free 0 bytes",
    )
    .expect("the abort should work");
    assert!(
        matches!(
            finish_run(&conn, &aborted.id, FINISHED_AT),
            Err(SyncRunsError::Sql(_))
        ),
        "a finish over an aborted run must refuse"
    );
}

#[test]
fn last_success_finished_at_answers_the_newest_success_by_the_run_order() {
    // The derived read's own ordering pin: two successes answer by
    // `started_at DESC, id DESC` (the D-054 runs order), not by
    // finished_at and not by insertion — a success that STARTED
    // later is "the last run with the success status" even when its
    // finish stamp is the earlier string. A fresh database answers
    // `None` (the pre-first-sync state).
    let (conn, _guard) = runs_db("last-success");
    assert_eq!(
        last_success_finished_at(&conn).expect("the read should work"),
        None,
        "no runs at all — no last-success time"
    );

    let project = seed_project(&conn, 1);
    // The earlier-starting success, finished LATER: its finished_at
    // string is the greater one, so a finished_at-ordered read would
    // pick it — the pin's discriminator.
    let earlier = create_running(
        &conn,
        "2026-11-21T11:00:00.000Z",
        &[run_project(&project, 1)],
    )
    .expect("the earlier run should create");
    mark_project_written(&conn, &earlier.id, &project).expect("the verdict should work");
    finish_run(&conn, &earlier.id, "2026-11-21T12:59:00.000Z")
        .expect("finishing the earlier run should work");
    // The later-starting success.
    let later = create_running(
        &conn,
        "2026-11-21T12:00:00.000Z",
        &[run_project(&project, 1)],
    )
    .expect("the later run should create");
    mark_project_written(&conn, &later.id, &project).expect("the verdict should work");
    finish_run(&conn, &later.id, "2026-11-21T12:30:00.000Z")
        .expect("finishing the later run should work");

    assert_eq!(
        last_success_finished_at(&conn)
            .expect("the read should work")
            .as_deref(),
        Some("2026-11-21T12:30:00.000Z"),
        "the newest START is the last success, whatever the finish stamps say"
    );
}

#[test]
fn last_attempt_answers_the_newest_terminal_row_not_a_live_one() {
    // The "last attempt" read's own pins (T-121): a LIVE run is not an
    // attempt yet — a newer `running` row must not win over an older
    // terminal one; every terminal verdict counts as an attempt (the
    // `interrupted` corpse the startup sweep recovered is the weakest
    // writer, and it still is one); the answer is the WHOLE row (the
    // `finished_at` D-147 stamped included); and a fresh database
    // answers `None`.
    let (conn, _guard) = runs_db("last-attempt");
    assert_eq!(
        last_attempt(&conn).expect("the read should work"),
        None,
        "no runs at all — no last attempt"
    );

    // The other `None` shape (round-1's P3-4 fix): a database where
    // ONLY a live run exists — the row is real, but a live run is not
    // an attempt yet, so the read still answers `None` (the
    // `noRuns`-while-live case D-217's no-runs arm names).
    let live_only_project = seed_project(&conn, 9);
    create_running(
        &conn,
        "2026-11-21T10:00:00.000Z",
        &[run_project(&live_only_project, 9)],
    )
    .expect("the live-only run should create");
    assert_eq!(
        last_attempt(&conn).expect("the read should work"),
        None,
        "a live run alone is no last attempt — its own answer is the \
         outcome its consumer reads"
    );

    let project = seed_project(&conn, 1);
    // The terminal row the read must answer, by the weakest terminal
    // writer there is (the sweep — no verdict of its own).
    let terminal = create_running(
        &conn,
        "2026-11-21T11:00:00.000Z",
        &[run_project(&project, 1)],
    )
    .expect("the terminal run should create");
    mark_interrupted_runs(&conn, "2026-11-21T11:05:00.000Z")
        .expect("the recovery sweep should take the corpse");
    // A LIVE run that started later: not an attempt yet — the pin's
    // discriminator (a naive "newest row" read answers it instead).
    create_running(
        &conn,
        "2026-11-21T12:00:00.000Z",
        &[run_project(&project, 2)],
    )
    .expect("the live run should create");

    let answered = last_attempt(&conn)
        .expect("the read should work")
        .expect("a terminal row exists — the read answers it");
    assert_eq!(
        answered.id, terminal.id,
        "the live run is not an attempt yet (it answers its own summary)"
    );
    assert_eq!(answered.status, SyncRunStatus::Interrupted);
    assert_eq!(
        answered.finished_at.as_deref(),
        Some("2026-11-21T11:05:00.000Z"),
        "the row the read answers carries its terminal stamp (D-147's invariant)"
    );

    // Terminal rows compete by the runs order, never by kind: a
    // same-start pair resolves by `id DESC` (UUIDv7 creation order,
    // the D-054 rule `last_success_finished_at` shares).
    let first_tie = create_running(&conn, STARTED_AT, &[run_project(&project, 3)])
        .expect("the first same-stamp run should create");
    mark_project_written(&conn, &first_tie.id, &project).expect("the verdict should work");
    finish_run(&conn, &first_tie.id, "2026-11-21T12:10:00.000Z")
        .expect("finishing the first tie should work");
    let second_tie = create_running(&conn, STARTED_AT, &[run_project(&project, 4)])
        .expect("the second same-stamp run should create");
    abort_run(
        &conn,
        &second_tie.id,
        "2026-11-21T12:11:00.000Z",
        "E_DISK_FULL",
        "free 0 bytes",
    )
    .expect("aborting the second tie should work");

    let answered = last_attempt(&conn)
        .expect("the read should work")
        .expect("the ties replaced the interrupted row as the newest terminals");
    assert_eq!(
        answered.id, second_tie.id,
        "same-millisecond starts resolve by UUIDv7 creation order"
    );
    assert_eq!(
        answered.status,
        SyncRunStatus::Aborted,
        "an aborted attempt is an attempt — the read never filters by kind"
    );
}

#[test]
fn mark_interrupted_runs_sweeps_every_running_row_and_no_terminal_one() {
    // §9.4's "recovery after a crash" (T-122/T-258): the startup sweep
    // takes EVERY `running` row a dead process left — two corpses here,
    // in one statement — and answers how many it took, while the
    // terminal rows (a `success`, an `aborted`) keep exactly what their
    // own writers wrote: the sweep is scoped to the `running` source
    // state, so it can never rewrite history.
    let (conn, _guard) = runs_db("interrupted");
    let project = seed_project(&conn, 1);
    let first = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("the first corpse should create");
    let second = create_running(&conn, STARTED_AT, &[run_project(&project, 2)])
        .expect("the second corpse should create");
    let finished = create_running(&conn, STARTED_AT, &[run_project(&project, 3)])
        .expect("the finished run should create");
    mark_project_written(&conn, &finished.id, &project).expect("the verdict should work");
    finish_run(&conn, &finished.id, "2026-11-21T12:00:00.000Z")
        .expect("finishing the run should work");
    let aborted = create_running(&conn, STARTED_AT, &[run_project(&project, 4)])
        .expect("the aborted run should create");
    abort_run(
        &conn,
        &aborted.id,
        "2026-11-21T12:01:00.000Z",
        "E_DISK_FULL",
        "free 0 bytes",
    )
    .expect("aborting the run should work");
    const RECOVERED_AT: &str = "2026-11-22T08:00:00.000Z";

    let taken = mark_interrupted_runs(&conn, RECOVERED_AT).expect("the startup sweep should work");

    assert_eq!(taken, 2, "exactly the two corpses were taken");
    for corpse in [&first, &second] {
        let row = get(&conn, &corpse.id)
            .expect("reading the corpse should work")
            .expect("the corpse must persist");
        assert_eq!(row.status, SyncRunStatus::Interrupted);
        assert_eq!(
            row.finished_at.as_deref(),
            Some(RECOVERED_AT),
            "the recovery moment is the terminal moment"
        );
        assert_eq!(row.error_code, None, "no §10 code names a process death");
        assert_eq!(row.details, None, "a crashed run claims no details line");
        assert_eq!(
            row.projects_written, 0,
            "phase 4's counters stay at their zeros"
        );
        assert_eq!(row.projects_failed, 0);
    }
    let kept = get(&conn, &finished.id)
        .expect("reading the finished run should work")
        .expect("the finished run must persist");
    assert_eq!(kept.status, SyncRunStatus::Success);
    assert_eq!(
        kept.finished_at.as_deref(),
        Some("2026-11-21T12:00:00.000Z")
    );
    let kept = get(&conn, &aborted.id)
        .expect("reading the aborted run should work")
        .expect("the aborted run must persist");
    assert_eq!(kept.status, SyncRunStatus::Aborted);
    assert_eq!(kept.error_code.as_deref(), Some("E_DISK_FULL"));
    assert_eq!(
        kept.finished_at.as_deref(),
        Some("2026-11-21T12:01:00.000Z")
    );
}

#[test]
fn mark_interrupted_runs_is_idempotent_and_touches_no_project_row() {
    // The convergence half (§9.4's "the project status already
    // reflects what was written at the time of the crash", T-F3): the
    // sweep never touches a `sync_run_projects` row — a `written` row
    // keeps claiming its write verbatim — and a second sweep matches
    // no row (`Ok(0)`): the next restart over an already-recovered
    // database is the honest no-op.
    let (conn, _guard) = runs_db("interrupted-idempotent");
    let project = seed_project(&conn, 1);
    let corpse = create_running(&conn, STARTED_AT, &[run_project(&project, 1)])
        .expect("the corpse should create");
    mark_project_written(&conn, &corpse.id, &project).expect("the crash-mid-write verdict");
    const RECOVERED_AT: &str = "2026-11-22T08:00:00.000Z";

    mark_interrupted_runs(&conn, RECOVERED_AT).expect("the first sweep should work");
    let second = mark_interrupted_runs(&conn, RECOVERED_AT).expect("the second sweep should work");

    assert_eq!(second, 0, "nothing is left running after the first sweep");
    let row = get(&conn, &corpse.id)
        .expect("reading the corpse should work")
        .expect("the corpse must persist");
    assert_eq!(
        row.status,
        SyncRunStatus::Interrupted,
        "not re-marked, still interrupted"
    );
    let rows = list_projects(&conn, &corpse.id).expect("reading the rows should work");
    assert_eq!(rows[0].status, SyncRunProjectStatus::Written);
    assert_eq!(
        rows[0].build_hash,
        Some("1".repeat(64)),
        "the fingerprint is intact"
    );
    assert_eq!(rows[0].error_code, None);
}
