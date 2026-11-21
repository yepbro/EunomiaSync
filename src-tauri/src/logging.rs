//! File logging with daily rotation (spec §5.3, §5.5, §7.7; SYS-011…013,
//! SEC-008).
//!
//! The app writes one trace log per day into the data directory's `logs/`
//! subfolder — `logs/eunomiasync.<date>.log`, where `<date>` is the UTC
//! `YYYY-MM-DD` stamp of the write — via `tracing` +
//! `tracing-appender`'s `RollingFileAppender` (`Rotation::DAILY`, filename
//! prefix `eunomiasync`, suffix `log`). The appender is pointed at
//! `<data dir>/logs/` explicitly: Tauri's `app_log_dir()` would move the
//! logs to `~/Library/Logs/…` on macOS, away from the spec §5.5 layout —
//! a `config_tests.rs` absence pin keeps that API out of the codebase so
//! the regression cannot pass silently.
//!
//! Ordering (SYS-011): the config is read **before** logging is set up,
//! and [`init`] applies the `log_level` it finds (`info` by default,
//! SYS-012) as the subscriber's maximum level. Nothing before `init` can
//! log — which is why `config::load_or_create` returns its warnings as a
//! `Vec<String>`; the `setup` hook re-emits them through this module
//! after `init`, so they land in the file.
//!
//! Writer plumbing: the rolling appender is wrapped in
//! `tracing_appender::non_blocking`, which moves the actual writes onto a
//! dedicated worker thread (a GUI app must not stall the UI thread on
//! disk IO). The [`WorkerGuard`] returned alongside the writer stops and
//! flushes the worker when dropped, so it must outlive every event:
//! [`init`] returns it inside [`Logger`], which the `setup` hook keeps
//! alive as managed state. Honest boundary, recorded because the
//! tempting claim is wrong: tauri's `Builder::run` never returns — the
//! event loop ends in `std::process::exit` (tao `EventLoop::run`), which
//! skips destructors, so the managed `Logger` is *not* dropped at exit
//! and the guard's flush does not run there. What actually protects the
//! log is the worker's per-batch flushing (each formatted line is handed
//! to the OS as it is processed); only lines still queued in the final
//! moments of the process can be lost. The guard's job is the opposite
//! direction: keeping it alive for the whole run is what stops the
//! worker from shutting down early — dropping it right after `init`
//! (e.g. letting a local binding go out of scope in `setup`) would
//! silently stop the file log.
//!
//! Failure policy: logging is diagnostics, not data — a broken log file
//! (a read-only `logs/` directory, a full disk, an occupied path) must
//! not abort the launch the way a broken layout or config does
//! (DECISIONS.md D-027/D-028). [`init`] degrades instead: it installs a
//! stderr subscriber with the same level filter and emits one `error`
//! event describing the failure, so diagnostics still exist on the
//! terminal while the app runs on without a file log. Both subscribers
//! are built with `log_internal_errors(false)`: the fmt layer's default
//! on a writer error is an `eprintln!` of its own, which panics when
//! stderr is unusable — precisely the degraded state this path exists
//! for (a Windows GUI release build has no console attached, and the
//! workspace's `panic = "abort"` would turn that panic into a process
//! abort inside `setup`). Writer errors are therefore dropped silently:
//! a log line that cannot be written is lost, never fatal.
//!
//! # SEC-008: events are constructed here, and only here
//!
//! SEC-008 (P0): *instruction text and file content never go into logs;
//! paths do.* The enforcement is structural, not advisory:
//!
//! - Every `tracing` event of this crate is constructed **in this
//!   module**, from the typed helpers below — a `config_tests.rs` text
//!   pin keeps the event macros (`info!`, `warn!`, …, `#[instrument]`)
//!   out of every other source file, so a future task cannot
//!   interpolate content into a message or auto-capture it as an
//!   `#[instrument]` field without consciously editing that pin.
//! - The helpers' signatures are the vocabulary: paths, operation codes
//!   and numeric IDs. There is deliberately **no parameter** that can
//!   carry instruction text or file content — [`file_event`], the
//!   primitive the upcoming variant/sync/backup tasks will log through,
//!   takes a [`FileOp`] code, a path and an optional numeric ID, and
//!   that is the whole payload (pinned by the module tests: a file event
//!   carries the path and the ID while the instruction text of the very
//!   operation stays out of the file).
//! - The one intentional exception is [`config_load_warning`]: a config
//!   parse error may quote the offending bytes of `config.toml` itself, per
//!   D-028 ("the warning points at the exact bytes that failed") — the
//!   machine-level config is user-written diagnostics input, not
//!   instruction text or library file content, which is what SEC-008
//!   protects.
//!
//! Library crates (`eunomia-core`, `eunomia-store`, `eunomia-fs`) do not
//! log today. When the first of them adopts `tracing`, that task must
//! extend the SEC-008 discipline to it (they cannot import this module —
//! the dependency points the other way); the macros pin covering
//! `crates/**` forces that decision to be conscious.
//!
//! [`init`] is the startup seam; the behavior of the subscriber it
//! builds (file name and folder, per-level filtering, the SEC-008
//! payload contract) is unit-tested below over `tempfile` roots by
//! driving real events through the subscriber and reading the real log
//! file back. Two facts are consciously outside what a unit test can
//! prove: the midnight rollover (a date change mid-run) is
//! `tracing-appender`'s own `Rotation::DAILY` contract — the dated file
//! name proves the rotation wiring, the crossing itself rests on the
//! crate plus the manual smoke run — and the global-install path
//! ([`tracing::subscriber::set_global_default`]) is process-global, so
//! installing it inside the parallel test suite is not observable there;
//! the `setup` wiring is pinned textually in `config_tests.rs` instead.

use std::fmt;
use std::path::{Path, PathBuf};

use tracing::Subscriber;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

use crate::config::LogLevel;
use crate::errors::AppError;

/// The log file name before the date stamp (spec §5.5:
/// `logs/eunomiasync.<date>.log`). The suffix is [`LOG_FILE_SUFFIX`];
/// `tracing-appender` joins them as `<prefix>.<date>.<suffix>`.
const LOG_FILE_PREFIX: &str = "eunomiasync";

/// The log file name extension (spec §5.5).
const LOG_FILE_SUFFIX: &str = "log";

/// Why [`init`] could not build the file log: the path it could not
/// create or open, plus the OS cause. Kept as data (like
/// `DataLayoutError`/`ConfigError`) because the degradation path logs
/// it: the message must carry the full path so the user can tell *which*
/// directory is broken. This error never aborts the launch — see the
/// module docs.
#[derive(Debug)]
pub(crate) struct LogInitError {
    /// The `logs/` directory the appender was pointed at.
    path: PathBuf,
    /// The underlying `tracing-appender` initialization failure.
    source: tracing_appender::rolling::InitError,
}

impl fmt::Display for LogInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to initialize the file log in `{}`: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for LogInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// What [`init`] installed — the [`Logger`] value is the app's keep-alive
/// for the non-blocking writer: dropping it stops and flushes the writer
/// thread, so the `setup` hook stores it as managed state for the whole
/// app run. Note tauri exits the process without dropping managed state
/// (see the module docs): the flush-at-drop path exists for tests and
/// hypothetical embedders, not for the app's own exit — the worker's
/// per-batch flushing is what persists production log lines.
#[must_use = "dropping the Logger stops the non-blocking writer and with it the file log"]
#[derive(Debug)]
pub(crate) enum Logger {
    /// The file log is live; the guard keeps the writer thread running.
    File {
        /// Stopped and flushed when this value drops.
        _guard: WorkerGuard,
    },
    /// The file log could not be initialized; events go to stderr
    /// instead (module docs: logging is diagnostics, never a reason to
    /// abort the launch).
    Stderr,
}

/// Builds the daily-rotating file subscriber for `logs_dir` at `level` —
/// the testable seam behind [`init`]. The returned guard must outlive
/// every event the subscriber formats (see [`Logger`]).
///
/// # Errors
///
/// Fails with [`LogInitError`] when the appender cannot be created — the
/// directory cannot be made or the file cannot be opened. Callers
/// degrade (module docs); they never abort the launch over it.
fn build_file_subscriber(
    logs_dir: &Path,
    level: LogLevel,
) -> Result<(impl Subscriber + Send + Sync + 'static, WorkerGuard), LogInitError> {
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .build(logs_dir)
        .map_err(|source| LogInitError {
            path: logs_dir.to_path_buf(),
            source,
        })?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let subscriber = fmt_subscriber(writer, level);
    Ok((subscriber, guard))
}

/// The fmt subscriber both sinks share: the given `writer`, the level
/// filter from the config, and `log_internal_errors(false)`. One
/// function for all three construction sites (file, stderr degradation,
/// and the failing-writer test) so the test exercises the production
/// path rather than a lookalike — see the module docs for why the fmt
/// layer's writer-error fallback (an `eprintln!` that panics on an
/// unusable stderr) must stay off.
fn fmt_subscriber<W>(writer: W, level: LogLevel) -> impl Subscriber + Send + Sync + 'static
where
    W: for<'a> tracing_subscriber::fmt::writer::MakeWriter<'a> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_max_level(tracing_level(level))
        .log_internal_errors(false)
        .finish()
}

/// Builds the degradation subscriber from the module docs: same level
/// filter, stderr writer. Blocking writes are acceptable here — this
/// path exists precisely because the disk is unreliable, and stderr may
/// be a pipe nobody reads slowly.
fn build_stderr_subscriber(level: LogLevel) -> impl Subscriber + Send + Sync + 'static {
    fmt_subscriber(std::io::stderr, level)
}

/// Initializes process-wide logging: installs the daily-rotating file
/// subscriber for `logs_dir` at `level` (SYS-011: called after the config
/// is loaded, with the `log_level` it holds), or — when the appender
/// cannot be built — the stderr subscriber plus one `error` event naming
/// the failure (module docs: logging never aborts the launch). The
/// degradation event is deliberately `error`, not `warn`: it must stay
/// visible when the user configured `log_level = "error"`, where a warn
/// would be filtered out by the very filter it is reporting about.
///
/// The returned [`Logger`] owns the writer guard; the caller must keep
/// it for the app lifetime (`app.manage`). Installing the global default
/// is idempotent in the only caller that exists (a single `setup` hook);
/// a second install would keep the first subscriber, which is harmless
/// and never happens in this app.
pub(crate) fn init(logs_dir: &Path, level: LogLevel) -> Logger {
    match build_file_subscriber(logs_dir, level) {
        Ok((subscriber, guard)) => {
            let _ = tracing::subscriber::set_global_default(subscriber);
            Logger::File { _guard: guard }
        }
        Err(error) => {
            let subscriber = build_stderr_subscriber(level);
            let _ = tracing::subscriber::set_global_default(subscriber);
            // Emitted through the stderr subscriber just installed, so
            // the degradation is itself logged — this is the one event
            // constructed directly in `init`, on a path where no file
            // exists to write it to. `error`, not `warn`: at
            // `log_level = "error"` a warn would be filtered by the very
            // filter it reports about.
            tracing::error!(error = %error, "file log unavailable; logging to stderr");
            Logger::Stderr
        }
    }
}

