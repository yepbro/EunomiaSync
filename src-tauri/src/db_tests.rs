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

use std::thread;
use std::time::{Duration, Instant};

use tauri::async_runtime;

use crate::db::{Db, sqlite_error_to_app_error};
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
