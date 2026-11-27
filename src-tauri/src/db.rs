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
//!   anything can touch the file (NFR-010, `E_DB_TOO_NEW`), and an
//!   integrity gate runs `PRAGMA quick_check` as the second step
//!   (spec §6.3, `E_DB_CORRUPT`, T-F11's damaged-DB half) — both on
//!   bare, read-only connections, because every later step writes.
//!   The app shell applies no SQLite flags or pragmas itself beyond
//!   the gates' read-only `busy_timeout`, and the store crate stays
//!   the only schema owner (D-010/D-017).
//! - [`Db::run`] is the executor the store-backed commands go
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
//! # Busy retries (§10 `E_DB_LOCKED`, §9.5 backoff)
//!
//! When `busy_timeout` (DB-001, set by `open_db`) expires under a
//! lock another connection holds, SQLite reports `SQLITE_BUSY` (or
//! `SQLITE_LOCKED`); §10's `E_DB_LOCKED` row prescribes "up to 3
//! automatic retries" before the operation is rejected on the wire.
//! The executor owns that cycle — see [`Db::run`]. Each retry re-runs
//! the **whole** operation in a **new** transaction: the failed
//! attempt rolled back completely (the paragraph above), so the retry
//! is a fresh run over whatever is committed when it starts — another
//! operation may have committed during the pause, which is precisely
//! the contention the cycle exists for. That is the idempotency
//! contract the `FnMut` operation signature now encodes: a retried
//! closure must be callable again, everything it does through the
//! `&Transaction` is undone by the rollback before the next call, and
//! the body must read its inputs inside the transaction (never cache
//! them across attempts). The pauses between attempts are
//! spec §9.5's 100/300/900 ms ladder — the same backoff the store's
//! file-copy retries use; the ladder is duplicated deliberately (the
//! store's copy is function-local and not visible here, the same
//! wording `schema_version_of` uses for the 5000 ms value) — and the
//! sleep runs inside the blocking closure (`std::thread::sleep` on
//! the blocking pool, never an async sleep parked on a worker).
//! Only the busy/locked class is retried; every other failure —
//! including the recorded fallback that also lands on `E_DB_LOCKED`
//! — surfaces on its first attempt.
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
//! base classifier the command tasks start from when mapping their
//! own statement errors. D-034 keeps the *contextual* choice with the
//! operation: which code a failure deserves is a command-level
//! decision, and the classifier's verdict can be refined there.
//!
//! Honest registry gap, recorded in DECISIONS.md D-037 and revised by
//! the T-025 entry: the §10 MVP registry has no generic "database
//! infrastructure failed" code. Busy/locked (`E_DB_LOCKED`),
//! corrupt/not-a-database (`E_DB_CORRUPT`), disk full
//! (`E_DISK_FULL`), read-only/permission (`E_NO_WRITE_ACCESS`) and
//! constraint violations (`E_VALIDATION`) map to their spec
//! situations; everything else — storage-layer I/O errors, a panicked
//! operation — surfaces as `E_DB_LOCKED` with the true reason
//! preserved in `details.os_message`. Since the retry cycle landed,
//! `E_DB_LOCKED` from a genuine busy means "retried three times,
//! still busy" — which a fallback producer did not do. The fallback
//! nevertheless stays (of the twenty codes its "operation rejected,
//! retry" effect remains the least harmful lie; the revision and its
//! mitigations are recorded in DECISIONS.md): fallback errors never
//! enter the retry cycle (only the busy/locked class does), and
//! `details.os_kind` separates the producers on the wire
//! (`DatabaseBusy` from a real busy, `SystemIoFailure`/… from the
//! fallback). The gap is revisited when the registry grows a fitting
//! code.
//!
//! This module logs two kinds of events itself: the §10/§9.5 retry
//! attempts ([`crate::logging::db_busy_retry`], a `warn` per attempt)
//! — invisible to every other layer, so nobody else could log them;
//! SEC-008 keeps their payload numeric — and the action-log mirror
//! flush ([`crate::logging::MirrorBuffer::flush_committed`], T-324/
//! D-228): the journal entries a committed operation buffered ride
//! their mirror lines out at the same boundary that knows they
//! committed — here, and only here. §10's "every error is written to
//! the log" belongs to the producing command tasks for everything
//! else, which know the project/path context the log line must carry
//! (SEC-008's facade vocabulary, D-032); the executor has none of it.
//!
//! # Consumed today
//!
//! The registered commands drive the executor through `managed_db`:
//! the sync pipeline records its run rows through it (T-118/D-138),
//! and the store-backed commands (`projects_list`, `activity_list`,
//! `scan_start`, `settings_get`/`settings_set`, the project/exclusion
//! mutations) read and write through it, mapping their store errors
//! through the classifier below — the journaling operations among
//! them through [`Db::run_with_mirror`] (T-324/D-228), whose buffer
//! the executor flushes after the commit. The `db_*` recovery pair and
//! `db_backups_list` do not: they handle the database file itself,
//! not its contents. The behavior tests below exercise the same
//! seam against real database files — the same shape `errors.rs`
//! (D-034) and `logging::file_event` (D-032) already use.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use jiff::Timestamp;
use rusqlite::{Connection, Transaction};
use tauri::async_runtime;

// The SQLite result-code categories rusqlite reports — the storage
// half of the classifier's input, aliased away from the registry's
// `ErrorCode` (crate::errors) the classifier maps onto.
use rusqlite::ErrorCode as SqliteCode;

use crate::errors::{AppError, ErrorCode, ErrorDetails};
use eunomia_store::backup_records::BackupRecordsError;
use eunomia_store::storage::{DailyBackupOutcome, StorageError};
use eunomia_store::sync_runs::SyncRunsError;

/// How many automatic retries a busy/locked failure earns (§10
/// `E_DB_LOCKED`: "Operation rejected; up to 3 automatic retries").
/// The original attempt is not a retry: an operation runs at most
/// `1 + BUSY_RETRY_COUNT` times.
const BUSY_RETRY_COUNT: usize = 3;