/// Maps the config's `log_level` (SYS-012) onto the `tracing` level the
/// subscriber filters with.
fn tracing_level(level: LogLevel) -> tracing::Level {
    match level {
        LogLevel::Trace => tracing::Level::TRACE,
        LogLevel::Debug => tracing::Level::DEBUG,
        LogLevel::Info => tracing::Level::INFO,
        LogLevel::Warn => tracing::Level::WARN,
        LogLevel::Error => tracing::Level::ERROR,
    }
}

// ---- SEC-008 event facade ---------------------------------------------------
//
// Everything below constructs the crate's tracing events. Signatures are
// the SEC-008 vocabulary: paths, codes, IDs — no parameter can carry
// instruction text or file content (see the module docs and the
// `config_tests.rs` macros pin).

/// The startup completed: the data directory, layout, config and log are
/// in place. Paths are allowed in logs (SEC-008); the effective level is
/// worth one line so a log file self-documents how verbose it is.
pub(crate) fn startup(data_dir: &Path, level: LogLevel) {
    tracing::info!(
        data_dir = %data_dir.display(),
        log_level = %level,
        "startup complete"
    );
}

/// One startup warning from the config seam (SYS-012: unknown keys,
/// broken-file fallbacks). The message is built by `config::load_or_create`
/// and carries the config path and the parse reason — including, by the
/// D-028 decision, the offending bytes of `config.toml` itself; that is
/// the one deliberate exception to "no file content in logs" (module
/// docs). This helper exists for that seam **only**: it is the only
/// `&str`-carrying helper in the facade precisely because its input is
/// the config loader's own diagnostic text — any other warning kind
/// gets its own typed helper, never free text through here.
pub(crate) fn config_load_warning(message: &str) {
    tracing::warn!(target: "eunomia::config", "{message}");
}

/// The startup database open rejected the file as created by a newer
/// app version (spec DB-004, NFR-010): §10's `E_DB_TOO_NEW` row —
/// "The app does not change the database", user action "Update the
/// app". Logged at `error` on purpose: the launch aborts on this
/// verdict, so the line must stay visible at `log_level = "error"`
/// (the same reasoning as the degradation event in [`init`]).
///
/// The payload is the error's own §7.10 vocabulary — the code literal
/// and its interpolation params (the database path and both schema
/// versions), values already curated by the error contract; nothing
/// free-text enters the log (SEC-008).
pub(crate) fn db_too_new(error: &AppError) {
    tracing::error!(
        target: "eunomia::db",
        code = error.code.as_str(),
        params = ?error.params,
        "database newer than the app; the app does not change the database — update the app"
    );
}

/// The startup database migration failed and was rolled back (spec
/// UPD-004): the database is unchanged, the user must install the
/// previous app version, and the pre-migration copy of the unchanged
/// state is the recovery pointer. Logged at `error` on purpose: the
/// launch aborts on this verdict, so the line must stay visible at
/// `log_level = "error"` (the same reasoning as [`db_too_new`]).
///
/// The payload is the error's own §7.10 vocabulary — the code literal
/// and its interpolation params (the database path and the copy
/// path), values already curated by the error contract; nothing
/// free-text enters the log (SEC-008).
pub(crate) fn db_migration_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::db",
        code = error.code.as_str(),
        params = ?error.params,
        "database migration failed and was rolled back; the database is \
         unchanged — install the previous app version; a copy of the \
         unchanged state is in db-backups/"
    );
}

/// The startup integrity gate rejected the database file as damaged
/// (spec §6.3, §10): `PRAGMA quick_check` failed — the §10
/// `E_DB_CORRUPT` row, "Start in recovery mode" over the untouched
/// file. Logged at `error` on purpose: since T-024 the launch
/// deliberately *continues* from this verdict into recovery mode (the
/// screen replacing the app needs a live process), and the line is
/// what marks the session as a recovery one in log triage — a verdict
/// this serious must stay visible at `log_level = "error"`.
///
/// The payload is the error's own §7.10 vocabulary — the code literal
/// and its interpolation params (the database path and SQLite's own
/// `quick_check` evidence), values already curated by the error
/// contract; nothing free-text enters the log (SEC-008 — SQLite's
/// finding describes pages, never instruction text or file content,
/// and is machine-generated).
pub(crate) fn db_corrupt(error: &AppError) {
    tracing::error!(
        target: "eunomia::db",
        code = error.code.as_str(),
        params = ?error.params,
        "database failed the startup integrity check (quick_check); the \
         app does not change the database — start in recovery mode: pick \
         a copy from db-backups/ or an empty database"
    );
}

/// The startup daily database copy was written (spec §10 ERR-001):
/// the `db-backups/daily-YYYYMMDD.sqlite3` copy of this UTC day.
/// `info`: a routine reliability event, not a failure. A day whose
/// copy already exists gets **no** line — that is ERR-001's idempotent
/// no-op, and a line per restart would only repeat itself.
///
/// Boundary with the `action_log` journal (spec §7.7): the journal
/// service exists (T-026), but LOG-001..004's "backup creation" is
/// the BKP-001 family — copies of a project's *target file* taken by
/// sync runs (`backup_records`) — not this ERR-001 database copy, so
/// this file-log line remains the daily copy's whole journal. Whether
/// the daily copy also deserves a `diagnostic` journal entry is a
/// visibility decision (LOG-005) for the task that wires startup
/// events into the journal.
pub(crate) fn db_backup_daily(path: &Path) {
    tracing::info!(
        target: "eunomia::db",
        path = %path.display(),
        "daily database copy written (VACUUM INTO)"
    );
}

/// The startup daily database copy could not be written (spec §10
/// ERR-001): `warn`, not `error`, because the launch deliberately
/// continues — the daily copy is extra reliability over a database
/// that opened and migrated cleanly (the recorded policy,
/// DECISIONS.md D-046), and an `error` line would read as a failed
/// launch in log triage. The error's own `Display` carries the
/// offending path and OS cause; nothing free-text is added
/// (SEC-008 — the payload is a path-bearing storage error).
pub(crate) fn db_backup_daily_failed(error: &eunomia_store::storage::StorageError) {
    tracing::warn!(
        target: "eunomia::db",
        error = %error,
        "daily database copy failed; continuing without it"
    );
}

/// The LOG-006 retention cleanup removed rows (spec §7.7, §6.4 "Log
/// and runs"): journal entries beyond `log.max_entries` or
/// `log.max_days`, sync runs beyond the last 500 (their per-project
/// outcome rows with them). `info`: a routine housekeeping event —
/// the counts are the audit fact (how much history the limits
/// reclaimed). A cleanup that removed **nothing** gets no line: it
/// is the idempotent common case (every second startup, every daily
/// tick of a quiet app), and a line per day would only repeat
/// itself — the same no-op policy as [`db_backup_daily`]'s.
///
/// Boundary with the `action_log` journal, mirroring D-046's: the
/// cleanup itself writes no journal entry (LOG-001…004's vocabulary
/// has no housekeeping member), so this file-log line is its whole
/// journal. The payload is numeric only (SEC-008).
pub(crate) fn db_cleanup(counts: &eunomia_store::action_log::CleanupCounts) {
    tracing::info!(
        target: "eunomia::db",
        log_entries = counts.log_entries,
        sync_runs = counts.sync_runs,
        "retention cleanup removed rows beyond the limits"
    );
}

/// The LOG-006 retention cleanup failed: `warn`, not `error`, for
/// the same reason as [`db_backup_daily_failed`] — the launch and
/// the app continue to work with an over-long journal, and an
/// `error` line would read as a failed launch in log triage. The
/// task keeps its daily tick, so the next day retries on its own.
/// The payload is the error's own §7.10 vocabulary (code literal,
/// params, OS facts from `details` — for this producer the D-037
/// fallback shape carrying a class-only diagnostic: the failing key
/// and failure class, never the stored cell's bytes); nothing
/// free-text is added (SEC-008).
pub(crate) fn db_cleanup_failed(error: &AppError) {
    tracing::warn!(
        target: "eunomia::db",
        code = error.code.as_str(),
        params = ?error.params,
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "retention cleanup failed; continuing without it (retried at the next daily tick)"
    );
}

/// A database operation hit SQLite's busy/locked state, its
/// `busy_timeout` expired, and the `Db::run` executor is replaying
/// the whole operation in a fresh transaction (spec §10 `E_DB_LOCKED`
/// row: "up to 3 automatic retries"; the pause is §9.5's backoff
/// ladder, shared with the store's file-retry helper). `warn`, one
/// line per retry: the retry is invisible to every other layer (a
/// successful replay never becomes an error), so the executor is the
/// only possible producer — the deliberate one exception to "the
/// executor does not log" (D-037), recorded with the retry task. The
/// payload is numeric only (SEC-008): the 1-based retry number and
/// the pause that follows; no paths, no content — the executor has no
/// project context, and the final verdict (after the last retry
/// fails) is the producing command's own §10 error line.
pub(crate) fn db_busy_retry(attempt: usize, pause_ms: u64) {
    tracing::warn!(
        target: "eunomia::db",
        attempt,
        pause_ms,
        "database busy; retrying the operation after the pause"
    );
}

