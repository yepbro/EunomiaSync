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
//!   right after the database is managed, as a spawned task the
//!   caller never waits on)
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
//!
//! # Protective DB copies (T-253, owner ruling 2026-11-21)
//!
//! The module's second responsibility is age-based retention of the
//! *protective* database copies in `db-backups/` — the `pre-migration-*`
//! scheme (D-038) and the `pre-recovery-*` scheme (T-024/D-049), both of
//! which had accumulated uncapped until the owner ruled on 2026-11-21:
//! copies older than **7 days** go, by age alone (never by count), and
//! the newest copy of each kind survives even past the window. This is
//! plain filesystem work — no database, no settings — so unlike the
//! LOG-006 half it does not ride the `Db::run` executor or the daily
//! tick: [`cleanup_db_backups`] is called once, at the end of a
//! successful `Db::open` (`db.rs`) — every migration run happens inside
//! an open, so the pass fires right after the copies a migration just
//! wrote, and every successful launch opens the database, so the pass
//! doubles as the startup run. Honest boundaries, recorded in the
//! DECISIONS.md T-253 entry: a launch whose open *fails* (recovery
//! mode) runs no pass — the aged copies wait for the recovery
//! operation's own reopen or the next successful launch; a preserve is
//! deliberately NOT a trigger (a restore's chosen copy must outlive a
//! failed publish); the `daily-*` copies stay governed by ERR-001's own
//! count-of-7 rule. Degrade, never abort applies here too: a pass that
//! cannot read or remove logs through the facade and fails silently to
//! the caller — it must never turn a successful open into a reported
//! failure.

use std::path::{Path, PathBuf};
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
/// (asynchronously — the caller does not wait for it; the database
/// is already open and managed when this is called, in the shared
/// session tail, D-186), then
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

// ---- Protective DB copies (T-253, owner ruling 2026-11-21) ------------------
//
// Age-based retention over `db-backups/`: `pre-migration-*` and
// `pre-recovery-*` copies older than 7 days are deleted, count never
// enters it, and the newest copy of each kind is protected even past
// the window. Everything the pass does is derived from the directory's
// current state, so a rerun after any interruption converges (the
// idempotency contract the LOG-006 half above already follows).

/// The name prefix of the store's pre-migration scheme (D-038's
/// `backup_file_name`: `pre-migration-v<N>-<stamp>.sqlite3`). A literal
/// duplicated from the crate this pass must not depend on — the store
/// owns the creating scheme, this module only reads it; a test pins the
/// parser against names built the creating way.
const PRE_MIGRATION_PREFIX: &str = "pre-migration-v";

/// The name prefix of recovery's preserved-damaged-database scheme
/// (`recovery.rs`'s `PRESERVED_PREFIX`, D-049). Duplicated here for the
/// same reason as [`PRE_MIGRATION_PREFIX`].
const PRE_RECOVERY_PREFIX: &str = "pre-recovery-";

/// The extension both protective schemes share (the store's literal in
/// `backup_file_name`, recovery's `COPY_EXTENSION`).
const PROTECTIVE_COPY_EXTENSION: &str = ".sqlite3";

/// How long a protective copy stays beyond this pass's reach: the
/// owner ruling of 2026-11-21 — strictly older than 7 days goes, so a
/// copy exactly at the window's edge survives (the boundary test pins
/// the millisecond beside it).
const PROTECTIVE_COPY_WINDOW: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The two protective-copy kinds, each with its own protected-newest
/// copy (the ruling protects "one fresh copy of each kind", not one
/// overall: a recovery must not cost the newest migration safety net,
/// and vice versa).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtectiveCopyKind {
    PreMigration,
    PreRecovery,
}

impl ProtectiveCopyKind {
    /// The file-name prefix that identifies the kind.
    fn prefix(self) -> &'static str {
        match self {
            ProtectiveCopyKind::PreMigration => PRE_MIGRATION_PREFIX,
            ProtectiveCopyKind::PreRecovery => PRE_RECOVERY_PREFIX,
        }
    }
}