/// The pauses between the original attempt and the retries, in order
/// (§9.5's backoff ladder — the same 100/300/900 ms the store's
/// `retry_transient_hold` uses for §9.5 file-write retries; the
/// ladder is duplicated deliberately, like the 5000 ms value in
/// `schema_version_of` — the store's copy is function-local and not
/// visible here). Indexed by the retry number: after
/// the original attempt fails, pause 100 ms; after retry 1, 300 ms;
/// after retry 2, 900 ms; retry 3's failure is the verdict. The
/// array length and [`BUSY_RETRY_COUNT`] are the same number by
/// construction — the loop reads the pause for its retry from here
/// and treats a missing entry as "retries exhausted".
const BUSY_RETRY_PAUSES_MS: [u64; BUSY_RETRY_COUNT] = [100, 300, 900];

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
    connection: Arc<Mutex<Connection>>,
    /// The `db-backups/` directory of the same data directory the
    /// connection was opened from (§5.5) — kept so the ERR-001 daily
    /// copy step ([`Db::ensure_daily_backup`]) writes next to the
    /// migration safety copies without a second path derivation at a
    /// call site (the same drift-avoidance reasoning as [`Db::open`]'s
    /// path handling, D-037).
    backups_dir: PathBuf,
    /// Whether this open bootstrapped the database from scratch: the
    /// file at the §5.5 path was missing or empty **before the open
    /// ran** — the launch-time fact §5.5's tip describes ("the app
    /// starts with an empty DB and says so"), observed here because
    /// `Db::open` is the single place a database comes into existence
    /// (the startup hook and the recovery reopen both go through it,
    /// so every open path publishes the fact without orchestration
    /// changes). The observation is the open's first statement: the
    /// load-bearing boundary is `open_db`, which writes the database
    /// header and schema — the gates before it leave at most a 0-byte
    /// file (D-040), and this predicate still reads a 0-byte file as
    /// fresh, so the placement ahead of them is belt-and-suspenders
    /// against a future step, not a present-day hazard. The recovery
    /// commands publish their copy/empty start through this same seam
    /// — a restore answers `false` (the copy held a database), a
    /// recreate answers `true` (§7.10's "starts with an empty
    /// database" is a fresh bootstrap). Consumed by the `app_info`
    /// command (T-031).
    created_fresh: bool,
    /// The "a write committed" seam (DB-022, T-101): fired by
    /// [`Db::run`] after a transaction **committed with at least one
    /// row change** — read-only transactions and no-op writes fire
    /// nothing, which is what keeps the dirty-recalc service's own
    /// snapshot read from re-triggering itself. `None` until
    /// [`Db::set_commit_hook`] installs one (the shared session tail
    /// does, for the startup open and the recovery reopen alike, with
    /// `dirty_recalc::commit_hook`); a `Db` without a hook behaves
    /// exactly as before the seam existed. The closure runs
    /// on the blocking pool while the connection mutex is still held,
    /// so it must never block — the installed one only `try_send`s
    /// onto a capacity-1 channel.
    commit_hook: Option<Arc<dyn Fn() + Send + Sync>>,
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
    /// database" (T-F11: the file stays byte-identical). As the
    /// **second** step, the integrity gate ([`reject_corrupt`]) runs
    /// `PRAGMA quick_check` (spec §6.3) on an existing, non-empty file
    /// — still on a bare, read-only connection, still before
    /// `open_db`, for the same reason: a damaged database is rejected
    /// as the typed `E_DB_CORRUPT` §10 situation with the file
    /// byte-identical (T-F11's damaged-DB half). Only then does
    /// `eunomia_store::storage::open_db` apply the pragma set (D-030)
    /// — it creates the file when missing, but never the parent
    /// directory: the caller (the `setup` hook) must have created the
    /// data-directory layout first — both halves of it, because
    /// `run_migrations` needs the existing `db-backups/` for its
    /// safety copies before it applies anything (D-038). Both paths
    /// are derived here from the one `data_dir` root through the
    /// `data_dirs` helpers, so the file and its backups cannot drift
    /// apart at a call site.
    ///
    /// Sidecar files of the rejected open, recorded for T-F11: both
    /// gates' bare connections only *read*, which writes nothing — a
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
    /// [`DbInitError::app_error`]), [`DbInitError::Corrupt`] when
    /// `quick_check` rejects the file as damaged (spec §6.3 — the
    /// typed wire form is `E_DB_CORRUPT`), [`DbInitError::Migration`]
    /// when a migration step fails — the step's transaction was rolled
    /// back, the database keeps its schema and data, and the error
    /// carries the copy of that state plus the "install the previous
    /// app version" instruction (spec UPD-004) — and
    /// [`DbInitError::Open`] when the file cannot be opened or
    /// configured, WAL cannot be enabled, or a safety copy cannot be
    /// written. A too-new, migration or generic open failure aborts
    /// the launch at the caller (D-027/D-028); the corrupt verdict is
    /// the exception since T-024 — the `setup` hook logs it and
    /// continues into recovery mode (§10's screen replacing the app),
    /// and the recovery commands (`commands.rs`/`recovery.rs`) call
    /// this same `Db::open` to reopen the replaced database. All of
    /// this runs after logging is up, so every typed §10 situation is
    /// also logged through the facade with its §10 code
    /// (`logging::db_too_new`, `logging::db_corrupt`,
    /// `logging::db_migration_failed`, wired in the `setup` hook).
    pub(crate) fn open(data_dir: &Path) -> Result<Self, DbInitError> {
        let db_path = crate::data_dirs::db_file_path(data_dir);
        let backups_dir = crate::data_dirs::db_backups_dir(data_dir);
        // Observed as the open's first statement, before any step can
        // touch the path (see the field docs): fresh iff the file is
        // definitively absent (NotFound) or definitively empty — no
        // previous database in either case (a 0-byte file is an empty
        // SQLite database, the exact shape a crashed first launch or a
        // gate connection leaves behind, D-040). Any other metadata
        // failure reads as not fresh: the path exists but cannot be
        // stat'ed, a state `open_db` fails on a moment later anyway,
        // so the flag never reaches the wire wrongly.
        let created_fresh = match std::fs::metadata(&db_path) {
            Ok(metadata) => metadata.len() == 0,
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        };
        // DB-004 order: the too-new gate runs before `open_db`, so a
        // newer database is rejected before the pragma set can touch
        // the file (see the method docs).
        reject_too_new(&db_path)?;
        // §6.3/§10 order: the integrity gate runs next, still before
        // `open_db` — `quick_check` is read-only, so a damaged
        // database is rejected with the file untouched (T-F11) while
        // the pragma set would already have written to it.
        reject_corrupt(&db_path)?;
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
        // The protective-copy retention pass (T-253, owner ruling
        // 2026-11-21): every successful open doubles as the pass's
        // startup run, and an upgrade run may just have written new
        // pre-migration copies — the pass retires the aged ones of both
        // protective kinds next to them, age by the name stamp, the
        // newest of each kind protected. A point call by contract:
        // `retention::cleanup_db_backups` is best-effort (it logs its
        // own failures through the facade) and can never fail this open.
        crate::retention::cleanup_db_backups(&backups_dir, Timestamp::now());
        // The orphaned-copy report (T-332, the owner ruling of
        // 2026-11-24): a read-only listing of `backups/` against the
        // `backup_records` rows, the same open-time trigger as the
        // pass above and the same best-effort contract — it logs its
        // own failures through the facade and can never fail this
        // open. Nothing is deleted here: the debris is made visible
        // and the decision stays with the user (D-190).
        crate::backup_orphans::report(data_dir, &connection);
        Ok(Db {
            connection: Arc::new(Mutex::new(connection)),
            backups_dir,
            created_fresh,
            commit_hook: None,
        })
    }

    /// Installs the "a write committed" seam ([`Db::run`]'s hook,
    /// DB-022/T-101) — the one writer of the [`Db::commit_hook`]
    /// field. Called once per open path by the shared session tail
    /// (`crate::start_database_session`: the `setup` hook's healthy
    /// launch and the recovery commands' reopen both run it) after a
    /// successful open, before the database is managed and the first
    /// operation can run; `&mut self` is the proof that no operation
    /// is in flight while the hook appears (every `run` clones the
    /// field's `Arc` under the same guarantee — which is also why the
    /// tail clones the retention handle only AFTER this call). The hook itself is built by
    /// [`crate::dirty_recalc::commit_hook`]; nothing in this module
    /// knows what it does — the seam is deliberately dumb.
    pub(crate) fn set_commit_hook(&mut self, hook: Arc<dyn Fn() + Send + Sync>) {
        self.commit_hook = Some(hook);
    }

    /// Whether this database was created from scratch by the open that
    /// produced this handle (see the field docs): the §5.5 "empty DB"
    /// hint `app_info` serves. `false` when no database is running at
    /// all — that is the caller's decision to make (recovery mode's
    /// honest answer is "no fresh database was created", which the
    /// `app_info` command encodes).
    pub(crate) fn created_fresh(&self) -> bool {
        self.created_fresh
    }

    /// The ERR-001 startup step (spec §10): one `VACUUM INTO` copy per
    /// UTC day — `db-backups/daily-YYYYMMDD.sqlite3` — with the newest
    /// 7 kept. The store's `eunomia_store::storage::daily_backup` owns
    /// the semantics (same-day idempotency by file name, retention by
    /// name order, the D-038 staging/publication mechanics); this
    /// method is the app-layer seam: it locks the one connection like
    /// any operation (at startup it is uncontended) and hands the
    /// store the `db-backups/` path derived once in [`Db::open`].
    ///
    /// `now` is the date seam — the injectable "today": only its UTC
    /// day names the copy, so the same-day and retention behavior is
    /// deterministic under test. The shared session tail passes the
    /// real clock.
    ///
    /// Not callable from inside a [`Db::run`] closure: this method
    /// takes the connection lock itself, and the `std::sync::Mutex` is
    /// not reentrant — an operation already holding the guard would
    /// deadlock. Its only caller today — that tail
    /// (`crate::start_database_session`) — runs before any operation
    /// exists (the `setup` hook before the event loop, the recovery
    /// command before its publication).
    ///
    /// **Degrade, never abort** (the recorded ERR-001 policy,
    /// DECISIONS.md D-046): the daily copy is extra reliability, not a
    /// precondition — a database that opened and migrated cleanly is
    /// worth running even when its redundant copy cannot be written
    /// (unlike a failed *pre-migration* copy, which aborts, D-038).
    /// The failure is returned for the caller to log (through the
    /// facade) and the launch continues; this method therefore never
    /// wraps it into a [`DbInitError`].
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Backup`] when the copy cannot be
    /// written (missing/unwritable directory, failed `VACUUM INTO`).
    /// Retention is best-effort and never surfaces as an error (see
    /// the store's `daily_backup` docs) — the caller's degradation
    /// decision, not an abort.
    pub(crate) fn ensure_daily_backup(
        &self,
        now: Timestamp,
    ) -> Result<DailyBackupOutcome, StorageError> {
        let connection = lock_recovering_poison(&self.connection);
        eunomia_store::storage::daily_backup(&connection, &self.backups_dir, now)
    }

    /// The §9.4 crash-recovery sweep (T-258): every `running` run row
    /// a dead process left behind goes `interrupted`, in ONE short
    /// transaction, answering how many rows were taken.
    ///
    /// Synchronous like [`ensure_daily_backup`], and for a stronger
    /// reason than precedent: the sweep is only correct over rows no
    /// live run owns, and the moments that is true by definition are
    /// the `setup` hook (before the event loop can serve a command)
    /// and the recovery commands' reopen tail (recovery mode never
    /// managed a `Db`, so no run of this process exists) — the same
    /// setup-completes-before-invokes assumption every managed state
    /// and the ERR-001 copy already rest on. A `Db::run`-shaped
    /// spawned pass could race this process's own first `sync_run` —
    /// the flag is in memory and invisible to SQL — and mark a LIVE
    /// run `interrupted`; a synchronous call at either caller cannot.
    ///
    /// The caller (the shared session tail the `setup` hook and
    /// `run_recovery` both run) owns the degrade posture: a refusal is logged, never fatal to the
    /// launch or the recovery (the D-046/D-054 startup-housekeeping
    /// shape), and the next restart retries the idempotent sweep
    /// (the store scopes the UPDATE to `running` rows, so a re-run
    /// over recovered state answers zero).
    ///
    /// # Errors
    ///
    /// Returns the §7.10 wire value of the underlying store failure
    /// through [`sync_runs_to_app_error`] — the shared SQLite
    /// classifier for [`eunomia_store::sync_runs::SyncRunsError::Sql`]
    /// (the sweep decodes no vocabulary cell, so the `Member` arm is
    /// unreachable through this writer and maps to the class-only
    /// fallback).
    pub(crate) fn mark_interrupted_runs(&self, finished_at: &str) -> Result<usize, AppError> {
        let mut connection = lock_recovering_poison(&self.connection);
        let transaction = connection
            .transaction()
            .map_err(sqlite_error_to_app_error)?;
        let taken = eunomia_store::sync_runs::mark_interrupted_runs(&transaction, finished_at)
            .map_err(sync_runs_to_app_error)?;
        transaction.commit().map_err(sqlite_error_to_app_error)?;
        Ok(taken)
    }

    /// Runs `operation` on the blocking pool inside one transaction
    /// and returns its result as the §7.10 wire value (SYS-001,
    /// SYS-002, NFR-008) — the plain form of [`Db::run_with_mirror`]:
    /// the operation cannot journal-and-mirror, so the mirror buffer
    /// the cycle still creates stays empty and its post-commit flush
    /// emits nothing. See [`Db::run_with_mirror`] for the cycle; the
    /// mirror half never engages here.
    pub(crate) async fn run<T, F>(&self, operation: F) -> Result<T, AppError>
    where
        F: FnMut(&Transaction<'_>) -> Result<T, AppError> + Send + 'static,
        T: Send + 'static,
    {
        // One cycle, one implementation: the same attempt/retry loop
        // with a buffer the closure never records into. The shadowing
        // `mut` rebinds the moved capture so the adapter closure can
        // call the FnMut operation once per attempt — the same idiom
        // the retry loop itself uses.
        let mut operation = operation;
        self.run_with_mirror(move |tx, _mirror| operation(tx)).await
    }

    /// The journaling form of [`Db::run`] (T-324/D-228): the operation
    /// receives, beside the `&Transaction`, a
    /// [`crate::logging::MirrorBuffer`] — every `action_log` entry it
    /// inserts it records with
    /// [`MirrorBuffer::record`](crate::logging::MirrorBuffer::record)
    /// — and the executor flushes the buffer **only after the
    /// transaction commits**, through
    /// [`MirrorBuffer::flush_committed`](crate::logging::MirrorBuffer::flush_committed).
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
    /// # The mirror invariant (why the buffer exists)
    ///
    /// The journal's transactional semantics (D-051: a rolled-back
    /// operation leaves no entry claiming it happened) extend to the
    /// journal's file-log echo: a mirror line emitted while the
    /// transaction is still open names a row that a rollback erases,
    /// and a busy retry would re-run the insert AND the mirror call,
    /// duplicating the line. The buffer closes both holes by moving
    /// the WHEN to the one place that knows the commit happened:
    ///
    /// - a **fresh buffer per attempt** — a busy retry re-runs the
    ///   whole closure, and the failed attempt's records die with its
    ///   rollback: only the successful attempt's buffer is flushed,
    ///   exactly one mirror line per committed row;
    /// - the **flush after `commit()` succeeds** — on the blocking
    ///   thread, under the connection guard (the same lock state the
    ///   pre-T-324 sites emitted their lines under; the tracing call
    ///   itself is the only work, and the DB-022 commit hook that
    ///   follows it runs under the same rule). The lines ride the
    ///   process-global subscriber, so they land wherever the file
    ///   log's worker takes them — the thread they are emitted from
    ///   is not observable in the file.
    ///
    /// A fail-then-log producer (D-051's error half) is served by the
    /// same rule, not exempt from it: its error entry rides a
    /// transaction that commits — the abort/next transaction — so its
    /// own buffer flushes with that commit.
    ///
    /// # Busy retries (§10 `E_DB_LOCKED`)
    ///
    /// When an attempt fails with SQLite's busy/locked class —
    /// `busy_timeout` (DB-001) expired under a lock another
    /// connection holds — the executor retries the whole operation up
    /// to [`BUSY_RETRY_COUNT`] more times, each in a fresh
    /// transaction, pausing [`BUSY_RETRY_PAUSES_MS`] between attempts
    /// (spec §9.5's ladder); after the last retry the failure
    /// surfaces as the `E_DB_LOCKED` §10 wire value the classifier
    /// builds. Only that class is retried, however the operation
    /// reports it: the executor sees the raw rusqlite error of its own
    /// `BEGIN`/`COMMIT` edges, and — for a statement inside the
    /// operation body — the `details.os_kind` the shared classifier
    /// recorded (`"DatabaseBusy"`/`"DatabaseLocked"`; see
    /// [`app_error_is_busy_locked`]). Everything else, including the
    /// recorded fallback that also lands on `E_DB_LOCKED` (an I/O
    /// failure, a bug-shaped rusqlite error), surfaces on the first
    /// attempt without pausing or re-running anything.
    ///
    /// The operation therefore runs as `FnMut`: a retry re-invokes it.
    /// Because the failed attempt rolled back completely, the retry is
    /// a fresh run — not a resume — over whatever is committed when it
    /// starts (another operation may have committed during the pause,
    /// the very contention the cycle exists for). For everything the
    /// body does through the `&Transaction` the transaction itself is
    /// the idempotency mechanism: a replay either re-derives its result
    /// on the new state or fails cleanly on it. The body must read its
    /// inputs inside the transaction (never cache them across
    /// attempts), and the journal mirror was
    /// the one in-transaction side effect that needed its own
    /// discipline — the buffer makes it transactional like everything
    /// else; a body with side effects *outside* the transaction must
    /// still make them idempotent itself (§9.5-staged writes being the
    /// house pattern).
    ///
    /// The cycle is not cancellable: dropping the future the caller
    /// awaits does not stop the underlying `spawn_blocking` run — its
    /// retries (and, on an eventual success, its commit and its
    /// mirror flush) run to completion, which a dropped command future
    /// may observe only later. A command task must not assume a
    /// cancelled call left the database untouched (this predates the
    /// retries — one attempt could already outlive its future — the
    /// cycle only widens the window to the §9.5 worst case).
    ///
    /// A panic inside the operation does not poison anything the next
    /// operation would notice: the unwinding rolls the transaction
    /// back, the executor recovers the mutex guard (see the module
    /// docs), and the panic surfaces as the fallback `E_DB_LOCKED`
    /// `AppError` carrying the panic message — in release builds the
    /// workspace's `panic = "abort"` ends the process before this
    /// mapping could run, so that branch is a dev/test path.
    pub(crate) async fn run_with_mirror<T, F>(&self, operation: F) -> Result<T, AppError>
    where
        F: FnMut(&Transaction<'_>, &mut crate::logging::MirrorBuffer) -> Result<T, AppError>
            + Send
            + 'static,
        T: Send + 'static,
    {
        // Clone the handle before entering the closure: the closure
        // must own its connection access ('static for spawn_blocking),
        // not borrow it from the managed state. The commit hook rides
        // along the same way (an `Arc` clone — the hook, when
        // installed, is shared with every run, never per-call).
        let connection = Arc::clone(&self.connection);
        let commit_hook = self.commit_hook.clone();
        let joined = async_runtime::spawn_blocking(move || {
            let mut operation = operation;
            // 0-based: attempt 0 is the original run; retries are
            // attempts 1..=BUSY_RETRY_COUNT. The pauses and the
            // logging are indexed by the retry number, so the ladder
            // reads exactly as §9.5 prescribes it.
            let mut attempt = 0;
            loop {
                // The guard is dropped inside `attempt_once` before
                // this returns — the connection is free during the
                // pause, and the next attempt re-locks it.
                match attempt_once(&connection, commit_hook.as_ref(), &mut operation) {
                    AttemptOutcome::Success(value) => return Ok(value),
                    AttemptOutcome::Failed(error) => return Err(error),
                    AttemptOutcome::Busy(error) => {
                        let Some(pause_ms) = BUSY_RETRY_PAUSES_MS.get(attempt) else {
                            // §10's "up to 3 automatic retries" is
                            // exhausted: the last failure is the
                            // verdict, already in its §7.10 form.
                            return Err(error);
                        };
                        crate::logging::db_busy_retry(attempt + 1, *pause_ms);
                        // A blocking sleep, on the blocking pool —
                        // the whole closure already runs there, and an
                        // async sleep would park this task while its
                        // operation future (the `.await`ed join below)
                        // holds nothing; §9.5's ladder is wall-clock
                        // pauses either way.
                        std::thread::sleep(Duration::from_millis(*pause_ms));
                        attempt += 1;
                    }
                }
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

/// One attempt of [`Db::run`]'s retry loop: lock, one deferred
/// transaction, the operation, commit — the whole D-037 executor,
/// factored out so the loop around it stays readable. The mutex guard
/// is a local, so it drops when this returns — before any retry
/// pause, keeping the connection free while the executor sleeps.
///
/// Busy/locked failures are separated from everything else at every
/// edge they can reach: the executor's own `BEGIN`/`COMMIT` (raw
/// rusqlite errors, checked before classification) and a statement
/// inside the operation body (already classified by the operation —
/// see [`app_error_is_busy_locked`] for how the busy class survives
/// that trip). A failed attempt has fully rolled back by the time
/// this returns (the `Transaction` drop sends the `ROLLBACK`, and a
/// failed `commit` drops it the same way), which is what makes the
/// caller's retry a fresh run rather than a resume.
///
/// `commit_hook` is the DB-022/T-101 "a write committed" seam: after
/// a successful commit, the transaction fires it **iff it changed at
/// least one row** — the `total_changes` delta over the guarded
/// connection, read while the guard is still held so no other
/// operation's writes can blur it. Reads and no-op writes stay
/// silent, a busy-retry run fires exactly once (only its successful
/// attempt reaches this branch), and a rolled-back attempt never
/// fires. The hook runs with the mutex still held (the guard drops at
/// the function's end), so it must never block.
///
/// The attempt's [`crate::logging::MirrorBuffer`] is created here,
/// fresh, per attempt (T-324/D-228): the operation records its
/// journal entries into it, and ONLY the successful-commit branch
/// flushes it — the rollback paths (an `Err` body, a failed commit,
/// a busy verdict) drop it unflushed, records and rolled-back rows
/// together. The flush runs under the same guard as the commit hook
/// (the tracing calls are the only work) and BEFORE the hook: the
/// mirror is the journal's own echo of the commit, the hook the
/// derived UI consequence. The buffer dying here is the whole
/// discipline — a retry's fresh attempt cannot inherit the failed
/// attempt's records, which is what keeps a busy retry to exactly
/// one mirror line per committed row.
fn attempt_once<T, F>(
    connection: &Mutex<Connection>,
    commit_hook: Option<&Arc<dyn Fn() + Send + Sync>>,
    operation: &mut F,
) -> AttemptOutcome<T>
where
    F: FnMut(&Transaction<'_>, &mut crate::logging::MirrorBuffer) -> Result<T, AppError>,
{
    let guard = lock_recovering_poison(connection);
    // The row-change baseline of this attempt: `total_changes` counts
    // every INSERT/UPDATE/DELETE the connection ever ran, so the
    // delta across the transaction is exactly "this transaction wrote
    // rows" — the discrimination DB-022's trigger needs (a read-only
    // transaction's commit fires nothing).
    let changes_before = guard.total_changes();
    let transaction = match guard.unchecked_transaction() {
        Ok(transaction) => transaction,
        // `BEGIN DEFERRED` takes no lock, so busy here is unlikely —
        // but the edge exists, and the loop treats it like any other.
        Err(error) if rusqlite_error_is_busy_locked(&error) => {
            return AttemptOutcome::Busy(sqlite_error_to_app_error(error));
        }
        Err(error) => return AttemptOutcome::Failed(sqlite_error_to_app_error(error)),
    };
    // Fresh per attempt: a busy retry's closure re-runs over a fresh
    // buffer, never over the failed attempt's records.
    let mut mirror = crate::logging::MirrorBuffer::new();
    match operation(&transaction, &mut mirror) {
        Ok(value) => match transaction.commit() {
            // A failed commit also drops the transaction (rusqlite
            // sends the ROLLBACK), so nothing half-applied stays
            // behind on that path either.
            Ok(()) => {
                // The commit happened: the journal entries the
                // operation recorded exist, so their mirror lines
                // may exist — before this point a line would name a
                // row a rollback could still erase (D-051's rule,
                // the mirror half).
                mirror.flush_committed();
                if guard.total_changes() != changes_before
                    && let Some(hook) = commit_hook
                {
                    hook();
                }
                AttemptOutcome::Success(value)
            }
            Err(error) if rusqlite_error_is_busy_locked(&error) => {
                AttemptOutcome::Busy(sqlite_error_to_app_error(error))
            }
            Err(error) => AttemptOutcome::Failed(sqlite_error_to_app_error(error)),
        },
        Err(error) if app_error_is_busy_locked(&error) => AttemptOutcome::Busy(error),
        Err(error) => AttemptOutcome::Failed(error),
    }
}

/// The tri-state result of one [`Db::run`] attempt: success, a
/// busy/locked failure the retry loop may replay, or any other
/// failure that surfaces as-is. Keeping "busy" distinct from "failed
/// with code `E_DB_LOCKED`" is the point of the type — the recorded
/// fallback also produces `E_DB_LOCKED`, and replaying it would add
/// §9.5 pauses to failures that are not transient locks.
enum AttemptOutcome<T> {
    /// The operation committed; carry its value.
    Success(T),
    /// The attempt failed with SQLite's busy/locked class — a retry
    /// candidate, carrying the `E_DB_LOCKED` wire value of the
    /// failure (returned verbatim when the retries are exhausted).
    Busy(AppError),
    /// Any other failure, surfaced on the first attempt.
    Failed(AppError),
}

/// Whether a raw rusqlite failure is SQLite's busy/locked class —
/// the only failures §10's `E_DB_LOCKED` row prescribes retries for.
/// rusqlite reports the *primary* result code in `code`
/// (`SQLITE_BUSY`/`SQLITE_LOCKED`); the extended forms
/// (`SQLITE_BUSY_SNAPSHOT`, `SQLITE_LOCKED_SHAREDCACHE`, …) fold into
/// these two, and both are transient by SQLite's own contract. The
/// shared-cache variant cannot occur in this app (`open_db` never
/// enables shared cache), but covering the class rather than one
/// member costs nothing and stays correct if that ever changes.
/// `pub(crate)`: exercised directly by the discriminator test in
/// `db_tests.rs` — the busy acceptance tests only ever drive the
/// body-path edge (`BEGIN DEFERRED` takes no lock, and a busy
/// `COMMIT` cannot be staged deterministically), so without a direct
/// test this predicate's members and negatives would have no coverage.
pub(crate) fn rusqlite_error_is_busy_locked(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(sqlite_error, _)
            if matches!(
                sqlite_error.code,
                SqliteCode::DatabaseBusy | SqliteCode::DatabaseLocked
            )
    )
}

/// Whether an operation's own `AppError` came from SQLite's
/// busy/locked class — the body's half of the retry detection. The
/// operation classifies its statement failures through the shared
/// [`sqlite_error_to_app_error`], which records the exact rusqlite
/// category in `details.os_kind`; the busy kinds are matched by the
/// same `Debug` names the classifier writes (derived from the enum,
/// not spelled here, so rusqlite renaming a variant moves both sides
/// together). Anything else an operation can return — its own
/// `E_VALIDATION`, the fallback `E_DB_LOCKED` of an unclassifiable
/// failure (`os_kind` `"SystemIoFailure"`, …) or a hand-built error
/// with no details at all — is not busy and never retries.
/// `pub(crate)` for the same reason as
/// [`rusqlite_error_is_busy_locked`]: the body-path half is covered
/// end to end by the acceptance tests, but its `DatabaseLocked`
/// member and its negatives are not reachable through them, so the
/// suite drives this discriminator directly too.
pub(crate) fn app_error_is_busy_locked(error: &AppError) -> bool {
    let busy_kind = format!("{:?}", SqliteCode::DatabaseBusy);
    let locked_kind = format!("{:?}", SqliteCode::DatabaseLocked);
    error
        .details
        .as_ref()
        .and_then(|details| details.os_kind.as_deref())
        .is_some_and(|kind| kind == busy_kind || kind == locked_kind)
}

#[cfg(test)]
impl Db {
    /// Test seam for the busy-retry cycle: replaces the connection's
    /// DB-001 `busy_timeout` (5000 ms, set by `open_db`) with a test
    /// value. The acceptance tests hold a real external lock and wait
    /// for the timeout to expire on every attempt — at the production
    /// value one test would sleep through 4 × 5 s of timeout plus the
    /// §9.5 pauses; a few-millisecond timeout keeps the busy path
    /// deterministic *and* fast. DB-001 is untouched in production:
    /// the method exists only under `cfg(test)` and no production
    /// caller can reach it. Call before the operations under test
    /// (like any operation, this takes the connection lock).
    pub(crate) fn set_busy_timeout_for_tests(&self, timeout: Duration) {
        let connection = lock_recovering_poison(&self.connection);
        connection
            .busy_timeout(timeout)
            .expect("the test busy timeout must be settable");
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
/// later: a damaged file in the integrity gate ([`reject_corrupt`],
/// `E_DB_CORRUPT`), everything else in `open_db` or `run_migrations`
/// (D-030's `StorageError::Open` names the path). Claiming "too new"
/// for a version that could not be read would misclassify those cases
/// and bypass their errors; D-040 records that an "unknown" verdict
/// degrades the byte-identity guarantee to the backstop, which runs
/// after the pragmas.
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
/// in the store. The recovery commands (`recovery.rs`, T-024) re-run
/// this gate over a chosen `db-backups/` copy before applying it, so a
/// copy made by a newer app version is rejected before it replaces
/// anything.
pub(crate) fn reject_too_new(db_path: &Path) -> Result<(), DbInitError> {
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

/// The §6.3/§10 startup integrity gate (NFR-007…010, T-F11's
/// damaged-DB half): an **existing, non-empty** database file must
/// pass `PRAGMA quick_check` before the app opens it for real.
///
/// Placement in [`Db::open`]'s chain — after the too-new gate, before
/// `open_db` — is the load-bearing decision, in both directions:
///
/// - *After* [`reject_too_new`]: DB-004 fixes that gate as the first
///   step (D-040), and its verdict stays decisive on a file that is
///   both too new and damaged — the version is a header fact the gate
///   answers cheaper (one pragma read) than a full-file scan. A file
///   that is only too new is presumably healthy (a newer app made it)
///   and never pays the scan.
/// - *Before* `open_db`: the DB-001 pragma set **writes** to the file
///   (the `journal_mode=WAL` switch rewrites the header of a
///   rollback-journal database, D-030/D-040), and §10's `E_DB_CORRUPT`
///   effect starts recovery mode over a database the app has not
///   touched — T-F11's "data not changed" criterion, asserted on the
///   bytes in the tests below (the mutation "gate after `open_db`"
///   fails the byte-identity assertion exactly).
///
/// The check runs on a **bare** connection — no DB-001 pragma set, the
/// same D-037/D-040 shape as the too-new gate; src-tauri owning
/// `Connection` directly for a read-only startup gate. The recovery
/// commands (`recovery.rs`, T-024) re-run this very gate over a chosen
/// `db-backups/` copy before applying it — one detection mechanic for
/// the startup file and the copies it may be replaced from. `quick_check`
/// is read-only: a rollback-journal database gains no `-journal` or
/// sidecars from it, and for a database already in WAL mode SQLite
/// creates the `-shm`/`-wal` siblings for the read and removes them on
/// the clean drop (the D-040 sidecar facts, verbatim). The accepted
/// D-040 boundaries apply here too, all of them SQLite's own crash
/// recovery running during the read — bytes move, logical content
/// does not: a WAL with pending frames is checkpointed by the clean
/// drop, and a hot rollback journal is rolled back.
///
/// Skip conditions, so the gate costs nothing where there is nothing
/// to verify: a **missing** file (a fresh first launch — the bare
/// `Connection::open` the too-new gate already performed may have
/// created a 0-byte one) and a **0-byte** file (a valid empty SQLite
/// database; `quick_check` would answer "ok") are not checked at all.
/// `quick_check` reads every page of the file, so this skip is also
/// the performance boundary the spec's "at startup" wording leaves
/// open: existing databases pay one full read at every startup (spec
/// §6.3 names the check there), fresh launches pay nothing.
///
/// Only a **definitive** corruption verdict rejects. SQLite reports
/// damage in two shapes, both handled (verified empirically against
/// the bundled build; both pinned by tests):
///
/// - the pragma query itself fails — a garbage file at prepare with
///   `SQLITE_NOTADB` ("file is not a database"); a
///   `SQLITE_CORRUPT` prepare/step failure is classified the same way
///   (the same-verdict arm; on the bundled build a malformed image
///   instead surfaces as the first finding row below, and the query
///   only errors on rows after it, which the first-row read never
///   reaches);
/// - the query returns a first row other than `ok` — a finding,
///   carried verbatim: a shrunk page count yields "Tree 2 page 2 …
///   invalid page number 3", and a smashed b-tree page a row that
///   itself carries the b-tree init error ("*** in database main ***
///   / Tree 2 page 2: btreeInitPage() returns error code 11") — both
///   possibly with the `***` prefix and an embedded newline.
///
/// Every other failure of the read — busy/locked beyond the timeout,
/// an I/O error, an unopenable file — is *not* a corruption verdict:
/// the gate proceeds and the canonical failure surfaces one step
/// later in `open_db`/`run_migrations` (D-030's `StorageError::Open`
/// names the path). Claiming "corrupt" for a file that could not be
/// read would steer the user to the destructive recovery on what may
/// be a transient lock — the same misclassification D-037 recorded
/// rejecting. This mirrors the too-new gate's "unknown → proceed"
/// policy (D-040) with the same consequence: a corrupt file whose
/// read fails for a non-corruption reason degrades to the generic
/// abort instead of the typed verdict.
pub(crate) fn reject_corrupt(db_path: &Path) -> Result<(), DbInitError> {
    // Skip the missing and the empty: nothing to verify (see the
    // method docs). `metadata` failing on an existing-but-unstatable
    // path is also "cannot verify" — `open_db` reports it canonically.
    let exists_non_empty = std::fs::metadata(db_path).is_ok_and(|metadata| metadata.len() > 0);
    if !exists_non_empty {
        return Ok(());
    }
    // A file the bare open cannot even open is not a corruption
    // verdict either (permissions, a directory): proceed, and let
    // `open_db` produce the canonical failure.
    let Ok(connection) = Connection::open(db_path) else {
        return Ok(());
    };
    // Same 5000 ms as `open_db`'s DB-001 set and the too-new gate (the
    // constant lives in the store); duplicated here for the same
    // reason — the gate must decide before the pragma set runs.
    let _ = connection.busy_timeout(std::time::Duration::from_millis(5000));
    match connection.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0)) {
        // `quick_check` answers exactly "ok" on a healthy database.
        Ok(finding) if finding == "ok" => Ok(()),
        // A finding row: SQLite's own evidence, verbatim.
        Ok(finding) => Err(DbInitError::Corrupt {
            path: db_path.to_owned(),
            detail: finding,
        }),
        // The check itself failed on the file — definitive damage: on
        // the bundled build this arm fires for a garbage file
        // (`NotADatabase`, "file is not a database"); a
        // `DatabaseCorrupt` failure is classified the same way. The
        // message is SQLite's own, not prose composed here.
        Err(error @ rusqlite::Error::SqliteFailure(sqlite_error, _))
            if matches!(
                sqlite_error.code,
                SqliteCode::NotADatabase | SqliteCode::DatabaseCorrupt
            ) =>
        {
            Err(DbInitError::Corrupt {
                path: db_path.to_owned(),
                detail: error.to_string(),
            })
        }
        // Anything else is "cannot verify", not "verified corrupt":
        // proceed to the canonical failure (see the method docs).
        Err(_) => Ok(()),
    }
}

/// Classifies a `run_migrations` failure for [`Db::open`]: the
/// D-038 backstop of the too-new gate, and the UPD-004 mapping of a
/// failed migration. The store's migration run itself rejects a
/// database newer than its set (`DatabaseTooFarAhead`), which — once
/// [`reject_too_new`] has passed — can only mean the file changed under
/// the open (a too-new file swapped or rewritten between the two
/// steps). The verdict is re-derived from the version *fact* on the
/// open connection rather than matched from the store's error enum:
/// src-tauri does not depend on `rusqlite_migration` (the store is its
/// only consumer), and the fact is all the classification needs. A
/// failed migration step otherwise becomes the typed
/// [`DbInitError::Migration`] with the store error and the copy of the
/// unchanged state preserved. Any other failure — and a version that
/// still reads within range on a non-migration error — keeps the
/// generic [`DbInitError::Open`] shape and with it the D-027 abort
/// path.
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
    // The copy pointer travels inside the store's migration error; the
    // too-new fact still wins over everything (the backstop's own
    // meaning).
    let backup = match &source {
        eunomia_store::storage::StorageError::Migration { backup, .. } => backup.clone(),
        _ => None,
    };
    match too_new {
        Some(database_schema_version) => DbInitError::TooNew {
            path: db_path.to_owned(),
            database_schema_version,
            app_schema_version,
        },
        None => match source {
            migration @ eunomia_store::storage::StorageError::Migration { .. } => {
                DbInitError::Migration {
                    path: db_path.to_owned(),
                    source: Box::new(migration),
                    backup,
                }
            }
            other => DbInitError::Open {
                path: db_path.to_owned(),
                source: Box::new(other),
            },
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
/// storage failures. Consumed by the executor and the recovery
/// commands' blocking closures.
pub(crate) fn join_failure_to_app_error(error: tauri::Error) -> AppError {
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
/// `BEGIN`/`COMMIT` errors and for the command tasks mapping their
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

/// Maps the run repository's failures onto the §7.10 wire — the ONE
/// spelling of the mapper every `sync_runs` consumer routes through
/// (seven spellings consolidated by T-264/D-149: the five per-phase
/// copies, the `commands.rs` one, and the inline match this file's
/// `mark_interrupted_runs` used to carry; the phases had their own
/// because `commands.rs` was a frozen zone when they landed,
/// D-130/D-132).
/// The two arms are the family's whole semantics: SQLite failures
/// through the shared base classifier above, a corrupt vocabulary
/// cell as the class-only fallback — the column is named, the stored
/// value never enters the diagnostic (SEC-008, the D-039
/// corrupt-cells policy).
///
/// The mapper answers the `Err` envelope only; it writes no row and
/// no log line. Which terminal state a failed run's row reaches is
/// decided beside it, by the row's own state (the D-138 P3-4 ruling
/// in `commands.rs`'s stop-run wrapper) — never by this mapping.
pub(crate) fn sync_runs_to_app_error(error: SyncRunsError) -> AppError {
    match error {
        SyncRunsError::Sql(source) => sqlite_error_to_app_error(source),
        SyncRunsError::Member { column, .. } => {
            let mut app_error = AppError::new(ErrorCode::DbLocked);
            app_error.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("SyncRunsCorrupt".to_owned()),
                os_message: Some(format!("a stored {column} cell is outside its vocabulary")),
            });
            app_error
        }
    }
}

/// Maps the backup-record repository's failures onto the §7.10 wire —
/// the [`sync_runs_to_app_error`] twin over the backups table's own
/// error enum (the label names which table's cell is corrupt). Same
/// two arms, same class-only rule (SEC-008); the kind label stays
/// distinct on purpose: `BackupRecordsCorrupt` over a
/// `backup_records` cell is a different triage fact than
/// `SyncRunsCorrupt` over a `sync_runs` cell.
pub(crate) fn backup_records_to_app_error(error: BackupRecordsError) -> AppError {
    match error {
        BackupRecordsError::Sql(source) => sqlite_error_to_app_error(source),
        BackupRecordsError::Member { column, .. } => {
            let mut app_error = AppError::new(ErrorCode::DbLocked);
            app_error.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("BackupRecordsCorrupt".to_owned()),
                os_message: Some(format!("a stored {column} cell is outside its vocabulary")),
            });
            app_error
        }
    }
}

/// The assertion-side twin of [`sync_runs_to_app_error`] for the
/// test suites (T-289, D-149's follow-up): the six per-suite
/// `SyncRunsError` → `AppError` helpers (`sync_phase0_tests.rs`'s
/// `sync_runs_app_error`, the `runs_app_error` of the phase 1–4
/// suites, `ipc_tests.rs`'s `sync_runs_read_error`) each carried
/// their own `Sql`-arm spelling; they all route through here now,
/// and the `Sql` arm rides the one production mapper, so a suite's
/// mapped envelope is exactly the envelope production puts on the
/// wire (the D-149 one-spelling law, now on the test side too). The
/// mapper family above stays contiguous — the twin is a `cfg(test)`
/// seam beside it, the [`Db::set_busy_timeout_for_tests`] shape.
///
/// # Panics
///
/// Panics if `error` carries the `Member` envelope — unreachable
/// from the suites, which read back only rows they wrote through
/// the sanctioned repositories, so the arm is a cannot-happen
/// guard: the panic names the arm and embeds the error's `Display`
/// (the offending value included, deliberately — assertion-side
/// wording that never reaches the wire; the wire's class-only rule
/// stays in [`sync_runs_to_app_error`], SEC-008). The guard is
/// pinned by `db_tests`'s
/// `the_assertion_side_helper_panics_on_the_unreachable_member_envelope`,
/// the `Sql` delegation by
/// `the_assertion_side_helper_routes_sql_through_the_one_mapper`.
#[cfg(test)]
pub(crate) fn sync_runs_app_error_for_tests(error: SyncRunsError) -> AppError {
    match error {
        error @ SyncRunsError::Member { .. } => panic!(
            "the assertion-side sync-run read hit the `Member` envelope, which the suites cannot produce (rows are read back through the sanctioned writers): {error}"
        ),
        error @ SyncRunsError::Sql(_) => sync_runs_to_app_error(error),
    }
}

/// Why the database could not be opened or migrated at startup. Kept
/// as data (the same shape as `DataLayoutError`/`ConfigError`) because
/// the `Display` text is what the startup failure surfaces — tauri
/// panics in the event-loop `Ready` handler with `Failed to setup
/// app: …`, which prints the `Display` form, so the path must be in
/// the message itself (D-027's reasoning, applied here too).
///
/// Three variants are named user situations rather than anonymous
/// infrastructure failures: [`DbInitError::TooNew`] (DB-004/NFR-010) —
/// a database created by a newer app version, which the app must not
/// change and the user answers with "Update the app" —
/// [`DbInitError::Corrupt`] (spec §6.3/§10, NFR-007…010) — a database
/// damaged enough to fail `PRAGMA quick_check`, which the app must
/// not change either and the user answers from the recovery screen —
/// and [`DbInitError::Migration`] (spec UPD-004) — a migration step
/// that failed and was rolled back, leaving the database unchanged,
/// with the user's remedy "install the previous app version" and the
/// pre-migration copy as the pointer. Their typed §7.10/§10 wire
/// forms are [`DbInitError::app_error`]; every other failure stays an
/// open error that aborts the launch (D-027/D-028).
#[derive(Debug)]
pub(crate) enum DbInitError {
    /// The file could not be opened or configured — every failure
    /// except the too-new gate and a rolled-back migration: open/pragma
    /// (`StorageError::Open`), WAL (`StorageError::JournalMode`),
    /// safety copy (`StorageError::Backup`).
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
    /// A migration step failed (spec UPD-004): the step's own
    /// transaction was rolled back, so the database keeps its schema
    /// and data unchanged, and the user's remedy is to install the
    /// previous app version — with the pre-migration copy of the
    /// unchanged state as the recovery pointer. `backup` is `None`
    /// when the run wrote no copy (the failure hit a database this run
    /// created): not the UPD-004 situation, so the error keeps the
    /// generic abort instead of the typed wire form.
    Migration {
        /// The database file whose migration failed.
        path: PathBuf,
        /// The store-layer failure. Boxed for the same size reason as
        /// [`DbInitError::Open`]'s source.
        source: Box<eunomia_store::storage::StorageError>,
        /// The copy of the unchanged database state (UPD-004's
        /// pointer), when the run wrote one.
        backup: Option<PathBuf>,
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
    /// The database file is damaged: `PRAGMA quick_check` failed (spec
    /// §6.3, §10 `E_DB_CORRUPT`, NFR-007…010, T-F11's damaged-DB half).
    /// Detected as the second step of [`Db::open`], after the too-new
    /// gate and before `open_db` — the DB-001 pragma set *writes* to
    /// the file, and §10's effect is "Start in recovery mode" on an
    /// unchanged file. `detail` is SQLite's own evidence, carried
    /// verbatim: the finding row `quick_check` returned (a page-level
    /// finding, or — for a malformed image on the bundled build — a
    /// row that itself carries the b-tree init error), or the error
    /// message when the query itself failed (a garbage file's "file is
    /// not a database") — never prose this layer composed.
    Corrupt {
        /// The rejected database file.
        path: PathBuf,
        /// SQLite's own finding or error text, carried to the wire
        /// params and the log verbatim.
        detail: String,
    },
}

impl DbInitError {
    /// The typed §7.10/§10 form when the failure is a named user
    /// situation: a too-new database maps to `E_DB_TOO_NEW` (DB-004,
    /// NFR-010) with the database path and both schema versions as
    /// interpolation params — the contract the "Update the app"
    /// screen (§10's user action; the future recovery/update task)
    /// renders. A rolled-back migration with its copy (UPD-004) maps
    /// to `E_UPDATE_FAILED` with the `errors.update_migration_failed`
    /// message key and the database and copy paths as params — see
    /// that arm's comment for why that code is the registry's best
    /// carrier. A damaged database maps to `E_DB_CORRUPT` (§10) with
    /// the path and SQLite's own `quick_check` evidence as params —
    /// the contract the recovery screen (§10's effect: data directory
    /// path, `db-backups/` list, "Open data folder" and "Start with
    /// empty database") interpolates; the default key fits
    /// (`errors.db_corrupt`, no per-instance override needed). `None`
    /// for every other init failure (including a migration failure
    /// without a copy: not the UPD-004 situation): their effect is the
    /// D-027/D-028 abort, and inventing a code for them is the
    /// registry gap D-037 records.
    pub(crate) fn app_error(&self) -> Option<AppError> {
        match self {
            DbInitError::TooNew {
                path,
                database_schema_version,
                app_schema_version,
            } => {
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
            DbInitError::Corrupt { path, detail } => {
                // A verdict, not an OS failure: the quick_check evidence
                // is SQLite's own text, and `details` (UI-011's collapsed
                // OS section) stays null like the other verdicts' — the
                // finding travels as an interpolation param so the
                // recovery screen can show it inline next to the path.
                let mut error = AppError::new(ErrorCode::DbCorrupt);
                error
                    .params
                    .insert("path".to_owned(), path.display().to_string());
                error.params.insert("detail".to_owned(), detail.clone());
                Some(error)
            }
            DbInitError::Migration {
                path,
                backup: Some(backup),
                ..
            } => {
                // The wire carrier is `E_UPDATE_FAILED` — the §10
                // registry (fixed by T-013) has no migration-failure
                // code, and this requirement is UPD-004, the Updates
                // section's own rollback rule: the new version cannot
                // be adopted ("Update not installed" — install the
                // previous one), with "Retry later" as the
                // complementary remedy (re-update to a fixed version).
                // The rejected alternatives lie harder about the
                // facts: `E_DB_CORRUPT` (the database passed the
                // migration's own rollback intact; §10's row would
                // steer the user to the destructive empty-database
                // recovery), `E_DB_TOO_NEW` (no version skew exists —
                // its action points the app version the wrong way)
                // and `E_DB_LOCKED` (not busy; retrying the same
                // binary fails again). The localized text comes from
                // the message key (NFR-006), so the UI shows the
                // UPD-004 instruction, not the §10 row's prose; the
                // mismatch is a recorded registry gap (DECISIONS.md,
                // with this task's entry).
                let mut error = AppError::new(ErrorCode::UpdateFailed);
                error.message_key = "errors.update_migration_failed".to_owned();
                error
                    .params
                    .insert("path".to_owned(), path.display().to_string());
                error
                    .params
                    .insert("backup_path".to_owned(), backup.display().to_string());
                Some(error)
            }
            // A fresh bootstrap has no copy and no previous state to
            // return to — the generic abort, honestly.
            DbInitError::Migration { backup: None, .. } | DbInitError::Open { .. } => None,
        }
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
            DbInitError::Migration {
                path,
                source,
                backup,
            } => {
                // Wording true for every producer of the variant: the
                // failed step rolled back (nothing of it survives) and
                // the database keeps the state of its last committed
                // migration — which for a multi-step run may be an
                // earlier step of the same run, and for a run that
                // created the database is "nothing applied yet". The
                // UPD-004 instruction and the copy pointer (this text
                // is what the startup panic prints) follow when a
                // copy exists.
                match backup {
                    Some(backup) => {
                        // The store error's own Display repeats the
                        // copy path (its self-contained form), so only
                        // the underlying migration failure is printed
                        // here — the abort text names the path once.
                        let store: &dyn std::error::Error = source.as_ref();
                        let root = store.source().unwrap_or(store);
                        write!(
                            f,
                            "the migration of the database at {} failed and \
                             was rolled back — the database keeps the state \
                             of its last committed migration; install the \
                             previous app version; a copy of that state is \
                             at {}: {}",
                            path.display(),
                            backup.display(),
                            root
                        )
                    }
                    None => write!(
                        f,
                        // No copy exists — and for the producers of
                        // this shape (a fresh bootstrap's failed step,
                        // or a version read that failed before any
                        // step ran) "was rolled back" would claim a
                        // transaction that may never have opened; the
                        // kept-state fact is the honest wording.
                        "the migration of the database at {} failed — the \
                         database keeps the state of its last committed \
                         migration; no pre-migration copy was written: {}",
                        path.display(),
                        source
                    ),
                }
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
            DbInitError::Corrupt { path, detail } => {
                write!(
                    f,
                    "the database at {} is damaged and cannot be opened: \
                     {detail}; the app does not change the database — \
                     start in recovery mode and pick a copy from \
                     db-backups/ or an empty database (E_DB_CORRUPT)",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for DbInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DbInitError::Open { source, .. } | DbInitError::Migration { source, .. } => {
                Some(source.as_ref())
            }
            // A verdict, not a wrapper: the version facts and the
            // quick_check evidence are the whole failure, already in
            // the message and the typed `AppError`.
            DbInitError::TooNew { .. } | DbInitError::Corrupt { .. } => None,
        }
    }
}