/// An `action_log` entry was written to the database journal (spec
/// §7.7), duplicated into the file log — the journal's diagnostic
/// half. The entry itself is the payload: the action code, the result
/// code, and the object/project/run IDs it carries, all already
/// curated by the store's typed [`LogEntry`] (SEC-008 — its `details`
/// is the typed enum that cannot hold instruction text, and this line
/// does not repeat even that: codes and IDs only).
///
/// **What is mirrored, and what is not.** The policy lives here, not
/// at the callers: they call this once per written entry, and this
/// function decides.
///
/// - `error` and `warn` entries mirror regardless of visibility — a
///   degradation or failure is what the file log exists for, and a
///   `diagnostic` one has no other surface (LOG-005: "only in the
///   file log and in diagnostics").
/// - An `info` entry mirrors in two cases: `result = error` — §10's
///   "every error is written to the log" must hold even when a
///   producer forgot the documented error-level pairing — and
///   `visibility = diagnostic`, whose only two surfaces are the file
///   log and diagnostics, so not mirroring would leave the database
///   journal as the entry's single home.
/// - The one shape that does **not** mirror: `info` + `user` + a
///   non-error result. That is the "Activity" window's own data (the
///   database journal already holds it), and mirroring every one
///   would double the file's volume with lines the per-event facade
///   functions (`file_event`, `db_backup_daily`, …) already write.
///
/// The first production caller is `settings_set` (T-030), which
/// journals its `settings.changed` entries inside the operation's
/// transaction and calls this once per entry — the info+user+ok shape
/// it writes today is the unmirrored one, so the mirror arms first
/// serve a future producer that writes warn/error/diagnostic
/// entries.
pub(crate) fn action_logged(entry: &eunomia_store::action_log::LogEntry<'_>) {
    // The payload block is repeated across the three level arms: the
    // tracing macro requires a constant level (its callsite caches
    // the level at compile time), and the house style is one explicit
    // macro per level — keep the three blocks identical.
    let entity = entry.entity;
    match mirror_level(entry) {
        None => {}
        Some(tracing::Level::ERROR) => tracing::error!(
            target: "eunomia::action_log",
            action = entry.action.as_str(),
            result = entry.result.as_str(),
            entity_type = entity.map(|e| e.kind.as_str()).unwrap_or(""),
            entity_id = entity.map(|e| e.id).unwrap_or(""),
            project_id = entry.project_id.unwrap_or(""),
            sync_run_id = entry.sync_run_id.unwrap_or(""),
            "action-log entry written (mirrored to the file log)"
        ),
        Some(tracing::Level::WARN) => tracing::warn!(
            target: "eunomia::action_log",
            action = entry.action.as_str(),
            result = entry.result.as_str(),
            entity_type = entity.map(|e| e.kind.as_str()).unwrap_or(""),
            entity_id = entity.map(|e| e.id).unwrap_or(""),
            project_id = entry.project_id.unwrap_or(""),
            sync_run_id = entry.sync_run_id.unwrap_or(""),
            "action-log entry written (mirrored to the file log)"
        ),
        // `mirror_level` only ever returns ERROR, WARN or INFO; this
        // arm is the INFO one (TRACE/DEBUG are never produced).
        Some(_) => tracing::info!(
            target: "eunomia::action_log",
            action = entry.action.as_str(),
            result = entry.result.as_str(),
            entity_type = entity.map(|e| e.kind.as_str()).unwrap_or(""),
            entity_id = entity.map(|e| e.id).unwrap_or(""),
            project_id = entry.project_id.unwrap_or(""),
            sync_run_id = entry.sync_run_id.unwrap_or(""),
            "action-log entry written (mirrored to the file log)"
        ),
    }
}

/// The tracing level an entry mirrors at, or `None` when it does not
/// — the policy half of [`action_logged`] (see there for the
/// reasoning; this is the decision table, split out for the test).
fn mirror_level(entry: &eunomia_store::action_log::LogEntry<'_>) -> Option<tracing::Level> {
    use eunomia_store::action_log::{Level, Outcome, Visibility};
    match (entry.level, entry.visibility, entry.result) {
        // Failures and degradations always mirror, at their own
        // level or better.
        (Level::Error, _, _) | (_, _, Outcome::Error) => Some(tracing::Level::ERROR),
        (Level::Warn, _, _) => Some(tracing::Level::WARN),
        // A diagnostic info entry: the file log is one of its only
        // two surfaces (LOG-005).
        (Level::Info, Visibility::Diagnostic, _) => Some(tracing::Level::INFO),
        // The Activity window's own data; not mirrored.
        (Level::Info, Visibility::User, _) => None,
    }
}

/// The `activity_list` command failed (T-027, §10's rule that every
/// error reaches the log with its code): the payload is the error's
/// own §7.10 vocabulary — the code literal, the interpolation params
/// (which name the rejected filter field/value/reason, or carry the
/// classified SQLite facts) and the OS facts from `details` when the
/// failure had any. `error` level: the Activity window shows the
/// failure to the user, and the log line is its durable half (the
/// `recovery_failed` rationale, applied to this command). A
/// *successful* list deliberately logs nothing — reading the journal
/// is not an event, and journaling it would feed the journal itself.
pub(crate) fn activity_list_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::action_log",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "the activity list query failed"
    );
}

/// The `settings_get` command failed (T-030, §10's rule that every
/// error reaches the log with its code): the payload is the error's
/// own §7.10 vocabulary, in the `activity_list_failed` shape. `error`
/// level: the Settings window cannot open on defaults it cannot trust,
/// and the log line is the failure's durable half. A *successful* read
/// deliberately logs nothing — reading settings is not an event (the
/// `activity_list_failed` rationale).
pub(crate) fn settings_get_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::settings",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "the settings read failed"
    );
}

/// The `settings_set` command failed (T-030, §10's every-error rule):
/// the payload is the error's own §7.10 vocabulary, in the
/// `activity_list_failed` shape. `error` level, and deliberately not
/// accompanied by any journal entry: the operation's transaction
/// rolled back (D-051), so no `settings.changed` entry exists for a
/// failed save — this line is the failure's only durable record, which
/// is exactly why the §10 rule matters here.
pub(crate) fn settings_set_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::settings",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "the settings save failed"
    );
}

/// The `app_info` command failed (T-031, §10's every-error rule): the
/// payload is the error's own §7.10 vocabulary, in the
/// `activity_list_failed` shape. The only reachable failure is the
/// missing `DataRoot` state (an uninitialized-app bug — the state is
/// managed before anything that could fail), so this line is a
/// wiring-canary record, not a user-facing situation the About window
/// acts on. Target `eunomia::system`, the §7.10 System-module group
/// the command belongs to (`data_folder_opened`/`_failed`).
pub(crate) fn app_info_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::system",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "the app_info command failed"
    );
}

/// A recovery operation preserved the damaged database (T-024, §10's
/// recovery screen): renamed — never deleted — from the database path
/// to `db-backups/pre-recovery-<UTC>.sqlite3`. `info` with both paths:
/// this is the audit line that answers "where did my old database
/// go?", and it must be reconstructable from the log alone. Emitted
/// before the replacement is published, so a crash between the two
/// still leaves the trail.
pub(crate) fn recovery_preserved(from: &Path, to: &Path) {
    tracing::info!(
        target: "eunomia::db",
        from = %from.display(),
        to = %to.display(),
        "recovery: the damaged database was preserved (renamed, not deleted)"
    );
}

/// A `db_recreate` completed: the damaged database is preserved under
/// its `pre-recovery-<UTC>` name and an empty database was created and
/// reopened at the database path (§7.10). `info`: the audit line of a
/// completed user-visible recovery action.
pub(crate) fn recovery_recreated(db_path: &Path) {
    tracing::info!(
        target: "eunomia::db",
        path = %db_path.display(),
        "recovery: started with an empty database (the damaged file is in db-backups/)"
    );
}

/// A `db_restore_backup` completed: the chosen copy replaced the
/// database and was reopened (§7.10). `info`, both paths — the source
/// copy stays where it was (never consumed), and the line is the audit
/// record of which copy became the database.
pub(crate) fn recovery_restored(backup: &Path, db_path: &Path) {
    tracing::info!(
        target: "eunomia::db",
        backup = %backup.display(),
        path = %db_path.display(),
        "recovery: the chosen copy replaced the database (the copy itself is untouched)"
    );
}

/// A recovery command failed (§10's rule that every error reaches the
/// log with its code): the payload is the error's own §7.10 vocabulary
/// — the code literal and its interpolation params (which name the
/// rejected backup, the failed step's path, or the gate evidence) —
/// exactly the `db_too_new`/`db_corrupt` shape, plus the OS facts from
/// `details` when the failure had any (§10 names the OS code a
/// first-class fact of every error line; the I/O-mapped errors carry
/// theirs only there). `error` level: the recovery screen shows the
/// failure to the user, and the log line is its durable half.
pub(crate) fn recovery_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::db",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "recovery operation failed"
    );
}

/// The data directory was opened in the OS file manager
/// (`app_open_data_folder`, §7.10). `info`: a user-visible action with
/// no failure state of its own — the audit fact is that the folder was
/// shown.
pub(crate) fn data_folder_opened(path: &Path) {
    tracing::info!(
        target: "eunomia::system",
        path = %path.display(),
        "data directory opened in the file manager"
    );
}

/// Opening the data directory in the OS file manager failed
/// (`app_open_data_folder`): the opener plugin's own failure, mapped
/// to its §7.10 wire form (the D-037 fallback code, the opener's text
/// in `details`). `error` level with the code literal and params —
/// the same §10 payload shape as [`recovery_failed`], OS facts
/// included when present.
pub(crate) fn data_folder_open_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::system",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "opening the data directory failed"
    );
}

/// The `dialog_pick_folder` command failed (T-073, §10's every-error
/// rule): the payload is the error's own §7.10 vocabulary, in the
/// `activity_list_failed` shape — a vanished pick carries
/// `E_PROJECT_UNAVAILABLE` with the picked path in `params` and the
/// OS refusal in `details`; a plugin failure carries the fallback
/// code with the plugin's text. `error` level: the folder picker is
/// the only road into the add-project flow, and the log line is the
/// failure's durable half. A *successful* pick deliberately logs
/// nothing: the session allowlist is in-memory bookkeeping, not an
/// event the journal or the file log records (reading the journal to
/// log the picker would be the `activity_list` self-feed, and the
/// future `project.added` entry covers the flow's real event).
pub(crate) fn dialog_pick_folder_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::system",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "the folder picker command failed"
    );
}

/// The `project_add_manual` command failed (T-074, §10's every-error
/// rule): the payload is the error's own §7.10 vocabulary, in the
/// `dialog_pick_folder_failed` shape — the §10 answers the flow can
/// reach (the `E_VALIDATION` SEC-003 rejection of a path from nowhere,
/// `E_NOT_A_REPO` for a folder outside any repository,
/// `E_PROJECT_UNAVAILABLE` for a refused probe, the `E_VALIDATION`
/// envelope of a §6.9 field, and the store fallbacks). `error` level:
/// the log line is the failure's durable half. The typed outcomes that
/// are NOT failures — the repo-root offer, the duplicate message, the
/// archive-restore offer — are success-shaped responses and
/// deliberately log nothing (nothing happened; the journal records
/// events, and a refusal wrote none), and a *successful* add is the
/// journal's own `project.added` entry, written inside the operation's
/// transaction (D-051) — never duplicated into the file log (the
/// info+user+ok unmirrored shape).
pub(crate) fn project_add_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::projects",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "adding a project by hand failed"
    );
}

/// The `exclusions_list` command failed (T-078, §10's every-error
/// rule): the `settings_get_failed` shape over the Projects target —
/// the reachable classes are the store fallback of a corrupt
/// exclusion cell (the `ProjectsCorrupt` diagnostic), the shared base
/// classifier's SQL classes (`ProjectsError::Sql` — busy exhaustion,
/// a corrupt database), and the honest no-database fallback. `error`
/// level per the facade contract; a successful read is never
/// journaled and logs nothing (reading is not an event — the
/// `activity_list` rationale).
pub(crate) fn exclusions_list_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::projects",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "listing the excluded paths failed"
    );
}

/// The `exclusion_remove` command failed (T-078, §10's every-error
/// rule): the `project_add_failed` shape — the reachable §10 answers
/// are the typed `E_VALIDATION` refusal of an id no row matches (a
/// stale screen's row), the shared base classifier's SQL classes
/// (`ProjectsError::Sql` through the list/delete reads,
/// `StorageError::Sql` through the journal insert — busy exhaustion,
/// a corrupt database, a constraint abort), and the store fallback of
/// a corrupt exclusion cell (`ProjectsCorrupt`). A successful removal
/// is the journal's own `project.changed` entry, written inside the
/// operation's transaction (D-051) — never duplicated into the file
/// log (the info+user+ok unmirrored shape).
pub(crate) fn exclusion_remove_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::projects",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "removing a path exclusion failed"
    );
}

