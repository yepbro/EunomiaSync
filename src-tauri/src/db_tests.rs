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

use std::path::{Path, PathBuf};
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
fn a_non_database_file_keeps_the_generic_abort_and_is_not_touched() {
    // Arrange: garbage bytes where the database belongs. The corrupt
    // file keeps its D-037 behavior — the launch aborts as a plain
    // open/migration failure (the `E_DB_CORRUPT` recovery screen is
    // the recovery task's), never as the too-new rejection.
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

    // Assert: no typed AppError (the D-027/D-028 abort path), the
    // message names the database file (the panic prints `Display`),
    // and the failed open modified nothing.
    assert!(
        error.app_error().is_none(),
        "a corrupt file must not masquerade as a too-new rejection"
    );
    assert!(
        error.to_string().contains(&db_path.display().to_string()),
        "the abort message must name the database file, got: {error}"
    );
    assert!(
        error.to_string().to_lowercase().contains("not a database"),
        "the store's corruption reason must stay in the message, got: {error}"
    );
    assert_eq!(
        file_bytes(&db_path),
        garbage.to_vec(),
        "the failed open must not modify the file"
    );
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