/// One `db-backups/` entry this pass recognized: its kind, the creation
/// stamp parsed from the file name, and the same-millisecond variant
/// (`-1`, `-2`, … — both creating schemes probe those spellings on
/// collisions, and the variant orders names within one stamp: `-2` was
/// written after `-1`, which was written after the plain name).
#[derive(Debug)]
struct ProtectiveCopy {
    kind: ProtectiveCopyKind,
    stamp: Timestamp,
    variant: u32,
    path: PathBuf,
}

/// The parsed name parts of a protective copy, before the caller pairs
/// them with the entry's real path — the parser knows only the name.
#[derive(Debug)]
struct ParsedCopyName {
    kind: ProtectiveCopyKind,
    stamp: Timestamp,
    variant: u32,
}

/// What one protective-copy pass removed, per kind — the audit fact the
/// facade line carries (SEC-008: numeric only).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BackupRetentionCounts {
    pub(crate) pre_migration: usize,
    pub(crate) pre_recovery: usize,
}

impl BackupRetentionCounts {
    /// Whether the pass removed anything (the no-op common case logs
    /// nothing — the same policy as `db_backup_daily`'s).
    fn removed_any(self) -> bool {
        self.pre_migration > 0 || self.pre_recovery > 0
    }
}

/// Parses one `db-backups/` entry `name` as a protective copy of
/// `kind`: every part must match the creating scheme exactly — the
/// kind's prefix, for pre-migration a decimal source schema version
/// (`pre-migration-v<N>-…`), a stamp in the schemes' own format (the
/// store's `utc_stamp` / recovery's `preserve_damaged`:
/// `%Y%m%dT%H%M%S%3fZ`), an optional decimal `-<variant>`, and the
/// `.sqlite3` extension.
///
/// Anything else answers `None`: a hand-renamed copy, a foreign file,
/// staging debris (`*.sqlite3.tmp` fails the extension check), a
/// sidecar sibling (`…-wal` likewise — the pass sweeps those together
/// with their main copy, never on their own). Such entries are **never
/// deleted on their own** — a file whose age cannot be established has
/// no age to judge, and deleting it would be exactly the count-based
/// guess the ruling replaces with age (the conservative half of the
/// age-source decision; see the DECISIONS.md T-253 entry).
fn parse_protective_copy(kind: ProtectiveCopyKind, name: &str) -> Option<ParsedCopyName> {
    let rest = name
        .strip_prefix(kind.prefix())?
        .strip_suffix(PROTECTIVE_COPY_EXTENSION)?;
    // Pre-migration names carry the source schema version between the
    // prefix and the stamp (`pre-migration-v2-<stamp>`); pre-recovery
    // names go straight to the stamp. The version is not part of the
    // kind's ordering — the ruling protects one fresh copy per *kind*,
    // across versions.
    let stamped = match kind {
        ProtectiveCopyKind::PreMigration => {
            let (version, stamped) = rest.split_once('-')?;
            if version.is_empty() || !version.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            stamped
        }
        ProtectiveCopyKind::PreRecovery => rest,
    };
    // The same-millisecond variant (`<stamp>-<N>`): `rsplit_once` is
    // safe because the stamp itself contains no `-`.
    let (stamp_text, variant) = match stamped.rsplit_once('-') {
        Some((stamp, n)) => (stamp, n.parse::<u32>().ok()?),
        None => (stamped, 0),
    };
    // jiff 1.0.1's `Timestamp::strptime` requires a parsed UTC offset,
    // and the schemes' trailing `Z` is a literal, not an offset
    // directive (verified: without `%z` the parse fails with "offset
    // required to parse timestamp") — the offset is appended and fed
    // explicitly instead: both creating schemes stamp `now_utc()`
    // values, so the stamp is UTC by construction. `%3f` stops at the
    // literal `Z`, and the parse still rejects out-of-range parts
    // (month 13, hour 25).
    let stamp = Timestamp::strptime("%Y%m%dT%H%M%S%3fZ%z", format!("{stamp_text}+0000")).ok()?;
    Some(ParsedCopyName {
        kind,
        stamp,
        variant,
    })
}