/// The `projects_reorder` command failed (T-080, §10's every-error
/// rule): the `exclusion_remove_failed` shape — the reachable §10
/// answers are the typed `E_VALIDATION` refusal of a list that does
/// not name the active set exactly once each (a stale screen's order),
/// the shared base classifier's SQL classes (`ProjectsError::Sql`
/// through the ordered read and the renumbering, `StorageError::Sql`
/// through the journal insert — busy exhaustion, a corrupt database, a
/// constraint abort), the store fallback of a corrupt projects cell
/// (`ProjectsCorrupt`), and the honest no-database fallback. A
/// successful reorder is the journal's own `project.changed` entry,
/// written inside the operation's transaction (D-051) — never
/// duplicated into the file log (the info+user+ok unmirrored shape);
/// the verified no-op (a re-sent unchanged order) writes nothing and
/// logs nothing.
pub(crate) fn projects_reorder_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::projects",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "saving the new project order failed"
    );
}

/// The `projects_list` command failed (T-244, §10's every-error rule,
/// closing the gap D-096 recorded for T-072): the
/// `projects_reorder_failed` shape — the reachable §10 answers are the
/// honest `NoDatabase` fallback (recovery mode), the shared base
/// classifier's SQL classes (`ProjectsError::Sql` through the ordered
/// read — busy exhaustion, a corrupt database), the store fallback of
/// a corrupt projects cell (`ProjectsCorrupt`), and the blocking-pool
/// join failure of the availability probes (`OperationPanicked`/
/// `OperationCancelled`/`OperationFailed`). `error` level per the
/// facade contract; a successful read is never journaled and logs
/// nothing (reading is not an event — the `activity_list` rationale).
pub(crate) fn projects_list_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::projects",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "listing the projects failed"
    );
}

/// The availability recheck failed (T-079, D-09/DB-032, §10's
/// every-error rule): the payload is the error's own §7.10 vocabulary,
/// in the `activity_list_failed` shape. Logged by the fire-and-forget
/// triggers (`spawn_recheck`) — the startup pass and the focus pass
/// have no caller to return to; the sync pipeline, when it adopts
/// `recheck_and_emit`, branches on the returned error itself and this
/// line is its durable half. The reachable answers are the honest
/// `NoDatabase` fallback (recovery mode) and the classified store
/// errors of the project-list read. `error` level per the facade
/// contract; a *successful* check logs nothing — probing folders that
/// mostly answer "fine" is not an event, and an unchanged diff emits
/// nothing either (the diff contract in `availability.rs`).
pub(crate) fn availability_check_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::availability",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "the project-availability recheck failed"
    );
}

/// Emitting the `state://project-availability` event failed (T-079):
/// the recheck itself succeeded — its diff was computed and is
/// returned to the caller — but the UI update did not leave the
/// process. `warn`, not `error`: the next check point re-detects the
/// same diff (the cache is deliberately left stale on a failed emit —
/// the at-least-once contract in `availability.rs`) and retries, so
/// this line marks a missed update, not lost state. The tauri error
/// itself is the whole payload — it carries no user paths.
pub(crate) fn availability_emit_failed(error: &tauri::Error) {
    tracing::warn!(
        target: "eunomia::availability",
        error = %error,
        "emitting the project-availability event failed; the next check point retries"
    );
}

/// A file operation on a project's library file — the SEC-008 core
/// event. The upcoming variant, sync and backup tasks log through this
/// helper: it takes the operation code, the file's path and the numeric
/// ID of the entity involved (variant ID, project ID — whatever the
/// operation is about), and *nothing else*. There is no content
/// parameter, so the instruction text being saved or synced cannot leak
/// into the log — the module tests pin exactly that contract.
///
/// No production caller exists yet (the variant/sync tasks are upcoming);
/// the SEC-008 test suite is the consumer that keeps this signature
/// honest until they land. Remove the `allow` when the first task calls
/// this.
// SEC-008: no content parameter, by design — see `file_event` docs.
#[allow(dead_code)]
pub(crate) fn file_event(op: FileOp, path: &Path, entity_id: Option<u64>) {
    // `entity_id` is emitted as a string field with an empty value when
    // absent (`entity_id=""`), keeping the line shape stable for the future
    // "Activity" window tooling (§7.7) without a second message format.
    tracing::info!(
        target: "eunomia::files",
        op = op.as_str(),
        path = %path.display(),
        entity_id = entity_id.map(|id| id.to_string()).unwrap_or_default(),
        "file operation"
    );
}

/// The operation codes [`file_event`] logs — a closed vocabulary (an
/// operation code in the log is always one of these words, never free
/// text). Extended consciously by the task that adds the next file
/// operation; SEC-006 (backup paths from IDs and slugs, not user
/// strings) keeps these operations' *paths* safe too.
///
/// Same status as [`file_event`]: tested today, first produced by the
/// upcoming variant/sync/backup tasks.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileOp {
    /// A variant's build was saved into the project's target file.
    VariantSaved,
    /// A previous variant build was restored into the target file.
    VariantRestored,
    /// A sync run wrote (or deliberately left) the target file.
    Synced,
    /// A copy of a target file was taken before a write.
    BackedUp,
}

impl FileOp {
    /// The word the log line carries (`op=…`).
    #[allow(dead_code)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FileOp::VariantSaved => "variant_saved",
            FileOp::VariantRestored => "variant_restored",
            FileOp::Synced => "synced",
            FileOp::BackedUp => "backed_up",
        }
    }
}

/// One step of the single-instance window restore (SYS-006) failed; the
/// remaining steps still run, so each failure is logged as it happens
/// (`single_instance.rs` documents the no-abort policy).
pub(crate) fn single_instance_step_failed(step: RestoreStep, error: &tauri::Error) {
    tracing::warn!(
        target: "eunomia::single_instance",
        step = step.as_str(),
        error = %error,
        "window restore step failed"
    );
}

/// A second launch arrived but no `main` window exists to bring forward
/// (shutdown race, or a future windowless instance) — SYS-006 has
/// nothing to do.
pub(crate) fn single_instance_no_main_window() {
    tracing::warn!(
        target: "eunomia::single_instance",
        "second launch handed off, but no `main` window exists — nothing to focus"
    );
}

/// The single-instance handoff finished; `failed_steps` counts the
/// restore steps that errored (0 — the common case — logs at `info`).
pub(crate) fn single_instance_handed_off(failed_steps: usize) {
    if failed_steps == 0 {
        tracing::info!(
            target: "eunomia::single_instance",
            "second launch handed off; `main` window focused"
        );
    } else {
        tracing::warn!(
            target: "eunomia::single_instance",
            failed_steps,
            "second launch handed off; `main` window restore was incomplete"
        );
    }
}

/// The steps of the single-instance restore sequence (show → unminimize
/// → focus), for [`single_instance_step_failed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestoreStep {
    /// `WebviewWindow::show`.
    Show,
    /// `WebviewWindow::unminimize`.
    Unminimize,
    /// `WebviewWindow::set_focus`.
    Focus,
}

impl RestoreStep {
    /// The word the log line carries (`step=…`).
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            RestoreStep::Show => "show",
            RestoreStep::Unminimize => "unminimize",
            RestoreStep::Focus => "focus",
        }
    }
}

#[cfg(test)]
mod tests {
    //! Behavior tests of the subscriber seam: a real subscriber is built
    //! over a `tempfile` directory, real events are driven through it
    //! with `tracing::subscriber::with_default` (the thread-local default
    //! — the global default cannot be installed from the parallel test
    //! suite), the writer guard is dropped to flush, and the actual log
    //! file is read back. What is asserted is what any reader of the log
    //! sees: the file name and folder, which events survived the level
    //! filter, and which values the events carry (the SEC-008 contract).
    //!
    //! Expected names are pinned as literals, never derived from
    //! [`super::LOG_FILE_PREFIX`]/[`super::LOG_FILE_SUFFIX`] — a mutated
    //! constant must fail its own test (the same trick as the
    //! `data_dirs.rs` and `config.rs` suites).

    use super::*;
    use std::error::Error as _;
    use std::fs;

