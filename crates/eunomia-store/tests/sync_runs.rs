//! Behavior tests for the `sync_runs` repositories (spec §6.2 DDL,
//! §9.4 phase 0 step 2; task T-112).
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
//! * the corrupt-cell policy (D-039): a `status` cell tampered
//!   outside the CHECK vocabulary surfaces as the typed `Member`
//!   refusal at the read, never as a guess.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::projects::{AddedVia, NewProject};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use eunomia_store::storage::{open_db, run_migrations};
use eunomia_store::sync_runs::{
    NewSyncRunProject, SyncRunProjectStatus, SyncRunStatus, SyncRunsError, create_running, get,
    list_projects,
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