/// Runs one protective-copy retention pass over `backups_dir` (T-253):
/// every `pre-migration-*`/`pre-recovery-*` `.sqlite3` file whose
/// name-stamp is strictly older than `now − 7 days` is deleted, except
/// the newest copy of each kind (max stamp, the variant breaking
/// same-stamp ties), which is never deleted — not past the window, not
/// alongside a fresh sibling (the ruling's edge: a sole 10-day-old copy
/// goes the moment a new one exists, and stays until then). A deleted
/// copy's `-journal`/`-wal`/`-shm` sidecar siblings are swept with it,
/// best-effort: they are inert rename artifacts of a copy whose main
/// file is already gone, not databases of their own.
///
/// `now` is the date seam, the same shape as [`run_once`]'s. The one
/// production call site passes `Timestamp::now()` right after a
/// migration run: a copy that run just wrote is the newest of its kind
/// whatever its stamp is relative to the pass's clock, so the fresh
/// copy is protected by construction (the newest rule, not the
/// instant).
///
/// The age source is the **name stamp**, deliberately not `mtime`: a
/// pre-recovery copy is a *rename* of the damaged database, so its
/// mtime is the damaged file's last-write time and can be arbitrarily
/// older than the preserve moment — mtime-based age would delete fresh
/// copies as "old" the day they are made. The stamp is the creating
/// scheme's own clock.
///
/// Degrade, never abort (the module policy): the pass returns nothing
/// and never fails to its caller — a `db-backups/` it cannot read, an
/// entry it cannot stat, and a file it cannot remove are each logged
/// through the facade as a `warn` and the pass moves on, so it can
/// never turn a successful `Db::open` into a reported failure. A pass
/// that removed something logs the counts at `info`; a no-op logs
/// nothing (the idempotent common case — every open of a healthy
/// directory).
pub(crate) fn cleanup_db_backups(backups_dir: &Path, now: Timestamp) {
    let entries = match std::fs::read_dir(backups_dir) {
        Ok(entries) => entries,
        Err(error) => {
            logging::db_backup_retention_failed(backups_dir, &error);
            return;
        }
    };
    let mut copies: Vec<ProtectiveCopy> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                // Consistent with the unreadable-directory case: a
                // partially unreadable directory degrades with a line,
                // not silently (the pass still judges what it could
                // list).
                logging::db_backup_retention_failed(backups_dir, &error);
                continue;
            }
        };
        if !entry.file_type().is_ok_and(|file_type| file_type.is_file()) {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let kind = if name.starts_with(PRE_MIGRATION_PREFIX) {
            ProtectiveCopyKind::PreMigration
        } else if name.starts_with(PRE_RECOVERY_PREFIX) {
            ProtectiveCopyKind::PreRecovery
        } else {
            // `daily-*` (ERR-001's own count-of-7 rule governs them),
            // user files, everything else: not this pass's business.
            continue;
        };
        if let Some(ParsedCopyName {
            kind,
            stamp,
            variant,
        }) = parse_protective_copy(kind, name)
        {
            copies.push(ProtectiveCopy {
                kind,
                stamp,
                variant,
                path: entry.path(),
            });
        }
    }
    let cutoff = now - PROTECTIVE_COPY_WINDOW;
    let mut counts = BackupRetentionCounts::default();
    for kind in [
        ProtectiveCopyKind::PreMigration,
        ProtectiveCopyKind::PreRecovery,
    ] {
        let of_kind: Vec<&ProtectiveCopy> =
            copies.iter().filter(|copy| copy.kind == kind).collect();
        let Some(newest) = of_kind
            .iter()
            .max_by_key(|copy| (copy.stamp, copy.variant))
            .copied()
        else {
            continue;
        };
        for copy in of_kind {
            // The protected newest is compared by path, not identity:
            // `max_by_key` borrows from the same Vec the loop walks,
            // and the path is unique in the directory by construction.
            if copy.path == newest.path || copy.stamp >= cutoff {
                continue;
            }
            match std::fs::remove_file(&copy.path) {
                Ok(()) => {
                    // The copy's sidecar siblings (renamed alongside by
                    // a preserve, D-049) are swept with it, best-effort:
                    // a `NotFound` is the common case (most copies never
                    // had any) and stays silent; anything else is a
                    // warn like the main removal's would be.
                    for suffix in SIDECAR_SUFFIXES {
                        let sidecar = append_suffix(&copy.path, suffix);
                        if let Err(error) = std::fs::remove_file(&sidecar)
                            && error.kind() != std::io::ErrorKind::NotFound
                        {
                            logging::db_backup_retention_failed(&sidecar, &error);
                        }
                    }
                    match kind {
                        ProtectiveCopyKind::PreMigration => counts.pre_migration += 1,
                        ProtectiveCopyKind::PreRecovery => counts.pre_recovery += 1,
                    }
                }
                Err(error) => {
                    logging::db_backup_retention_failed(&copy.path, &error);
                }
            }
        }
    }
    if counts.removed_any() {
        logging::db_backup_retention(&counts);
    }
}

