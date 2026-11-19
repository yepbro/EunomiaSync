//! The LOG-006 retention task: journal and sync-run cleanup at
//! startup and once a day (spec §7.7 LOG-006, §6.4 "Log and runs";
//! DECISIONS.md D-054).
//!
//! The store owns the semantics —
//! [`eunomia_store::action_log::cleanup`] decides what "beyond the
//! limits" means (the disjunction of `log.max_entries` and
//! `log.max_days`, the last 500 runs, the strict age boundary, the
//! `ts DESC, id DESC` determinism). This module owns the *when* and
//! the *how of the app side*:
//!
//! - **One operation, one transaction.** [`run_once`] is a single
//!   [`Db::run`] closure (SYS-001/SYS-002, D-037): it reads both
//!   `log.*` settings and prunes in the **same** transaction, so the
//!   limits the prune enforces cannot shift under it mid-operation,
//!   and the whole cleanup commits or rolls back as one set.
//! - **At startup, then once a day.** [`spawn_daily_cleanup`] spawns
//!   one task on tauri's async runtime that runs the cleanup
//!   immediately (the startup run — LOG-006's "at startup", fired
//!   right after the database is managed, off the `setup` thread)
//!   and then every [`DAILY_INTERVAL`]. Honest boundary, recorded in
//!   D-054: the interval is 24 h counted from launch, not a UTC
//!   midnight boundary — an app kept running prunes once per day on
//!   its own schedule, and the ERR-001 daily copy's same-day-by-name
//!   mechanic (D-046) is deliberately not replicated here because
//!   cleanup has no day-named artifact to key on; it is idempotent,
//!   so "once a day" needs no same-day guard.
//! - **Cancellation is process exit.** The task holds no handle
//!   anyone could await, and the app has no exit hook to signal one
//!   from. That is safe rather than sloppy: a tick interrupted by
//!   exit is interrupted inside one `Db::run` transaction, which
//!   SQLite's crash recovery rolls back whole, and the next startup
//!   runs the cleanup again (the startup half of the contract). A
//!   sleeping timer costs nothing; the JoinHandle is dropped on
//!   purpose.
//! - **Degrade, never abort — and keep ticking.** A failed cleanup
//!   is logged through the facade ([`crate::logging::db_cleanup_failed`],
//!   a `warn` — the D-046 daily-copy policy applied to housekeeping:
//!   the launch continues, the app is fully usable with an over-long
//!   journal) and the loop sleeps on: the failure may be transient
//!   (a busy connection, a full disk since freed), and the next tick
//!   retries. A cleanup that removed something is logged at `info`
//!   with the counts; a no-op logs nothing — it is the idempotent
//!   common case, and a line per day would only repeat itself.
//!
//! The journal boundary mirrors D-046's: the cleanup itself writes no
//! `action_log` entry (LOG-001…004's vocabulary has no housekeeping
//! member — extending the closed enum beyond the requirement's list
//! is a decision this task does not make), so the two facade lines
//! are its whole journal.

use std::time::Duration;

use jiff::Timestamp;
use rusqlite::Transaction;

use crate::db::Db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::logging;
use eunomia_store::action_log::{self, CleanupCounts, LogLimits};
use eunomia_store::settings::{self, SettingKey, SettingValue};
use eunomia_store::storage::StorageError;

/// How long the retention task sleeps between cleanups (LOG-006:
/// "cleanup runs at startup and once a day"). 24 h counted from the
/// run it follows — see the module docs for why that reading of
/// "once a day" is the honest one.
const DAILY_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Spawns the retention task: the startup cleanup runs immediately
/// (asynchronously — the `setup` hook does not wait for it; the
/// database is already open and managed when this is called), then
/// the task ticks every [`DAILY_INTERVAL`]. The task owns its `Db`
/// clone (a handle to the one connection, D-037's cheap-clone shape)
/// for the whole app run. The loop body is [`cleanup_loop`] — the
/// seam the lifecycle tests drive with a bounded tick count and a
/// short interval; this function only fixes the production interval
/// (24 h, forever) and moves the future onto the runtime.
pub(crate) fn spawn_daily_cleanup(db: Db) {
    tauri::async_runtime::spawn(cleanup_loop(db, DAILY_INTERVAL, None));
}

