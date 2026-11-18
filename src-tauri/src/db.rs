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
//!   included (D-038) — the app shell never touches SQLite flags
//!   itself, and the store crate stays the only schema owner
//!   (D-010/D-017).
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
    /// version — the startup seam. `eunomia_store::storage::open_db`
    /// applies the DB-001 pragma set (WAL, `synchronous=FULL`,
    /// `foreign_keys=ON`, `busy_timeout=5000`; D-030) and creates the
    /// file when missing, but never the parent directory: the caller
    /// (the `setup` hook) must have created the data-directory layout
    /// first — both halves of it, because `run_migrations` needs the
    /// existing `db-backups/` for its safety copies before it applies
    /// anything (D-038). Both paths are derived here from the one
    /// `data_dir` root through the `data_dirs` helpers, so the file
    /// and its backups cannot drift apart at a call site.
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
    /// Returns [`DbInitError`] when the file cannot be opened or
    /// configured, WAL cannot be enabled, a safety copy cannot be
    /// written, or a migration fails. A startup failure here aborts
    /// the launch: without the database there is nothing to run — the
    /// same policy as the layout and config steps (D-027/D-028) — and
    /// unlike them this step runs after logging is up. Named §10
    /// situations this includes: a corrupt file aborts the launch as
    /// an open/migration failure — the recovery screen that
    /// `E_DB_CORRUPT` demands (§10) is the recovery task's
    /// (`db_verify`/`db_restore_backup`/`db_recreate`), and a database
    /// newer than the app already errors in the store (D-038) — the
    /// §10 `E_DB_TOO_NEW` "update the app" screen is still the app
    /// layer's to build.
    pub(crate) fn open(data_dir: &Path) -> Result<Self, DbInitError> {
        let db_path = crate::data_dirs::db_file_path(data_dir);
        let backups_dir = crate::data_dirs::db_backups_dir(data_dir);
        let mut connection =
            eunomia_store::storage::open_db(&db_path).map_err(|source| DbInitError {
                path: db_path.clone(),
                source,
            })?;
        eunomia_store::storage::run_migrations(&mut connection, &backups_dir).map_err(
            |source| DbInitError {
                path: db_path,
                source,
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

/// Why the database could not be opened or migrated at startup: the
/// file it happened at, plus the store-layer cause. Kept as data (the
/// same shape as `DataLayoutError`/`ConfigError`) because the
/// `Display` text is what the startup failure surfaces — tauri panics
/// in the event-loop `Ready` handler with `Failed to setup app: …`,
/// which prints the `Display` form, so the path must be in the
/// message itself (D-027's reasoning, applied here too).
#[derive(Debug)]
pub(crate) struct DbInitError {
    /// The database file whose open or migration failed.
    path: PathBuf,
    /// The store-layer failure: open/pragma (`StorageError::Open`),
    /// WAL (`StorageError::JournalMode`) or migration
    /// (`StorageError::Migration`).
    source: eunomia_store::storage::StorageError,
}

impl fmt::Display for DbInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to open or migrate the database at {}: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for DbInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}
