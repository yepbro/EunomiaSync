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
/// file (the list of `db-backups/` copies and the "start with an
/// empty database" option are the recovery screen's, the future
/// recovery task's). Logged at `error` on purpose: the launch aborts
/// on this verdict, so the line must stay visible at
/// `log_level = "error"` (the same reasoning as [`db_too_new`]).
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
}