/// The ticker's loop: one pass, then sleep `interval`, repeated
/// `ticks` times — `None` forever (the production shape).
///
/// `ticks` exists for the tests, and that is its whole story: an
/// infinite future cannot be awaited, and aborting a spawned one
/// proves nothing about the ticks that should have followed. A
/// bounded run can: the lifecycle tests prove the first pass fires
/// with no external trigger and that a failed pass does not end the
/// loop — the next tick runs (and prunes) after the fault is
/// repaired.
async fn cleanup_loop(db: Db, interval: Duration, ticks: Option<u32>) {
    let mut remaining = ticks;
    loop {
        // `Some(0)` is "no passes", not "one pass": guard before the
        // body so the decrement below always sees a positive count.
        if remaining == Some(0) {
            return;
        }
        // The date seam: `run_once` takes "now" as a parameter, so
        // the age arithmetic is deterministic under test; the loop
        // passes the real clock, once per tick.
        let now = Timestamp::now();
        match run_once(&db, now).await {
            Ok(counts) if counts.log_entries > 0 || counts.sync_runs > 0 => {
                logging::db_cleanup(&counts);
            }
            Ok(_) => {}
            Err(error) => logging::db_cleanup_failed(&error),
        }
        // A bounded run ends after its last pass — no trailing sleep
        // a caller would wait out for nothing.
        if let Some(count) = &mut remaining {
            *count -= 1;
            if *count == 0 {
                return;
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// Runs one retention pass as one `Db::run` operation: both `log.*`
/// settings are read and the prune executes inside the **same**
/// transaction (SYS-002 — the limits and the deletes describe one
/// consistent state, and a mid-operation failure rolls the whole
/// pass back).
///
/// `now` is the date seam (the D-046 `daily_backup` shape): the
/// caller passes the real clock in production and a fixed instant in
/// tests, so the age boundary is testable without touching the
/// system clock.
///
/// # Errors
///
/// Returns the operation's [`AppError`]: a SQL failure through the
/// shared base classifier, a corrupt `log.*` settings cell as the
/// D-037 fallback shape with a **class-only** diagnostic in
/// `details.os_message` — the key and the failure class, never the
/// stored cell's own text (the §10 registry has no
/// "housekeeping failed" code; the caller logs, never shows it).
pub(crate) async fn run_once(db: &Db, now: Timestamp) -> Result<CleanupCounts, AppError> {
    db.run(move |tx| {
        let limits = read_limits(tx)?;
        action_log::cleanup(tx, &limits, now).map_err(storage_to_app_error)
    })
    .await
}

/// Reads the two LOG-006 limits from the settings repository — the
/// single validation owner of their ranges (D-039): whatever `get`
/// returns is §6.5-valid, so the cleanup receives in-range values by
/// construction, and a corrupt cell fails the operation explicitly
/// (the settings corrupt-cells policy) instead of defaulting.
fn read_limits(tx: &Transaction<'_>) -> Result<LogLimits, AppError> {
    let max_entries = settings::get(tx, SettingKey::LogMaxEntries).map_err(storage_to_app_error)?;
    let max_days = settings::get(tx, SettingKey::LogMaxDays).map_err(storage_to_app_error)?;
    match (max_entries, max_days) {
        // The only variant `from_json` accepts for these keys (the
        // kind check, D-039); the arm is exhaustive over the rest so
        // a future kind change is a conscious edit here.
        (SettingValue::Int(max_entries), SettingValue::Int(max_days)) => Ok(LogLimits {
            max_entries,
            max_days,
        }),
        (other_entries, other_days) => {
            // Unreachable through the validated settings path: `get`
            // itself rejects a stored value of the wrong kind. Kept
            // as an error, not a panic, so a future repository change
            // degrades into a logged cleanup failure, not a crashed
            // startup task.
            Err(unclassified_storage_shape(&format!(
                "log.max_entries = {}, log.max_days = {}",
                kind_name(&other_entries),
                kind_name(&other_days)
            )))
        }
    }
}

/// The name of a setting value's variant, for the unreachable-kind
/// error message (diagnostics only — the values themselves never
/// enter the message).
fn kind_name(value: &SettingValue) -> &'static str {
    match value {
        SettingValue::Scheme(_) => "a scheme",
        SettingValue::BaseName(_) => "a base name",
        SettingValue::ExtraFile(_) => "an extra file",
        SettingValue::Bool(_) => "a boolean",
        SettingValue::Int(_) => "an integer",
        SettingValue::Str(_) => "a string",
        SettingValue::StrList(_) => "a string list",
    }
}

/// Maps a storage failure of the retention pass onto the §7.10 wire
/// shape the degrade path logs. A SQL failure keeps its classified
/// form (the shared base classifier, D-037). Everything else — a
/// corrupt settings cell is the realistic case — takes the recorded
/// fallback code with a **class-only** diagnostic in `details`: the
/// variant's own `Display` is deliberately not used, because several
/// `SettingsError` variants embed the stored cell's text (a member
/// string, a rejected value) and `os_message` feeds the §10 log line
/// — the T-027 `corrupt_cell_diagnostic` rule, applied to this seam:
/// the class and the key (this seam's own literal) travel, the
/// stored text does not.
fn storage_to_app_error(error: StorageError) -> AppError {
    match error {
        StorageError::Sql(sqlite) => crate::db::sqlite_error_to_app_error(sqlite),
        StorageError::Settings { key, .. } => unclassified_storage_shape(&format!(
            "reading the {key} setting failed (corrupt cell or version skew, D-039)"
        )),
        StorageError::ActionLog { .. } => unclassified_storage_shape(
            "an action-log read failed (corrupt cell or version skew, D-039)",
        ),
        // Unreachable from this seam (the reads here produce only the
        // `Sql`/`Settings` shapes above); kept honest anyway — the
        // class word identifies the variant without echoing its
        // payload.
        other => unclassified_storage_shape(&format!(
            "a storage failure of class {} (no further detail recorded)",
            storage_class(&other)
        )),
    }
}

/// The variant's class word — a fixed label, never the variant's
/// `Display`/`Debug` (either can embed stored values; a one-word
/// class cannot).
fn storage_class(error: &StorageError) -> &'static str {
    match error {
        StorageError::Migration { .. } => "migration",
        StorageError::Sql(_) => "sql",
        StorageError::Open { .. } => "open",
        StorageError::JournalMode { .. } => "journal-mode",
        StorageError::Backup { .. } => "backup",
        StorageError::Settings { .. } => "settings",
        StorageError::ActionLog { .. } => "action-log",
        StorageError::Permissions { .. } => "permissions",
    }
}

/// Builds the D-037 fallback wire value for a non-SQL storage
/// failure, with the reason in `details`.
fn unclassified_storage_shape(reason: &str) -> AppError {
    let mut app_error = AppError::new(ErrorCode::DbLocked);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("StorageError".to_owned()),
        os_message: Some(reason.to_owned()),
    });
    app_error
}

