//! The application database state and the one-transaction command
//! executor (spec SYS-001, SYS-002, NFR-008; DECISIONS.md D-037).
//!
//! SYS-002 pins the shape: the whole app shares **one** SQLite
//! connection, and each user operation runs as **one** transaction.
//! This module owns both halves:
//!
//! - [`Db`] is the managed state: the single connection behind a
//!   [`std::sync::Mutex`], opened through `eunomia_store::storage::open_db`
//!   (the DB-001 pragma set, D-030) and migrated with the store's
//!   `run_migrations`, safety copies into the layout's `db-backups/`
//!   included (D-038) — but only after a DB-004 gate rejects a
//!   database newer than this app as the very first step, before
//!   anything can touch the file (NFR-010, `E_DB_TOO_NEW`). The app
//!   shell applies no SQLite flags or pragmas itself (the gate's bare
//!   connection only reads), and the store crate stays the only
//!   schema owner (D-010/D-017).
//! - [`Db::run`] is the executor every future database command goes
//!   through: it moves the operation onto the blocking thread pool
//!   with [`tauri::async_runtime::spawn_blocking`] (SYS-001), locks
//!   the connection, opens one transaction, hands the operation a
//!   `&Transaction`, and commits — or, on any error, rolls the whole
//!   operation back (NFR-008 via spec 6.9: "An entity is never saved
//!   partly: the whole operation runs in one transaction").
//!
//! # Why `std::sync::Mutex`, not `tokio::sync::Mutex`
//!
//! The guard is locked and released *inside* the `spawn_blocking`
//! closure, on the blocking thread — it is never held across an
//! `.await`. A `std::sync::Mutex` makes that a structural property:
//! its guard is not `Send`, so code that tried to hold it across a
//! suspension would not compile as a spawned task. The async mutex
//! exists precisely for guards that must survive an `.await`; taking
//! it here would *invite* the pattern SYS-002 forbids — an operation
//! holding the only connection while parked on the runtime — in
//! exchange for a slower uncontended lock path.
//!
//! # Why `spawn_blocking`, not `block_in_place`
//!
//! SYS-001 names `spawn_blocking`, and `block_in_place` only works on
//! multi-threaded runtimes (it panics on a current-thread one);
//! `tauri::async_runtime::spawn_blocking` runs on tauri's global
//! runtime, whose dedicated blocking pool keeps SQLite's synchronous
//! I/O off the async workers — the "commands do not block the async
//! runtime" half of this task's acceptance criteria.
//!
//! # Transaction boundary
//!
//! `BEGIN DEFERRED` … `COMMIT` via rusqlite's
//! [`Connection::unchecked_transaction`] (it takes `&self`, so it
//! works through the mutex guard without extra `&mut` plumbing).
//! Deferred is the right default: no lock is taken until the first
//! statement, and `busy_timeout` (set by `open_db`) turns a write
//! collision into a wait instead of an immediate error. The operation
//! receives `&Transaction`, not `&Connection`: every statement it
//! runs joins the one open transaction, and it cannot accidentally
//! open a second one. The honest caveat: `Transaction` derefs to
//! `Connection`, so an operation *can* still end the transaction
//! deliberately (`tx.execute_batch("COMMIT")`) and write outside the
//! boundary afterwards — the type prevents accidents, not sabotage;
//! the executor's final `COMMIT` would then fail and the operation
//! surfaces an error. On `Ok` the executor commits; on `Err` the
//! `Transaction` value is dropped, and rusqlite's `Drop` sends the
//! `ROLLBACK` — the operation's changes vanish as a set, which the
//! mid-operation-failure test below proves against a real database
//! file.
//!
//! # Poisoning
//!
//! If an operation panics, the unwinding drops the open
//! `Transaction` first, so the rollback runs before the mutex guard
//! is poisoned — the connection holds no half-applied state. The
//! executor therefore recovers the guard with `into_inner` instead of
//! propagating the poison: the next operation starts from a clean,
//! rolled-back connection. A test panics one operation and runs the
//! next to completion to pin this.
//!
//! # Error mapping (the base layer)
//!
//! The executor classifies the storage errors it can see at its own
//! edges — `BEGIN`/`COMMIT` failures and a panic inside the operation
//! — with [`sqlite_error_to_app_error`], which is also the shared
//! base classifier future command tasks start from when mapping their
//! own statement errors. D-034 keeps the *contextual* choice with the
//! operation: which code a failure deserves is a command-level
//! decision, and the classifier's verdict can be refined there.
//!
//! Honest registry gap, recorded in DECISIONS.md D-037: the §10 MVP
//! registry has no generic "database infrastructure failed" code.
//! Busy/locked (`E_DB_LOCKED`), corrupt/not-a-database
//! (`E_DB_CORRUPT`), disk full (`E_DISK_FULL`), read-only/permission
//! (`E_NO_WRITE_ACCESS`) and constraint violations (`E_VALIDATION`)
//! map to their spec situations; everything else — storage-layer I/O
//! errors, a panicked operation — surfaces as `E_DB_LOCKED` with the
//! true reason preserved in `details.os_message`. That fallback is
//! the least harmful of the twenty codes: its §10 effect ("operation
//! rejected, retry") is what a user can actually do with an
//! unclassifiable storage failure, while the plausible alternative
//! would lie harder (`E_DB_CORRUPT` forces the recovery screen).
//! The gap is revisited when the registry grows a fitting code (with
//! the T-025 retry cycle).
//!
//! This module does not log: §10's "every error is written to the
//! log" belongs to the producing command tasks, which know the
//! project/path context the log line must carry (SEC-008's facade
//! vocabulary, D-032); the executor has none of it.
//!
//! # Tested today, consumed later
//!
//! No registered command uses the executor yet: §7.10/D-009 fix
//! every command name, and none of them belongs to this task (the
//! `db_*` names are the recovery-screen task's, `settings_*` the
//! settings task's, …). Until the first such task lands, the
//! executor is exercised by the behavior tests below against real
//! database files — the same shape `errors.rs` (D-034) and
//! `logging::file_event` (D-032) already use.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, Transaction};
use tauri::async_runtime;

