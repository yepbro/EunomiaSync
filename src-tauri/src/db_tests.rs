//! Behavior tests of the DB executor (SYS-001, SYS-002, NFR-008;
//! DECISIONS.md D-037), driven through the same seam the future
//! commands will use: [`crate::db::Db`] opened over a real database
//! file under a `tempfile` root — real pragmas, real WAL, real
//! transactions, real rollback. No store internals are touched: the
//! probe table is created by the tests themselves, so these tests
//! survive schema changes in `eunomia-store` (the store's own suite
//! covers the schema).
//!
//! The acceptance criteria of the T-023 card map to tests directly:
//!
//! - *an error mid-operation rolls back all of its changes* —
//!   [`a_failing_mid_operation_rolls_back_every_change`] (an explicit
//!   operation error) and
//!   [`a_sqlite_failure_mid_operation_rolls_back_and_maps_to_the_wire`]
//!   (a real constraint violation classified into the §10 registry);
//! - *commands do not block the async runtime* —
//!   [`a_long_operation_leaves_the_async_runtime_responsive`]: while
//!   enough long operations to occupy every runtime worker are in
//!   flight, an async round trip between two spawned tasks still
//!   completes in a fraction of the operations' duration. If the
//!   executor ran the operations inline on the async workers instead
//!   of `spawn_blocking`'s blocking pool, every worker would be
//!   parked inside `thread::sleep` and the round trip could only
//!   complete after the operations — which is what the assertion
//!   rules out;
//! - SYS-002's serialization —
//!   [`concurrent_operations_serialize_on_the_one_connection`]: the
//!   one connection is structural (`Connection` is `!Sync`, so it
//!   cannot be shared without the mutex); what the test proves on
//!   top is that concurrent operations queue on it without
//!   deadlocking, every transaction commits whole, and nothing
//!   half-interleaves.
//!
//! Two wiring pins keep the startup and the mechanism from silently
//! unwiring, in the `config_tests.rs` style: the `setup` hook must
//! open and manage the database after logging, and the executor must
//! go through `tauri::async_runtime::spawn_blocking`.
//!
//! The T-018 gate (DB-004/NFR-010, the "DB newer than the app" half
//! of the T-F11 failure scenario) is covered by the tests at the
//! bottom: a newer database — however it was created — is rejected as
//! the typed `E_DB_TOO_NEW` error and the file stays byte-identical,
//! while every other open failure keeps its D-027 abort shape.
//!
//! The T-020 gate (spec §6.3/§10, the "damaged DB" half of T-F11)
//! follows it: a file that fails `PRAGMA quick_check` — a garbage
//! file, a malformed page, a finding row — is rejected as the typed
//! `E_DB_CORRUPT` error with the file byte-identical.
//!
//! The T-021 daily backup (spec §10 ERR-001) is covered at the bottom:
//! one `VACUUM INTO` copy per UTC day through the [`Db`] seam, into
//! the layout's `db-backups/` — same-day idempotency, the 7-copy
//! retention, and the degrade-never-abort failure policy (a failed
//! copy is an error the caller logs while the database stays usable).
//!
//! The T-025 busy-retry cycle (§10 `E_DB_LOCKED`, §9.5 backoff)
//! follows it: a second, external connection holding the write lock
//! deterministically drives the executor through its retries — three
//! replays then the `E_DB_LOCKED` rejection, a release between
//! attempts letting the operation commit on a retry, and the cycle's
//! selectivity (an operation's own errors, constraint violations and
//! the recorded fallback never replay). The busy timeout is shrunk
//! through the test seam (`TEST_BUSY_TIMEOUT`); DB-001's production
//! value is untouched.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use tauri::async_runtime;

use crate::db::{
    Db, backup_records_to_app_error, sqlite_error_to_app_error, sync_runs_to_app_error,
};
use crate::errors::AppError;

/// The wall-clock budget the responsive-runtime probe must stay
/// within while the long operations are in flight. The operations
/// sleep for [`LONG_OPERATION_MS`] on the blocking pool (serialized
/// by the connection mutex, SYS-002, so the in-flight window spans
/// all of them); a healthy runtime schedules the probe within
/// milliseconds, and an inline-blocking regression overshoots the
/// budget by at least one full operation — the budget sits several
/// times below a single operation's duration.
const PROBE_BUDGET: Duration = Duration::from_millis(100);

/// How long each long operation parks its blocking thread. Chosen so
/// that one operation always outlives the probe budget several times
/// over, on both sides of the assertion.
const LONG_OPERATION_MS: Duration = Duration::from_millis(250);