#[cfg(test)]
mod tests {
    //! The seam tests: the limits mapping, and the whole `run_once`
    //! pass over a real `Db` in a temp data directory (the same
    //! shape `db_tests.rs` uses for the executor). The retention
    //! *semantics* — disjunction, boundaries, cascade, determinism —
    //! are pinned by the store's own integration suite; what these
    //! tests pin is the app side: limits come from the settings table
    //! (stored values, not defaults, are honored), the pass runs
    //! through the `Db::run` executor, and the fallback error shape
    //! carries the storage reason.

    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A temp data directory holding the minimal layout `Db::open`
    /// needs (`db/` for the file, `db-backups/` for the migration
    /// safety copies, D-038) and the `Db` opened from it. Returns
    /// the directory for teardown — the connection closes with the
    /// `Db` value, and the Windows teardown rule (close before
    /// remove) is honored by dropping first.
    fn test_db(label: &str) -> (Db, PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be past the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "eunomia-retention-{label}-{}-{nanos}",
            std::process::id(),
        ));
        std::fs::create_dir_all(dir.join("db")).expect("db must be creatable");
        std::fs::create_dir_all(dir.join("db-backups")).expect("db-backups must be creatable");
        let db = Db::open(&dir).expect("opening a fresh database in a temp directory must work");
        (db, dir)
    }

    /// Closes the connection before removing the temporary directory
    /// (the Windows teardown rule).
    fn teardown(db: Db, dir: PathBuf) {
        drop(db);
        std::fs::remove_dir_all(&dir).expect("the temp directory should be removable");
    }

    /// Seeds journal rows with explicit timestamps (the accumulated
    /// history under test) through the executor.
    fn seed_log_rows(db: &Db, timestamps: &[String]) {
        for ts in timestamps {
            let sql = "INSERT INTO action_log (ts, level, visibility, action, result)
                       VALUES (?1, 'info', 'user', 'project.added', 'ok')"
                .to_owned();
            let ts = ts.clone();
            tauri::async_runtime::block_on(db.run(move |tx| {
                tx.execute(&sql, rusqlite::params![ts])
                    .map_err(|error| super::storage_to_app_error(error.into()))?;
                Ok(())
            }))
            .expect("seeding a journal row through the executor must work");
        }
    }

    /// How many journal rows exist, read through the executor.
    fn log_rows(db: &Db) -> i64 {
        tauri::async_runtime::block_on(db.run(|tx| {
            tx.query_row("SELECT COUNT(*) FROM action_log", [], |row| row.get(0))
                .map_err(|error| super::storage_to_app_error(error.into()))
        }))
        .expect("counting the journal through the executor must work")
    }

    /// `2026-11-19T00:00:00.<millis>Z` — a fixed oracle timestamp
    /// near the tests' fixed `now`.
    fn fixed_ts(millis_of_second: u64) -> String {
        format!("2026-11-19T00:00:00.{millis_of_second:03}Z")
    }

    /// The DB-002 text of an instant whole days before the fixed
    /// `now` (`2026-11-19T00:00:00Z`) plus milliseconds — an oracle
    /// built from arithmetic on the fixed epoch millisecond value,
    /// the same trick as the store suite's `ts` helper.
    fn ts_days_back(days: i64, millis_of_second: u32) -> String {
        Timestamp::from_millisecond(
            1_795_046_400_000 - days * 86_400_000 + i64::from(millis_of_second),
        )
        .expect("a fixed instant must parse")
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
    }

    #[test]
    fn run_once_honors_stored_settings_over_the_defaults() {
        // LOG-006's "limits come from settings": a stored
        // `log.max_entries` (here the §6.5 floor, 1000 — not the
        // 10 000 default) governs the prune. With 1001 accumulated
        // entries and the default the journal would survive whole;
        // with the stored 1000 exactly 1000 remain.
        let (db, dir) = test_db("stored-limits");
        let timestamps: Vec<String> = (0..1001).map(|i| fixed_ts(i % 1000)).collect();
        seed_log_rows(&db, &timestamps);
        tauri::async_runtime::block_on(db.run(|tx| {
            settings::set(tx, SettingKey::LogMaxEntries, SettingValue::Int(1000))
                .map_err(super::storage_to_app_error)?;
            Ok(())
        }))
        .expect("storing the setting through the executor must work");

        let now =
            Timestamp::from_millisecond(1_795_046_400_000).expect("a fixed instant must parse");
        let counts = tauri::async_runtime::block_on(run_once(&db, now))
            .expect("the retention pass must succeed");

        assert_eq!(
            counts.log_entries, 1,
            "the stored limit prunes, not the default"
        );
        assert_eq!(log_rows(&db), 1000);
        teardown(db, dir);
    }

    #[test]
    fn run_once_on_a_fresh_database_is_a_clean_noop() {
        // The every-first-launch case: no settings rows (the
        // repository's §6.5 defaults apply — 10 000/180), no journal,
        // no runs. The pass must succeed and remove nothing.
        let (db, dir) = test_db("fresh-noop");

        let now =
            Timestamp::from_millisecond(1_795_046_400_000).expect("a fixed instant must parse");
        let counts = tauri::async_runtime::block_on(run_once(&db, now))
            .expect("the retention pass over an empty database must succeed");

        assert_eq!(counts.log_entries, 0);
        assert_eq!(counts.sync_runs, 0);
        teardown(db, dir);
    }

    #[test]
    fn a_corrupt_settings_cell_fails_the_pass_with_the_reason_preserved() {
        // The D-039 corrupt-cells policy meets the degrade policy: a
        // tampered `log.max_entries` row fails the read explicitly,
        // the pass returns an error (the caller logs it and the app
        // continues), and the fallback shape carries a class-only
        // diagnostic naming the key — never the stored cell's own
        // text (the T-027 `corrupt_cell_diagnostic` rule).
        let (db, dir) = test_db("corrupt-setting");
        tauri::async_runtime::block_on(db.run(|tx| {
            tx.execute(
                "INSERT INTO settings (key, value, updated_at)
                 VALUES ('log.max_entries', 'not json', '2026-01-01T00:00:00.000Z')",
                [],
            )
            .map_err(|error| super::storage_to_app_error(error.into()))?;
            Ok(())
        }))
        .expect("planting the corrupt cell through the executor must work");

        let now =
            Timestamp::from_millisecond(1_795_046_400_000).expect("a fixed instant must parse");
        let error = tauri::async_runtime::block_on(run_once(&db, now))
            .expect_err("a corrupt settings cell must fail the retention pass");

        assert_eq!(
            error.code.as_str(),
            "E_DB_LOCKED",
            "the recorded D-037 fallback code"
        );
        let details = error
            .details
            .expect("the fallback carries the reason in details");
        assert_eq!(details.os_kind.as_deref(), Some("StorageError"));
        let message = details.os_message.unwrap_or_default();
        assert!(
            message.contains("log.max_entries") && message.contains("corrupt cell"),
            "the diagnostic names the key and the class, got: {message:?}"
        );
        assert!(
            !message.contains("not json"),
            "the stored cell's own text must not reach the log surface, got: {message:?}"
        );
        teardown(db, dir);
    }

    #[test]
    fn run_once_honors_a_stored_max_days_on_the_age_criterion() {
        // The second limit travels the seam too: a stored
        // `log.max_days = 7` (not the 180 default) prunes the
        // 10-day-old entry and keeps yesterday's — at the default
        // both survive, and `log.max_entries` stays at its 10 000
        // default, so the age criterion alone fires.
        let (db, dir) = test_db("stored-days");
        let ten_days_old = ts_days_back(10, 500);
        let yesterday = ts_days_back(1, 500);
        seed_log_rows(&db, &[ten_days_old, yesterday.clone()]);
        tauri::async_runtime::block_on(db.run(|tx| {
            settings::set(tx, SettingKey::LogMaxDays, SettingValue::Int(7))
                .map_err(super::storage_to_app_error)?;
            Ok(())
        }))
        .expect("storing the setting through the executor must work");

        // The fixed now is 2026-11-19T00:00:00.500Z; the cutoff at 7
        // days sits exactly at the 10-day-old entry's stamp minus 3
        // days — far past it — while yesterday's entry is inside.
        let now =
            Timestamp::from_millisecond(1_795_046_400_500).expect("a fixed instant must parse");
        let counts = tauri::async_runtime::block_on(run_once(&db, now))
            .expect("the retention pass must succeed");

        assert_eq!(counts.log_entries, 1, "the stored day limit prunes");
        let surviving_ts: String = tauri::async_runtime::block_on(db.run(|tx| {
            tx.query_row("SELECT ts FROM action_log", [], |row| row.get(0))
                .map_err(|error| super::storage_to_app_error(error.into()))
        }))
        .expect("reading the survivor through the executor must work");
        assert_eq!(
            surviving_ts, yesterday,
            "exactly yesterday's entry survives the age criterion"
        );
        teardown(db, dir);
    }

    #[test]
    fn read_limits_returns_the_section_6_5_defaults_on_a_fresh_database() {
        // The defaults half of "limits come from settings": with no
        // stored rows the seam reads 10 000/180 — the §6.5 defaults —
        // through the same `read_limits` the pass uses (the settings
        // repository's own tests cover `get`; this pins the wiring of
        // both keys through this seam, so a copy-paste between the
        // two reads cannot pass).
        let (db, dir) = test_db("default-limits");

        let limits = tauri::async_runtime::block_on(db.run(read_limits))
            .expect("reading the limits over a fresh database must succeed");

        assert_eq!(limits.max_entries, 10_000);
        assert_eq!(limits.max_days, 180);
        teardown(db, dir);
    }

    #[test]
    fn the_first_tick_fires_with_no_external_trigger() {
        // The ticker's lifecycle, half one: a bounded loop run (the
        // production spawn shape just fixes the interval and removes
        // the tick bound) performs its first pass by itself — nothing
        // has to poke the task after spawning. An oversized journal
        // shrinks to the stored limit inside the loop body.
        let (db, dir) = test_db("first-tick");
        let timestamps: Vec<String> = (0..1001).map(|i| fixed_ts(i % 1000)).collect();
        seed_log_rows(&db, &timestamps);
        tauri::async_runtime::block_on(db.run(|tx| {
            settings::set(tx, SettingKey::LogMaxEntries, SettingValue::Int(1000))
                .map_err(super::storage_to_app_error)?;
            Ok(())
        }))
        .expect("storing the setting through the executor must work");

        tauri::async_runtime::block_on(cleanup_loop(db.clone(), Duration::from_millis(1), Some(1)));

        assert_eq!(
            log_rows(&db),
            1000,
            "the loop's own first pass must prune without any external trigger"
        );
        teardown(db, dir);
    }

    #[test]
    fn a_failed_tick_does_not_end_the_loop() {
        // The ticker's lifecycle, half two: the first tick fails (a
        // corrupt `log.max_days` cell — the pass errors and the
        // journal stays whole, the whole pass being one transaction),
        // the fault is repaired while the loop sleeps, and the second
        // tick of the SAME loop run prunes. A loop that died on its
        // first error would leave the journal oversized and time this
        // test out. The timeline: tick 1 is immediate (a failed
        // settings read, microseconds), the repair rides after it,
        // tick 2 fires at the 100 ms interval — a ~95 ms margin past
        // the repair (the residual boundary is recorded at the
        // repair below).
        let (db, dir) = test_db("failed-tick");
        let timestamps: Vec<String> = (0..1001).map(|i| fixed_ts(i % 1000)).collect();
        seed_log_rows(&db, &timestamps);
        tauri::async_runtime::block_on(db.run(|tx| {
            tx.execute(
                "INSERT INTO settings (key, value, updated_at)
                 VALUES ('log.max_days', 'not json', '2026-01-01T00:00:00.000Z')",
                [],
            )
            .map_err(|error| super::storage_to_app_error(error.into()))?;
            Ok(())
        }))
        .expect("planting the corrupt cell through the executor must work");

        tauri::async_runtime::spawn(cleanup_loop(
            db.clone(),
            Duration::from_millis(100),
            Some(2),
        ));
        // Tick 1 has failed by now: it is an immediately-scheduled,
        // microsecond-scale failing read, and the 100 ms interval
        // leaves the repair a ~95 ms margin before tick 2. The
        // residual timing boundary is accepted and asymmetric: a
        // scheduler that delayed tick 1 past the repair would weaken
        // this run to the first-tick property (both ticks succeed)
        // rather than flake it. Repair both keys — the upsert
        // replaces the corrupt cell.
        std::thread::sleep(Duration::from_millis(5));
        tauri::async_runtime::block_on(db.run(|tx| {
            settings::set(tx, SettingKey::LogMaxEntries, SettingValue::Int(1000))
                .map_err(super::storage_to_app_error)?;
            settings::set(tx, SettingKey::LogMaxDays, SettingValue::Int(180))
                .map_err(super::storage_to_app_error)?;
            Ok(())
        }))
        .expect("repairing the settings between ticks must work");

        // Tick 2 (at the 100 ms mark) must prune; poll for it.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while log_rows(&db) != 1000 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            log_rows(&db),
            1000,
            "the second tick must prune after the first tick's failure — \
             a loop that ends on its first error leaves the journal oversized"
        );
        // Let the bounded task return and drop its `Db` before the
        // teardown removes the directory (the Windows teardown rule).
        std::thread::sleep(Duration::from_millis(50));
        teardown(db, dir);
    }

    /// The fallback builder is pure — one direct probe of the wire
    /// shape, independent of any database.
    #[test]
    fn the_fallback_shape_carries_the_reason() {
        let error = unclassified_storage_shape("a probe reason");
        assert_eq!(error.code.as_str(), "E_DB_LOCKED");
        let details = error.details.expect("details are always attached");
        assert_eq!(details.os_code, None);
        assert_eq!(details.os_kind.as_deref(), Some("StorageError"));
        assert_eq!(details.os_message.as_deref(), Some("a probe reason"));
    }
}