// The SQLite result-code categories rusqlite reports — the storage
// half of the classifier's input, aliased away from the registry's
// `ErrorCode` (crate::errors) the classifier maps onto.
use rusqlite::ErrorCode as SqliteCode;

use crate::errors::{AppError, ErrorCode, ErrorDetails};

/// The application database state (SYS-002): the one SQLite
/// connection behind a mutex, published as managed state by the
/// `setup` hook and shared by every command through [`Db::run`].
///
/// Cloning is deliberate and cheap: a clone is another handle to the
/// same connection (an `Arc` inside), which is what lets [`Db::run`]
/// move the handle into the `'static` `spawn_blocking` closure
/// without borrowing the managed state across the `.await`.
#[derive(Clone)]
pub(crate) struct Db {
    /// The one connection (SYS-002). `std::sync::Mutex` — see the
    /// module docs for why not the async one: the guard never crosses
    /// an await point.
    #[allow(dead_code)] // consumed by the first database command task; tested today
    connection: Arc<Mutex<Connection>>,
}

impl Db {
    /// Opens the database file and migrates it to the latest schema
    /// version — the startup seam. As the **first** step, the DB-004
    /// gate ([`reject_too_new`]) rejects a database newer than this
    /// app: `PRAGMA user_version` is read on a bare connection, and a
    /// version above the store's `latest_schema_version` fails with
    /// [`DbInitError::TooNew`] — the typed `E_DB_TOO_NEW` §10
    /// situation (NFR-010) — before `open_db` runs, because the
    /// DB-001 pragma set *writes* to the file (the `journal_mode=WAL`
    /// switch rewrites the header of a rollback-journal database), and
    /// §10's `E_DB_TOO_NEW` effect is "The app does not change the
    /// database" (T-F11: the file stays byte-identical). Only then
    /// does `eunomia_store::storage::open_db` apply the pragma set
    /// (D-030) — it creates the file when missing, but never the
    /// parent directory: the caller (the `setup` hook) must have
    /// created the data-directory layout first — both halves of it,
    /// because `run_migrations` needs the existing `db-backups/` for
    /// its safety copies before it applies anything (D-038). Both
    /// paths are derived here from the one `data_dir` root through
    /// the `data_dirs` helpers, so the file and its backups cannot
    /// drift apart at a call site.
    ///
    /// Sidecar files of the rejected open, recorded for T-F11: the
    /// gate's bare connection only *reads*, which writes nothing — a
    /// rollback-journal (default-mode) database gains no `-wal`/`-shm`
    /// siblings at all, and for a database already in WAL mode SQLite
    /// creates both siblings for the read and removes them again when
    /// the connection drops (the tests below assert the end state:
    /// main file byte-identical, no sidecar debris). A WAL file with
    /// *pending frames* (a newer app killed before checkpointing) is
    /// checkpointed by that same clean drop — content-preserving
    /// SQLite recovery; DECISIONS.md records why that boundary is
    /// accepted.
    ///
    /// The `Vec<PathBuf>` of pre-migration copies the migration run
    /// returns (empty for fresh and no-op runs, D-038) is deliberately
    /// dropped: this module does not log (D-037 — the §10 log line
    /// needs the facade vocabulary and a producing task), so the list
    /// has no consumer until the startup-log/first-command task takes
    /// it over.
    ///
    /// # Errors
    ///
    /// Returns [`DbInitError::TooNew`] when the database was created
    /// by a newer app version (its typed wire form is
    /// [`DbInitError::app_error`]), and [`DbInitError::Open`] when the
    /// file cannot be opened or configured, WAL cannot be enabled, a
    /// safety copy cannot be written, or a migration fails. A startup
    /// failure of either kind aborts the launch: without the database
    /// there is nothing to run — the same policy as the layout and
    /// config steps (D-027/D-028) — and unlike them this step runs
    /// after logging is up, so the too-new rejection is also logged
    /// through the facade with its §10 code (`logging::db_too_new`,
    /// wired in the `setup` hook). The §10 screens both errors
    /// describe — "Update the app" for `E_DB_TOO_NEW`, recovery for
    /// `E_DB_CORRUPT` — are the future recovery/update task's to
    /// build; the typed error on this seam is their contract, and a
    /// corrupt file keeps aborting as a plain open/migration failure
    /// until then (`db_verify`/`db_restore_backup`/`db_recreate`).
    pub(crate) fn open(data_dir: &Path) -> Result<Self, DbInitError> {
        let db_path = crate::data_dirs::db_file_path(data_dir);
        let backups_dir = crate::data_dirs::db_backups_dir(data_dir);
        // DB-004 order: the too-new gate runs before `open_db`, so a
        // newer database is rejected before the pragma set can touch
        // the file (see the method docs).
        reject_too_new(&db_path)?;
        let mut connection =
            eunomia_store::storage::open_db(&db_path).map_err(|source| DbInitError::Open {
                path: db_path.clone(),
                source: Box::new(source),
            })?;
        eunomia_store::storage::run_migrations(&mut connection, &backups_dir).map_err(
            |source| {
                // The backstop for a file that became newer *during* the
                // open (see `classify_migration_failure`).
                classify_migration_failure(&connection, &db_path, source)
            },
        )?;
        Ok(Db {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Runs `operation` on the blocking pool inside one transaction
    /// and returns its result as the §7.10 wire value (SYS-001,
    /// SYS-002, NFR-008).
    ///
    /// The operation sees a `&Transaction`: every statement it runs
    /// joins the one open transaction, and a nested `BEGIN` is rejected
    /// by SQLite at runtime (a deliberate `execute_batch("COMMIT")`
    /// escape stays reachable through the Deref — see the module docs). `Ok` commits; `Err` — including a classified SQLite
    /// failure, see [`sqlite_error_to_app_error`] — rolls the whole
    /// operation back. The connection mutex serializes operations:
    /// two `run` calls may sit on the blocking pool concurrently, but
    /// their transactions never interleave on the connection.
    ///
    /// A panic inside the operation does not poison anything the next
    /// operation would notice: the unwinding rolls the transaction
    /// back, the executor recovers the mutex guard (see the module
    /// docs), and the panic surfaces as the fallback `E_DB_LOCKED`
    /// `AppError` carrying the panic message — in release builds the
    /// workspace's `panic = "abort"` ends the process before this
    /// mapping could run, so that branch is a dev/test path.
    #[allow(dead_code)] // consumed by the first database command task; tested today
    pub(crate) async fn run<T, F>(&self, operation: F) -> Result<T, AppError>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T, AppError> + Send + 'static,
        T: Send + 'static,
    {
        // Clone the handle before entering the closure: the closure
        // must own its connection access ('static for spawn_blocking),
        // not borrow it from the managed state.
        let connection = Arc::clone(&self.connection);
        let joined = async_runtime::spawn_blocking(move || {
            let guard = lock_recovering_poison(&connection);
            let transaction = guard
                .unchecked_transaction()
                .map_err(sqlite_error_to_app_error)?;
            match operation(&transaction) {
                // A failed commit also drops the transaction (rusqlite
                // sends the ROLLBACK), so nothing half-applied stays
                // behind on that path either.
                Ok(value) => transaction
                    .commit()
                    .map_err(sqlite_error_to_app_error)
                    .map(|()| value),
                Err(error) => Err(error),
            }
        })
        .await;
        match joined {
            // The operation's (or transaction's) own outcome.
            Ok(outcome) => outcome,
            // The join can only fail because the closure panicked (or
            // the task was cancelled at runtime shutdown).
            Err(join_error) => Err(join_failure_to_app_error(join_error)),
        }
    }
}

/// Reads `PRAGMA user_version` (DB-004) from a **bare** connection at
/// `db_path` — no DB-001 pragma set, no configuration. Reading a
/// pragma executes no write of the app's own: a rollback-journal
/// database gains no `-journal` or sidecars from it (a WAL-mode
/// database gets its `-shm`/`-wal` for the read and loses them on the
/// clean drop — see [`Db::open`]). The connection does carry the
/// DB-001 `busy_timeout` (5000 ms, the same value `open_db` applies):
/// with the default zero timeout a momentary writer lock would fail
/// the read here, defer to `open_db` — which then writes to the very
/// file this gate exists to protect — and leave the rejection to the
/// post-pragma backstop; waiting for the verdict keeps the gate the
/// deciding step.
///
/// Every failure is still deliberately reported as `None` (version
/// unknown): this read exists only to answer "is the database
/// *definitely* newer than this app?", and every other state — the
/// file does not exist yet (a bare `Connection::open` creates a 0-byte
/// one, exactly what `open_db` would create a moment later), the
/// parent directory is missing, the bytes are not a database, a lock
/// that outlasts the timeout — has its canonical failure mode one step
/// later, in `open_db` or `run_migrations` (D-030's `StorageError::Open`
/// names the path; a corrupt file keeps its D-037 abort). Claiming
/// "too new" for a version that could not be read would misclassify
/// those cases and bypass their errors; D-040 records that an
/// "unknown" verdict degrades the byte-identity guarantee to the
/// backstop, which runs after the pragmas.
fn schema_version_of(db_path: &Path) -> Option<i64> {
    let connection = Connection::open(db_path).ok()?;
    // Same 5000 ms as `open_db`'s DB-001 set (the constant lives in
    // the store); the value is duplicated here deliberately — one
    // number is the cost of the gate deciding before the pragma set.
    let _ = connection.busy_timeout(std::time::Duration::from_millis(5000));
    connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .ok()
}

/// The DB-004/NFR-010 gate: a database whose `PRAGMA user_version`
/// exceeds the schema versions this app knows — the store's
/// `latest_schema_version`, the number of embedded migrations — is not
/// opened at all.
///
/// This check is the *first* step of [`Db::open`], before
/// `open_db`: the DB-001 pragma set writes to the file (the
/// `journal_mode=WAL` switch rewrites the header of a rollback-journal
/// database), and §10's `E_DB_TOO_NEW` effect is "The app does not
/// change the database" — the T-F11 byte-identity criterion. The gate
/// itself opens a bare connection and reads one pragma (see
/// [`schema_version_of`]); src-tauri owning `Connection` directly is
/// the D-037 shape, and the schema knowledge (what "latest" is) stays
/// in the store.
fn reject_too_new(db_path: &Path) -> Result<(), DbInitError> {
    let app_schema_version = eunomia_store::storage::latest_schema_version();
    if let Some(database_schema_version) = schema_version_of(db_path)
        && database_schema_version > app_schema_version as i64
    {
        return Err(DbInitError::TooNew {
            path: db_path.to_owned(),
            database_schema_version,
            app_schema_version,
        });
    }
    Ok(())
}

/// Classifies a `run_migrations` failure for [`Db::open`]: the
/// D-038 backstop of the too-new gate. The store's migration run
/// itself rejects a database newer than its set
/// (`DatabaseTooFarAhead`), which — once [`reject_too_new`] has
/// passed — can only mean the file changed under the open (a too-new
/// file swapped or rewritten between the two steps). The verdict is
/// re-derived from the version *fact* on the open connection rather
/// than matched from the store's error enum: src-tauri does not
/// depend on `rusqlite_migration` (the store is its only consumer),
/// and the fact is all the classification needs. Any other failure —
/// and a version that still reads within range — keeps the generic
/// [`DbInitError::Open`] shape and with it the D-027 abort path.
pub(crate) fn classify_migration_failure(
    connection: &Connection,
    db_path: &Path,
    source: eunomia_store::storage::StorageError,
) -> DbInitError {
    let app_schema_version = eunomia_store::storage::latest_schema_version();
    let too_new = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .ok()
        .filter(|version| *version > app_schema_version as i64);
    match too_new {
        Some(database_schema_version) => DbInitError::TooNew {
            path: db_path.to_owned(),
            database_schema_version,
            app_schema_version,
        },
        None => DbInitError::Open {
            path: db_path.to_owned(),
            source: Box::new(source),
        },
    }
}

/// Locks the connection, recovering from a poisoned mutex and
/// clearing the poison.
///
/// A poisoned guard means an earlier operation panicked — and by the
/// time the guard poisoned, the unwinding had already dropped the
/// open `Transaction`, whose `Drop` sent the `ROLLBACK`. The
/// connection therefore carries no half-applied state, and the honest
/// response is to keep using it instead of failing every later
/// operation for the rest of the process lifetime. The poison is
/// cleared as well (`clear_poison`): the failure it described has
/// been fully handled, so a future `lock()` (here or in a second
/// consumer of `Db`) must not keep seeing it.
fn lock_recovering_poison(connection: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    connection.lock().unwrap_or_else(|poisoned| {
        connection.clear_poison();
        poisoned.into_inner()
    })
}

/// Maps a blocking-pool join failure to the §7.10 wire value,
/// separating the two failure shapes the join can report.
///
/// A **panic** — the realistic case; release builds abort on the
/// panic before this runs — surfaces the panic payload's own message
/// (`panic!("…")` payloads downcast to `&str`/`String`), not the
/// runtime's prose about it. A **cancellation** (the task aborted at
/// runtime shutdown) is reported as cancelled, not panicked. The
/// rollback already ran during the unwinding in the panic case (see
/// [`lock_recovering_poison`]); what is left is not losing the
/// failure silently. The registry has no "internal error" code
/// (D-037's gap), so both land on the same fallback as unclassifiable
/// storage failures.
#[allow(dead_code)] // consumed by the first database command task; tested today
fn join_failure_to_app_error(error: tauri::Error) -> AppError {
    let (os_kind, os_message) = match error {
        tauri::Error::JoinError(join) if join.is_panic() => {
            let payload = join.into_panic();
            let message = payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a database operation panicked".to_owned());
            ("OperationPanicked", message)
        }
        tauri::Error::JoinError(join) => ("OperationCancelled", join.to_string()),
        // spawn_blocking joins cannot fail any other way today; keep
        // the arm total rather than unwrapping.
        other => ("OperationFailed", other.to_string()),
    };
    let mut app_error = AppError::new(ErrorCode::DbLocked);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some(os_kind.to_owned()),
        os_message: Some(os_message),
    });
    app_error
}

/// The base-layer classifier from SQLite failures to the §10 wire
/// contract — the shared starting point for the executor's own
/// `BEGIN`/`COMMIT` errors and for future command tasks mapping their
/// statement errors. D-034 keeps the *contextual* choice with the
/// operation: which code a failure deserves is a command-level
/// decision, and the classifier's verdict can be refined there.
///
/// The classified error always carries the diagnostics in `details`:
/// the exact SQLite category in `os_kind` (rusqlite's `Debug` name —
/// `DatabaseBusy`, `SystemIoFailure`, …, so no two classes collapse
/// into one label) and in `os_message` both the failure text and the
/// numeric result code — rusqlite's `Display` prints only the
/// message when SQLite provides one (e.g. "UNIQUE constraint failed:
/// …"), so the code is appended explicitly; it is what distinguishes
/// `SQLITE_CONSTRAINT_UNIQUE` from `_FOREIGNKEY` from `_CHECK`, the
/// facts a §10 log line and UI-011's Details section are for. The
/// unmapped classes land on the recorded fallback (see the module
/// docs and DECISIONS.md D-037).
#[allow(dead_code)] // consumed by the first database command task; tested today
pub(crate) fn sqlite_error_to_app_error(error: rusqlite::Error) -> AppError {
    let code = match &error {
        rusqlite::Error::SqliteFailure(sqlite_error, _) => match sqlite_error.code {
            SqliteCode::DatabaseBusy | SqliteCode::DatabaseLocked => ErrorCode::DbLocked,
            SqliteCode::DatabaseCorrupt | SqliteCode::NotADatabase => ErrorCode::DbCorrupt,
            SqliteCode::DiskFull => ErrorCode::DiskFull,
            SqliteCode::ReadOnly | SqliteCode::PermissionDenied | SqliteCode::CannotOpen => {
                ErrorCode::NoWriteAccess
            }
            SqliteCode::ConstraintViolation => ErrorCode::Validation,
            _ => ErrorCode::DbLocked,
        },
        // A statement bound to the wrong types or a query returning
        // the wrong shape is a programming bug, not a user situation;
        // the fallback keeps it on the wire with the reason attached.
        _ => ErrorCode::DbLocked,
    };
    let (os_kind, os_message) = match &error {
        rusqlite::Error::SqliteFailure(sqlite_error, _) => (
            format!("{:?}", sqlite_error.code),
            format!(
                "{error} (sqlite result code {})",
                sqlite_error.extended_code
            ),
        ),
        _ => ("RusqliteError".to_owned(), error.to_string()),
    };
    let mut app_error = AppError::new(code);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some(os_kind),
        os_message: Some(os_message),
    });
    app_error
}