    /// A `logs/` directory under a temp root, as the layout step leaves
    /// it. The `TempDir` is returned alongside so it stays alive.
    fn logs_dir() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("failed to create a temp directory");
        let root = temp.path().join("logs");
        fs::create_dir(&root).expect("creating the temp logs directory must work");
        (temp, root)
    }

    /// The log files currently in `dir` — every entry whose name matches
    /// the spec §5.5 shape `eunomiasync.<date>.log` with a `YYYY-MM-DD`
    /// date stamp. The date shape is checked character by character
    /// (digits, dashes at 4 and 7) rather than parsed as a real calendar
    /// date: the requirement is the file *name* format, and the appender
    /// guarantees a real date.
    fn dated_log_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in fs::read_dir(dir).expect("the logs directory must be readable") {
            let path = entry.expect("every logs entry must be readable").path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let rest = name
                .strip_prefix("eunomiasync.")
                .and_then(|rest| rest.strip_suffix(".log"));
            let Some(date) = rest else {
                continue;
            };
            let bytes = date.as_bytes();
            let shaped = bytes.len() == 10
                && bytes[4] == b'-'
                && bytes[7] == b'-'
                && bytes
                    .iter()
                    .enumerate()
                    .all(|(i, b)| b.is_ascii_digit() || ((i == 4 || i == 7) && *b == b'-'));
            if shaped {
                files.push(path);
            }
        }
        files.sort();
        files
    }

    /// Reads the single log file the tests expect to exist, asserting
    /// there is exactly one dated file (a second one would mean the
    /// rotation or naming regressed into something the tests did not
    /// drive).
    fn read_log_file(dir: &Path) -> String {
        let files = dated_log_files(dir);
        assert_eq!(
            files.len(),
            1,
            "expected exactly one dated log file, found: {files:?}"
        );
        fs::read_to_string(&files[0]).expect("the log file must be readable")
    }

    /// Emits one event per tracing level, each with a message unique to
    /// its level, plus a path field on one of them — the payload shape
    /// the filtering assertions key off. Lives here (inside this
    /// module's file) so the macros-only-in-this-module pin holds.
    fn emit_one_event_per_level() {
        tracing::error!("probe-error");
        tracing::warn!(path = "/probe/warn", "probe-warn");
        tracing::info!("probe-info");
        tracing::debug!("probe-debug");
        tracing::trace!("probe-trace");
    }

    #[test]
    fn writes_the_log_into_the_logs_directory_with_the_dated_name() {
        // Arrange: a real subscriber over a temp logs directory at the
        // default level.
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");

        // Act: one event through the subscriber, then flush by dropping
        // the guard (the same flush the app exit performs).
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("startup probe event");
        });
        drop(guard);

        // Assert: exactly one file, in the given directory (read_log_file
        // reads from `root`, which pins the folder — not some working
        // directory or an `app_log_dir()` default), named
        // `eunomiasync.<date>.log` (spec §5.5), carrying the event.
        let log = read_log_file(&root);
        assert!(
            log.contains("startup probe event"),
            "the log file must carry the emitted event, got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn each_configured_level_admits_exactly_its_events() {
        // Arrange & Act & Assert: for every SYS-012 spelling, a fresh
        // subscriber over a fresh directory receives one event per
        // tracing level; exactly the events at or above the configured
        // level may survive. This is the "log_level from config.toml is
        // applied to the log" criterion, driven end to end.
        //
        // The cases must stay ordered most-verbose-first: tracing caches
        // callsite interest across dispatcher swaps, and running a
        // strict subscriber first could cache a lower interest that a
        // later, more verbose subscriber in the same process then
        // inherits. Most-verbose-first keeps every later case's probes
        // enabled; the assertion still proves the filter (the strict
        // levels drop their probes).
        let cases = [
            (
                LogLevel::Trace,
                vec![
                    "probe-error",
                    "probe-warn",
                    "probe-info",
                    "probe-debug",
                    "probe-trace",
                ],
            ),
            (
                LogLevel::Debug,
                vec!["probe-error", "probe-warn", "probe-info", "probe-debug"],
            ),
            (
                LogLevel::Info,
                vec!["probe-error", "probe-warn", "probe-info"],
            ),
            (LogLevel::Warn, vec!["probe-error", "probe-warn"]),
            (LogLevel::Error, vec!["probe-error"]),
        ];
        for (level, expected_visible) in cases {
            let (temp, root) = logs_dir();
            let (subscriber, guard) = build_file_subscriber(&root, level)
                .expect("building the subscriber must work for every level");
            tracing::subscriber::with_default(subscriber, emit_one_event_per_level);
            drop(guard);

            let log = read_log_file(&root);
            for probe in [
                "probe-error",
                "probe-warn",
                "probe-info",
                "probe-debug",
                "probe-trace",
            ] {
                let should_appear = expected_visible.contains(&probe);
                assert_eq!(
                    log.contains(probe),
                    should_appear,
                    "at log_level={level:?}: `{probe}` must {}appear in the log",
                    if should_appear { "" } else { "not " }
                );
            }
            drop(temp);
        }
    }

    #[test]
    fn file_events_carry_paths_and_ids_but_never_content() {
        // Arrange: everything a real variant-save/sync implementation
        // holds: the instruction text (a realistic AGENTS.md body with
        // distinctive markers), the project's target-file path, and the
        // variant's database ID. The SEC-008 criterion, adapted to the
        // code that exists today: the features are not implemented yet,
        // so the test drives the facade exactly the way those tasks will
        // (the content stays in their local scope; only the path, the
        // operation code and the ID cross into the logging API — there
        // is no parameter that could carry the content).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");
        let instruction_text = "\
            # Agent Guidelines\n\
            SECRET-INSTRUCTION-MARKER: always answer in the user's language.\n\
            Follow every step of SECRET-RECIPE-4f2a before touching the build.\n\
            The deployment token is sk-live-9d1e2f3a4b5c.\n\
            # Section\nSECRET-BODY-77c0 more instructions here.\n";
        let target_file = temp
            .path()
            .join("projects")
            .join("eunomia")
            .join("AGENTS.md");

        // Act: a variant save and a sync run, logged the way the tasks
        // will log them — the text exists in scope the whole time and
        // has nowhere to go.
        tracing::subscriber::with_default(subscriber, || {
            file_event(FileOp::VariantSaved, &target_file, Some(42));
            file_event(FileOp::Synced, &target_file, None);
            let _ = instruction_text;
        });
        drop(guard);

        // Assert: the path, the ID and both operation codes are in the
        // log (SEC-008: "paths do"), and no marker of the instruction
        // text is (SEC-008: "instruction text and file content never go
        // into logs"). The ID assertion pins the `entity_id="42"` field,
        // not the bare digits: the fmt line carries a wall-clock
        // timestamp whose seconds routinely contain "42" on their own
        // (string-valued fields print quoted — stable fmt-layer
        // behavior), and the path assertion pins the full field value
        // so a truncated path still fails.
        let log = read_log_file(&root);
        assert!(
            log.contains(&format!("path={}", target_file.display())),
            "the file event must carry the full target path, got: {log}"
        );
        assert!(
            log.contains("entity_id=\"42\""),
            "the file event must carry the entity ID field, got: {log}"
        );
        assert!(
            log.contains("op=\"variant_saved\"") && log.contains("op=\"synced\""),
            "the file event must carry the operation codes, got: {log}"
        );
        for marker in [
            "SECRET-INSTRUCTION-MARKER",
            "SECRET-RECIPE-4f2a",
            "sk-live-9d1e2f3a4b5c",
            "SECRET-BODY-77c0",
            "always answer in the user's language",
        ] {
            assert!(
                !log.contains(marker),
                "SEC-008 violation: instruction text leaked into the log \
                 (`{marker}`), got: {log}"
            );
        }
    }

    #[test]
    fn config_warnings_land_in_the_file_log() {
        // Arrange: the warnings the config seam returns (SYS-012) are
        // re-emitted after `init` — they must survive as warn events in
        // the file at the default level, carrying the config path.
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");
        let config_path = temp.path().join("config.toml");

        // Act: the startup wiring's calls, in its order.
        tracing::subscriber::with_default(subscriber, || {
            config_load_warning(&format!(
                "config: ignoring unknown key `verbosity` in {}",
                config_path.display()
            ));
        });
        drop(guard);

        // Assert
        let log = read_log_file(&root);
        assert!(
            log.contains("unknown key `verbosity`") && log.contains("config.toml"),
            "the config warning must land in the file log with its path, \
             got: {log}"
        );
    }

    #[test]
    fn a_too_new_database_logs_its_code_path_and_versions() {
        // Arrange: the §10 shape of the rejection — the `E_DB_TOO_NEW`
        // code with the database path and both schema versions as its
        // interpolation params (the payload `DbInitError::app_error`
        // builds; the param values themselves are pinned by the db
        // suite, this test pins what the facade does with them).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");
        let mut error = AppError::new(crate::errors::ErrorCode::DbTooNew);
        error.params.insert(
            "path".to_owned(),
            temp.path()
                .join("db")
                .join("eunomiasync.sqlite3")
                .display()
                .to_string(),
        );
        error
            .params
            .insert("database_schema_version".to_owned(), "2".to_owned());
        error
            .params
            .insert("app_schema_version".to_owned(), "1".to_owned());

        // Act: the `setup` hook's call for this §10 situation.
        tracing::subscriber::with_default(subscriber, || {
            db_too_new(&error);
        });
        drop(guard);

        // Assert: the line carries the code literal (§10: "every
        // error is written to the log with its code … path …"), the
        // path and both versions from the params, and the §10 user
        // action.
        let log = read_log_file(&root);
        assert!(
            log.contains("code=\"E_DB_TOO_NEW\""),
            "the log line must carry the §10 code literal, got: {log}"
        );
        assert!(
            log.contains("eunomiasync.sqlite3") && log.contains("database_schema_version"),
            "the log line must carry the path and the schema versions, \
             got: {log}"
        );
        assert!(
            log.contains("update the app"),
            "the log line must name the §10 user action, got: {log}"
        );

        // And the verdict survives the strictest level: the launch
        // aborts on it, so `log_level = "error"` must not filter it.
        let (temp_strict, root_strict) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root_strict, LogLevel::Error)
            .expect("building the subscriber must work");
        tracing::subscriber::with_default(subscriber, || {
            db_too_new(&error);
        });
        drop(guard);
        assert!(
            read_log_file(&root_strict).contains("code=\"E_DB_TOO_NEW\""),
            "the abort verdict must survive log_level = \"error\""
        );
        drop(temp_strict);
    }

    #[test]
    fn a_rolled_back_migration_logs_its_code_paths_and_copy() {
        // Arrange: the §7.10 shape of the UPD-004 rejection — the
        // `E_UPDATE_FAILED` code with the migration-specific message
        // key and the database and copy paths as its interpolation
        // params (the payload `DbInitError::app_error` builds; the
        // param values themselves are pinned by the db suite, this
        // test pins what the facade does with them).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");
        let mut error = AppError::new(crate::errors::ErrorCode::UpdateFailed);
        error.message_key = "errors.update_migration_failed".to_owned();
        error.params.insert(
            "path".to_owned(),
            temp.path()
                .join("db")
                .join("eunomiasync.sqlite3")
                .display()
                .to_string(),
        );
        error.params.insert(
            "backup_path".to_owned(),
            temp.path()
                .join("db-backups")
                .join("pre-migration-v1-20261118T101500123Z.sqlite3")
                .display()
                .to_string(),
        );

        // Act: the `setup` hook's call for this situation.
        tracing::subscriber::with_default(subscriber, || {
            db_migration_failed(&error);
        });
        drop(guard);

        // Assert: the line carries the code literal (§10: "every error
        // is written to the log with its code … path …"), both paths
        // from the params, and the UPD-004 user action.
        let log = read_log_file(&root);
        assert!(
            log.contains("code=\"E_UPDATE_FAILED\""),
            "the log line must carry the code literal, got: {log}"
        );
        assert!(
            log.contains("eunomiasync.sqlite3")
                && log.contains("pre-migration-v1-")
                && log.contains("backup_path"),
            "the log line must carry the database and copy paths, got: {log}"
        );
        assert!(
            log.contains("install the previous app version"),
            "the log line must name the UPD-004 user action, got: {log}"
        );

        // And the verdict survives the strictest level: the launch
        // aborts on it, so `log_level = "error"` must not filter it.
        let (temp_strict, root_strict) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root_strict, LogLevel::Error)
            .expect("building the subscriber must work");
        tracing::subscriber::with_default(subscriber, || {
            db_migration_failed(&error);
        });
        drop(guard);
        assert!(
            read_log_file(&root_strict).contains("code=\"E_UPDATE_FAILED\""),
            "the abort verdict must survive log_level = \"error\""
        );
        drop(temp_strict);
    }

    #[test]
    fn a_corrupt_database_logs_its_code_path_and_evidence() {
        // Arrange: the §10 shape of the rejection — the `E_DB_CORRUPT`
        // code with the database path and SQLite's own quick_check
        // evidence as its interpolation params (the payload
        // `DbInitError::app_error` builds; the param values themselves
        // are pinned by the db suite, this test pins what the facade
        // does with them).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");
        let mut error = AppError::new(crate::errors::ErrorCode::DbCorrupt);
        error.params.insert(
            "path".to_owned(),
            temp.path()
                .join("db")
                .join("eunomiasync.sqlite3")
                .display()
                .to_string(),
        );
        error.params.insert(
            "detail".to_owned(),
            "*** in database main ***\nTree 2 page 2: invalid page number 3".to_owned(),
        );

        // Act: the `setup` hook's call for this §10 situation.
        tracing::subscriber::with_default(subscriber, || {
            db_corrupt(&error);
        });
        drop(guard);

        // Assert: the line carries the code literal (§10: "every error
        // is written to the log with its code … path …"), the path and
        // SQLite's evidence from the params, and the §10 recovery
        // action.
        let log = read_log_file(&root);
        assert!(
            log.contains("code=\"E_DB_CORRUPT\""),
            "the log line must carry the §10 code literal, got: {log}"
        );
        assert!(
            log.contains("eunomiasync.sqlite3") && log.contains("invalid page number"),
            "the log line must carry the path and the quick_check evidence, \
             got: {log}"
        );
        assert!(
            log.contains("recovery mode"),
            "the log line must name the §10 effect, got: {log}"
        );

        // And the verdict survives the strictest level: the launch
        // aborts on it, so `log_level = "error"` must not filter it.
        let (temp_strict, root_strict) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root_strict, LogLevel::Error)
            .expect("building the subscriber must work");
        tracing::subscriber::with_default(subscriber, || {
            db_corrupt(&error);
        });
        drop(guard);
        assert!(
            read_log_file(&root_strict).contains("code=\"E_DB_CORRUPT\""),
            "the abort verdict must survive log_level = \"error\""
        );
        drop(temp_strict);
    }

    #[test]
    fn a_busy_retry_logs_its_attempt_and_pause_and_nothing_else() {
        // Arrange & Act: the executor's retry call, driven through a
        // real subscriber the way `Db::run` emits it — once per retry,
        // with the 1-based retry number and the §9.5 pause it is about
        // to take.
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");

        tracing::subscriber::with_default(subscriber, || {
            db_busy_retry(1, 100);
            db_busy_retry(3, 900);
        });
        drop(guard);

        // Assert: a warn line per retry carrying both numeric fields —
        // the whole payload is the SEC-008-safe vocabulary (no paths,
        // no content; the executor has no project context to
        // interpolate).
        let log = read_log_file(&root);
        assert!(
            log.contains("WARN"),
            "a busy retry must log at warn, got: {log}"
        );
        assert!(
            log.contains("attempt=1") && log.contains("pause_ms=100"),
            "the first retry line must carry the 1-based attempt and \
             its §9.5 pause, got: {log}"
        );
        assert!(
            log.contains("attempt=3") && log.contains("pause_ms=900"),
            "the third retry line must carry its attempt and pause, got: {log}"
        );
        assert!(
            log.contains("database busy"),
            "the line must name the situation, got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn action_log_entries_mirror_by_level_result_and_visibility() {
        // Arrange: five journal entries the way a future command task
        // builds them — a routine user action (info, not mirrored), a
        // diagnostic info entry (mirrored: the file log is one of its
        // only two surfaces), a diagnostic warn, a failed operation's
        // fail-then-log error entry, and the defensive case: a failure
        // whose producer forgot the error-level pairing (info + user +
        // result=error — still mirrored, at ERROR). All beside a
        // realistic instruction text that stays in scope throughout.
        // The error entry carries a typed details payload too; the
        // mirrored line must not repeat even that (codes and IDs only).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");
        let instruction_text = "\
            SECRET-INSTRUCTION-MARKER: always answer in the user's language.\n\
            Follow every step of SECRET-RECIPE-4f2a before touching the build.\n\
            The deployment token is sk-live-9d1e2f3a4b5c.\n";
        let entity_id = "0193a2aa-6c9e-7c8e-9c6d-1a2b3c4d5e6f";
        let project_id = "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f";
        let sync_run_id = "0193a2cc-2222-7333-9c6d-1a2b3c4d5e6f";

        // Act: the journal-write call a command task makes once per
        // entry; the facade decides what reaches the file.
        tracing::subscriber::with_default(subscriber, || {
            use eunomia_store::action_log::{
                Action, Details, Entity, EntityType, FailureCode, Level, LogEntry, Outcome,
                Visibility,
            };
            let info_user = LogEntry::new(
                Action::SettingsChanged,
                Level::Info,
                Visibility::User,
                Outcome::Ok,
            );
            let info_diagnostic = LogEntry::new(
                Action::ProjectArchived,
                Level::Info,
                Visibility::Diagnostic,
                Outcome::Ok,
            );
            let mut warn_entry = LogEntry::new(
                Action::SyncProjectResult,
                Level::Warn,
                Visibility::Diagnostic,
                Outcome::Skipped,
            );
            warn_entry.entity = Some(Entity {
                kind: EntityType::Project,
                id: project_id,
            });
            warn_entry.sync_run_id = Some(sync_run_id);
            let mut error_entry = LogEntry::new(
                Action::VariantChanged,
                Level::Error,
                Visibility::User,
                Outcome::Error,
            );
            error_entry.entity = Some(Entity {
                kind: EntityType::Variant,
                id: entity_id,
            });
            error_entry.project_id = Some(project_id);
            error_entry.details = Some(Details::Failure {
                code: FailureCode::new("E_CONTENT_RESERVED").unwrap(),
                path: Some(temp.path().join("repo").join("AGENTS.md")),
            });
            // The forgotten-pairing case: a failure written at info.
            let info_failure = LogEntry::new(
                Action::HeadingChanged,
                Level::Info,
                Visibility::User,
                Outcome::Error,
            );
            action_logged(&info_user);
            action_logged(&info_diagnostic);
            action_logged(&warn_entry);
            action_logged(&error_entry);
            action_logged(&info_failure);
            let _ = instruction_text;
        });
        drop(guard);

        // Assert: everything except the info+user+ok entry is
        // mirrored, each at the right level (the level word appears
        // on the fmt line), with codes and IDs only (SEC-008's
        // allowed vocabulary), and no marker of the instruction
        // text — or of the details payload — leaked. The entity_id
        // assertion pins the quoted field value (the fmt layer quotes
        // string fields), not the bare digits.
        let log = read_log_file(&root);
        assert!(
            log.contains("WARN") && log.contains("ERROR") && log.contains("INFO"),
            "warn, error and diagnostic-info entries are mirrored, got: {log}"
        );
        assert!(
            log.contains("action=\"sync.project_result\"") && log.contains("result=\"skipped\""),
            "the warn mirror must carry the action and result codes, got: {log}"
        );
        assert!(
            log.contains(&format!("sync_run_id=\"{sync_run_id}\"")),
            "the warn mirror must carry the sync-run ID, got: {log}"
        );
        assert!(
            log.contains("action=\"variant.changed\"") && log.contains("result=\"error\""),
            "the error mirror must carry the action and result codes, got: {log}"
        );
        assert!(
            log.contains(&format!("entity_id=\"{entity_id}\"")),
            "the error mirror must carry the entity ID, got: {log}"
        );
        assert!(
            log.contains("entity_type=\"variant\""),
            "the error mirror must carry the entity type code, got: {log}"
        );
        assert!(
            log.contains(&format!("project_id=\"{project_id}\"")),
            "the error mirror must carry the project ID, got: {log}"
        );
        assert!(
            log.contains("action=\"project.archived\""),
            "a diagnostic info entry must be mirrored (LOG-005: the file \
             log is one of its only two surfaces), got: {log}"
        );
        assert!(
            log.contains("action=\"heading.changed\""),
            "an info entry with result=error must still mirror (the §10 \
             pairing, defended), got: {log}"
        );
        assert!(
            !log.contains("action=\"settings.changed\""),
            "an info+user+ok entry must not be mirrored into the file log, got: {log}"
        );
        for marker in [
            "SECRET-INSTRUCTION-MARKER",
            "SECRET-RECIPE-4f2a",
            "sk-live-9d1e2f3a4b5c",
            "always answer in the user's language",
        ] {
            assert!(
                !log.contains(marker),
                "SEC-008 violation: instruction text leaked into the mirrored \
                 line (`{marker}`), got: {log}"
            );
        }
        drop(temp);
    }

    #[test]
    fn the_mirror_policy_table_is_the_documented_one() {
        // The decision table, driven directly: every (level,
        // visibility, result) combination maps to the documented
        // tracing level or to no line at all.
        use eunomia_store::action_log::{Action, Level, LogEntry, Outcome, Visibility};
        let cases: [(Level, Visibility, Outcome, Option<tracing::Level>); 12] = [
            (
                Level::Error,
                Visibility::User,
                Outcome::Ok,
                Some(tracing::Level::ERROR),
            ),
            (
                Level::Error,
                Visibility::Diagnostic,
                Outcome::Error,
                Some(tracing::Level::ERROR),
            ),
            (
                Level::Error,
                Visibility::User,
                Outcome::Error,
                Some(tracing::Level::ERROR),
            ),
            (
                Level::Warn,
                Visibility::User,
                Outcome::Ok,
                Some(tracing::Level::WARN),
            ),
            (
                Level::Warn,
                Visibility::Diagnostic,
                Outcome::Skipped,
                Some(tracing::Level::WARN),
            ),
            // §10's pairing, defended: a failure written at warn or
            // info still mirrors at ERROR.
            (
                Level::Warn,
                Visibility::User,
                Outcome::Error,
                Some(tracing::Level::ERROR),
            ),
            (
                Level::Info,
                Visibility::User,
                Outcome::Error,
                Some(tracing::Level::ERROR),
            ),
            (
                Level::Info,
                Visibility::Diagnostic,
                Outcome::Error,
                Some(tracing::Level::ERROR),
            ),
            (
                Level::Info,
                Visibility::Diagnostic,
                Outcome::Ok,
                Some(tracing::Level::INFO),
            ),
            (
                Level::Info,
                Visibility::Diagnostic,
                Outcome::Skipped,
                Some(tracing::Level::INFO),
            ),
            // The Activity window's own data.
            (Level::Info, Visibility::User, Outcome::Ok, None),
            (Level::Info, Visibility::User, Outcome::Skipped, None),
        ];
        for (level, visibility, result, expected) in cases {
            let entry = LogEntry::new(Action::ErrorReported, level, visibility, result);
            assert_eq!(
                mirror_level(&entry),
                expected,
                "{level:?}/{visibility:?}/{result:?}"
            );
        }
    }

    #[test]
    fn an_uncreatable_logs_dir_fails_and_names_the_path() {
        // Arrange: the would-be logs directory sits under a regular
        // file, so no directory or file can be created beneath it — a
        // portable stand-in for permission failures (the appender also
        // creates the directory itself, so this covers that arm too).
        let temp = tempfile::tempdir().expect("failed to create a temp directory");
        let blocker = temp.path().join("blocker");
        fs::write(&blocker, b"not a directory").expect("writing a temp file must work");
        let root = blocker.join("logs");

        // Act
        let result = build_file_subscriber(&root, LogLevel::Info);

        // Assert: the failure is reported (never a panic) and carries
        // the full logs path — this is the message the degradation warn
        // event logs. (Destructured instead of `expect_err`: the Ok side
        // is an opaque subscriber type without `Debug`.)
        let error = match result {
            Ok(_) => panic!("initializing under a regular file must fail"),
            Err(error) => error,
        };
        let expected_path = root.display().to_string();
        assert!(
            error.to_string().contains(&expected_path),
            "the error must name the logs directory (`{expected_path}`), \
             got: {error}"
        );
        assert!(
            error.source().is_some(),
            "the OS cause must stay reachable through the error chain"
        );
    }

    /// A [`tracing_subscriber::fmt::writer::MakeWriter`] whose writer
    /// always fails — the portable stand-in for the broken-stderr state
    /// the degradation path exists for (a Windows GUI build with no
    /// console, a closed pipe).
    struct MakeFailingWriter;

    impl<'a> tracing_subscriber::fmt::writer::MakeWriter<'a> for MakeFailingWriter {
        type Writer = FailingWriter;

        fn make_writer(&'a self) -> FailingWriter {
            FailingWriter
        }
    }

    struct FailingWriter;

    impl std::io::Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "nope"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "nope"))
        }
    }

    #[test]
    fn a_failing_writer_never_panics_or_aborts_the_emitter() {
        // Arrange: the shared production builder (`fmt_subscriber` — the
        // exact function both app sinks use) over a writer that always
        // errors. The fmt layer's default behavior on a writer error is
        // an `eprintln!` of its own — which panics when stderr is
        // unusable, and the workspace's `panic = "abort"` would turn
        // that into a process abort inside `setup` (module docs). With
        // `log_internal_errors(false)` the event is dropped instead.
        //
        // Honest boundary: a unit test cannot make the real stderr
        // unwritable (the harness captures it), so the eprintln panic
        // itself is not reproducible here — the `config_tests.rs` text
        // pin keeps the flag present in the shared builder, and this
        // test pins the emitter-side contract: an event whose write
        // fails must be dropped without unwinding, on the production
        // construction path.
        let subscriber = fmt_subscriber(MakeFailingWriter, LogLevel::Info);

        // Act & Assert: the emission must return normally — any panic
        // here (the eprintln fallback, an unwrap in the layer) fails
        // this test by unwinding.
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("this line cannot be written and must be dropped");
        });
    }

    #[test]
    fn a_written_daily_backup_logs_its_path() {
        // The ERR-001 success line: one `info` event carrying the copy
        // path (paths are allowed in logs, SEC-008) — nothing else.
        let (_temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");

        tracing::subscriber::with_default(subscriber, || {
            db_backup_daily(Path::new("/data/db-backups/daily-20261118.sqlite3"));
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("daily-20261118.sqlite3"),
            "the line must carry the copy path, got: {log}"
        );
        assert!(
            log.contains("daily database copy written"),
            "the line must say what happened, got: {log}"
        );
    }

    #[test]
    fn a_failed_daily_backup_logs_its_reason_and_stays_a_warning() {
        // The ERR-001 degradation line: a `warn` that survives the
        // default `info` level (the launch continues — an `error` would
        // read as a failed launch in log triage), carrying the storage
        // error's own text (which names the offending path).
        let (_temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Warn)
            .expect("building the subscriber must work");
        let error = eunomia_store::storage::StorageError::Backup {
            path: PathBuf::from("/data/db-backups/daily-20261118.sqlite3"),
            source: eunomia_store::storage::BackupError::Directory,
        };

        tracing::subscriber::with_default(subscriber, || {
            db_backup_daily_failed(&error);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains(" WARN ") && log.contains("daily database copy failed"),
            "the failure must be a visible warning, got: {log}"
        );
        assert!(
            log.contains("daily-20261118.sqlite3"),
            "the line must carry the failed path from the error's own Display, got: {log}"
        );
        assert!(
            log.contains("continuing without it"),
            "the line must say the launch continues, got: {log}"
        );
    }

    #[test]
    fn recovery_lines_carry_both_paths_and_the_action() {
        // The T-024 audit lines: preserve names where the damaged
        // database went and where it came from; recreate/restore name
        // what the database now is and where the copy came from. All
        // `info` (completed user-visible actions), all paths-only
        // (SEC-008: no database content crosses into the log).
        let (_temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");

        tracing::subscriber::with_default(subscriber, || {
            recovery_preserved(
                Path::new("/data/db/eunomiasync.sqlite3"),
                Path::new("/data/db-backups/pre-recovery-20261118T120000000Z.sqlite3"),
            );
            recovery_recreated(Path::new("/data/db/eunomiasync.sqlite3"));
            recovery_restored(
                Path::new("/data/db-backups/daily-20261117.sqlite3"),
                Path::new("/data/db/eunomiasync.sqlite3"),
            );
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("from=/data/db/eunomiasync.sqlite3")
                && log.contains("to=/data/db-backups/pre-recovery-20261118T120000000Z.sqlite3")
                && log.contains("preserved"),
            "the preserve line must carry both paths, got: {log}"
        );
        assert!(
            log.contains("started with an empty database"),
            "the recreate line must name the action, got: {log}"
        );
        assert!(
            log.contains("backup=/data/db-backups/daily-20261117.sqlite3")
                && log.contains("the chosen copy replaced the database"),
            "the restore line must name the copy and the action, got: {log}"
        );
    }

    #[test]
    fn a_failed_recovery_logs_its_code_params_and_os_facts() {
        // §10's rule for the recovery commands: every error reaches
        // the log with its code — the same payload shape as the
        // startup verdicts (code literal + params), at `error` level
        // so it survives the strictest config — plus the OS facts a
        // file-mechanics failure carries in `details` (the round-2
        // pin: the line must surface them, not only code + params).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Error)
            .expect("building the subscriber must work");
        let mut error = AppError::new(crate::errors::ErrorCode::NoWriteAccess);
        error
            .params
            .insert("name".to_owned(), "../daily-20261117.sqlite3".to_owned());
        error
            .params
            .insert("reason".to_owned(), "not a plain file name".to_owned());
        error.details = Some(crate::errors::ErrorDetails {
            os_code: Some(13),
            os_kind: Some("PermissionDenied".to_owned()),
            os_message: Some("Permission denied (os error 13)".to_owned()),
        });

        tracing::subscriber::with_default(subscriber, || {
            recovery_failed(&error);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("code=\"E_NO_WRITE_ACCESS\""),
            "the line must carry the §10 code literal, got: {log}"
        );
        assert!(
            log.contains("os_code=Some(13)")
                && log.contains("PermissionDenied")
                && log.contains("Permission denied (os error 13)"),
            "the line must carry the OS facts from details (§10: the OS code \
             is a first-class fact of every error line), got: {log}"
        );
        assert!(
            log.contains("../daily-20261117.sqlite3") && log.contains("not a plain file name"),
            "the line must carry the error's params (which name the rejected \
             selection), got: {log}"
        );
        assert!(
            log.contains("recovery operation failed"),
            "the line must say what failed, got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn a_failed_activity_list_logs_its_code_params_and_os_facts() {
        // The `activity_list` command's §10 line (T-027): every error
        // reaches the log with its code — the same payload shape as
        // the recovery commands' failures (code literal + params +
        // the OS facts when present), at `error` level so it survives
        // the strictest config. The params here name the rejected
        // filter field/value/reason, the shape the command's
        // validation rejections build.
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Error)
            .expect("building the subscriber must work");
        let mut error = AppError::new(crate::errors::ErrorCode::Validation);
        error
            .params
            .insert("field".to_owned(), "from_ts".to_owned());
        error
            .params
            .insert("value".to_owned(), "2026-11-10T00:00:00Z".to_owned());
        error.params.insert(
            "reason".to_owned(),
            "must be a DB-002 UTC timestamp (YYYY-MM-DDTHH:MM:SS.sssZ)".to_owned(),
        );

        tracing::subscriber::with_default(subscriber, || {
            activity_list_failed(&error);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("code=\"E_VALIDATION\""),
            "the line must carry the §10 code literal, got: {log}"
        );
        assert!(
            log.contains("from_ts")
                && log.contains("2026-11-10T00:00:00Z")
                && log.contains("must be a DB-002 UTC timestamp"),
            "the line must carry the rejection's params (the rejected filter), got: {log}"
        );
        assert!(
            log.contains("the activity list query failed"),
            "the line must say what failed, got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn failed_settings_commands_log_their_code_params_and_os_facts() {
        // The two settings commands' §10 lines (T-030): every error
        // reaches the log with its code — the same payload shape as
        // the `activity_list` failure line, so one test drives both
        // facades: a validation rejection (the save path's shape —
        // key/value/reason params) through `settings_set_failed`, and
        // a corrupt-cell fallback (the read path's shape — the
        // `SettingsCorrupt` OS facts) through `settings_get_failed`.
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Error)
            .expect("building the subscriber must work");
        let mut rejected = AppError::new(crate::errors::ErrorCode::Validation);
        rejected
            .params
            .insert("key".to_owned(), "scan.max_depth".to_owned());
        rejected.params.insert("value".to_owned(), "21".to_owned());
        rejected.params.insert(
            "reason".to_owned(),
            "21 is outside the range 1..=20".to_owned(),
        );
        let mut corrupt = AppError::new(crate::errors::ErrorCode::DbLocked);
        corrupt.details = Some(crate::errors::ErrorDetails {
            os_code: None,
            os_kind: Some("SettingsCorrupt".to_owned()),
            os_message: Some("a stored settings value cell is not valid JSON".to_owned()),
        });

        tracing::subscriber::with_default(subscriber, || {
            settings_get_failed(&corrupt);
            settings_set_failed(&rejected);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("code=\"E_DB_LOCKED\"") && log.contains("SettingsCorrupt"),
            "the read line must carry its code and diagnostic, got: {log}"
        );
        assert!(
            log.contains("code=\"E_VALIDATION\"")
                && log.contains("scan.max_depth")
                && log.contains("21"),
            "the save line must carry the rejection's key/value, got: {log}"
        );
        assert!(
            log.contains("the settings read failed") && log.contains("the settings save failed"),
            "each command's line must say what failed, got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn a_failed_app_info_command_logs_its_code_and_diagnostic() {
        // The `app_info` command's §10 line (T-031), in the same payload
        // shape: the only reachable failure is the missing `DataRoot`
        // bug-state, so the test drives exactly that shape — the
        // fallback code and the `DataRootMissing` diagnostic the
        // command builds for it.
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Error)
            .expect("building the subscriber must work");
        let mut missing = AppError::new(crate::errors::ErrorCode::DbLocked);
        missing.details = Some(crate::errors::ErrorDetails {
            os_code: None,
            os_kind: Some("DataRootMissing".to_owned()),
            os_message: Some("the data directory was not resolved at startup".to_owned()),
        });

        tracing::subscriber::with_default(subscriber, || {
            app_info_failed(&missing);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("code=\"E_DB_LOCKED\"") && log.contains("DataRootMissing"),
            "the line must carry its code and diagnostic, got: {log}"
        );
        assert!(
            log.contains("the app_info command failed"),
            "the line must say what failed, got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn the_dialog_failure_line_carries_the_code_and_payload() {
        // The picker command's §10 edge: a vanish-race failure
        // (E_PROJECT_UNAVAILABLE, the picked path in params, the OS
        // refusal in details) reaches the log line with its code
        // literal — the `app_info_failed` pin shape, applied to the
        // T-073 command. A successful pick logs nothing; that half is
        // structural (no tracing call exists on the success path).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Error)
            .expect("building the subscriber must work");
        let mut vanished = AppError::new(crate::errors::ErrorCode::ProjectUnavailable);
        vanished
            .params
            .insert("path".to_owned(), "/repos/gone".to_owned());
        let vanished = vanished.with_os_error(&std::io::Error::from_raw_os_error(2));

        tracing::subscriber::with_default(subscriber, || {
            dialog_pick_folder_failed(&vanished);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("code=\"E_PROJECT_UNAVAILABLE\""),
            "the line must carry its code literal, got: {log}"
        );
        assert!(
            log.contains("the folder picker command failed"),
            "the line must say what failed, got: {log}"
        );
        assert!(
            log.contains("/repos/gone") && log.contains("os_code=Some(2)"),
            "the params and the OS refusal travel on the line (SEC-008: paths may), got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn the_project_add_failure_line_carries_the_code_and_payload() {
        // The add command's §10 edge (T-074): a rejected add — here
        // §10's `E_NOT_A_REPO` with the picked path in params —
        // reaches the log line with its code literal, in the
        // `dialog_pick_folder_failed` shape. The three typed non-add
        // outcomes (offer, duplicate, archive offer) log nothing;
        // that half is structural (no tracing call exists on those
        // paths — they return `Ok` before the logging edge).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Error)
            .expect("building the subscriber must work");
        let mut not_a_repo = AppError::new(crate::errors::ErrorCode::NotARepo);
        not_a_repo
            .params
            .insert("path".to_owned(), "/repos/plain".to_owned());

        tracing::subscriber::with_default(subscriber, || {
            project_add_failed(&not_a_repo);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("code=\"E_NOT_A_REPO\""),
            "the line must carry its code literal, got: {log}"
        );
        assert!(
            log.contains("adding a project by hand failed"),
            "the line must say what failed, got: {log}"
        );
        assert!(
            log.contains("/repos/plain") && log.contains("os_code=None"),
            "the params travel on the line and no OS fact is invented, got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn the_exclusion_failure_lines_carry_the_code_and_payload() {
        // The two exclusion commands' §10 edges (T-078), in the
        // `project_add_failed` pin shape — the round-2 review's ask:
        // the list's reachable class is the corrupt-cell fallback
        // (E_DB_LOCKED with the `ProjectsCorrupt` diagnostic), the
        // remove's is the typed unknown-id refusal (E_VALIDATION with
        // the field/value/reason params). A successful remove is the
        // journal entry itself and a successful read logs nothing;
        // both halves are structural (no tracing call exists on those
        // paths).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Error)
            .expect("building the subscriber must work");
        let mut corrupt = AppError::new(crate::errors::ErrorCode::DbLocked);
        corrupt.details = Some(crate::errors::ErrorDetails {
            os_code: None,
            os_kind: Some("ProjectsCorrupt".to_owned()),
            os_message: Some("a stored reason cell is outside its vocabulary".to_owned()),
        });
        let mut unknown_id = AppError::new(crate::errors::ErrorCode::Validation);
        unknown_id
            .params
            .insert("field".to_owned(), "id".to_owned());
        unknown_id.params.insert(
            "value".to_owned(),
            "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned(),
        );
        unknown_id
            .params
            .insert("reason".to_owned(), "no exclusion with this id".to_owned());

        tracing::subscriber::with_default(subscriber, || {
            exclusions_list_failed(&corrupt);
            exclusion_remove_failed(&unknown_id);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("listing the excluded paths failed")
                && log.contains("code=\"E_DB_LOCKED\"")
                && log.contains("os_kind=Some(\"ProjectsCorrupt\")"),
            "the list edge carries its code literal and the corrupt-cell diagnostic, got: {log}"
        );
        assert!(
            log.contains("removing a path exclusion failed")
                && log.contains("code=\"E_VALIDATION\"")
                && log.contains("no exclusion with this id"),
            "the remove edge carries its code literal and the refusal's reason, got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn the_projects_reorder_failure_line_carries_the_code_and_payload() {
        // The reorder command's §10 edge (T-080), in the exclusion pin
        // shape (the round-1 review's ask): the reachable classes are
        // the typed refusal of a stale list (E_VALIDATION with the
        // field/value/reason params) and the corrupt-cell fallback of
        // the command's own `list(All)` read (E_DB_LOCKED with the
        // `ProjectsCorrupt` diagnostic). A successful reorder is the
        // journal entry itself and the verified no-op writes nothing —
        // both halves structural (no tracing call exists on those
        // paths).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Error)
            .expect("building the subscriber must work");
        let mut stale_list = AppError::new(crate::errors::ErrorCode::Validation);
        stale_list
            .params
            .insert("field".to_owned(), "ordered_ids".to_owned());
        stale_list.params.insert(
            "value".to_owned(),
            "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned(),
        );
        stale_list.params.insert(
            "reason".to_owned(),
            "the list must name every active project exactly once: 1 not an active project, \
             0 missing, 0 repeated"
                .to_owned(),
        );
        let mut corrupt = AppError::new(crate::errors::ErrorCode::DbLocked);
        corrupt.details = Some(crate::errors::ErrorDetails {
            os_code: None,
            os_kind: Some("ProjectsCorrupt".to_owned()),
            os_message: Some("a stored state cell is outside its vocabulary".to_owned()),
        });

        tracing::subscriber::with_default(subscriber, || {
            projects_reorder_failed(&stale_list);
            projects_reorder_failed(&corrupt);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("saving the new project order failed")
                && log.contains("code=\"E_VALIDATION\"")
                && log.contains("ordered_ids"),
            "the refusal line carries its code literal and the field, got: {log}"
        );
        assert!(
            log.contains("code=\"E_DB_LOCKED\"")
                && log.contains("os_kind=Some(\"ProjectsCorrupt\")"),
            "the corrupt-cell line carries the fallback code and diagnostic, got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn the_projects_list_failure_line_carries_the_code_and_payload() {
        // The list command's §10 edge (T-244, the D-096 follow-up), in
        // the reorder pin shape: the reachable classes are the honest
        // no-database fallback of recovery mode (the `NoDatabase`
        // diagnostic) and the store fallback of a corrupt projects
        // cell surfaced by the widened read (the `ProjectsCorrupt`
        // diagnostic) — both fixed by the T-072 IPC pins this test
        // mirrors; the SQL classes and the blocking-pool join failure
        // travel the same one-line shape. A successful read logs
        // nothing; that half is structural (no tracing call exists on
        // that path — the command returns before the edge).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Error)
            .expect("building the subscriber must work");
        let mut no_database = AppError::new(crate::errors::ErrorCode::DbLocked);
        no_database.details = Some(crate::errors::ErrorDetails {
            os_code: None,
            os_kind: Some("NoDatabase".to_owned()),
            os_message: Some("no database is running (recovery mode?)".to_owned()),
        });
        let mut corrupt = AppError::new(crate::errors::ErrorCode::DbLocked);
        corrupt.details = Some(crate::errors::ErrorDetails {
            os_code: None,
            os_kind: Some("ProjectsCorrupt".to_owned()),
            os_message: Some("a stored state cell is outside its vocabulary".to_owned()),
        });

        tracing::subscriber::with_default(subscriber, || {
            projects_list_failed(&no_database);
            projects_list_failed(&corrupt);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("listing the projects failed")
                && log.contains("code=\"E_DB_LOCKED\"")
                && log.contains("os_kind=Some(\"NoDatabase\")")
                && log.contains("os_message=Some(\"no database is running (recovery mode?)\")")
                && log.contains("os_code=None"),
            "the recovery-mode line carries its code literal and the \
             no-database diagnostic, got: {log}"
        );
        assert!(
            log.contains("code=\"E_DB_LOCKED\"")
                && log.contains("os_kind=Some(\"ProjectsCorrupt\")")
                && log
                    .contains("os_message=Some(\"a stored state cell is outside its vocabulary\")"),
            "the corrupt-cell line carries the corrupt-cell diagnostic, got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn data_folder_lines_carry_the_path_and_level() {
        // The opener command's two lines: a success `info` with the
        // data directory path, and a failure `error` with the mapped
        // wire payload (code + params; the opener's own text lives in
        // `details`, which the line format prints as the params debug
        // only when present — the pinned fact here is the code and
        // the path).
        let (_temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");

        tracing::subscriber::with_default(subscriber, || {
            data_folder_opened(Path::new("/data"));
            let mut error = AppError::new(crate::errors::ErrorCode::DbLocked);
            error.details = Some(crate::errors::ErrorDetails {
                os_code: None,
                os_kind: Some("OpenerError".to_owned()),
                os_message: Some("xdg-open not found".to_owned()),
            });
            data_folder_open_failed(&error);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("path=/data") && log.contains("data directory opened"),
            "the success line must carry the path, got: {log}"
        );
        assert!(
            log.contains("code=\"E_DB_LOCKED\"")
                && log.contains("opening the data directory failed"),
            "the failure line must carry the mapped code literal, got: {log}"
        );
        assert!(
            log.contains("xdg-open not found"),
            "the failure line must carry the opener's own text from details, \
             got: {log}"
        );
    }

    #[test]
    fn retention_lines_carry_counts_and_the_failure_reason() {
        // The LOG-006 housekeeping pair (D-054): a cleanup that
        // removed something logs its counts at `info`; a failed one
        // logs at `warn` with the fallback wire shape — code literal
        // plus the storage failure's own text from `details` — and
        // neither line can carry anything but numbers and codes
        // (SEC-008: housekeeping has no paths or content to begin
        // with, and the payload shape pins that).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");

        tracing::subscriber::with_default(subscriber, || {
            db_cleanup(&eunomia_store::action_log::CleanupCounts {
                log_entries: 214,
                sync_runs: 3,
            });
            let mut error = AppError::new(crate::errors::ErrorCode::DbLocked);
            error.details = Some(crate::errors::ErrorDetails {
                os_code: None,
                os_kind: Some("StorageError".to_owned()),
                os_message: Some("invalid setting log.max_days: range".to_owned()),
            });
            db_cleanup_failed(&error);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("log_entries=214") && log.contains("sync_runs=3"),
            "the success line must carry the removal counts, got: {log}"
        );
        assert!(
            log.contains("retention cleanup removed rows beyond the limits"),
            "the success line must say what happened, got: {log}"
        );
        assert!(
            log.contains("code=\"E_DB_LOCKED\"") && log.contains("log.max_days"),
            "the failure line must carry the code literal and the storage \
             layer's own reason, got: {log}"
        );
        assert!(
            log.contains("WARN") && log.contains("retention cleanup failed"),
            "the failure line must be a warn naming the failed cleanup, got: {log}"
        );
        drop(temp);
    }
}