/// The sibling spellings SQLite owns next to a database file and a
/// preserve renames alongside its target (recovery's
/// `SIDECAR_SUFFIXES`, D-049): `pre-recovery-<stamp>.sqlite3-wal` and
/// friends. Duplicated here for the same reason as the name prefixes —
/// the creating scheme's owner is another module; the sweep test pins
/// the spellings.
const SIDECAR_SUFFIXES: [&str; 3] = ["-journal", "-wal", "-shm"];

/// `<path><suffix>` — the suffix goes after the whole file name, the
/// exact arithmetic `recovery.rs`'s own `append_suffix` uses for the
/// sibling spellings (duplicated here for the same reason as the name
/// prefixes: the creating scheme's owner is another module).
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
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

    // ---- Protective DB copies (T-253) ----------------------------------
    //
    // The pass's world is one directory of files, so every test seeds
    // literal-shaped names (the store's/recovery's schemes, built by
    // `protective_stamp` from arithmetic on a fixed epoch) into a temp
    // `db-backups/`, runs the seam with the fixed `now`, and asserts on
    // the observable directory state. The stamp format the tests use is
    // a literal of its own — never the production format string — so a
    // mutated production format cannot satisfy the parser tests with
    // itself (the same mutation honesty as the log-file name pins).

    /// The fixed `now` of the protective-copy tests:
    /// 2026-11-21T12:00:00.000Z in epoch milliseconds — two days and
    /// twelve hours after the journal tests' anchor.
    const PROTECTIVE_NOW_MS: i64 = 1_795_262_400_000;

    /// A `%Y%m%dT%H%M%S%3fZ` stamp for an instant whole days before the
    /// fixed `now`, plus a signed millisecond offset (negative offsets
    /// reach *past* the days-back boundary — the strict-window test's
    /// one-millisecond-older side) — the name-side oracle, built from
    /// arithmetic on the fixed epoch exactly the way `ts_days_back`
    /// builds the journal-side one. The format literal here is the
    /// test's own (see the section comment).
    fn protective_stamp(days_back: i64, millis_offset: i64) -> String {
        Timestamp::from_millisecond(PROTECTIVE_NOW_MS - days_back * 86_400_000 + millis_offset)
            .expect("a fixed instant must parse")
            .strftime("%Y%m%dT%H%M%S%3fZ")
            .to_string()
    }

    /// The pass's fixed `now`.
    fn protective_now() -> Timestamp {
        Timestamp::from_millisecond(PROTECTIVE_NOW_MS).expect("a fixed instant must parse")
    }

    /// A temp `db-backups/` directory — the pass's whole world.
    fn protective_backups_dir(label: &str) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("failed to create a temp directory");
        let dir = temp.path().join(label);
        std::fs::create_dir(&dir).expect("creating the temp backups directory must work");
        (temp, dir)
    }

    /// Seeds one file into `dir` and returns its path.
    fn seed_copy(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"protective copy fixture")
            .expect("writing the fixture copy must work");
        path
    }

    /// The sorted file names currently in `dir` — the observable state
    /// every pass test asserts on.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("the temp backups directory must be readable")
            .map(|entry| {
                entry
                    .expect("every entry must be readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_stamp_helper_matches_the_creating_schemes_shape() {
        // The oracle's own pin: `protective_stamp` must produce the
        // store's/recovery's literal stamp shape (`utc_stamp`,
        // `preserve_damaged`) — a fixed instant, digit for digit.
        assert_eq!(protective_stamp(0, 123), "20261121T120000123Z");
        assert_eq!(protective_stamp(9, 500), "20261112T120000500Z");
    }

    #[test]
    fn the_parser_accepts_both_schemes_including_variants() {
        // Direct probes of the pure parser: both prefixes, the
        // pre-migration version digit, the same-millisecond `-N`
        // variant, and the stamp it extracts (compared as an instant
        // against the arithmetic anchor, not against the parser's own
        // output).
        let stamp = protective_stamp(3, 250);
        let parsed = parse_protective_copy(
            ProtectiveCopyKind::PreMigration,
            &format!("pre-migration-v2-{stamp}.sqlite3"),
        )
        .expect("a scheme-shaped pre-migration name must parse");
        // Compared as an instant against an independently parsed
        // literal (jiff's `Display` trims trailing fractional zeros, so
        // string comparison would be format-sensitive, not value-).
        assert_eq!(
            parsed.stamp,
            "2026-11-18T12:00:00.250Z"
                .parse::<Timestamp>()
                .expect("the expected instant must parse")
        );
        assert_eq!(parsed.variant, 0);

        let variant = parse_protective_copy(
            ProtectiveCopyKind::PreMigration,
            &format!("pre-migration-v10-{stamp}-3.sqlite3"),
        )
        .expect("the suffixed variant must parse");
        assert_eq!(
            variant.variant, 3,
            "the same-millisecond variant is carried"
        );

        let plain = parse_protective_copy(
            ProtectiveCopyKind::PreRecovery,
            &format!("pre-recovery-{stamp}.sqlite3"),
        )
        .expect("a scheme-shaped pre-recovery name must parse");
        assert_eq!(
            plain.stamp,
            "2026-11-18T12:00:00.250Z"
                .parse::<Timestamp>()
                .expect("the expected instant must parse")
        );

        // The rejections: a version that is not decimal digits, a stamp
        // the format does not parse, a foreign extension, a name outside
        // both schemes.
        assert!(
            parse_protective_copy(
                ProtectiveCopyKind::PreMigration,
                &format!("pre-migration-vX-{stamp}.sqlite3")
            )
            .is_none(),
            "a non-digit schema version is not this pass's file"
        );
        assert!(
            parse_protective_copy(
                ProtectiveCopyKind::PreRecovery,
                "pre-recovery-notastamp.sqlite3"
            )
            .is_none(),
            "an unparseable stamp is not this pass's file"
        );
        assert!(
            parse_protective_copy(
                ProtectiveCopyKind::PreRecovery,
                &format!("pre-recovery-{stamp}-x.sqlite3")
            )
            .is_none(),
            "a non-numeric same-millisecond variant is not this pass's file"
        );
        assert!(
            parse_protective_copy(
                ProtectiveCopyKind::PreMigration,
                &format!("pre-migration-v2-{stamp}.sqlite3.tmp")
            )
            .is_none(),
            "staging debris is not this pass's file"
        );
        assert!(
            parse_protective_copy(ProtectiveCopyKind::PreRecovery, "daily-20261121.sqlite3")
                .is_none(),
            "a daily copy is not a protective copy"
        );
    }

    #[test]
    fn aged_copies_of_both_kinds_go_while_recent_ones_stay() {
        // The ruling's core: strictly older than 7 days goes, per kind;
        // inside the window stays. A daily copy sits among them to pin
        // that ERR-001's own scheme is untouched.
        let (temp, dir) = protective_backups_dir("aged-and-recent");
        let aged_migration = seed_copy(
            &dir,
            &format!("pre-migration-v1-{}.sqlite3", protective_stamp(10, 0)),
        );
        let recent_migration = seed_copy(
            &dir,
            &format!("pre-migration-v3-{}.sqlite3", protective_stamp(2, 0)),
        );
        let aged_recovery = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(8, 0)),
        );
        let recent_recovery = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(1, 0)),
        );
        let daily = seed_copy(&dir, "daily-20261120.sqlite3");

        cleanup_db_backups(&dir, protective_now());

        assert!(
            !aged_migration.exists(),
            "the 10-day-old pre-migration copy is beyond the window and not the newest of its kind"
        );
        assert!(recent_migration.exists(), "the 2-day-old copy stays");
        assert!(
            !aged_recovery.exists(),
            "the 8-day-old pre-recovery copy is beyond the window and not the newest of its kind"
        );
        assert!(recent_recovery.exists(), "the 1-day-old copy stays");
        assert!(
            daily.exists(),
            "the ERR-001 daily scheme is not this pass's business"
        );
        drop(temp);
    }

    #[test]
    fn the_newest_copy_of_a_kind_survives_even_past_the_window() {
        // The protection half: one fresh copy of each kind survives
        // even when it is itself older than the window — a sole
        // pre-recovery copy 10 days old stays, and of two aged
        // pre-migration copies only the older one goes.
        let (temp, dir) = protective_backups_dir("protected-newest");
        let sole_recovery = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(10, 0)),
        );
        let older_migration = seed_copy(
            &dir,
            &format!("pre-migration-v1-{}.sqlite3", protective_stamp(12, 0)),
        );
        let newest_migration = seed_copy(
            &dir,
            &format!("pre-migration-v2-{}.sqlite3", protective_stamp(10, 500)),
        );

        cleanup_db_backups(&dir, protective_now());

        assert!(
            sole_recovery.exists(),
            "a kind's only copy is its newest and stays even past the window"
        );
        assert!(
            !older_migration.exists(),
            "the 12-day-old copy is neither inside the window nor the newest of its kind"
        );
        assert!(
            newest_migration.exists(),
            "the newest pre-migration copy is protected even past the window"
        );
        drop(temp);
    }

    #[test]
    fn a_fresh_copy_retires_the_aged_sole_one() {
        // The ruling's recorded edge: a sole 10-day-old copy stays
        // exactly until a new one exists — then the old one goes and
        // the fresh one stays. This is the shape every real call site
        // produces (the pass runs at creation time, `now` being the
        // new copy's own stamp).
        let (temp, dir) = protective_backups_dir("retired-by-the-fresh");
        let aged = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(10, 0)),
        );
        let fresh = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(0, 0)),
        );

        cleanup_db_backups(&dir, protective_now());

        assert!(
            !aged.exists(),
            "the aged copy is no longer the newest of its kind once a fresh one exists"
        );
        assert!(fresh.exists(), "the fresh copy never goes");
        drop(temp);
    }

    #[test]
    fn the_seven_day_boundary_is_strict() {
        // "Older than 7 days": exactly at the window's edge a copy
        // stays; one millisecond past it, a non-newest copy goes. The
        // newest-of-kind (1 day old) makes the other two eligible.
        let (temp, dir) = protective_backups_dir("boundary");
        let newest = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(1, 0)),
        );
        let at_the_edge = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(7, 0)),
        );
        let past_the_edge = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(7, -1)),
        );

        cleanup_db_backups(&dir, protective_now());

        assert!(newest.exists());
        assert!(
            at_the_edge.exists(),
            "a copy exactly 7 days old is not older than the window"
        );
        assert!(
            !past_the_edge.exists(),
            "a copy one millisecond past the 7-day edge is beyond the window"
        );
        drop(temp);
    }

    #[test]
    fn an_empty_directory_is_a_quiet_noop() {
        // The first-launch state of `db-backups/`: no copies at all.
        // The pass must succeed and remove nothing.
        let (temp, dir) = protective_backups_dir("empty");

        cleanup_db_backups(&dir, protective_now());

        assert_eq!(names_in(&dir), Vec::<String>::new());
        drop(temp);
    }

    #[test]
    fn a_repeated_pass_removes_nothing_more() {
        // Idempotency: the second pass over a directory the first one
        // already cleaned converges to the same state — the protected
        // newest of a kind is not "aged" no matter how often the pass
        // looks at it.
        let (temp, dir) = protective_backups_dir("idempotent");
        seed_copy(
            &dir,
            &format!("pre-migration-v1-{}.sqlite3", protective_stamp(9, 0)),
        );
        seed_copy(
            &dir,
            &format!("pre-migration-v2-{}.sqlite3", protective_stamp(10, 0)),
        );
        seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(8, 0)),
        );
        seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(9, 500)),
        );
        cleanup_db_backups(&dir, protective_now());
        let after_first = names_in(&dir);
        assert_eq!(
            after_first.len(),
            2,
            "one protected copy of each kind remains"
        );

        cleanup_db_backups(&dir, protective_now());

        assert_eq!(
            names_in(&dir),
            after_first,
            "the second pass must be a no-op over a cleaned directory"
        );
        drop(temp);
    }

    #[test]
    fn same_stamp_variants_order_toward_the_higher_one() {
        // Two copies share a stamp 10 days old; the `-1` variant was
        // written after the plain name, so it is the protected newest
        // and the plain name goes.
        let (temp, dir) = protective_backups_dir("variant-order");
        let plain = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(10, 0)),
        );
        let variant = seed_copy(
            &dir,
            &format!("pre-recovery-{}-1.sqlite3", protective_stamp(10, 0)),
        );

        cleanup_db_backups(&dir, protective_now());

        assert!(
            !plain.exists(),
            "the plain same-stamp name is not the newest once a variant exists"
        );
        assert!(
            variant.exists(),
            "the higher same-stamp variant is the protected newest"
        );
        drop(temp);
    }

    #[test]
    fn undatable_and_foreign_names_are_never_touched() {
        // A renamed copy, a foreign file, staging debris: no age can be
        // established, so nothing is deleted — the conservative half of
        // the age-source decision. One valid aged non-newest copy sits
        // among them so the test discriminates (a pass that deleted
        // nothing at all would pass a survival-only assertion, but not
        // this one's removal check).
        let (temp, dir) = protective_backups_dir("conservative");
        let renamed = seed_copy(&dir, "pre-recovery-my-copy.sqlite3");
        let debris = seed_copy(
            &dir,
            &format!("pre-migration-v1-{}.sqlite3.tmp", protective_stamp(30, 0)),
        );
        let foreign = seed_copy(&dir, "notes.txt");
        seed_copy(
            &dir,
            &format!("pre-migration-v1-{}.sqlite3", protective_stamp(9, 0)),
        );
        seed_copy(
            &dir,
            &format!("pre-migration-v2-{}.sqlite3", protective_stamp(1, 0)),
        );

        cleanup_db_backups(&dir, protective_now());

        assert!(
            renamed.exists() && debris.exists() && foreign.exists(),
            "names without an establishable age are never this pass's files"
        );
        assert_eq!(
            names_in(&dir),
            vec![
                "notes.txt".to_owned(),
                format!("pre-migration-v1-{}.sqlite3.tmp", protective_stamp(30, 0)),
                format!("pre-migration-v2-{}.sqlite3", protective_stamp(1, 0)),
                "pre-recovery-my-copy.sqlite3".to_owned(),
            ],
            "exactly the valid aged non-newest copy was removed, and only it"
        );
        drop(temp);
    }

    #[test]
    fn a_directory_that_cannot_be_read_is_survived_quietly() {
        // The degrade half: `db-backups/` being unreadable (here: a
        // regular file where the directory should be) logs a warning
        // and returns — it must never panic and never surface to the
        // caller that triggered the pass.
        let temp = tempfile::tempdir().expect("failed to create a temp directory");
        let not_a_dir = temp.path().join("db-backups");
        std::fs::write(&not_a_dir, b"occupying file").expect("writing the blocker must work");

        cleanup_db_backups(&not_a_dir, protective_now());

        assert!(not_a_dir.is_file(), "the blocker itself is untouched");
    }

    #[test]
    fn a_retired_copy_takes_its_sidecar_siblings_with_it() {
        // The sweep half of a removal: a preserve renames `-wal`/
        // `-shm`/`-journal` alongside its target (D-049), so the main
        // copy's retirement must sweep them too — otherwise the disk
        // leak this task exists to stop would survive in the sidecars.
        // The protected newest's sidecar, by contrast, stays: its main
        // file does.
        let (temp, dir) = protective_backups_dir("sidecar-sweep");
        let newest = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(1, 0)),
        );
        let kept_sidecar = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3-wal", protective_stamp(1, 0)),
        );
        let aged = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(9, 0)),
        );
        let aged_wal = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3-wal", protective_stamp(9, 0)),
        );
        let aged_journal = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3-journal", protective_stamp(9, 0)),
        );
        let aged_shm = seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3-shm", protective_stamp(9, 0)),
        );

        cleanup_db_backups(&dir, protective_now());

        assert!(
            !aged.exists() && !aged_wal.exists() && !aged_journal.exists() && !aged_shm.exists(),
            "the retired copy and all three sidecar spellings go together"
        );
        assert!(
            newest.exists() && kept_sidecar.exists(),
            "the protected newest copy and its sidecar stay"
        );
        drop(temp);
    }

    #[test]
    fn a_partially_cleaned_directory_converges_on_the_rerun() {
        // The interrupted-pass shape (the idempotency principle's
        // "wherever the previous run stopped"): a previous pass deleted
        // one of three aged non-newest copies and died; the rerun must
        // retire exactly the remaining two, not re-decide, not skip.
        let (temp, dir) = protective_backups_dir("partial");
        seed_copy(
            &dir,
            &format!("pre-recovery-{}.sqlite3", protective_stamp(1, 0)),
        );
        for days_back in [8, 9, 10] {
            seed_copy(
                &dir,
                &format!("pre-recovery-{}.sqlite3", protective_stamp(days_back, 0)),
            );
        }
        // The interruption: one aged copy already gone by hand.
        std::fs::remove_file(dir.join(format!("pre-recovery-{}.sqlite3", protective_stamp(9, 0))))
            .expect("simulating the interrupted earlier pass must work");

        cleanup_db_backups(&dir, protective_now());

        assert_eq!(
            names_in(&dir),
            vec![format!("pre-recovery-{}.sqlite3", protective_stamp(1, 0))],
            "the rerun retires exactly the two remaining aged copies"
        );
        drop(temp);
    }

    #[test]
    fn a_large_directory_is_processed_linearly_to_its_verdict() {
        // Rule 6's large-input edge: a few hundred entries (aged and
        // recent, both kinds, undatable noise among them) must reach
        // exactly the ruling's verdict — the newest of each kind plus
        // everything inside the window, nothing else.
        let (temp, dir) = protective_backups_dir("large");
        let keep: Vec<String> = [0, 3, 6]
            .iter()
            .map(|&days| format!("pre-migration-v1-{}.sqlite3", protective_stamp(days, 0)))
            .chain(
                [0, 5]
                    .iter()
                    .map(|&days| format!("pre-recovery-{}.sqlite3", protective_stamp(days, 0))),
            )
            .chain(std::iter::once("daily-20261120.sqlite3".to_owned()))
            .collect();
        for name in &keep {
            seed_copy(&dir, name);
        }
        for day in 8..250 {
            let name = if day % 2 == 0 {
                format!("pre-migration-v3-{}.sqlite3", protective_stamp(day, 0))
            } else {
                format!(
                    "pre-recovery-{}-{}.sqlite3",
                    protective_stamp(day, 0),
                    day % 3
                )
            };
            seed_copy(&dir, &name);
        }

        cleanup_db_backups(&dir, protective_now());

        let mut expected = keep;
        expected.sort();
        assert_eq!(names_in(&dir), expected);
        drop(temp);
    }
}