/// Why the database could not be opened or migrated at startup. Kept
/// as data (the same shape as `DataLayoutError`/`ConfigError`) because
/// the `Display` text is what the startup failure surfaces — tauri
/// panics in the event-loop `Ready` handler with `Failed to setup
/// app: …`, which prints the `Display` form, so the path must be in
/// the message itself (D-027's reasoning, applied here too).
///
/// One variant is a named §10 user situation rather than an anonymous
/// infrastructure failure: [`DbInitError::TooNew`] (DB-004/NFR-010) —
/// a database created by a newer app version, which the app must not
/// change and the user answers with "Update the app". Its typed
/// §7.10/§10 wire form is [`DbInitError::app_error`]; every other
/// failure stays an open/migration error that aborts the launch
/// (D-027/D-028 — the corrupt-file recovery screen is the recovery
/// task's).
#[derive(Debug)]
pub(crate) enum DbInitError {
    /// The file could not be opened, configured or migrated — every
    /// failure except the too-new gate: open/pragma
    /// (`StorageError::Open`), WAL (`StorageError::JournalMode`),
    /// safety copy (`StorageError::Backup`), migration
    /// (`StorageError::Migration`).
    Open {
        /// The database file whose open or migration failed.
        path: PathBuf,
        /// The store-layer failure. Boxed: `StorageError` grew past
        /// clippy's `result_large_err` threshold once the settings
        /// repository (T-028) added its `Settings` variant, and this
        /// error crosses the `Result` return of [`Db::open`] on every
        /// startup.
        source: Box<eunomia_store::storage::StorageError>,
    },
    /// The database was created by a newer app version: `PRAGMA
    /// user_version` exceeds what this binary's embedded migration set
    /// knows (DB-004). Detected as the first step of [`Db::open`],
    /// before anything touches the file, so the file is left
    /// byte-identical (§10 `E_DB_TOO_NEW`, T-F11).
    TooNew {
        /// The rejected database file.
        path: PathBuf,
        /// The schema version the file carries (`PRAGMA user_version`).
        database_schema_version: i64,
        /// The latest schema version this app knows.
        app_schema_version: usize,
    },
}