/// A database over a real file in a fresh temp directory, opened the
/// way the `setup` hook opens it — full data-directory layout first
/// (`ensure_data_layout`, so `db/` and `db-backups/` exist as the
/// store requires), then `Db::open` from the root — and migrated to
/// the current schema, with the probe table the tests below read and
/// write. The `TempDir` is returned alongside so it outlives the `Db`
/// (dropping it first would delete the file under an open connection —
/// harmless for SQLite on Linux, but the ordering costs nothing).
fn probe_db() -> (tempfile::TempDir, Db) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let db = Db::open(temp.path()).expect("opening a fresh database in a temp directory must work");
    let migrated = db.clone();
    async_runtime::block_on(migrated.run(|tx| {
        tx.execute(
            "CREATE TABLE probe (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE)",
            [],
        )
        .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("creating the probe table must work");
    (temp, db)
}

/// Reads the probe table row count through the executor itself — the
/// same view a command would get, on the same connection, inside its
/// own transaction.
fn probe_count(db: &Db) -> i64 {
    let db = db.clone();
    async_runtime::block_on(db.run(|tx| {
        tx.query_row("SELECT COUNT(*) FROM probe", [], |row| row.get::<_, i64>(0))
            .map_err(sqlite_error_to_app_error)
    }))
    .expect("counting the probe table must work")
}

#[test]
fn a_failing_mid_operation_rolls_back_every_change() {
    // Arrange: a healthy database and an operation that inserts one
    // row, then fails on its own terms — the shape of every command
    // that validates as it writes (spec 6.9: an entity is never
    // saved partly).
    let (_temp, db) = probe_db();
    let failing = db.clone();

    // Act: the operation inserts a row and then returns an error.
    let error = async_runtime::block_on(failing.run::<(), _>(|tx| {
        tx.execute("INSERT INTO probe (name) VALUES ('first')", [])
            .map_err(sqlite_error_to_app_error)?;
        Err(AppError::new(crate::errors::ErrorCode::Validation))
    }))
    .expect_err("the operation must surface its error");

    // Assert: the caller sees the error itself (not a wrapped or
    // replaced one), and the row inserted before the failure is gone
    // — the rollback undid the whole operation, not the tail of it.
    assert_eq!(
        error,
        AppError::new(crate::errors::ErrorCode::Validation),
        "the operation's own AppError must reach the caller unchanged"
    );
    assert_eq!(
        probe_count(&db),
        0,
        "a mid-operation failure must roll back every change of the \
         operation (NFR-008)"
    );
}

#[test]
fn a_sqlite_failure_mid_operation_rolls_back_and_maps_to_the_wire() {
    // Arrange: one committed row, and an operation whose second
    // insert trips a real UNIQUE constraint — a genuine rusqlite
    // error taking the classifier path future commands will use.
    let (_temp, db) = probe_db();
    let seed = db.clone();
    async_runtime::block_on(seed.run(|tx| {
        tx.execute("INSERT INTO probe (name) VALUES ('committed')", [])
            .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("seeding one committed row must work");
    let failing = db.clone();

    // Act: 'kept' inserts fine, the duplicate 'committed' fails.
    let error = async_runtime::block_on(failing.run(|tx| {
        tx.execute("INSERT INTO probe (name) VALUES ('kept')", [])
            .map_err(sqlite_error_to_app_error)?;
        tx.execute("INSERT INTO probe (name) VALUES ('committed')", [])
            .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect_err("the duplicate insert must fail");

    // Assert: the failure maps to the §10 wire contract — a
    // constraint violation is the E_VALIDATION situation, and the
    // serialized form carries the literal code, the default message
    // key and the SQLite reason in details (the diagnostics UI-011
    // collapses). And the operation left nothing behind: 'kept' is
    // gone with it; only the earlier, committed row remains.
    let wire = serde_json::to_value(&error).expect("the error must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(wire["message_key"], serde_json::json!("errors.validation"));
    assert!(
        wire["details"]["os_message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "the SQLite reason must travel in details.os_message, got: {wire}"
    );
    assert_eq!(
        probe_count(&db),
        1,
        "the failed operation must roll back entirely — only the \
         previously committed row may remain"
    );
    let names = names_in_probe(&db);
    assert_eq!(names, vec!["committed".to_owned()]);
}

/// Reads the probe `name` column through the executor, ordered —
/// shared by the assertions that care about *which* rows survived.
fn names_in_probe(db: &Db) -> Vec<String> {
    let db = db.clone();
    async_runtime::block_on(db.run(|tx| {
        let mut statement = tx
            .prepare("SELECT name FROM probe ORDER BY name")
            .map_err(sqlite_error_to_app_error)?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sqlite_error_to_app_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sqlite_error_to_app_error)?;
        Ok(names)
    }))
    .expect("reading the probe names must work")
}

#[test]
fn a_successful_operation_commits_all_of_its_changes() {
    // Arrange & Act: one operation writing two rows — the unit the
    // transaction protects.
    let (_temp, db) = probe_db();
    let writing = db.clone();
    async_runtime::block_on(writing.run(|tx| {
        tx.execute("INSERT INTO probe (name) VALUES ('one')", [])
            .map_err(sqlite_error_to_app_error)?;
        tx.execute("INSERT INTO probe (name) VALUES ('two')", [])
            .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("a healthy operation must commit");

    // Assert: both changes are durable on the connection — visible
    // from a fresh transaction afterwards, the way a later command
    // would read them.
    assert_eq!(
        names_in_probe(&db),
        vec!["one".to_owned(), "two".to_owned()],
        "a committed operation must keep every change (SYS-002)"
    );
}

#[test]
fn concurrent_operations_serialize_on_the_one_connection() {
    // Arrange: more concurrent operations than runtime workers, each
    // writing its own rows inside its own transaction. The one
    // connection is enforced structurally (`Connection` is `!Sync`,
    // so sharing it requires the mutex), so this test cannot catch a
    // "missing mutex" — that does not compile; what it proves is the
    // behavior on top of that structure: concurrent operations queue
    // on the connection without deadlocking, every transaction
    // commits whole, and nothing half-interleaves.
    const OPERATIONS: usize = 8;
    const ROWS_PER_OPERATION: usize = 3;

    let (_temp, db) = probe_db();
    let start = Instant::now();
    async_runtime::block_on(async {
        let mut handles = Vec::new();
        for index in 0..OPERATIONS {
            let db = db.clone();
            handles.push(async_runtime::spawn(async move {
                db.run(move |tx| {
                    for row in 0..ROWS_PER_OPERATION {
                        tx.execute(
                            "INSERT INTO probe (name) VALUES (?1)",
                            [format!("op-{index}-row-{row}")],
                        )
                        .map_err(sqlite_error_to_app_error)?;
                    }
                    Ok(())
                })
                .await
            }));
        }
        for handle in handles {
            handle
                .await
                .expect("a spawned operation must not panic")
                .expect("every serialized operation must commit");
        }
    });

    // Assert: every operation committed every row — none was rejected,
    // none deadlocked (bounded by the sum of the operations' simple
    // writes), and none was silently interleaved into another's
    // transaction (a lost or duplicated row would show in the count).
    let expected = (OPERATIONS * ROWS_PER_OPERATION) as i64;
    assert_eq!(
        probe_count(&db),
        expected,
        "all {expected} rows from {OPERATIONS} concurrent operations \
         must be committed"
    );
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "serialization must not deadlock (bounded by the sum of the \
         operations' durations)"
    );
}

#[test]
fn a_long_operation_leaves_the_async_runtime_responsive() {
    // Arrange: enough long operations to occupy every async worker
    // thread — twice `available_parallelism` — each parking its
    // thread for LONG_OPERATION_MS inside the operation, plus the
    // async round-trip probe: a ping from the driving future to a
    // spawned task and its pong back, both of which the runtime must
    // schedule for the round trip to complete.
    let workers = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    let operations = workers * 2;

    let (_temp, db) = probe_db();
    let start = Instant::now();
    let round_trip = async_runtime::block_on(async {
        let mut handles = Vec::new();
        for index in 0..operations {
            let db = db.clone();
            handles.push(async_runtime::spawn(async move {
                db.run(move |tx| {
                    tx.execute(
                        "INSERT INTO probe (name) VALUES (?1)",
                        [format!("long-{index}")],
                    )
                    .map_err(sqlite_error_to_app_error)?;
                    thread::sleep(LONG_OPERATION_MS);
                    Ok(())
                })
                .await
            }));
        }
        // The probe: two spawned-task hops while the operations run.
        let (ping_sender, mut ping_receiver) = async_runtime::channel::<()>(1);
        let (pong_sender, mut pong_receiver) = async_runtime::channel::<()>(1);
        async_runtime::spawn(async move {
            ping_receiver
                .recv()
                .await
                .expect("the probe ping must be deliverable while operations run");
            pong_sender
                .send(())
                .await
                .expect("the probe pong must be sendable while operations run");
        });
        ping_sender
            .send(())
            .await
            .expect("the probe ping must be deliverable while operations run");
        pong_receiver
            .recv()
            .await
            .expect("the probe pong must arrive while operations run");
        let round_trip = start.elapsed();

        for handle in handles {
            handle
                .await
                .expect("a spawned operation must not panic")
                .expect("a long operation must still commit");
        }
        round_trip
    });

    // Assert: the probe completed while the operations were still in
    // flight (they serialize on the connection mutex, so the in-flight
    // window spans the whole set; every single operation outlives the
    // probe budget several times over). Had the executor run the
    // operations inline on the async workers, the operations — twice
    // the worker count — would have parked every worker inside
    // `thread::sleep`, and the probe could only complete after at
    // least one full operation finished, far beyond the budget. All
    // the while every operation committed.
    assert!(
        round_trip < PROBE_BUDGET,
        "an async round trip took {round_trip:?} while \
         {operations} operations were in flight: the async runtime \
         must stay responsive while database operations run \
         (SYS-001, NFR-008)"
    );
    assert_eq!(
        probe_count(&db),
        operations as i64,
        "every long operation must have committed its row"
    );
}

#[test]
fn a_panicking_operation_rolls_back_and_the_next_one_still_works() {
    // Arrange: an operation that panics after writing a row — the
    // worst mid-operation failure there is.
    let (_temp, db) = probe_db();
    let panicking = db.clone();

    // Act: the executor must not propagate the panic (it unwinds on
    // the blocking thread), and the unwinding must roll the open
    // transaction back before the mutex poisons.
    let error = async_runtime::block_on(panicking.run::<(), _>(|tx| {
        tx.execute("INSERT INTO probe (name) VALUES ('doomed')", [])
            .map_err(sqlite_error_to_app_error)?;
        panic!("operation bug");
    }))
    .expect_err("a panicking operation must surface as an AppError");

    // Assert: the fallback of the registry gap (D-037) — E_DB_LOCKED
    // with the panic preserved in details — nothing of the operation
    // was committed, and the connection still works for the next
    // operation (the poison was recovered, not propagated).
    assert_eq!(
        error.code,
        crate::errors::ErrorCode::DbLocked,
        "a panic must surface through the recorded fallback, got: {:?}",
        error.code
    );
    assert!(
        error
            .details
            .as_ref()
            .and_then(|details| details.os_message.as_deref())
            .is_some_and(|message| message.contains("operation bug")),
        "the panic message must stay visible in details.os_message, \
         got: {:?}",
        error.details
    );
    assert_eq!(
        probe_count(&db),
        0,
        "the panicking operation must roll back"
    );
    let next = db.clone();
    async_runtime::block_on(next.run(|tx| {
        tx.execute("INSERT INTO probe (name) VALUES ('after-recovery')", [])
            .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("the next operation must run on the recovered connection");
    assert_eq!(names_in_probe(&db), vec!["after-recovery".to_owned()]);
}

#[test]
fn sqlite_failures_map_to_registry_codes_and_the_wire_form() {
    // Arrange: the §10 situations the classifier covers, built from
    // real SQLite result codes through rusqlite's own error shape —
    // the exact input `BEGIN`/`COMMIT` and future command statements
    // produce. Expected literals are pinned from the §10 table, never
    // read from the enum. All nine mapped classes are here: the
    // or-groups of the classifier (corrupt+notadb,
    // readonly+perm+cannot-open) are exercised on every member.
    use rusqlite::ffi::{
        Error as SqliteError, SQLITE_BUSY, SQLITE_CANTOPEN, SQLITE_CONSTRAINT, SQLITE_CORRUPT,
        SQLITE_FULL, SQLITE_IOERR, SQLITE_NOTADB, SQLITE_PERM, SQLITE_READONLY,
    };

    let cases: [(SqliteError, &str, &str, &str); 9] = [
        (
            SqliteError::new(SQLITE_BUSY),
            "E_DB_LOCKED",
            "errors.db_locked",
            "DatabaseBusy",
        ),
        (
            SqliteError::new(SQLITE_CORRUPT),
            "E_DB_CORRUPT",
            "errors.db_corrupt",
            "DatabaseCorrupt",
        ),
        (
            SqliteError::new(SQLITE_NOTADB),
            "E_DB_CORRUPT",
            "errors.db_corrupt",
            "NotADatabase",
        ),
        (
            SqliteError::new(SQLITE_FULL),
            "E_DISK_FULL",
            "errors.disk_full",
            "DiskFull",
        ),
        (
            SqliteError::new(SQLITE_READONLY),
            "E_NO_WRITE_ACCESS",
            "errors.no_write_access",
            "ReadOnly",
        ),
        (
            SqliteError::new(SQLITE_PERM),
            "E_NO_WRITE_ACCESS",
            "errors.no_write_access",
            "PermissionDenied",
        ),
        (
            SqliteError::new(SQLITE_CANTOPEN),
            "E_NO_WRITE_ACCESS",
            "errors.no_write_access",
            "CannotOpen",
        ),
        (
            SqliteError::new(SQLITE_CONSTRAINT),
            "E_VALIDATION",
            "errors.validation",
            "ConstraintViolation",
        ),
        // The recorded fallback (D-037): no generic infrastructure
        // code exists in the twenty-code registry — but the class and
        // the numeric code still reach the wire (no collapse into an
        // "other" label, no lost result code).
        (
            SqliteError::new(SQLITE_IOERR),
            "E_DB_LOCKED",
            "errors.db_locked",
            "SystemIoFailure",
        ),
    ];

    // Act & Assert: each failure class serializes to its §10 wire
    // form — the code literal, the default message key, the exact
    // SQLite category, and the failure text with its numeric result
    // code in details — the shape the frontend renders.
    for (sqlite_error, code, message_key, sqlite_kind) in cases {
        let app_error = sqlite_error_to_app_error(rusqlite::Error::SqliteFailure(
            sqlite_error,
            Some("sqlite says no".to_owned()),
        ));
        let wire = serde_json::to_value(&app_error).expect("a classified error must serialize");
        assert_eq!(wire["code"], serde_json::json!(code));
        assert_eq!(wire["message_key"], serde_json::json!(message_key));
        assert_eq!(
            wire["details"]["os_kind"],
            serde_json::json!(sqlite_kind),
            "the exact SQLite category must survive classification"
        );
        let message = wire["details"]["os_message"]
            .as_str()
            .expect("os_message must be a string");
        assert!(
            message.contains("sqlite says no"),
            "the SQLite reason must not be lost in classification, got: {message}"
        );
        assert!(
            message.contains(&format!(
                "(sqlite result code {})",
                sqlite_error.extended_code
            )),
            "the numeric SQLite result code must stay attached, got: {message}"
        );
    }
}

#[test]
fn sync_runs_failures_map_to_the_wire_through_the_one_mapper() {
    // Arrange: the consolidated `sync_runs_to_app_error` — the ONE
    // spelling every `sync_runs` consumer routes through since the
    // five per-phase copies and the `commands.rs` one were deleted
    // (T-264). The `Sql` arm delegates to the shared classifier, so
    // the oracle here is the §10 table's literals (one representative
    // per or-group plus the unmapped fallback — the classifier's own
    // table test above pins all nine members); the `Member` arm is
    // the mapper's own semantics and is pinned in full: the corrupt
    // vocabulary cell surfaces class-only — the column is named, the
    // stored value never enters the wire (SEC-008, the D-039
    // corrupt-cells policy the per-phase suites pinned before the
    // copies dissolved).
    use eunomia_store::backup_records::BackupRecordsError;
    use eunomia_store::sync_runs::SyncRunsError;
    use rusqlite::ffi::{Error as SqliteError, SQLITE_BUSY, SQLITE_CONSTRAINT, SQLITE_FULL};

    use crate::errors::ErrorCode;

    let sql_cases: [(SqliteError, &str, &str, &str); 3] = [
        (
            SqliteError::new(SQLITE_BUSY),
            "E_DB_LOCKED",
            "errors.db_locked",
            "DatabaseBusy",
        ),
        (
            SqliteError::new(SQLITE_FULL),
            "E_DISK_FULL",
            "errors.disk_full",
            "DiskFull",
        ),
        (
            SqliteError::new(SQLITE_CONSTRAINT),
            "E_VALIDATION",
            "errors.validation",
            "ConstraintViolation",
        ),
    ];

    // Act & Assert: every classified store failure reaches the wire
    // exactly as the classifier shapes it — the delegation is the
    // mapper's contract, not a re-classification.
    for (sqlite_error, code, message_key, sqlite_kind) in sql_cases {
        let app_error = sync_runs_to_app_error(SyncRunsError::Sql(rusqlite::Error::SqliteFailure(
            sqlite_error,
            Some("the runs table says no".to_owned()),
        )));
        let wire = serde_json::to_value(&app_error).expect("a classified error must serialize");
        assert_eq!(wire["code"], serde_json::json!(code));
        assert_eq!(wire["message_key"], serde_json::json!(message_key));
        assert_eq!(wire["details"]["os_kind"], serde_json::json!(sqlite_kind));
    }

    // A non-`SqliteFailure` rusqlite error is not re-examined: it
    // lands on the classifier's recorded fallback (the arm the old
    // phase-0 pin covered — `InvalidQuery` stays `E_DB_LOCKED`).
    let fallback = sync_runs_to_app_error(SyncRunsError::Sql(rusqlite::Error::InvalidQuery));
    assert_eq!(fallback.code, ErrorCode::DbLocked);

    // The `Member` arm: the full class-only envelope, and the stored
    // value appears nowhere in the serialized wire — the hostile
    // `found` text proves the exclusion, not just the message shape.
    let member = sync_runs_to_app_error(SyncRunsError::Member {
        column: "status",
        found: "paused<script>".to_owned(),
    });
    let wire = serde_json::to_value(&member).expect("the corrupt-cell error must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(wire["message_key"], serde_json::json!("errors.db_locked"));
    assert_eq!(
        wire["details"]["os_kind"],
        serde_json::json!("SyncRunsCorrupt")
    );
    assert_eq!(
        wire["details"]["os_message"],
        serde_json::json!("a stored status cell is outside its vocabulary"),
        "the stored value must not enter the class-only diagnostic"
    );
    let wire_text = wire.to_string();
    assert!(
        !wire_text.contains("paused<script>"),
        "the offending stored value must not appear anywhere in the wire, got: {wire_text}"
    );

    // The backups-table sibling keeps its own distinct label — the
    // two corrupt-cell kinds name different tables, a deliberate
    // difference the consolidation preserves (T-264).
    let record = backup_records_to_app_error(BackupRecordsError::Member {
        column: "status",
        found: "ok<script>".to_owned(),
    });
    let record_wire =
        serde_json::to_value(&record).expect("the corrupt-record error must serialize");
    assert_eq!(
        record_wire["details"]["os_kind"],
        serde_json::json!("BackupRecordsCorrupt")
    );
    assert_eq!(
        record_wire["details"]["os_message"],
        serde_json::json!("a stored status cell is outside its vocabulary")
    );
}

#[test]
fn setup_opens_and_manages_the_database_after_logging() {
    // Arrange & Act: `lib.rs`, compile-time constant, with comments
    // stripped (the shared `config_tests.rs` helper). The DB step must
    // run after the logging startup line — the SYS-011 order already
    // pinned by `config_tests.rs` ends there — and publish the state
    // immediately, before `setup` returns: a command could otherwise
    // arrive at a window with no managed database.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    let markers = [
        "logging::startup(&data_dir, level);",
        "db::Db::open(",
        "app.manage(database);",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                lib.matches(marker).count(),
                1,
                "lib.rs must contain `{marker}` exactly once, live (a \
                 commented-out or duplicated line must not count)"
            );
            lib.find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();

    // Assert: open after logging, manage right after open. Honest
    // boundary, same as the other wiring pins: this proves the wiring
    // exists and stays wired, not that the OS file opened — the seam
    // behavior is covered by the tests above plus the manual first
    // launch.
    for pair in positions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "lib.rs must open and manage the database after logging \
             (SYS-011 order; D-037); marker positions: {positions:?}"
        );
    }
}

#[test]
fn the_executor_runs_operations_through_spawn_blocking() {
    // Arrange & Act: `db.rs`, compile-time constant, with comments
    // stripped so prose about the mechanism cannot satisfy the pin.
    let db_module = crate::config_tests::strip_comments(include_str!("db.rs"));

    // Assert: the SYS-001 mechanism itself — tauri's blocking pool —
    // must stay the only way an operation reaches the connection.
    // Swapping it for inline work on the async worker (the regression
    // the responsive-runtime test catches behaviorally) turns this red
    // first, at review speed.
    assert_eq!(
        db_module.matches("async_runtime::spawn_blocking(").count(),
        1,
        "db.rs must run every operation through \
         `tauri::async_runtime::spawn_blocking` exactly once (SYS-001, \
         D-037): blocking SQLite work never runs on the async workers"
    );
}

// ---- T-018: the DB-004 too-new gate (NFR-010, T-F11's new-DB half) --------

/// The latest schema version this app knows, read through the same
/// store API the gate compares against — the tests derive their
/// "newer" fixture versions from it, so they stay correct whenever
/// the embedded migration set grows.
fn app_schema_version() -> i64 {
    eunomia_store::storage::latest_schema_version() as i64
}

/// Creates a database file "by hand" (the T-F11 wording): a plain
/// SQLite connection — no DB-001 pragma set, the default rollback
/// (DELETE) journal mode — with `PRAGMA user_version` set to
/// `version`. This is the byte-level worst case for the gate: a file
/// the production `open_db` would *modify* (the WAL switch rewrites
/// the header), so any ordering that runs the pragmas before the
/// version check fails the byte-identity assertions below.
fn hand_made_database(db_path: &Path, version: i64) {
    let connection =
        rusqlite::Connection::open(db_path).expect("creating a database file by hand must work");
    connection
        .pragma_update(None, "user_version", version)
        .expect("setting user_version by hand must work");
}

/// Reads the database file's bytes — the T-F11 assertion compares the
/// actual bytes, not a proxy (hashes included).
fn file_bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("the database file must be readable")
}

/// The WAL sidecar sibling paths of `db_path` (`-wal`/`-shm`, the
/// names SQLite itself creates, spec §5.5).
fn sidecar_paths(db_path: &Path) -> [PathBuf; 2] {
    let wal = {
        let mut name = db_path.as_os_str().to_os_string();
        name.push("-wal");
        PathBuf::from(name)
    };
    let shm = {
        let mut name = db_path.as_os_str().to_os_string();
        name.push("-shm");
        PathBuf::from(name)
    };
    [wal, shm]
}

/// Asserts the §7.10 wire form of a too-new rejection: the §10
/// literal `E_DB_TOO_NEW`, its default message key, the params a
/// "Update the app" screen interpolates — the database path and both
/// schema versions, the expected values pinned from the fixture, not
/// read back from the code — and `details: null` (a verdict, not an
/// OS failure; D-034's optional-details contract).
fn assert_too_new_wire(app_error: &AppError, db_path: &Path, database_schema_version: i64) {
    let wire = serde_json::to_value(app_error).expect("the typed error must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_DB_TOO_NEW"));
    assert_eq!(wire["message_key"], serde_json::json!("errors.db_too_new"));
    assert_eq!(
        wire["params"]["path"],
        serde_json::json!(db_path.display().to_string()),
        "the params must carry the rejected database path, got: {wire}"
    );
    assert_eq!(
        wire["params"]["database_schema_version"],
        serde_json::json!(database_schema_version.to_string()),
        "the params must carry the file's schema version, got: {wire}"
    );
    assert_eq!(
        wire["params"]["app_schema_version"],
        serde_json::json!(app_schema_version().to_string()),
        "the params must carry the app's schema version, got: {wire}"
    );
    assert_eq!(
        wire["details"],
        serde_json::json!(null),
        "a too-new rejection carries no OS details, got: {wire}"
    );
}

/// Asserts the rejection left the whole `db/` and `db-backups/`
/// layout exactly as the fixture left it: the one database file, no
/// sidecars, and no safety copies — broadening the T-F11 "data not
/// changed" check beyond the main file's bytes.
fn assert_nothing_else_was_written(root: &Path, db_path: &Path) {
    let db_entries: Vec<String> =
        std::fs::read_dir(db_path.parent().expect("the layout creates db/"))
            .expect("the db directory must be readable")
            .map(|entry| {
                entry
                    .expect("every db entry must be readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
    let expected = db_path
        .file_name()
        .expect("the database file has a name")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        db_entries,
        vec![expected],
        "the rejected open must leave exactly the database file in db/"
    );
    let backups = crate::data_dirs::db_backups_dir(root);
    let backup_entries: Vec<String> = std::fs::read_dir(&backups)
        .expect("the db-backups directory must be readable")
        .map(|entry| {
            entry
                .expect("every backup entry must be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        backup_entries,
        Vec::<String>::new(),
        "the rejected open must write no safety copies"
    );
}

#[test]
fn a_newer_database_is_rejected_as_e_db_too_new_and_left_byte_identical() {
    // Arrange: the full layout plus a hand-made database one version
    // ahead (T-F11's "DB newer than the app" half) in rollback
    // journal mode — the file open_db's WAL switch would rewrite.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    let newer = app_schema_version() + 1;
    hand_made_database(&db_path, newer);
    let bytes_before = file_bytes(&db_path);

    // Act: the tested path — the same `Db::open` the `setup` hook
    // runs.
    let Err(error) = Db::open(temp.path()) else {
        panic!("a newer database must be rejected");
    };

    // Assert: the typed §10/§7.10 rejection, and the §10 effect "The
    // app does not change the database" verified on the actual bytes
    // (T-F11: the file is byte-identical), with no WAL sidecars left
    // next to it — the gate never opened a WAL-capable connection.
    let app_error = error
        .app_error()
        .expect("a too-new rejection must map to the typed AppError");
    assert_too_new_wire(&app_error, &db_path, newer);
    assert_eq!(
        file_bytes(&db_path),
        bytes_before,
        "the rejected database file must be byte-identical (T-F11)"
    );
    let [wal, shm] = sidecar_paths(&db_path);
    assert!(
        !wal.exists() && !shm.exists(),
        "the rejected open must not leave WAL sidecar files behind"
    );
    assert_nothing_else_was_written(temp.path(), &db_path);
    // The panic-path diagnostic (the abort's `Display` is the console
    // message until the "Update the app" screen exists): it names the
    // path and the §10 code literal.
    let message = error.to_string();
    assert!(
        message.contains(&db_path.display().to_string())
            && message.contains("E_DB_TOO_NEW")
            && message.contains(&newer.to_string()),
        "the abort message must name the path, the version and the \
         code literal, got: {message}"
    );
}

#[test]
fn a_much_newer_database_is_rejected_the_same_way() {
    // Arrange: a version far ahead — the gate compares, it does not
    // estimate distance, and a future database several versions newer
    // behaves exactly like one a single step ahead.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    let much_newer = app_schema_version() + 5;
    hand_made_database(&db_path, much_newer);
    let bytes_before = file_bytes(&db_path);

    // Act
    let Err(error) = Db::open(temp.path()) else {
        panic!("a much newer database must be rejected");
    };

    // Assert: the same typed rejection with the file's own version in
    // the params, and the file untouched.
    let app_error = error
        .app_error()
        .expect("a too-new rejection must map to the typed AppError");
    assert_too_new_wire(&app_error, &db_path, much_newer);
    assert_eq!(
        file_bytes(&db_path),
        bytes_before,
        "the rejected database file must be byte-identical (T-F11)"
    );
    assert_nothing_else_was_written(temp.path(), &db_path);
}

#[test]
fn an_up_to_date_database_opens_normally() {
    // The gate's boundary: a database at exactly the app's latest
    // version — the normal state of every existing user's database,
    // and the shape a `>=` mutation of the gate's comparison would
    // wrongly reject. It must open (the migration run is a no-op) and
    // keep its version.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    let backups_dir = crate::data_dirs::db_backups_dir(temp.path());
    {
        let mut connection = eunomia_store::storage::open_db(&db_path)
            .expect("opening a fresh database through the store must work");
        eunomia_store::storage::run_migrations(&mut connection, &backups_dir)
            .expect("migrating the fresh database must work");
    }

    // Act & Assert: the re-open succeeds and is usable — a plain
    // operation runs on the opened connection, and the version is
    // still the app's latest.
    let db = Db::open(temp.path()).expect("an up-to-date database must open");
    let probed = db.clone();
    let version = async_runtime::block_on(probed.run(|tx| {
        tx.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(sqlite_error_to_app_error)
    }))
    .expect("reading user_version through the executor must work");
    assert_eq!(version, app_schema_version());
}

#[test]
fn a_newer_wal_database_is_rejected_and_leaves_no_sidecar_files() {
    // Arrange: the production shape — a database this app's own store
    // made: `open_db` (DB-001 pragma set, WAL mode) plus
    // `run_migrations`, then "aged" newer by bumping `user_version`
    // the way a newer app version would leave it. The creating
    // connection's clean close checkpoints the bump into the main
    // file and removes the sidecars, so the fixture starts settled.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    let backups_dir = crate::data_dirs::db_backups_dir(temp.path());
    let newer = app_schema_version() + 1;
    {
        let mut connection = eunomia_store::storage::open_db(&db_path)
            .expect("opening a fresh database through the store must work");
        eunomia_store::storage::run_migrations(&mut connection, &backups_dir)
            .expect("migrating the fresh database must work");
        connection
            .pragma_update(None, "user_version", newer)
            .expect("bumping user_version must work");
    }
    let bytes_before = file_bytes(&db_path);

    // Act
    let Err(error) = Db::open(temp.path()) else {
        panic!("a newer WAL database must be rejected");
    };

    // Assert: the same typed rejection; the main file byte-identical;
    // and — the sidecar fact of the gate — the bare read the gate
    // performed on the WAL-mode file left no `-wal`/`-shm` debris
    // (SQLite creates both for the read and removes them on the clean
    // drop; recorded in DECISIONS.md with the too-new task).
    let app_error = error
        .app_error()
        .expect("a too-new rejection must map to the typed AppError");
    assert_too_new_wire(&app_error, &db_path, newer);
    assert_eq!(
        file_bytes(&db_path),
        bytes_before,
        "the rejected WAL database file must be byte-identical (T-F11)"
    );
    let [wal, shm] = sidecar_paths(&db_path);
    assert!(
        !wal.exists() && !shm.exists(),
        "the rejected open must not leave WAL sidecar files behind"
    );
    assert_nothing_else_was_written(temp.path(), &db_path);
}

#[test]
fn a_migration_failure_on_a_now_too_new_file_classifies_as_too_new() {
    // The D-038 backstop of the gate: `reject_too_new` passed, then
    // the file became newer under the open (swapped or rewritten
    // between the two steps) and `run_migrations` failed. The
    // classification is re-derived from the version fact on the open
    // connection — any failure with the fact present maps to the
    // typed rejection; the same failure without it keeps the generic
    // shape with the store error preserved.
    use rusqlite::ffi::{Error as SqliteError, SQLITE_IOERR};

    // Arrange: the two connections the classifier sees — one carrying
    // a too-new version, one within range — and one real store-layer
    // failure shape.
    let too_new = rusqlite::Connection::open_in_memory().unwrap();
    too_new
        .pragma_update(None, "user_version", app_schema_version() + 1)
        .unwrap();
    let within_range = rusqlite::Connection::open_in_memory().unwrap();
    within_range
        .pragma_update(None, "user_version", app_schema_version())
        .unwrap();
    let db_path = Path::new("/data/db/eunomiasync.sqlite3");

    // Act & Assert: with the too-new fact the failure classifies as
    // the typed rejection carrying both versions…
    let classified = crate::db::classify_migration_failure(
        &too_new,
        db_path,
        eunomia_store::storage::StorageError::Sql(rusqlite::Error::SqliteFailure(
            SqliteError::new(SQLITE_IOERR),
            None,
        )),
    );
    let crate::db::DbInitError::TooNew {
        database_schema_version,
        app_schema_version: latest_in_error,
        ..
    } = classified
    else {
        panic!("a too-new version fact must classify as TooNew, got: {classified:?}");
    };
    assert_eq!(database_schema_version, app_schema_version() + 1);
    assert_eq!(latest_in_error as i64, app_schema_version());

    // …and without it the same failure keeps the generic shape with
    // the store error preserved inside.
    let classified = crate::db::classify_migration_failure(
        &within_range,
        db_path,
        eunomia_store::storage::StorageError::Sql(rusqlite::Error::SqliteFailure(
            SqliteError::new(SQLITE_IOERR),
            None,
        )),
    );
    assert!(
        matches!(
            classified,
            crate::db::DbInitError::Open { ref source, .. }
                if matches!(
                    **source,
                    eunomia_store::storage::StorageError::Sql(_)
                )
        ),
        "an in-range version must keep the generic Open shape, got: {classified:?}"
    );
}

#[test]
fn setup_logs_a_too_new_database_through_the_facade_before_aborting() {
    // Arrange & Act: `lib.rs`, compile-time constant, comments
    // stripped (the shared helper). The too-new branch of the DB step
    // must log through the facade (D-032: tracing events live in
    // `logging.rs` only) before returning the error that aborts the
    // launch.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    let markers = [
        "db::Db::open(&data_dir)",
        "logging::db_too_new(",
        "return Err(error.into());",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                lib.matches(marker).count(),
                1,
                "lib.rs must contain `{marker}` exactly once, live (a \
                 commented-out or duplicated line must not count)"
            );
            lib.find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();

    // Assert: open, facade log, abort — in that order. Honest
    // boundary, same as the other wiring pins: this proves the wiring
    // exists and stays wired; the seam behavior is covered by the
    // tests above.
    for pair in positions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "lib.rs must log the too-new rejection through the facade \
             before aborting (DB-004, D-032); marker positions: {positions:?}"
        );
    }
}

// ---- T-019: the UPD-004 rolled-back migration --------------------------------

/// A database file whose first embedded migration must fail on it: a
/// valid SQLite database (it survives `open_db` and the too-new gate)
/// that already contains a `settings` table, which
/// `migrations/0001_init.sql` creates — the step dies on "table
/// settings already exists". This is the intentionally broken
/// migration reachable through the real seams; the store suite proves
/// the broken-step-in-an-upgrade-run shape on its private driver.
fn conflicting_settings_database(db_path: &Path) {
    let connection =
        rusqlite::Connection::open(db_path).expect("creating the fixture database must work");
    connection
        .execute(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT, updated_at TEXT)",
            [],
        )
        .expect("creating the conflicting table must work");
    connection
        .execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('k', 'v', '2026-11-18T10:00:00.000Z')",
            [],
        )
        .expect("seeding the fixture row must work");
}

/// Reads the full schema of the fixture as comparable rows — the
/// "database unchanged" proof compares the complete `sqlite_master`
/// inventory, not a proxy. `sql` is `COALESCE`d: internal auto-index
/// rows carry NULL.
fn settings_schema(connection: &rusqlite::Connection) -> Vec<(String, String, String)> {
    let mut statement = connection
        .prepare("SELECT type, name, COALESCE(sql, '') FROM sqlite_master ORDER BY type, name")
        .expect("sqlite_master must be readable");
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("sqlite_master rows must be readable");
    rows.map(|row| row.expect("every sqlite_master row must decode"))
        .collect()
}

#[test]
fn a_failing_bootstrap_migration_aborts_with_the_unchanged_database() {
    // The honest-degradation half of UPD-004 at the real seam: the
    // migration fails on a database this run created (a fresh
    // bootstrap — `user_version = 0`, no copies exist), so the typed
    // error must say the database is unchanged and that no copy was
    // written, without inventing a pointer, and must keep the generic
    // abort (no wire form: this is not the "install the previous
    // version" situation).
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    conflicting_settings_database(&db_path);
    let (schema_before, row_before) = {
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        let row: String = connection
            .query_row("SELECT value FROM settings", [], |row| row.get(0))
            .unwrap();
        (settings_schema(&connection), row)
    };

    // Act
    let error = match Db::open(temp.path()) {
        Ok(_) => panic!("the conflicting table must fail the migration"),
        Err(error) => error,
    };

    // Assert: the typed Migration error without a copy, honest about
    // there being none, and the generic abort (no wire form).
    let crate::db::DbInitError::Migration { backup: None, .. } = error else {
        panic!("a failed migration must classify as Migration, got: {error:?}");
    };
    assert!(
        error.app_error().is_none(),
        "a fresh bootstrap failure has no UPD-004 wire form"
    );
    let message = error.to_string();
    assert!(
        message.contains(&db_path.display().to_string()),
        "the abort message must name the database file, got: {message}"
    );
    assert!(
        message.contains("keeps the state of its last committed migration"),
        "the abort message must state the kept state, got: {message}"
    );
    assert!(
        message.contains("no pre-migration copy"),
        "the abort message must be honest about the missing copy, got: {message}"
    );

    // The database itself is unchanged: schema, data and version.
    {
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        assert_eq!(
            settings_schema(&connection),
            schema_before,
            "the failed migration must not change the schema"
        );
        let row: String = connection
            .query_row("SELECT value FROM settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            row, row_before,
            "the failed migration must not change the data"
        );
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 0, "no migration step was applied");
    }

    // A fresh run writes no copies: the backup directory stays empty.
    let backups = crate::data_dirs::db_backups_dir(temp.path());
    let backup_entries = std::fs::read_dir(&backups)
        .expect("the db-backups directory must be readable")
        .count();
    assert_eq!(backup_entries, 0, "no copies, no debris");
}

#[test]
fn a_migration_failure_with_a_copy_names_it_in_display_and_wire_form() {
    // The UPD-004 situation proper: a migration failed on an existing
    // database, the store's error carries the copy of the unchanged
    // state, and the app layer must surface it everywhere the user (or
    // the future screen task) looks — the abort `Display` text, and
    // the typed §7.10 wire form.
    //
    // Honest boundary: src-tauri does not depend on
    // `rusqlite_migration`, so the fixture cannot synthesize the
    // store's variant; it obtains a real one by driving the store (the
    // conflicting-table fixture above) and re-shapes its `backup`
    // field to the under-test case. This pins the path *plumbing* on a
    // real store error; the copy file's validity (VACUUM INTO,
    // integrity, exact state) is the store suite's proof, unreachable
    // through `Db::open` until a second embedded migration exists.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    let backups_dir = crate::data_dirs::db_backups_dir(temp.path());
    conflicting_settings_database(&db_path);
    let real_error = {
        let mut connection = eunomia_store::storage::open_db(&db_path)
            .expect("opening the fixture database must work");
        eunomia_store::storage::run_migrations(&mut connection, &backups_dir)
            .expect_err("the conflicting table must fail the migration")
    };
    let eunomia_store::storage::StorageError::Migration {
        source,
        backup: None,
    } = real_error
    else {
        panic!("the store must report a migration failure");
    };
    // A stand-in file at the real path shape the store hands over —
    // this test pins the plumbing; the copy's content and validity are
    // the store suite's proof.
    let backup_path = backups_dir.join("pre-migration-v1-20261118T101500123Z.sqlite3");
    std::fs::write(&backup_path, b"fixture stand-in for the pre-migration copy")
        .expect("writing the fixture copy must work");
    let store_error = eunomia_store::storage::StorageError::Migration {
        source,
        backup: Some(backup_path.clone()),
    };

    // Act: the classifier — a connection within range (the database is
    // at the failed step's source state, not newer than the app).
    let within_range = rusqlite::Connection::open_in_memory().unwrap();
    within_range
        .pragma_update(None, "user_version", app_schema_version())
        .unwrap();
    let error = crate::db::classify_migration_failure(&within_range, &db_path, store_error);

    // Assert: the typed variant preserves everything.
    let crate::db::DbInitError::Migration {
        path,
        backup: Some(backup),
        ..
    } = &error
    else {
        panic!("a migration failure must classify as Migration, got: {error:?}");
    };
    assert_eq!(path, &db_path);
    assert_eq!(backup, &backup_path, "the copy pointer must survive");

    // The abort Display (the panic prints it) names the copy and the
    // UPD-004 user action.
    let message = error.to_string();
    assert!(
        message.contains("install the previous app version"),
        "the abort message must carry the UPD-004 instruction, got: {message}"
    );
    assert!(
        message.contains(&backup_path.display().to_string()),
        "the abort message must name the copy, got: {message}"
    );

    // The wire form the future screen renders: E_UPDATE_FAILED with
    // the migration-specific message key and both paths as params —
    // pinned as literals, never read back from the code.
    let app_error = error
        .app_error()
        .expect("a migration failure with a copy must have the wire form");
    let wire = serde_json::to_value(&app_error).expect("the typed error must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_UPDATE_FAILED"));
    assert_eq!(
        wire["message_key"],
        serde_json::json!("errors.update_migration_failed")
    );
    assert_eq!(
        wire["params"]["path"],
        serde_json::json!(db_path.display().to_string()),
        "the params must carry the database path, got: {wire}"
    );
    assert_eq!(
        wire["params"]["backup_path"],
        serde_json::json!(backup_path.display().to_string()),
        "the params must carry the copy path, got: {wire}"
    );
    assert_eq!(
        wire["details"],
        serde_json::json!(null),
        "a rolled-back migration carries no OS details, got: {wire}"
    );
}

#[test]
fn a_backup_failure_keeps_the_generic_open_abort() {
    // A failed safety-copy write is not a migration failure (UPD-004
    // speaks of a migration that ran and was rolled back): it keeps
    // D-038's own `StorageError::Backup` shape and the generic abort,
    // proven at the store seam by its own suite — this pins the app
    // layer's half, that such an error does not masquerade as the
    // typed Migration rejection.
    let within_range = rusqlite::Connection::open_in_memory().unwrap();
    within_range
        .pragma_update(None, "user_version", app_schema_version())
        .unwrap();
    let classified = crate::db::classify_migration_failure(
        &within_range,
        Path::new("/data/db/eunomiasync.sqlite3"),
        eunomia_store::storage::StorageError::Backup {
            path: PathBuf::from("/data/db-backups"),
            source: eunomia_store::storage::BackupError::Directory,
        },
    );
    assert!(
        matches!(classified, crate::db::DbInitError::Open { .. }),
        "a backup failure must keep the generic Open abort, got: {classified:?}"
    );
    assert!(
        classified.app_error().is_none(),
        "a backup failure has no wire form of its own"
    );
}

#[test]
fn setup_logs_a_rolled_back_migration_through_the_facade_before_aborting() {
    // Arrange & Act: `lib.rs`, compile-time constant, comments
    // stripped (the shared helper). The migration branch of the DB
    // step must log through its own facade event before returning the
    // error that aborts the launch (UPD-004; the same wiring shape as
    // the too-new pin above).
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    let markers = [
        "db::Db::open(&data_dir)",
        "logging::db_migration_failed(",
        "return Err(error.into());",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                lib.matches(marker).count(),
                1,
                "lib.rs must contain `{marker}` exactly once, live (a \
                 commented-out or duplicated line must not count)"
            );
            lib.find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();

    // Assert: open, facade log, abort — in that order.
    for pair in positions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "lib.rs must log the rolled-back migration through the facade \
             before aborting (UPD-004, D-032); marker positions: {positions:?}"
        );
    }
}

// ---- T-020: the §6.3 quick_check gate (T-F11's damaged-DB half) ---------------

/// Builds a database whose damage `quick_check` reports as a **finding
/// row**: the file is a valid hand-made SQLite database (header and
/// `user_version` intact, so the too-new gate passes), but the
/// in-header page count (offset 28, big-endian) is shrunk below the
/// pages the file actually holds — the first row `quick_check` returns
/// is a page-level finding ("Tree 2 page 2 … invalid page number 3" in
/// the verified shape), not "ok".
fn database_with_a_quick_check_finding(db_path: &Path) {
    let connection =
        rusqlite::Connection::open(db_path).expect("creating the fixture database must work");
    connection
        .execute("CREATE TABLE marker (value TEXT NOT NULL)", [])
        .expect("creating the fixture table must work");
    // A row big enough to spill the table b-tree past page 2, so the
    // shrunk page count orphans real content (the verified CLI shape:
    // a 6000-character text puts the leaf's cell out of the "valid"
    // page range).
    connection
        .execute(
            "INSERT INTO marker (value) VALUES (hex(randomblob(3000)))",
            [],
        )
        .expect("seeding the fixture row must work");
    drop(connection);

    let mut bytes = file_bytes(db_path);
    let page_size = u16::from_be_bytes([bytes[16], bytes[17]]) as usize;
    let page_size = if page_size == 1 { 65536 } else { page_size };
    let pages = bytes.len() / page_size;
    assert!(
        pages >= 3,
        "the fixture must hold at least 3 pages (found {pages}), or the \
         shrunk count has nothing to orphan"
    );
    bytes[28..32].copy_from_slice(&((pages - 1) as u32).to_be_bytes());
    std::fs::write(db_path, &bytes).expect("damaging the fixture file must work");
}

/// Builds a database whose damage `quick_check` reports as an
/// **error**: the file's header stays valid (the too-new gate still
/// reads `user_version`), but the first page after the header page —
/// the table b-tree root — is overwritten with non-page bytes, so the
/// check fails with SQLite's own "database disk image is malformed"
/// instead of returning rows.
fn database_with_a_malformed_page(db_path: &Path) {
    let connection =
        rusqlite::Connection::open(db_path).expect("creating the fixture database must work");
    connection
        .execute("CREATE TABLE marker (value TEXT NOT NULL)", [])
        .expect("creating the fixture table must work");
    connection
        .execute("INSERT INTO marker (value) VALUES ('user data')", [])
        .expect("seeding the fixture row must work");
    drop(connection);

    let mut bytes = file_bytes(db_path);
    let page_size = u16::from_be_bytes([bytes[16], bytes[17]]) as usize;
    let page_size = if page_size == 1 { 65536 } else { page_size };
    assert!(
        bytes.len() >= page_size + 16,
        "the fixture must hold a second page to damage"
    );
    // Smash the page-type byte (and a few neighbors) of the table's
    // root page: no valid b-tree page starts like this.
    bytes[page_size..page_size + 16].copy_from_slice(b"NOT-A-BTREE-PAGE");
    std::fs::write(db_path, &bytes).expect("damaging the fixture file must work");
}

/// Builds a **WAL-mode** damaged database — the production shape: a
/// database this app's own store made (`open_db` + `run_migrations`;
/// the clean close checkpoints and settles the file with the WAL
/// flag in its header and no sidecars), then a page of the settled
/// file is smashed by hand. The DELETE-mode fixtures above cannot
/// produce sidecars at all; this fixture makes the gate's bare read
/// exercise its WAL path — SQLite creates the `-shm`/`-wal` siblings
/// for the read and must remove them on the clean drop (the D-040
/// sidecar facts, here on the corrupt rejection path).
fn damaged_wal_database(db_path: &Path, backups_dir: &Path) {
    let mut connection = eunomia_store::storage::open_db(db_path)
        .expect("opening a fresh database through the store must work");
    eunomia_store::storage::run_migrations(&mut connection, backups_dir)
        .expect("migrating the fresh database must work");
    drop(connection);

    let mut bytes = file_bytes(db_path);
    let page_size = u16::from_be_bytes([bytes[16], bytes[17]]) as usize;
    let page_size = if page_size == 1 { 65536 } else { page_size };
    assert!(
        bytes.len() >= page_size + 16,
        "the migrated fixture must hold a second page to damage"
    );
    // Smash the root page of the first object after the schema page:
    // the header (and `user_version`) stays valid, the b-tree walk
    // does not.
    bytes[page_size..page_size + 16].copy_from_slice(b"NOT-A-BTREE-PAGE");
    std::fs::write(db_path, &bytes).expect("damaging the fixture file must work");
}

/// Asserts the §7.10 wire form of a corrupt rejection: the §10
/// literal `E_DB_CORRUPT`, its default message key, the params a
/// recovery screen interpolates — the database path and SQLite's own
/// evidence, non-empty but not pinned word-for-word (SQLite owns that
/// text) — and `details: null` (a verdict, not an OS failure; D-034's
/// optional-details contract).
fn assert_corrupt_wire(app_error: &AppError, db_path: &Path) {
    let wire = serde_json::to_value(app_error).expect("the typed error must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_DB_CORRUPT"));
    assert_eq!(wire["message_key"], serde_json::json!("errors.db_corrupt"));
    assert_eq!(
        wire["params"]["path"],
        serde_json::json!(db_path.display().to_string()),
        "the params must carry the rejected database path, got: {wire}"
    );
    let detail = wire["params"]["detail"].as_str().unwrap_or_default();
    assert!(
        !detail.is_empty(),
        "the params must carry SQLite's quick_check evidence, got: {wire}"
    );
    assert_eq!(
        wire["details"],
        serde_json::json!(null),
        "a corrupt rejection carries no OS details, got: {wire}"
    );
}

#[test]
fn a_corrupt_database_is_rejected_as_e_db_corrupt_and_left_byte_identical() {
    // Arrange: the full layout plus a hand-made database whose damage
    // `quick_check` reports as a finding row (T-F11's "damaged DB"
    // half) in rollback journal mode — the file open_db's WAL switch
    // would rewrite if the gate ran after it.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    database_with_a_quick_check_finding(&db_path);
    let bytes_before = file_bytes(&db_path);

    // Act: the tested path — the same `Db::open` the `setup` hook
    // runs.
    let Err(error) = Db::open(temp.path()) else {
        panic!("a damaged database must be rejected");
    };

    // Assert: the typed §10/§7.10 rejection, and §10's "start in
    // recovery mode" over a database the app did not touch — verified
    // on the actual bytes (T-F11), with no WAL sidecars and no safety
    // copies (the gate never opened a WAL-capable connection). The
    // detail pin proves this is the finding-row shape (SQLite's page
    // finding), not the error shape — the classification the gate
    // docs and D-045 document, which would otherwise drift silently.
    let app_error = error
        .app_error()
        .expect("a corrupt rejection must map to the typed AppError");
    assert_corrupt_wire(&app_error, &db_path);
    let detail = app_error
        .params
        .get("detail")
        .expect("the corrupt rejection carries SQLite's evidence");
    assert!(
        detail.contains("invalid page number"),
        "the finding-row shape carries SQLite's own page finding, got: {detail}"
    );
    assert!(
        !detail.to_lowercase().contains("malformed"),
        "a finding row must not be confused with the error shape, got: {detail}"
    );
    assert_eq!(
        file_bytes(&db_path),
        bytes_before,
        "the rejected database file must be byte-identical (T-F11)"
    );
    let [wal, shm] = sidecar_paths(&db_path);
    assert!(
        !wal.exists() && !shm.exists(),
        "the rejected open must not leave WAL sidecar files behind"
    );
    assert_nothing_else_was_written(temp.path(), &db_path);
    // The panic-path diagnostic (the abort's `Display` is the console
    // message until the recovery screen exists): it names the path and
    // the §10 code literal.
    let message = error.to_string();
    assert!(
        message.contains(&db_path.display().to_string())
            && message.contains("E_DB_CORRUPT")
            && message.contains("recovery mode"),
        "the abort message must name the path, the code literal and the \
         §10 effect, got: {message}"
    );
}

#[test]
fn a_malformed_database_is_rejected_as_e_db_corrupt_and_left_byte_identical() {
    // The other detection shape: the damage makes `quick_check` itself
    // fail with SQLite's "database disk image is malformed" error
    // rather than return a finding row — the rejection must be the
    // same typed verdict over the same untouched file.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    database_with_a_malformed_page(&db_path);
    let bytes_before = file_bytes(&db_path);

    // Act
    let Err(error) = Db::open(temp.path()) else {
        panic!("a malformed database must be rejected");
    };

    // Assert: the same typed rejection, the same untouched file. The
    // detail is SQLite's own evidence (carried verbatim), pinned to
    // this shape's marker: on the bundled build a malformed image
    // surfaces as a finding row that itself carries the b-tree init
    // error — the counterpart of the page-count pin above, so neither
    // detection shape can drift into the other unnoticed.
    let app_error = error
        .app_error()
        .expect("a malformed database must map to the typed AppError");
    assert_corrupt_wire(&app_error, &db_path);
    let detail = app_error
        .params
        .get("detail")
        .expect("the corrupt rejection carries SQLite's evidence");
    assert!(
        detail.contains("btreeInitPage"),
        "the malformed shape carries SQLite's own b-tree error, got: {detail}"
    );
    assert!(
        !detail.contains("invalid page number"),
        "the malformed shape must not be confused with the page-count finding, got: {detail}"
    );
    assert_eq!(
        file_bytes(&db_path),
        bytes_before,
        "the rejected database file must be byte-identical (T-F11)"
    );
    assert_nothing_else_was_written(temp.path(), &db_path);
}

#[test]
fn a_damaged_wal_database_is_rejected_and_leaves_no_sidecar_files() {
    // The production shape of the damaged DB: the file is WAL-mode
    // (this app's own store made it), so the gate's bare read creates
    // the `-shm`/`-wal` siblings for the read and must remove them on
    // the clean drop — the D-040 sidecar facts, exercised on the
    // corrupt rejection path. The DELETE-mode fixtures cannot produce
    // sidecars at all; this is the meaningful half of the no-debris
    // assertion.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    let backups_dir = crate::data_dirs::db_backups_dir(temp.path());
    damaged_wal_database(&db_path, &backups_dir);
    let bytes_before = file_bytes(&db_path);

    // Act
    let Err(error) = Db::open(temp.path()) else {
        panic!("a damaged WAL database must be rejected");
    };

    // Assert: the typed rejection, the main file byte-identical, and
    // no sidecar debris next to it.
    let app_error = error
        .app_error()
        .expect("a damaged WAL database must map to the typed AppError");
    assert_corrupt_wire(&app_error, &db_path);
    assert_eq!(
        file_bytes(&db_path),
        bytes_before,
        "the rejected WAL database file must be byte-identical (T-F11)"
    );
    let [wal, shm] = sidecar_paths(&db_path);
    assert!(
        !wal.exists() && !shm.exists(),
        "the rejected open must not leave WAL sidecar files behind"
    );
    assert_nothing_else_was_written(temp.path(), &db_path);
}

#[test]
fn an_unopenable_database_path_is_not_a_corrupt_verdict() {
    // The gate's honesty guard: a path the bare open cannot even open
    // (here: a directory where the database belongs — `metadata` sees
    // a non-empty "file") is "cannot verify", not "verified corrupt":
    // the gate proceeds and `open_db` produces the canonical generic
    // abort naming the path. The same branch covers a busy/locked file
    // beyond the busy timeout — untestable without a multi-second
    // stall, so this test pins the branch's other arm.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    std::fs::create_dir(&db_path).expect("creating a directory at the db path must work");

    // Act
    let Err(error) = Db::open(temp.path()) else {
        panic!("an unopenable path must fail the open");
    };

    // Assert: the generic `Open` abort with no typed verdict — the
    // corrupt rejection must not fire on a file that could not be
    // read at all.
    assert!(
        matches!(error, crate::db::DbInitError::Open { .. }),
        "an unopenable path must keep the generic Open abort, got: {error:?}"
    );
    assert!(
        error.app_error().is_none(),
        "no typed verdict may be invented for an unopenable path"
    );
    assert!(
        error.to_string().contains(&db_path.display().to_string()),
        "the abort message must name the database path, got: {error}"
    );
}

#[test]
fn a_non_database_file_is_rejected_as_e_db_corrupt_and_not_touched() {
    // Arrange: garbage bytes where the database belongs — the third
    // detection shape: `quick_check` fails at prepare with
    // SQLITE_NOTADB, "file is not a database". Until T-020 this kept
    // the D-037 generic abort; the integrity gate now types it. It
    // must still never masquerade as the too-new rejection.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    let garbage = b"definitely not a sqlite database, just bytes in the way";
    std::fs::write(&db_path, garbage).expect("writing the fixture file must work");

    // Act
    let Err(error) = Db::open(temp.path()) else {
        panic!("a non-database file must fail the open");
    };

    // Assert: the typed `E_DB_CORRUPT` rejection with SQLite's own
    // "not a database" evidence in the params, the message naming the
    // file (the panic prints `Display`), and the failed open having
    // modified nothing.
    let app_error = error
        .app_error()
        .expect("a garbage file must map to the typed corrupt rejection");
    assert_corrupt_wire(&app_error, &db_path);
    assert!(
        !matches!(error, crate::db::DbInitError::TooNew { .. }),
        "a garbage file must not masquerade as a too-new rejection"
    );
    let message = error.to_string();
    assert!(
        message.contains(&db_path.display().to_string()),
        "the abort message must name the database file, got: {message}"
    );
    let detail = app_error
        .params
        .get("detail")
        .expect("the corrupt rejection carries SQLite's evidence");
    assert!(
        detail.to_lowercase().contains("not a database"),
        "SQLite's corruption reason must stay in the detail, got: {detail}"
    );
    assert_eq!(
        file_bytes(&db_path),
        garbage.to_vec(),
        "the failed open must not modify the file"
    );
}

#[test]
fn an_empty_database_file_is_not_corrupt_and_opens() {
    // The gate's skip condition (T-F11 says "data not changed", not
    // "every file must pass the check"): a 0-byte file is a valid
    // empty SQLite database — nothing to verify — so the open proceeds
    // as a fresh bootstrap instead of failing the integrity check.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    std::fs::write(&db_path, b"").expect("creating the empty fixture file must work");

    // Act & Assert: the open succeeds and migrates the empty file —
    // the version fact a fresh bootstrap leaves behind.
    let db = Db::open(temp.path()).expect("an empty database file must open, not fail the check");
    let probed = db.clone();
    let version = async_runtime::block_on(probed.run(|tx| {
        tx.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(sqlite_error_to_app_error)
    }))
    .expect("reading user_version through the executor must work");
    assert_eq!(version, app_schema_version());
}

#[test]
fn a_too_new_and_damaged_database_answers_too_new_first() {
    // The gates' order: DB-004 fixes the too-new gate as the first
    // step (D-040), so a file that is both newer than the app and
    // damaged answers "too new" — the cheaper, header-only verdict —
    // and never pays the full-file scan or surfaces the damage.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db_path = crate::data_dirs::db_file_path(temp.path());
    database_with_a_quick_check_finding(&db_path);
    {
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        connection
            .pragma_update(None, "user_version", app_schema_version() + 1)
            .expect("bumping user_version must work");
    }
    let bytes_before = file_bytes(&db_path);

    // Act
    let Err(error) = Db::open(temp.path()) else {
        panic!("a newer database must be rejected");
    };

    // Assert: the too-new rejection, not the corrupt one, over the
    // same untouched file.
    assert!(
        matches!(error, crate::db::DbInitError::TooNew { .. }),
        "the too-new gate must decide before the integrity gate, got: {error:?}"
    );
    assert_eq!(
        file_bytes(&db_path),
        bytes_before,
        "the rejected database file must be byte-identical (T-F11)"
    );
}

#[test]
fn setup_logs_a_corrupt_database_and_continues_in_recovery_mode() {
    // Arrange & Act: `lib.rs`, compile-time constant, comments
    // stripped (the shared helper). Since T-024 the corrupt branch of
    // the DB step logs through its own facade event and returns Ok —
    // recovery mode: the §10 screen replaces the app, so the process
    // must stay alive (the abort stays for the other init shapes; its
    // own pin in recovery_tests.rs covers the same file from the
    // recovery side).
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    let markers = [
        "db::Db::open(&data_dir)",
        "logging::db_corrupt(",
        "return Ok(());",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                lib.matches(marker).count(),
                1,
                "lib.rs must contain `{marker}` exactly once, live (a \
                 commented-out or duplicated line must not count)"
            );
            lib.find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();

    // Assert: open, facade log, continue — in that order.
    for pair in positions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "lib.rs must log the corrupt rejection through the facade \
             before continuing into recovery mode (§6.3, §10, T-024); \
             marker positions: {positions:?}"
        );
    }
}

// ---- T-021: the ERR-001 daily backup (spec §10) ------------------------------

/// A fixed UTC instant of 2026-11-18 — the date seam's test input, so
/// the day the daily copy is named for never depends on the wall clock
/// (the seam itself, `Db::ensure_daily_backup(now)`, takes it as a
/// parameter; the `setup` hook passes the real clock).
fn daily_seam_now() -> jiff::Timestamp {
    "2026-11-18T10:00:00.000Z"
        .parse()
        .expect("the fixed seam timestamp must parse")
}

/// The `db-backups/` directory of a probe database's data directory —
/// the layout created it (D-027), `Db::open` derived it (D-037), and
/// the daily step must write into exactly it.
fn backups_dir_of(temp: &tempfile::TempDir) -> PathBuf {
    temp.path().join("db-backups")
}

/// The sorted `daily-*.sqlite3` names in `dir`.
fn daily_backup_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("reading the backup directory must work")
        .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
        .filter(|name| name.starts_with("daily-"))
        .collect();
    names.sort();
    names
}

#[test]
fn the_daily_backup_writes_a_valid_copy_into_the_layouts_db_backups() {
    // The app-layer half of ERR-001: the copy lands in the §5.5
    // `db-backups/` next to the migration safety copies, as a valid
    // database holding the state at the moment of the copy.
    let (temp, db) = probe_db();
    // The probe table with one row — the data the copy must preserve.
    let writer = db.clone();
    async_runtime::block_on(writer.run(|tx| {
        tx.execute("INSERT INTO probe (name) VALUES ('daily-marker')", [])
            .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("seeding the probe row must work");

    let outcome = db
        .ensure_daily_backup(daily_seam_now())
        .expect("the daily copy must be written");
    assert_eq!(
        outcome,
        eunomia_store::storage::DailyBackupOutcome::Written(
            backups_dir_of(&temp).join("daily-20261118.sqlite3")
        ),
        "the copy is named for the UTC day of the seam input, in the layout's db-backups/"
    );

    // The copy is a valid database with the data (the store suite pins
    // the deeper mechanics; this pins the app-layer path plumbing).
    let copy = rusqlite::Connection::open(backups_dir_of(&temp).join("daily-20261118.sqlite3"))
        .expect("the copy must open");
    let integrity: String = copy
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    let marker: String = copy
        .query_row("SELECT name FROM probe", [], |row| row.get(0))
        .unwrap();
    assert_eq!(marker, "daily-marker", "the copy preserves the data");
}

#[test]
fn a_same_day_rerun_of_the_daily_backup_writes_no_second_copy() {
    // The ERR-001 acceptance criterion, through the app-layer seam:
    // a second run on the same UTC day must not create a second copy.
    let (temp, db) = probe_db();

    let first = db
        .ensure_daily_backup(daily_seam_now())
        .expect("the first run of the day writes the copy");
    assert!(matches!(
        first,
        eunomia_store::storage::DailyBackupOutcome::Written(_)
    ));

    // A later instant of the same UTC day — the restart shape; the
    // store named the file from the date, so this is the same name.
    let later = db.ensure_daily_backup(
        "2026-11-18T22:30:00.000Z"
            .parse()
            .expect("the fixed seam timestamp must parse"),
    );
    assert_eq!(
        later.expect("a same-day rerun must succeed"),
        eunomia_store::storage::DailyBackupOutcome::AlreadyPresent(
            backups_dir_of(&temp).join("daily-20261118.sqlite3")
        ),
        "the second run of the day must find the copy and write nothing"
    );
    assert_eq!(
        daily_backup_names(&backups_dir_of(&temp)),
        vec!["daily-20261118.sqlite3"],
        "exactly one copy exists for the day"
    );
}

#[test]
fn the_eighth_daily_copy_through_the_db_seam_keeps_the_last_seven() {
    // The ERR-001 acceptance criterion at the app layer: seven days of
    // history plus today's copy is one too many — the oldest goes, and
    // the pre-migration safety copies are outside the cap.
    let (temp, db) = probe_db();
    let backups = backups_dir_of(&temp);
    for date in [
        "20261111", "20261112", "20261113", "20261114", "20261115", "20261116", "20261117",
    ] {
        std::fs::write(backups.join(format!("daily-{date}.sqlite3")), b"history").unwrap();
    }
    let pre_migration = backups.join("pre-migration-v1-20261110T101500123Z.sqlite3");
    std::fs::write(&pre_migration, b"safety copy").unwrap();

    let outcome = db
        .ensure_daily_backup(daily_seam_now())
        .expect("today's copy must be written");
    assert!(matches!(
        outcome,
        eunomia_store::storage::DailyBackupOutcome::Written(_)
    ));

    let mut expected: Vec<String> = [
        "20261112", "20261113", "20261114", "20261115", "20261116", "20261117", "20261118",
    ]
    .iter()
    .map(|date| format!("daily-{date}.sqlite3"))
    .collect();
    expected.sort();
    assert_eq!(
        daily_backup_names(&backups),
        expected,
        "eight daily copies existed; the oldest is deleted and the last 7 remain"
    );
    assert!(
        pre_migration.is_file(),
        "the retention never deletes pre-migration copies"
    );
}

#[cfg(unix)]
#[test]
fn a_failed_daily_copy_degrades_and_the_database_stays_usable() {
    // The D-046 policy, pinned at the seam: a copy that cannot be
    // written is an error returned to the caller (which logs it and
    // continues the launch) — the database itself keeps working.
    use std::os::unix::fs::PermissionsExt;

    let (temp, db) = probe_db();
    let backups = backups_dir_of(&temp);

    // No write rights in the backup directory: the copy cannot be
    // staged. (Skipped as root, which bypasses permission bits — the
    // same gate as the store suite.)
    let is_root = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&backups).unwrap()) == 0;
    if is_root {
        return;
    }
    std::fs::set_permissions(&backups, std::fs::Permissions::from_mode(0o555)).unwrap();
    let failed = db.ensure_daily_backup(daily_seam_now());
    std::fs::set_permissions(&backups, std::fs::Permissions::from_mode(0o755)).unwrap();

    let error = failed.expect_err("the unwritable directory must fail the copy");
    assert!(
        matches!(error, eunomia_store::storage::StorageError::Backup { .. }),
        "the failure keeps its own shape, got: {error:?}"
    );
    assert!(
        daily_backup_names(&backups).is_empty(),
        "no copy and no debris survive the failure"
    );

    // Degradation, not abort: the next operation through the executor
    // commits on the very same connection.
    let writer = db.clone();
    async_runtime::block_on(writer.run(|tx| {
        tx.execute(
            "INSERT INTO probe (name) VALUES ('after-the-failed-copy')",
            [],
        )
        .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("the database must stay usable after a failed daily copy");
    assert_eq!(probe_count(&db), 1);
}

#[test]
fn setup_runs_the_daily_backup_after_open_and_before_managing() {
    // Wiring pin (the `config_tests.rs` style): the ERR-001 step must
    // run at startup, after the database is open, before the state is
    // published — and its failure arms must log through the facade.
    // Extended by T-029: the LOG-006 retention task is spawned right
    // after the manage step (the cleanup runs on the already-managed
    // database, its startup pass off the setup thread).
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    let markers = [
        "db::Db::open(&data_dir)",
        "database.ensure_daily_backup(jiff::Timestamp::now())",
        "logging::db_backup_daily(",
        "logging::db_backup_daily_failed(",
        "app.manage(database);",
        "retention::spawn_daily_cleanup(",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                lib.matches(marker).count(),
                1,
                "lib.rs must contain `{marker}` exactly once, live (a \
                 commented-out or duplicated line must not count)"
            );
            lib.find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();

    for pair in positions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "lib.rs must open the database, run the ERR-001 daily \
             backup (logging written copies and failures through the \
             facade), manage the state, and only then spawn the \
             LOG-006 retention task (spec §10, D-046; D-054); \
             marker positions: {positions:?}"
        );
    }

    // The never-abort half of the policy, pinned as text because the
    // order above cannot see it: between the daily step and the manage
    // call the hook must not bail out — no `return`, no `?` — or a
    // failed copy would abort the launch exactly as a failed
    // pre-migration copy does (the opposite of D-046).
    let step = &lib[positions[1]..positions[4]];
    assert!(
        !step.contains('?') && !step.contains("return"),
        "the daily backup step must degrade, never abort: no `?` or \
         `return` between the call and `app.manage` (D-046), got: {step}"
    );
}

// ---- T-025: the §10 busy-retry cycle (E_DB_LOCKED, §9.5 backoff) --------------

/// The busy timeout the retry tests run with, installed through the
/// [`Db::set_busy_timeout_for_tests`] seam: at the DB-001 production
/// value (5000 ms) every busy attempt would wait the full five
/// seconds, and the three-retry test would sleep through 4 × 5 s of
/// timeout on top of the §9.5 pauses; a few milliseconds keep the
/// busy path deterministic (the external lock is held regardless) and
/// fast. Production never calls the seam — DB-001 is untouched.
const TEST_BUSY_TIMEOUT: Duration = Duration::from_millis(25);

/// Opens a **second, external connection** to the probe database (the
/// way another process would hold the file) and takes the write lock:
/// `BEGIN EXCLUSIVE` — in WAL mode the same immediate write lock a
/// competing writer needs. Held until the returned connection is
/// dropped, so every write attempt through the executor deterministically
/// waits out its busy timeout and fails with `SQLITE_BUSY`.
fn exclusive_write_holder(db_path: &Path) -> rusqlite::Connection {
    let connection =
        rusqlite::Connection::open(db_path).expect("opening the external connection must work");
    connection
        .execute_batch("BEGIN EXCLUSIVE")
        .expect("taking the exclusive write lock must work");
    connection
}

/// The sum of §9.5's pauses — the wall-clock floor a three-retry run
/// can never undercut (`thread::sleep` guarantees at least the asked
/// duration), asserting the backoff happened at all without an
/// upper-bound flake on a loaded machine.
const BUSY_RETRY_PAUSES_TOTAL: Duration = Duration::from_millis(100 + 300 + 900);

#[test]
fn a_busy_database_is_retried_three_times_then_rejected_as_e_db_locked() {
    // The card's acceptance criterion: a second connection holds the
    // lock; the operation through `Db::run` is retried exactly three
    // times, then surfaces `E_DB_LOCKED` (§10's row: "busy_timeout
    // expired … up to 3 automatic retries"). The attempts are counted
    // where they are observable — the operation body itself, once per
    // attempt, in a fresh transaction each time.
    let (temp, db) = probe_db();
    db.set_busy_timeout_for_tests(TEST_BUSY_TIMEOUT);
    let holder = exclusive_write_holder(&crate::data_dirs::db_file_path(temp.path()));
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let counting = db.clone();
    let seen_by_operation = std::sync::Arc::clone(&attempts);

    let started = Instant::now();
    let error = async_runtime::block_on(counting.run(move |tx| {
        seen_by_operation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tx.execute("INSERT INTO probe (name) VALUES ('never-committed')", [])
            .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect_err("a lock held across every retry must end in the rejection");
    let elapsed = started.elapsed();
    // The lock must be gone before the recovery assertions below.
    drop(holder);

    // Exactly four invocations: the original attempt plus three
    // automatic retries — not five, not three: §10's "up to 3
    // automatic retries", observed on the body that was re-run.
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        4,
        "the executor must run the operation once plus exactly three \
         retries (§10 E_DB_LOCKED)"
    );

    // The rejection is the classifier's §7.10 wire form: the §10
    // literal, its default message key, the real SQLite category in
    // details — and no interpolation params: UI-011's params are
    // "where" values (a path, a project); the busy situation has
    // none, the diagnostics live in details.
    let wire = serde_json::to_value(&error).expect("the rejection must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(wire["message_key"], serde_json::json!("errors.db_locked"));
    assert_eq!(wire["params"], serde_json::json!({}));
    assert_eq!(
        wire["details"]["os_kind"],
        serde_json::json!("DatabaseBusy"),
        "the verdict must carry the real SQLite busy category, got: {wire}"
    );

    // The §9.5 backoff was paid: the pauses alone floor the run at
    // 100+300+900 ms (a retry loop without pauses would finish in
    // milliseconds of busy timeouts).
    assert!(
        elapsed >= BUSY_RETRY_PAUSES_TOTAL,
        "three retries must pause §9.5's 100/300/900 ms between attempts \
         (took {elapsed:?})"
    );

    // Nothing of the failed attempts survived (every attempt rolled
    // back), and the very same connection serves the next operation
    // once the lock is gone — the rejection is transient by design.
    assert_eq!(probe_count(&db), 0);
    let writer = db.clone();
    async_runtime::block_on(writer.run(|tx| {
        tx.execute("INSERT INTO probe (name) VALUES ('after-the-lock')", [])
            .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("the connection must serve the next operation after the lock");
    assert_eq!(names_in_probe(&db), vec!["after-the-lock".to_owned()]);
}

#[test]
fn a_lock_released_between_retries_lets_the_operation_commit() {
    // The recovery half of the retry cycle: the lock does not last,
    // and the operation succeeds on a retry. The release is
    // synchronized, not timed — the operation body reports its failed
    // attempt and *waits* for the test thread to drop the holder
    // before returning the busy error, so the retry starts only after
    // the release fully completed; the run's correctness never
    // depends on the 100 ms pause being long enough for anything.
    let (temp, db) = probe_db();
    db.set_busy_timeout_for_tests(TEST_BUSY_TIMEOUT);
    let db_path = crate::data_dirs::db_file_path(temp.path());
    let holder = exclusive_write_holder(&db_path);
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    let (failure_sender, failure_receiver) = std::sync::mpsc::channel::<()>();
    let (release_sender, release_receiver) = std::sync::mpsc::channel::<()>();
    let running = db.clone();
    let seen_by_operation = std::sync::Arc::clone(&attempts);
    let task = async_runtime::spawn(async move {
        running
            .run(move |tx| {
                seen_by_operation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if let Err(error) = tx.execute("INSERT INTO probe (name) VALUES ('late')", []) {
                    // Report the busy failure, then hold this attempt
                    // open until the test thread has dropped the
                    // holder — the retry that follows runs against a
                    // genuinely free lock, not a racing release.
                    let classified = sqlite_error_to_app_error(error);
                    let _ = failure_sender.send(());
                    release_receiver
                        .recv_timeout(Duration::from_secs(30))
                        .expect("the test thread must release the lock");
                    return Err(classified);
                }
                Ok(())
            })
            .await
    });

    // The first attempt failed busy (recv: deterministic — the holder
    // is up). Anchor the pause measurement before the release: the
    // 100 ms pause starts once the body returns, which the send below
    // unblocks — measuring from here keeps the anchor ahead of the
    // event it bounds. Then release the lock, let the retry run.
    let started = Instant::now();
    failure_receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("the first attempt must reach its busy failure");
    drop(holder);
    release_sender
        .send(())
        .expect("the operation must still be waiting for the release");

    async_runtime::block_on(task)
        .expect("the run future must not panic")
        .expect("the retry must commit once the lock is gone");

    // Attempt two committed: exactly two invocations (the failed one
    // plus the successful retry), the row durable, and the §9.5 pause
    // paid in between (at least the first 100 ms rung).
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the operation must succeed on its first retry"
    );
    assert_eq!(names_in_probe(&db), vec!["late".to_owned()]);
    assert!(
        started.elapsed() >= Duration::from_millis(100),
        "the retry must follow §9.5's first pause"
    );
}

#[test]
fn non_busy_failures_are_not_retried() {
    // The retry cycle's selectivity: only SQLite's busy/locked class
    // replays. An operation's own error (the validation shape every
    // command produces) and a real constraint violation both surface
    // on their first attempt — one invocation each, no pauses, no
    // replays. Replaying a validation failure would re-run an
    // operation whose rejection is deterministic.
    let (_temp, db) = probe_db();
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    let own_error_attempts = std::sync::Arc::clone(&attempts);
    let failing = db.clone();
    attempts.store(0, std::sync::atomic::Ordering::SeqCst);
    let error = async_runtime::block_on(failing.run::<(), _>(move |tx| {
        own_error_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tx.execute("INSERT INTO probe (name) VALUES ('doomed')", [])
            .map_err(sqlite_error_to_app_error)?;
        Err(AppError::new(crate::errors::ErrorCode::Validation))
    }))
    .expect_err("the operation's own error must surface");
    assert_eq!(error.code, crate::errors::ErrorCode::Validation);
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "an operation's own error must not be retried"
    );

    let constraint_attempts = std::sync::Arc::clone(&attempts);
    let seeding = db.clone();
    async_runtime::block_on(seeding.run(|tx| {
        tx.execute("INSERT INTO probe (name) VALUES ('committed')", [])
            .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("seeding the committed row must work");
    attempts.store(0, std::sync::atomic::Ordering::SeqCst);
    let violating = db.clone();
    let error = async_runtime::block_on(violating.run(move |tx| {
        constraint_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tx.execute("INSERT INTO probe (name) VALUES ('committed')", [])
            .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect_err("the constraint violation must surface");
    let wire = serde_json::to_value(&error).expect("the error must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        wire["details"]["os_kind"],
        serde_json::json!("ConstraintViolation")
    );
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a constraint violation must not be retried"
    );
}

#[test]
fn the_fallback_e_db_locked_is_not_retried() {
    // The gap-fallback revision pin (D-037, revised by the T-025
    // entry): an unclassifiable storage failure — here an I/O-class
    // rusqlite error classified through the shared base classifier —
    // lands on `E_DB_LOCKED` *without* entering the retry cycle. The
    // discriminator is the busy/locked class, not the wire code; a
    // fallback producer that replayed would charge §9.5 pauses and
    // re-run an operation for a failure that is not transient, while
    // its `os_kind` (`SystemIoFailure`) keeps the two producers
    // separable on the wire and in the §10 log line.
    use rusqlite::ffi::{Error as SqliteError, SQLITE_IOERR};

    let (_temp, db) = probe_db();
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let seen_by_operation = std::sync::Arc::clone(&attempts);
    let failing = db.clone();

    let error = async_runtime::block_on(failing.run::<(), _>(move |_tx| {
        seen_by_operation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(sqlite_error_to_app_error(rusqlite::Error::SqliteFailure(
            SqliteError::new(SQLITE_IOERR),
            None,
        )))
    }))
    .expect_err("the fallback error must surface");

    assert_eq!(
        error.code,
        crate::errors::ErrorCode::DbLocked,
        "the recorded fallback keeps its E_DB_LOCKED carrier (D-037, \
         revised with the T-025 entry)"
    );
    let wire = serde_json::to_value(&error).expect("the error must serialize");
    assert_eq!(
        wire["details"]["os_kind"],
        serde_json::json!("SystemIoFailure")
    );
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the fallback E_DB_LOCKED must not be retried: only SQLite's \
         busy/locked class enters the cycle"
    );
}

#[test]
fn the_busy_discriminators_match_only_the_busy_locked_class() {
    // The retry loop's two selectors, driven directly on synthesized
    // result codes — the acceptance tests only ever exercise the
    // body-path `DatabaseBusy` edge (`BEGIN DEFERRED` takes no lock,
    // and a busy `COMMIT` cannot be staged deterministically), so
    // without this test the raw-error half and the `DatabaseLocked`
    // members would have no coverage at all. The extended codes fold
    // into the primary class (rusqlite masks `& 0xff`), which is the
    // documented claim this pins.
    use rusqlite::ffi::{
        Error as SqliteError, SQLITE_BUSY, SQLITE_BUSY_SNAPSHOT, SQLITE_IOERR, SQLITE_LOCKED,
        SQLITE_LOCKED_SHAREDCACHE,
    };

    let busy_family = [
        SqliteError::new(SQLITE_BUSY),
        SqliteError::new(SQLITE_BUSY_SNAPSHOT),
        SqliteError::new(SQLITE_LOCKED),
        SqliteError::new(SQLITE_LOCKED_SHAREDCACHE),
    ];
    for sqlite_error in busy_family {
        let rusqlite_error = rusqlite::Error::SqliteFailure(sqlite_error, None);
        assert!(
            crate::db::rusqlite_error_is_busy_locked(&rusqlite_error),
            "the raw-error discriminator must match {:?} (sqlite result \
             code {})",
            sqlite_error.code,
            sqlite_error.extended_code
        );
        // The same error through the shared classifier must trip the
        // AppError half — the busy tags survive classification.
        let classified =
            sqlite_error_to_app_error(rusqlite::Error::SqliteFailure(sqlite_error, None));
        assert!(
            crate::db::app_error_is_busy_locked(&classified),
            "the classified-error discriminator must match the busy \
             class (os_kind = {:?})",
            classified.details.and_then(|details| details.os_kind)
        );
    }

    // And the negatives: an I/O failure is the recorded fallback's
    // producer — classified onto E_DB_LOCKED yet never a retry
    // candidate — and an error with no details (a hand-built one) is
    // not busy either.
    let io_error = rusqlite::Error::SqliteFailure(SqliteError::new(SQLITE_IOERR), None);
    assert!(
        !crate::db::rusqlite_error_is_busy_locked(&io_error)
            && !crate::db::app_error_is_busy_locked(&sqlite_error_to_app_error(io_error)),
        "an I/O failure is not busy — the fallback E_DB_LOCKED must \
         never enter the retry cycle"
    );
    assert!(
        !crate::db::app_error_is_busy_locked(&AppError::new(crate::errors::ErrorCode::DbLocked)),
        "a hand-built error with no details must not look busy"
    );
}

#[test]
fn the_connection_lock_is_free_during_retry_pauses() {
    // The "guard dropped before the pause" property (ARCHITECTURE.md,
    // D-050): while a busy operation is between attempts, the one
    // connection must serve other operations — otherwise every
    // retried operation would serialize the whole app behind its
    // §9.5 pauses. Pinned deterministically: the retried operation
    // reports its busy failure and holds its attempt open until the
    // test acks; a read operation is queued on the connection before
    // that ack, and the read provably acquires the lock (and runs,
    // WAL readers being unaffected by the external write lock) while
    // the retried run is still mid-cycle — the run's result channel
    // is still empty at that moment, because the run cannot progress
    // past its next attempt's lock while the read holds its own.
    let (temp, db) = probe_db();
    db.set_busy_timeout_for_tests(TEST_BUSY_TIMEOUT);
    let db_path = crate::data_dirs::db_file_path(temp.path());
    let holder = exclusive_write_holder(&db_path);

    let (attempt_failed, attempt_failed_rx) = std::sync::mpsc::channel::<()>();
    let (release_attempt, release_attempt_rx) = std::sync::mpsc::channel::<()>();
    let (read_started, read_started_rx) = std::sync::mpsc::channel::<()>();
    let (release_read, release_read_rx) = std::sync::mpsc::channel::<()>();
    let (run_finished, run_finished_rx) = std::sync::mpsc::channel::<()>();

    let busy = db.clone();
    let busy_task = async_runtime::spawn(async move {
        let finished = run_finished;
        let outcome = busy
            .run(move |tx| {
                if let Err(error) = tx.execute("INSERT INTO probe (name) VALUES ('pausing')", []) {
                    let classified = sqlite_error_to_app_error(error);
                    let _ = attempt_failed.send(());
                    release_attempt_rx
                        .recv_timeout(Duration::from_secs(30))
                        .expect("the test thread must let the attempt return");
                    return Err(classified);
                }
                Ok(())
            })
            .await;
        let _ = finished.send(());
        outcome
    });
    // Wait for attempt 1's busy failure, then queue the read on the
    // connection (behind the still-held guard of attempt 1).
    attempt_failed_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("the busy attempt must reach its failure");
    let reading = db.clone();
    let read_started_sender = read_started;
    let read_task = async_runtime::spawn(async move {
        reading
            .run(move |tx| {
                tx.query_row("SELECT COUNT(*) FROM probe", [], |row| row.get::<_, i64>(0))
                    .map_err(sqlite_error_to_app_error)?;
                let _ = read_started_sender.send(());
                release_read_rx
                    .recv_timeout(Duration::from_secs(30))
                    .expect("the test thread must let the read return");
                Ok(())
            })
            .await
    });

    // Let attempt 1 return: the executor logs, then sleeps §9.5's
    // first pause with the guard released — the queued read must now
    // acquire the connection and run its statement.
    release_attempt
        .send(())
        .expect("the busy attempt must still be waiting");
    read_started_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("the read must run during the retried operation's pause");
    // The retried run is provably still in flight: its next attempt
    // needs the connection, which the waiting read holds (and the
    // run's own completion signal has not fired).
    assert!(
        run_finished_rx.try_recv().is_err(),
        "the retried operation must still be mid-cycle while the read \
         runs — the connection lock is free during the pauses"
    );

    // Endgame: free the external lock, let the read return, and the
    // retried run's next attempt commits against the free database.
    // Honest residual, documented for the record: the read is spawned
    // before the release but nothing proves it reached `Mutex::lock`
    // by then — correctness of the *acquire* still leans on §9.5's
    // 100 ms pause being enough for a pre-spawned task to take the
    // free mutex (the provable part is the ordering: once the read
    // runs, the retried operation demonstrably has not finished). If
    // a pathological scheduler ever starves the read past all three
    // pauses, the failure is loud — a 30 s channel timeout — never a
    // silent wrong-pass.
    drop(holder);
    release_read
        .send(())
        .expect("the read must still be waiting");
    async_runtime::block_on(busy_task)
        .expect("the retried run future must not panic")
        .expect("the retried run must commit once the lock is gone");
    async_runtime::block_on(read_task)
        .expect("the read future must not panic")
        .expect("the read must have committed");
    assert_eq!(names_in_probe(&db), vec!["pausing".to_owned()]);
}

#[test]
fn busy_retries_are_logged_through_the_facade() {
    // Wiring pin (the `config_tests.rs` style): the retry loop's only
    // observable side effect besides the replay itself is the facade
    // warn line — the call must exist in `db.rs`, exactly once, live
    // (a commented-out or duplicated line must not count), keeping
    // the executor's retries visible in the file log (D-032: tracing
    // events live in `logging.rs` only).
    let db_module = crate::config_tests::strip_comments(include_str!("db.rs"));
    assert_eq!(
        db_module.matches("logging::db_busy_retry(").count(),
        1,
        "db.rs must log every busy retry through the facade, exactly \
         one live call site (§10 observability, D-032)"
    );
}

// ---- SEC-007 modes after the real first run (Unix only) -------------------
//
// The T-022 acceptance criterion: a Linux/macOS test checks the modes
// after the first run. The "first run" here is the real startup seam —
// `ensure_data_layout` + `Db::open` + the ERR-001 daily copy, the same
// steps the `setup` hook runs — over a temp root, so the assertions
// cover the two enforcement layers together: the app layer's directory
// modes and the store's file modes. Windows inherits the profile's
// rights (SEC-007), so both the tests and the enforcement are
// Unix-gated.

/// Reads a path's permission bits masked to the standard `0o777`.
#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .unwrap_or_else(|err| panic!("stat of {} must work: {err}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

#[cfg(unix)]
#[test]
fn the_first_startup_leaves_the_sec007_modes() {
    // Arrange: a clean system — nothing exists under the root.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    let root = temp.path().to_path_buf();

    // Act: the startup steps, in the `setup` hook's order — the
    // layout, the database, the day's copy.
    crate::data_dirs::ensure_data_layout(&root)
        .expect("creating the data-directory layout in a temp directory must work");
    let db = Db::open(&root).expect("opening a fresh database in a temp directory must work");
    db.ensure_daily_backup(jiff::Timestamp::now())
        .expect("the day's copy must be written");

    // Assert: the exact SEC-007 modes — the two backups directories
    // `0700`, the database file `0600`, the day's copy `0600` (a copy
    // of the database's data, D-046), and the WAL/-shm siblings
    // owner-only (they carry un-checkpointed database frames; the
    // owner bits are SQLite's choice, the absence of group/other
    // access is the requirement).
    assert_eq!(
        mode_of(&root.join("backups")),
        0o700,
        "`backups/` (SEC-007)"
    );
    assert_eq!(
        mode_of(&root.join("db-backups")),
        0o700,
        "`db-backups/` (SEC-007)"
    );
    let db_file = root.join("db").join("eunomiasync.sqlite3");
    assert_eq!(mode_of(&db_file), 0o600, "the database file (SEC-007)");
    for sibling in ["eunomiasync.sqlite3-wal", "eunomiasync.sqlite3-shm"] {
        let path = root.join("db").join(sibling);
        assert!(
            path.is_file(),
            "`{sibling}` must exist under the open WAL connection"
        );
        assert_eq!(
            mode_of(&path) & 0o077,
            0,
            "`{sibling}` must not be accessible by group or other (SEC-007)"
        );
    }
    let daily: Vec<_> = std::fs::read_dir(root.join("db-backups"))
        .expect("db-backups/ must be readable")
        .map(|entry| entry.expect("every entry must be readable").path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("daily-"))
        })
        .collect();
    assert_eq!(daily.len(), 1, "exactly one daily copy exists: {daily:?}");
    assert_eq!(
        mode_of(&daily[0]),
        0o600,
        "the daily copy holds the database's data (SEC-007, D-046)"
    );

    // The connection stays open for the sibling assertions above; the
    // temp directory needs the handles released (the Windows rule,
    // harmless on the Unix this test runs under).
    drop(db);
}

#[cfg(unix)]
#[test]
fn a_second_startup_keeps_the_sec007_modes_and_repairs_drift() {
    // The idempotency half of the criterion: running the startup seams
    // again must leave the modes exactly as required — including when
    // the previous run's modes drifted in between (here loosened by
    // hand, standing in for a restore or a tool with its own mode
    // policy), which is what the re-assertion exists for.
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    let root = temp.path().to_path_buf();
    crate::data_dirs::ensure_data_layout(&root).expect("the first layout run must work");
    drop(Db::open(&root).expect("the first open must work"));

    // Loosen both kinds of objects the requirement covers.
    std::fs::set_permissions(
        root.join("db-backups"),
        std::fs::Permissions::from_mode(0o755),
    )
    .expect("loosening the directory mode must work");
    std::fs::set_permissions(
        root.join("db").join("eunomiasync.sqlite3"),
        std::fs::Permissions::from_mode(0o644),
    )
    .expect("loosening the file mode must work");

    // Act: the next startup — the same seams, the same order.
    crate::data_dirs::ensure_data_layout(&root).expect("the second layout run must work");
    let db = Db::open(&root).expect("the second open must work");

    // Assert: converged back, exactly the required modes.
    assert_eq!(
        mode_of(&root.join("db-backups")),
        0o700,
        "the loosened backups directory must be re-tightened (SEC-007)"
    );
    assert_eq!(
        mode_of(&root.join("db").join("eunomiasync.sqlite3")),
        0o600,
        "the loosened database file must be re-tightened (SEC-007)"
    );

    drop(db);
}

// ---- Protective-copy retention wiring (T-253) ------------------------------

/// The `%Y%m%dT%H%M%S%3fZ` stamp of an instant whole days before
/// `now` — the protective-copy name shape as both creating schemes
/// write it (D-038's `utc_stamp`, recovery's `preserve_damaged`), built
/// from the real clock because the wiring under test (`Db::open`) calls
/// the pass with `Timestamp::now()`: fixtures dated relative to "now"
/// cannot rot with the calendar.
fn protective_stamp_days_before(now: jiff::Timestamp, days: u64) -> String {
    (now - std::time::Duration::from_secs(days * 86_400))
        .strftime("%Y%m%dT%H%M%S%3fZ")
        .to_string()
}

#[test]
fn a_successful_open_runs_the_protective_copy_retention_pass() {
    // The T-253 wiring pin: `Db::open` is the pass's startup trigger.
    // A fresh bootstrap writes no pre-migration copies of its own (the
    // D-038 fresh-run rule), so the directory's state after the open is
    // the pass's verdict over the seeded history alone: the 12-day-old
    // copy goes, the 10-day-old one survives as the newest of its kind
    // (versions do not split the kind), and the daily scheme stays
    // untouched beside them.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let backups = temp.path().join("db-backups");
    let now = jiff::Timestamp::now();
    let aged = backups.join(format!(
        "pre-migration-v1-{}.sqlite3",
        protective_stamp_days_before(now, 12)
    ));
    std::fs::write(&aged, b"fixture stand-in for an aged copy")
        .expect("seeding the aged copy must work");
    let newest = backups.join(format!(
        "pre-migration-v3-{}.sqlite3",
        protective_stamp_days_before(now, 10)
    ));
    std::fs::write(&newest, b"fixture stand-in for the newest copy")
        .expect("seeding the newest copy must work");
    let daily = backups.join(format!("daily-{}.sqlite3", now.strftime("%Y%m%d")));
    std::fs::write(&daily, b"fixture stand-in for a daily copy")
        .expect("seeding the daily copy must work");

    let database = Db::open(temp.path()).expect("the open must succeed");

    assert!(
        !aged.exists(),
        "the open's retention pass must retire the aged non-newest copy"
    );
    assert!(
        newest.exists(),
        "the newest copy of the kind is protected even past the window"
    );
    assert!(
        daily.exists(),
        "the ERR-001 daily scheme stays outside the protective-copy pass"
    );
    drop(database);
}

#[test]
fn the_interrupted_sweep_marks_corpse_rows_over_the_db_seam() {
    // The §9.4 crash-recovery sweep through the synchronous [`Db`]
    // seam (T-258): the corpse rows a dead process left — planted by
    // raw SQL, the honest shape a crashed process actually leaves
    // behind — go `interrupted` with `finished_at` stamped and
    // `error_code` left NULL, a `success` row keeps its own terminal
    // shape, and the sweep is idempotent (a second pass takes zero).
    // Read back by raw SQL on purpose: the assertion oracle is the
    // schema, not the store's own DTOs.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout must work");
    let db = Db::open(temp.path()).expect("opening a fresh database must work");
    let seeder = db.clone();
    async_runtime::block_on(seeder.run(|tx| {
        tx.execute_batch(
            "INSERT INTO sync_runs (id, started_at, status) VALUES \
                 ('corpse-1', '2026-11-21T12:00:00.000Z', 'running'); \
             INSERT INTO sync_runs (id, started_at, status) VALUES \
                 ('corpse-2', '2026-11-21T12:05:00.000Z', 'running'); \
             INSERT INTO sync_runs (id, started_at, finished_at, status) VALUES \
                 ('settled', '2026-11-21T11:00:00.000Z', \
                  '2026-11-21T11:01:00.000Z', 'success');",
        )
        .map_err(sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("planting the crash-shaped rows must work");

    let taken = db
        .mark_interrupted_runs("2026-11-22T08:00:00.000Z")
        .expect("the sweep must work over the synchronous seam");

    assert_eq!(taken, 2, "exactly the two corpses were taken");
    let reader = db.clone();
    let rows: Vec<(String, String, Option<String>, Option<String>)> =
        async_runtime::block_on(reader.run(|tx| {
            let mut statement = tx
                .prepare(
                    "SELECT id, status, finished_at, error_code FROM sync_runs \
                          ORDER BY started_at",
                )
                .map_err(sqlite_error_to_app_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })
                .map_err(sqlite_error_to_app_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sqlite_error_to_app_error)?;
            Ok(rows)
        }))
        .expect("reading the rows back must work");
    assert_eq!(
        rows,
        vec![
            (
                "settled".to_owned(),
                "success".to_owned(),
                Some("2026-11-21T11:01:00.000Z".to_owned()),
                None
            ),
            (
                "corpse-1".to_owned(),
                "interrupted".to_owned(),
                Some("2026-11-22T08:00:00.000Z".to_owned()),
                None
            ),
            (
                "corpse-2".to_owned(),
                "interrupted".to_owned(),
                Some("2026-11-22T08:00:00.000Z".to_owned()),
                None
            ),
        ],
        "the corpses are interrupted with the recovery stamp and no \
         code; the settled row keeps its own shape"
    );
    let again = db
        .mark_interrupted_runs("2026-11-22T09:00:00.000Z")
        .expect("the second sweep must work");
    assert_eq!(
        again, 0,
        "nothing is left running — the sweep is idempotent"
    );
    drop(db);
}