impl DbInitError {
    /// The typed §7.10/§10 form when the failure is a named user
    /// situation: a too-new database maps to `E_DB_TOO_NEW` (DB-004,
    /// NFR-010) with the database path and both schema versions as
    /// interpolation params — the contract the "Update the app"
    /// screen (§10's user action; the future recovery/update task)
    /// renders. `None` for every other init failure: their effect is
    /// the D-027/D-028 abort, and inventing a code for them is the
    /// registry gap D-037 records.
    pub(crate) fn app_error(&self) -> Option<AppError> {
        let DbInitError::TooNew {
            path,
            database_schema_version,
            app_schema_version,
        } = self
        else {
            return None;
        };
        let mut error = AppError::new(ErrorCode::DbTooNew);
        error
            .params
            .insert("path".to_owned(), path.display().to_string());
        error.params.insert(
            "database_schema_version".to_owned(),
            database_schema_version.to_string(),
        );
        error.params.insert(
            "app_schema_version".to_owned(),
            app_schema_version.to_string(),
        );
        Some(error)
    }
}

impl fmt::Display for DbInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbInitError::Open { path, source } => {
                write!(
                    f,
                    "failed to open or migrate the database at {}: {}",
                    path.display(),
                    source
                )
            }
            DbInitError::TooNew {
                path,
                database_schema_version,
                app_schema_version,
            } => {
                write!(
                    f,
                    "the database at {} has schema version \
                     {database_schema_version}, newer than this app supports \
                     ({app_schema_version}); the app does not change the \
                     database — update the app (E_DB_TOO_NEW)",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for DbInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DbInitError::Open { source, .. } => Some(source),
            // A verdict, not a wrapper: the version facts are the
            // whole failure, already in the message and the typed
            // `AppError`.
            DbInitError::TooNew { .. } => None,
        }
    }
}
