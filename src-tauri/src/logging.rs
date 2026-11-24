//! File logging with daily + 10 MB rotation (spec §5.3, §5.5, §7.7;
//! SYS-011…013, SEC-008; the 2026-11-21 owner ruling "5 files ×
//! 10 MB" — D-129/T-253 shipped the count half, T-255 the size half).
//!
//! The app writes its trace log into the data directory's `logs/`
//! subfolder — `logs/eunomiasync.<date>.log`, where `<date>` is the UTC
//! `YYYY-MM-DD` stamp of the write — via `tracing` + a custom writer of
//! this module, [`CappedDailyWriter`], moved onto a dedicated thread by
//! `tracing_appender::non_blocking`. The writer rotates when the UTC
//! day changes **and** when the current file can hold no more than the
//! ruling's 10 MB ([`MAX_LOG_FILE_BYTES`]); at most [`MAX_LOG_FILES`]
//! files survive — the writer's own prune runs when it is built and
//! before every rotation, deleting the oldest beyond the cap. The
//! first file of a day carries the exact §5.5 name; a day whose volume
//! passes 10 MB continues into same-day parts
//! `eunomiasync.<date>.2.log`, `.3.log`, … — T-255's recorded reading
//! of the ruling against the §5.5 name scheme (see the DECISIONS.md
//! T-255 entry): the ruling is unsatisfiable without same-day files,
//! and the §5.5 structure block lists primary shapes only (the
//! `pre-recovery-*` precedent, T-024/D-049). The writer is pointed at
//! `<data dir>/logs/` explicitly: Tauri's `app_log_dir()` would move the
//! logs to `~/Library/Logs/…` on macOS, away from the spec §5.5 layout —
//! a `config_tests.rs` absence pin keeps that API out of the codebase so
//! the regression cannot pass silently.
//!
//! Why a custom writer (the card's sanctioned choice over a
//! post-rotation size check): `tracing-appender` 0.2.5 has no
//! size-based rotation — its `Rotation` is time-based only
//! (Minutely/Hourly/Daily/Weekly/Never, `rolling.rs:490-509`) — and the
//! rolling appender decides rotation solely from the wall clock inside
//! its private write path (`rolling.rs:226-241`; `should_rollover`,
//! `rolling.rs:758-770`), with no public API to force a mid-day file
//! switch; its builder surface is rotation/prefix/suffix/symlink/
//! max-files and nothing else (`builder.rs:9-15`). Renaming the open
//! file out from under it is no substitute either: the appender keeps
//! writing into its open handle and reopens by its own date name on
//! its own clock, and rename-while-open semantics vary by Windows
//! version and filesystem (std's default share mode permits the
//! rename, but the vacated name's reuse while the handle lives is not
//! a promise std makes) — the trick is neither supported nor portable.
//! The size half therefore required owning the
//! file; the crate's remaining role is the `non_blocking` worker
//! thread, which this module keeps verbatim — and taking the rolling
//! appender out of the path also retires its `eprintln!`-on-failure
//! sites (`rolling.rs:704-707`, `:718-724`, `:741-746`), the Windows
//! GUI abort exposure D-129 flagged (see the failure-policy paragraph
//! below for the one `eprintln!` left in the crate).
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
//! The same policy governs the writer's own rotation mid-run: a prune
//! that cannot read or delete, or a next part file that cannot be
//! opened, is reported **once** through the facade
//! ([`log_rotation_failed`]) and then ignored — the writer keeps
//! writing into the current file (the 10 MB cap is exceeded, never the
//! launch) and every following write retries the rotation, so a healed
//! disk heals the log. Reporting once per degradation episode — the
//! gate re-arms on the next successful rotation — rather than per
//! line, is what keeps a persistently broken rotation from turning
//! into a warn storm through the very pipeline it is failing; and the
//! report is an *attempt*: it rides the same lossy channel as every
//! log line, and a full queue drops it (the honest boundary of
//! diagnosing a pipeline through itself). The report is safe to route
//! through the subscriber because `non_blocking`'s channel is lossy by
//! default (`try_send`, never blocking — `non_blocking.rs:228-233`
//! for the default builder, `:238-243` for the write arm) and the
//! worker thread drops writer errors silently (`worker.rs:81-83`), so
//! the writer can never deadlock on its own diagnostics. A
//! construction-time prune failure stays fully silent: no subscriber
//! exists yet to report through, and housekeeping must not stop the
//! log. The one `eprintln!` left reachable in the vendored crate is
//! the worker's final-flush failure at guard-drop shutdown
//! (`worker.rs:86-88`) — the app never performs that shutdown (tao
//! exits via `std::process::exit`, skipping destructors; see the
//! guard paragraph below), and the whole workspace's own sources are
//! kept `eprintln!`-free in production code by the workspace-level
//! clippy gate (`[workspace.lints.clippy]` `print_stderr`/
//! `print_stdout` in the root `Cargo.toml`, inherited by every member
//! through `[lints] workspace = true`), the structural encoding of the
//! D-032/D-129 Windows analysis.
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
//!   tested, still-unused primitive (its docs carry the full D-163
//!   status), takes a [`FileOp`] code, a path and an optional numeric
//!   ID, and that is the whole payload (pinned by the module tests: a
//!   file event carries the path and the ID while the instruction text
//!   of the very operation stays out of the file).
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
//! file back. The rotation's decision logic (the size check, the file
//! naming, the prune verdict) is extracted into pure functions tested
//! directly, with only the thin IO seam riding the real clock. Facts
//! consciously outside what a unit test can prove: the midnight
//! rollover (a date change mid-run) — the dated file name proves the
//! rotation wiring, the crossing itself rests on the clock plus the
//! manual smoke run; a tracing event emitted from *inside* the writer
//! (the [`log_rotation_failed`] degradation line) — it is produced on
//! the `non_blocking` worker thread, where the tests' thread-local
//! `with_default` subscriber does not exist, so the degradation suite
//! drives the writer directly on the test thread instead, and only the
//! production global-install path carries the line; and the global
//! install itself ([`tracing::subscriber::set_global_default`]) is
//! process-global, so installing it inside the parallel test suite is
//! not observable there; the `setup` wiring is pinned textually in
//! `config_tests.rs` instead.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use tracing::Subscriber;
use tracing_appender::non_blocking::WorkerGuard;

use crate::config::LogLevel;
use crate::errors::AppError;

/// The log file name before the date stamp (spec §5.5:
/// `logs/eunomiasync.<date>.log`). The suffix is [`LOG_FILE_SUFFIX`];
/// the module's own [`log_file_name`] joins them as
/// `<prefix>.<date>.<suffix>` (part 1) and `<prefix>.<date>.<part>.
/// <suffix>` (same-day parts).
const LOG_FILE_PREFIX: &str = "eunomiasync";

/// The log file name extension (spec §5.5).
const LOG_FILE_SUFFIX: &str = "log";

/// The maximum number of log files kept (T-253, owner ruling
/// 2026-11-21; T-255 completed the ruling's other half): the writer's
/// own prune — run when the writer is built (every startup) and before
/// every rotation — deletes the oldest beyond this many, ordering by
/// the (date, part) stamps parsed from the file names, never mtime
/// (the D-129 retention rule: a name whose age cannot be established
/// is not ours to judge, so foreign files are never deleted). The
/// cap counts same-day parts like any other file: a chatty day can
/// spend all five slots on itself — exactly the ruling's "5 файлов ×
/// 10 МБ (≈50 МБ максимум)" model, with [`MAX_LOG_FILE_BYTES`]
/// bounding each file.
const MAX_LOG_FILES: usize = 5;

/// The per-file size cap (T-255, the ruling's other half): when the
/// current file can hold no more than this many bytes, the next write
/// rolls into a new same-day part. A write that lands exactly at the
/// cap stays (the file reaches it precisely; the write *after* that
/// rolls), so no file exceeds the cap — except a single line that is
/// oversized on its own, which lands alone in its part and rolls the
/// next write (the alternative would be a loop of empty parts).
const MAX_LOG_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Why [`init`] could not build the file log: the path it could not
/// create or open, plus the OS cause. Kept as data (like
/// `DataLayoutError`/`ConfigError`) because the degradation path logs
/// it: the message must carry the full path so the user can tell *which*
/// directory is broken. This error never aborts the launch — see the
/// module docs.
#[derive(Debug)]
pub(crate) struct LogInitError {
    /// The `logs/` directory the writer was pointed at.
    path: PathBuf,
    /// The underlying file-open failure.
    source: io::Error,
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

/// Builds the daily+size-rotating file subscriber for `logs_dir` at
/// `level` — the testable seam behind [`init`]. The returned guard must
/// outlive every event the subscriber formats (see [`Logger`]).
///
/// # Errors
///
/// Fails with [`LogInitError`] when today's log file cannot be opened —
/// the directory cannot be read or the file cannot be created. Callers
/// degrade (module docs); they never abort the launch over it.
fn build_file_subscriber(
    logs_dir: &Path,
    level: LogLevel,
) -> Result<(impl Subscriber + Send + Sync + 'static, WorkerGuard), LogInitError> {
    build_file_subscriber_capped(logs_dir, level, MAX_LOG_FILES, MAX_LOG_FILE_BYTES)
}

/// The parameterized seam behind [`build_file_subscriber`]: the
/// production constants enter there and only there, and the behavior
/// suite drives this function with small caps — the `fmt_subscriber`
/// precedent, so the tests exercise the production path rather than a
/// lookalike.
fn build_file_subscriber_capped(
    logs_dir: &Path,
    level: LogLevel,
    max_files: usize,
    max_log_file_bytes: u64,
) -> Result<(impl Subscriber + Send + Sync + 'static, WorkerGuard), LogInitError> {
    let appender =
        CappedDailyWriter::new(logs_dir, max_files, max_log_file_bytes).map_err(|source| {
            LogInitError {
                path: logs_dir.to_path_buf(),
                source,
            }
        })?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let subscriber = fmt_subscriber(writer, level);
    Ok((subscriber, guard))
}

// ---- The rotation writer (T-255) --------------------------------------------
//
// Pure decision units first (name grammar, size check, prune verdict —
// tested directly below), then the thin IO seam that rides the real
// clock (`today_utc_string` is its only clock read).

/// The rotation key of one log file: the UTC `YYYY-MM-DD` stamp its
/// name carries and the 1-based same-day part number. Part 1 is the
/// unsuffixed spec §5.5 name; parts ≥ 2 are the same-day size-rolled
/// files (see the module docs for the recorded reading). The derived
/// ordering is chronological for canonical keys: fixed-width dates
/// compare lexicographically, the part numerically.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FileKey {
    date: String,
    part: u32,
}

/// The file name of one part: the exact §5.5 shape for part 1, the
/// ruling's same-day part shape (`.<part>` before the suffix) for
/// parts ≥ 2.
fn log_file_name(key: &FileKey) -> String {
    if key.part == 1 {
        format!("{LOG_FILE_PREFIX}.{}.{LOG_FILE_SUFFIX}", key.date)
    } else {
        format!(
            "{LOG_FILE_PREFIX}.{}.{}.{LOG_FILE_SUFFIX}",
            key.date, key.part
        )
    }
}

/// The inverse of [`log_file_name`], and the grammar the prune trusts.
/// Accepts only canonical names — the spec-shaped part 1 and parts ≥ 2
/// in decimal without leading zeros, the date a real calendar date in
/// the exact zero-padded `YYYY-MM-DD` shape (the dashes and padding
/// are checked by hand because jiff's parser also admits the basic
/// `YYYYMMDD` form, which is not our naming; jiff then rejects a
/// hand-made `2026-02-30`). Anything else — foreign names, backups,
/// `.1.log`, leading-zero parts — is not ours and is never deleted
/// (the D-129 rule: a file whose age cannot be established has no age
/// to judge).
fn parse_log_file_name(name: &str) -> Option<FileKey> {
    let rest = name
        .strip_prefix(LOG_FILE_PREFIX)?
        .strip_prefix('.')?
        .strip_suffix(LOG_FILE_SUFFIX)?
        .strip_suffix('.')?;
    let (date, part) = match rest.rsplit_once('.') {
        None => (rest, 1),
        Some((date, digits)) => {
            if digits.is_empty() {
                return None;
            }
            let leading = digits.as_bytes()[0];
            let canonical = leading != b'0'
                && !(digits.len() == 1 && leading == b'1')
                && digits.bytes().all(|b| b.is_ascii_digit());
            if !canonical {
                return None;
            }
            (date, digits.parse::<u32>().ok()?)
        }
    };
    let bytes = date.as_bytes();
    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| b.is_ascii_digit() || ((i == 4 || i == 7) && *b == b'-'));
    if !shaped {
        return None;
    }
    date.parse::<jiff::civil::Date>().ok()?;
    Some(FileKey {
        date: date.to_owned(),
        part,
    })
}

/// Which file the next `incoming` bytes belong in: the current one, the
/// first file of `now_date` when the UTC day changed, or the next
/// same-day part when the current file is past holding anything more.
/// The decision is pure — the size half's arithmetic is testable
/// without a clock or a disk.
///
/// Boundary semantics (pinned by tests): a write that would land
/// exactly at the cap stays (a part reaches the cap precisely; the
/// write *after* it rolls); a write larger than the whole cap on an
/// empty file also stays, landing alone in its part — anything else
/// would spin empty parts forever; the date change wins over size.
fn next_write_target(
    current: &FileKey,
    current_len: u64,
    now_date: &str,
    incoming: usize,
    cap: u64,
) -> FileKey {
    if now_date != current.date {
        FileKey {
            date: now_date.to_owned(),
            part: 1,
        }
    } else if current_len > 0 && current_len.saturating_add(incoming as u64) > cap {
        // A part number at `u32::MAX` cannot exist in practice (each
        // part holds at least one line); degrade to the current file
        // rather than panic regardless.
        match current.part.checked_add(1) {
            Some(part) => FileKey {
                date: current.date.clone(),
                part,
            },
            None => current.clone(),
        }
    } else {
        current.clone()
    }
}

/// Which of `keys` a prune may delete so at most `keep` remain: the
/// oldest by (date, part), never a protected key. Keys only ever come
/// from [`parse_log_file_name`], so a foreign file cannot appear here
/// at all. `keep` is the post-prune target *before* the file about to
/// be opened is created — the same semantics as the crate's
/// `max_log_files` (its builder docs: keep `n - 1`, then create).
fn prune_plan(keys: &[FileKey], keep: usize, protected: &[FileKey]) -> Vec<FileKey> {
    let mut sorted = keys.to_vec();
    sorted.sort();
    sorted.dedup();
    let excess = sorted.len().saturating_sub(keep);
    sorted
        .into_iter()
        .filter(|key| !protected.contains(key))
        .take(excess)
        .collect()
}

/// Every parseable log file in `dir` with its key. Linear in the
/// directory's size (the prune runs per rotation, and the cap itself
/// bounds the count of ours; foreign files are skipped, not judged).
fn scan_log_files(dir: &Path) -> io::Result<Vec<(FileKey, PathBuf)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type().is_ok_and(|file_type| file_type.is_file()) {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if let Some(key) = parse_log_file_name(name) {
            files.push((key, entry.path()));
        }
    }
    Ok(files)
}

/// Today's UTC date as the canonical `YYYY-MM-DD` stamp — the rotation
/// seam's only read of the real clock (D-032's precedent: the midnight
/// crossing itself is the clock-plus-smoke boundary; the stamp's shape
/// and calendar truth are jiff's).
fn today_utc_string() -> String {
    jiff::Timestamp::now()
        .to_zoned_datetime(jiff::tz::TimeZone::UTC)
        .date()
        .to_string()
}

/// Today's UTC epoch day — the allocation-free half of the clock seam:
/// the writer compares this per write and reformats the stamp only when
/// the day changed. Exact by construction: unix seconds floor to UTC
/// epoch days, which map one-to-one onto civil dates (`div_euclid` so
/// pre-1970 timestamps, should a clock ever produce one, still floor).
fn today_epoch_day() -> i64 {
    jiff::Timestamp::now().as_second().div_euclid(86_400)
}

/// The custom writer behind the file log (T-255): the size half of the
/// owner's "5 files × 10 MB" ruling, succeeding T-253's count half.
/// Owns one open file; before every batch it decides — purely, via
/// [`next_write_target`] — whether the bytes belong in the current
/// file, the next same-day part, or the first file of a new UTC day,
/// and rotates by pruning ([`prune_plan`], keep `max_files - 1`) and
/// opening the target.
///
/// Single-threaded by construction: `non_blocking` moves the writer to
/// its worker thread and every `write` happens there (the degradation
/// suite drives it directly instead), so the state needs no locks.
///
/// Idempotent across restarts (the make-operations-idempotent rule):
/// construction opens *today's highest existing part* and continues at
/// its true length, so a rerun appends, a part left by an interrupted
/// rotation is finished rather than restarted, gaps in part numbers
/// are never refilled, and a part already at the cap rolls on its
/// first write.
struct CappedDailyWriter {
    /// The `logs/` directory every rotation works in.
    directory: PathBuf,
    /// The ruling's count half — see [`MAX_LOG_FILES`].
    max_files: usize,
    /// The ruling's size half — see [`MAX_LOG_FILE_BYTES`].
    cap: u64,
    /// The currently open file; every write lands here.
    file: File,
    /// The open file's key.
    key: FileKey,
    /// The cached UTC date stamp the rotation decides against —
    /// refreshed only when [`Self::day`] changes, so the per-write
    /// clock check is one division and no allocation.
    today: String,
    /// The UTC epoch day of [`Self::today`] (derived from the same
    /// stamp at construction, so the two cannot straddle a midnight).
    day: i64,
    /// Bytes already in the current file — the size half's accounting,
    /// initialized from the file's length at open so a restart
    /// continues a part at its true size.
    len: u64,
    /// Whether a rotation failure has been reported in the current
    /// degradation episode (one line per episode — a persistently
    /// broken rotation must not turn into a warn storm through the
    /// pipeline it is failing; a successful rotation re-arms the gate).
    rotation_reported: bool,
}

impl CappedDailyWriter {
    /// Creates the writer over `directory`, then opens today's highest
    /// existing part. The construction prune mirrors the
    /// keep-`n`-then-create reasoning only when a file is about to be
    /// created: a day with no file yet prunes to `max_files - 1` so the
    /// new file lands the total at the cap, while a restart over
    /// today's existing file prunes to `max_files` — a directory
    /// already at the cap loses nothing to a mere restart (the
    /// round-1 review's P2-2). If the directory scan fails, the
    /// today-part defaults to 1: an exotic state (the directory is
    /// openable but not listable) where the writer may continue part 1
    /// out of chronological order — the cap still holds from each
    /// opened file's true length, and the next rotation converges. A
    /// construction-time prune failure is silent (no subscriber exists
    /// yet to report through, and housekeeping must not stop the log);
    /// only an unopenable file fails construction — that is the
    /// [`LogInitError`] path the caller degrades on.
    fn new(directory: &Path, max_files: usize, cap: u64) -> io::Result<Self> {
        let date = today_utc_string();
        let scan = scan_log_files(directory).unwrap_or_default();
        let today_exists = scan.iter().any(|(key, _)| key.date == date);
        let today_part = scan
            .iter()
            .map(|(key, _)| key)
            .filter(|key| key.date == date)
            .map(|key| key.part)
            .max()
            .unwrap_or(1);
        let key = FileKey {
            date,
            part: today_part,
        };
        let keep = if today_exists {
            max_files
        } else {
            max_files.saturating_sub(1)
        };
        Self::prune(directory, keep, std::slice::from_ref(&key), &mut false);
        let (file, len) = Self::open(directory, &key)?;
        // One clock read feeds both the stamp and its epoch day — the
        // pair cannot straddle a midnight (a straddling `day` would
        // suppress the first day-change check until the next day).
        // If midnight strikes between the scan's stamp and this read,
        // the writer opens the old day's part and the first write's
        // date comparison rotates it — still correct.
        let now = jiff::Timestamp::now();
        let today = now
            .to_zoned_datetime(jiff::tz::TimeZone::UTC)
            .date()
            .to_string();
        let day = now.as_second().div_euclid(86_400);
        Ok(Self {
            directory: directory.to_path_buf(),
            max_files,
            cap,
            file,
            key,
            today,
            day,
            len,
            rotation_reported: false,
        })
    }

    /// Opens `key`'s file for append, creating it (and the directory —
    /// the crate's own `create_writer` behavior, kept so a missing
    /// `logs/` recovers instead of degrading; in the app the layout
    /// step has already made it), and reports its current length — a
    /// part left by an interrupted rotation is continued, not
    /// restarted.
    fn open(directory: &Path, key: &FileKey) -> io::Result<(File, u64)> {
        let path = directory.join(log_file_name(key));
        let open = || fs::OpenOptions::new().append(true).create(true).open(&path);
        let file = open().or_else(|error| {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|_| error)?;
            }
            open()
        })?;
        let len = file.metadata()?.len();
        Ok((file, len))
    }

    /// Deletes the oldest parseable log files beyond `keep`, protecting
    /// `protected` — during a rotation that is the file being replaced
    /// (its handle is still open) and the file about to be opened.
    /// Protecting the open file makes the outcome identical everywhere:
    /// where an open file cannot be removed the deletion would merely
    /// fail, and where it can (POSIX unlink, Windows' share-delete) the
    /// log would keep writing into an unlinked, nameless file — the
    /// next prune, with the handle closed, converges instead. Failures
    /// are reported once per degradation episode through the facade,
    /// then ignored: housekeeping must never stop the log (D-032's
    /// policy).
    fn prune(directory: &Path, keep: usize, protected: &[FileKey], reported: &mut bool) {
        let files = match scan_log_files(directory) {
            Ok(files) => files,
            Err(error) => {
                Self::report_rotation_failure(reported, RotationStep::Prune, directory, &error);
                return;
            }
        };
        let keys: Vec<FileKey> = files.iter().map(|(key, _)| key.clone()).collect();
        for key in prune_plan(&keys, keep, protected) {
            let Some((_, path)) = files.iter().find(|(candidate, _)| candidate == &key) else {
                continue;
            };
            if let Err(error) = fs::remove_file(path) {
                Self::report_rotation_failure(reported, RotationStep::Prune, path, &error);
            }
        }
    }

    /// Applies [`next_write_target`] (with the cached [`Self::today`]
    /// stamp as the clock input): on a target change, prunes and opens
    /// the new file; on failure, reports once per degradation episode
    /// and keeps writing to the current file — the cap is exceeded,
    /// never the launch — and every following write retries the
    /// rotation, so a healed disk heals the log. A successful rotation
    /// resets the report gate: the next *new* failure in the same
    /// process gets its own line (a failing rotation never succeeds,
    /// so this cannot re-enable a storm).
    fn rotate_if_due(&mut self, incoming: usize) {
        let target = next_write_target(&self.key, self.len, &self.today, incoming, self.cap);
        if target == self.key {
            return;
        }
        Self::prune(
            &self.directory,
            self.max_files.saturating_sub(1),
            &[self.key.clone(), target.clone()],
            &mut self.rotation_reported,
        );
        match Self::open(&self.directory, &target) {
            Ok((file, len)) => {
                // A raw `File` writes through to the OS unbuffered, so
                // replacing the handle loses nothing already handed to
                // the system; the flush is belt-and-braces.
                let _ = self.file.flush();
                self.file = file;
                self.len = len;
                self.key = target;
                self.rotation_reported = false;
            }
            Err(error) => {
                let path = self.directory.join(log_file_name(&target));
                Self::report_rotation_failure(
                    &mut self.rotation_reported,
                    RotationStep::Open,
                    &path,
                    &error,
                );
            }
        }
    }

    /// The once-per-episode gate for [`log_rotation_failed`] (reset by
    /// a successful rotation — see [`rotate_if_due`]). The report is an
    /// *attempt*: it rides the same lossy channel as every log line,
    /// and a full queue drops it — the honest boundary of diagnosing a
    /// pipeline through itself.
    fn report_rotation_failure(
        reported: &mut bool,
        step: RotationStep,
        path: &Path,
        error: &io::Error,
    ) {
        if *reported {
            return;
        }
        *reported = true;
        log_rotation_failed(step, path, error);
    }
}

impl io::Write for CappedDailyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // The cheap epoch-day check first; the stamp (and its
        // allocation) is refreshed only on a day change.
        let day = today_epoch_day();
        if day != self.day {
            self.day = day;
            self.today = today_utc_string();
        }
        self.rotate_if_due(buf.len());
        if let Err(error) = self.file.write_all(buf) {
            // A partial write (some bytes landed, then the disk
            // refused) would leave the size ledger undercounting —
            // resync from the file so the next rotation decision sees
            // the file's true length (the round-2 review's P3).
            if let Ok(len) = self.file.metadata().map(|metadata| metadata.len()) {
                self.len = len;
            }
            return Err(error);
        }
        self.len += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
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

/// The protective-copy retention pass removed copies beyond the 7-day
/// window (T-253, owner ruling 2026-11-21): `pre-migration-*` and
/// `pre-recovery-*` files in `db-backups/`, aged by the creation stamp
/// in their names, the newest of each kind protected even past the
/// window. `info`: a routine housekeeping event in the `db_cleanup`
/// shape — the counts are the audit fact. A pass that removed
/// **nothing** gets no line: it is the idempotent common case (every
/// open of a healthy directory), the same no-op policy as
/// [`db_backup_daily`]'s. The payload is numeric only (SEC-008), and
/// this file-log line is the pass's whole journal (LOG-001…004's
/// vocabulary has no housekeeping member — the D-054 boundary).
pub(crate) fn db_backup_retention(counts: &crate::retention::BackupRetentionCounts) {
    tracing::info!(
        target: "eunomia::db",
        pre_migration = counts.pre_migration,
        pre_recovery = counts.pre_recovery,
        "protective-copy retention removed db-backups/ copies beyond the 7-day window"
    );
}

/// One step of the protective-copy retention pass failed (T-253): the
/// `db-backups/` directory could not be read, or one aged copy could
/// not be removed. `warn`, not `error`, for the same reason as
/// [`db_backup_daily_failed`] — the pass is housekeeping over copies
/// that already exist, the open/preserve that triggered it has
/// succeeded, and the next open retries on its own (idempotent pass).
/// The path and the OS cause are the payload (SEC-008: paths are
/// allowed; the io error carries no stored text).
pub(crate) fn db_backup_retention_failed(path: &Path, error: &std::io::Error) {
    tracing::warn!(
        target: "eunomia::db",
        path = %path.display(),
        error = %error,
        "protective-copy retention failed; continuing without it (retried at the next open)"
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
///   database journal already holds it), and mirroring every such
///   entry would double the file's volume with the window's own
///   lines. The per-event facade functions write their lines
///   directly, not as mirrors of journal entries (`db_backup_daily`
///   today; `file_event` once its first producer lands — D-163).
///
/// The first production caller is `settings_set` (T-030), which
/// journals its `settings.changed` entries inside the operation's
/// transaction and calls this once per entry — the info+user+ok shape,
/// i.e. the unmirrored one. The sync pipeline (phase 4) is the
/// warn/error producer today: each `sync.project_result` row and the
/// `sync.run_finished` summary ride their outcome's level at user
/// visibility (`project_entry_shape`), so the mirror arms serve it —
/// a skipped or failed row's warning or error reaches the file log
/// through them.
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
/// copy stays where it was (the restore never consumes it; T-253's
/// age-based retention may retire an aged copy later, at the reopen,
/// like any other aged copy), and the line is the audit record of
/// which copy became the database.
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
/// reach (the `E_SYNC_RUNNING` SYS-005 refusal while a sync holds
/// the global flag (T-105), the `E_VALIDATION` SEC-003 rejection of
/// a path from nowhere, `E_NOT_A_REPO` for a folder outside any
/// repository, `E_PROJECT_UNAVAILABLE` for a refused probe, the
/// `E_VALIDATION` envelope of a §6.9 field, and the store fallbacks).
/// `error` level: the log line is the failure's durable half. The
/// typed outcomes that are NOT failures — the repo-root offer, the
/// duplicate message, the archive-restore offer — are success-shaped
/// responses and deliberately log nothing (nothing happened; the
/// journal records events, and a refusal wrote none), and a
/// *successful* add is the journal's own `project.added` entry,
/// written inside the operation's transaction (D-051) — never
/// duplicated into the file log (the info+user+ok unmirrored shape).
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
/// are the `E_SYNC_RUNNING` SYS-005 refusal while a sync holds the
/// global flag (T-105), the typed `E_VALIDATION` refusal of an id no
/// row matches (a stale screen's row), the shared base classifier's
/// SQL classes (`ProjectsError::Sql` through the list/delete reads,
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
/// answers are the `E_SYNC_RUNNING` SYS-005 refusal while a sync
/// holds the global flag (T-105), the typed `E_VALIDATION` refusal of
/// a list that does not name the active set exactly once each (a
/// stale screen's order), the shared base classifier's SQL classes
/// (`ProjectsError::Sql` through the ordered read and the
/// renumbering, `StorageError::Sql` through the journal insert — busy
/// exhaustion, a corrupt database, a constraint abort), the store
/// fallback of a corrupt projects cell (`ProjectsCorrupt`), and the
/// honest no-database fallback. A successful reorder is the journal's
/// own `project.changed` entry, written inside the operation's
/// transaction (D-051) — never duplicated into the file log (the
/// info+user+ok unmirrored shape); the verified no-op (a re-sent
/// unchanged order) writes nothing and logs nothing.
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

/// The `library_get` command failed (T-083, §7.10 Library module):
/// the `projects_list_failed` shape over the Library target. The
/// reachable §10 answers are the honest `NoDatabase` fallback
/// (recovery mode) and the classified store errors of the one
/// transaction — the shared SQLite base classifier for `Sql`, the
/// D-037 fallback for a corrupt vocabulary cell (class-only text,
/// SEC-008's rule: an instruction title or variant name never reaches
/// the log line).
pub(crate) fn library_get_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::library",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "reading the library tree failed"
    );
}

/// The `sync_run` command failed (T-118, §9.4, §10's every-error rule):
/// the `projects_list_failed` shape over the Sync target. The reachable
/// §10 answers are the `E_SYNC_RUNNING` SYS-004 refusal of a second
/// start (§9.4 phase 0 step 1), the honest `NoDatabase` fallback
/// (recovery mode), the typed `E_DISK_FULL`/`E_BACKUP_FAILED` envelopes
/// of the two in-phase aborts (their run rows carry the verdicts —
/// phase 1's tail rule, phase 2's step 2), and the classified store
/// errors of any phase's transactions — each of those last ones leaves
/// a terminal `aborted` row behind (the P3-4 ruling, D-138), so the
/// line and the row agree, with ONE carve-out: a refused abort leaves
/// the row `running` and takes the [`sync_run_abort_refused`] line
/// beside this one. Per-project outcomes never reach this edge:
/// `skipped`/`failed` projects are rows of a finished run, journaled by
/// phase 4 (D-051) and carried by [`sync_project_refused`] — a line
/// here would double the §7.7 record.
pub(crate) fn sync_run_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::sync",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "running a sync failed"
    );
}

/// The best-effort abort of a failed run was itself refused (T-118,
/// the P3-4 wrapper's degraded half): the run row stays `running` with
/// the flag already released — the startup `interrupted` recovery's
/// case — and this line is the only record of WHY no terminal row
/// exists. `error` level: the next reader of `sync_runs` meets a
/// running row that will never finish on its own; the original error
/// (`sync_run_failed`'s line, the same call) names the cause that
/// stopped the run, this one names the abort that could not land —
/// two facts, two lines, neither pretending the run finished.
pub(crate) fn sync_run_abort_refused(code: &str) {
    tracing::error!(
        target: "eunomia::sync",
        code,
        "writing the failed run's aborted row was refused; the run row stays running"
    );
}

/// A sync run stopped by its user before the backups (T-118): the D-25
/// gate's answer (`NeedsConfirmation` — the UI re-calls with
/// `confirm_empty=true` or declines) and the cancel point's (§9.4's
/// "declining cancels the whole run"; after phase 2 no stop exists).
/// `info` + the fixed class-only `reason`: the stop is a decision, not
/// a failure, and the run row's `aborted` details cell carries the same
/// sentence — the §7.7 journal of the run itself is phase 4's (an
/// aborted run writes no journal entries; the store's action
/// vocabulary has no cancel member — recorded in D-138).
pub(crate) fn sync_run_aborted(reason: &str) {
    tracing::info!(
        target: "eunomia::sync",
        reason,
        "the sync run was stopped before the backups"
    );
}

/// The crash-recovery sweep's record line (§9.4's "recovery after a
/// crash", T-258): N run rows a dead process left `running` were
/// marked `interrupted`. Emitted at the TWO moments no run can be
/// live — the `setup` hook (§9.4's literal "at startup") and the
/// recovery commands' reopen tail (a restored copy may carry the
/// corpses; recovery mode never managed a `Db`, so no run of this
/// process exists). `warn`: a process death is a degraded fact the
/// user should find in the log, but the launch itself is healthy —
/// the corpses are history now, the files are intact by §9.5's
/// atomicity, and the hashes were recalculated by the DB-022 startup
/// pass (T-101, already wired). The count is the whole payload:
/// SEC-008's class-only posture — run ids are DB keys, not user
/// content, and the count is the fact triage needs. A clean database
/// (zero corpses) logs nothing here: the sweep's no-op is not an
/// event (the `AlreadyPresent` posture).
pub(crate) fn sync_runs_interrupted(count: usize) {
    tracing::warn!(
        target: "eunomia::sync",
        count,
        "sync runs left running by a dead process were marked interrupted"
    );
}

/// The crash-recovery sweep's refusal line (§9.4, T-258): the corpse
/// rows stay `running` — exactly the state the sweep exists to clear,
/// so the next reader of `sync_runs` meets rows that will never
/// finish on their own. Emitted at the sweep's two callers (the
/// `setup` hook and the recovery commands' reopen tail). `error` per
/// the facade contract; both callers continue anyway (the D-046/
/// D-054 startup-housekeeping degrade), and the next restart retries
/// the idempotent sweep. The `code` is the store failure's §10
/// literal — the class-only fact, never the error's params.
pub(crate) fn sync_runs_interrupt_failed(code: &str) {
    tracing::error!(
        target: "eunomia::sync",
        code,
        "marking interrupted the runs left running by a dead process was refused; \
         the rows stay running and the next restart retries"
    );
}

/// One per-project refusal of a finished run (T-118, §10's every-error
/// rule: "every error is written to the log with its code, project,
/// path, and OS code"): the §10 code travels as the code literal, the
/// OS facts ride `details` when an OS call produced the refusal (the
/// row has no column for them — D-131's round-1 P3-5 and D-133 added
/// them to the outcomes FOR this line), and the message is the
/// refusal's own class-only Display (paths and codes, never file
/// content — SEC-008 holds at the producers). `failed` picks the
/// level: a skip is the deliberate warn of a project the run walked
/// past, a failure the error of a write that refused. The §7.7 journal
/// entries are phase 4's (D-051); this is the file log's half.
pub(crate) fn sync_project_refused(
    failed: bool,
    code: &str,
    os_code: Option<i32>,
    os_kind: Option<&str>,
    message: &str,
) {
    // `tracing::event!` takes its level as a macro-time constant, so
    // the two levels are two spelled events over the one payload.
    if failed {
        tracing::error!(
            target: "eunomia::sync",
            code,
            os_code = ?os_code,
            os_kind = ?os_kind,
            message,
            "a project's write of the run was refused"
        );
    } else {
        tracing::warn!(
            target: "eunomia::sync",
            code,
            os_code = ?os_code,
            os_kind = ?os_kind,
            message,
            "a project of the run was skipped"
        );
    }
}

/// The never-fatal aftermath of a finished run (T-118, D-134's
/// follow-up: "T-118's log lines consume RetentionReport/
/// JournalReport/DirtyReport"): the retention sweep's, the journal's
/// and the after-commit dirty pass's reported refusals, plus the
/// watcher's after-sync refresh (SYS-008) — counted and carried as
/// their own class-only lines (the reports are Display strings the
/// services built; SEC-008 holds at their producers). `warn`: the
/// verdict is committed, the summary was served, and the next trigger
/// retries everything that refused (D-122/D-121's at-least-once). The
/// `watch` param is the §10 code literal of the refresh refusal — the
/// class-only fact, never the error's params.
pub(crate) fn sync_run_degraded(
    retention: &[String],
    journal: &[String],
    dirty: Option<&str>,
    watch: Option<&str>,
) {
    tracing::warn!(
        target: "eunomia::sync",
        retention_failures = retention.len(),
        journal_failures = journal.len(),
        dirty_failure = dirty,
        watch_failure = watch,
        retention_lines = ?retention,
        journal_lines = ?journal,
        "the finished sync's housekeeping reported failures; the verdict stands"
    );
}

/// The `scan_start` command's ONE failure edge (T-076, the D-096
/// law): every `Err` path of the command — the `E_SYNC_RUNNING`
/// second-start refusal, the SEC-003 rejection, a vanished root,
/// the settings/registry read, the blocking-pool join — flows
/// through here with the error's own §7.10 vocabulary. `error`
/// level per §10's every-error rule. A *successful* scan logs
/// nothing: a read-only walk that answers its result is not an
/// event (the `AlreadyPresent` posture); the cancelled stop is the
/// decision's own info line ([`scan_cancelled`]).
pub(crate) fn scan_start_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::scan",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "running a repository scan failed"
    );
}

/// The `scan_commit` command's ONE failure edge (T-077, the D-096
/// law): every `Err` path of the command — the `E_SYNC_RUNNING`
/// SYS-005 refusal, the SEC-003 discovered-set rejection of a path no
/// completed scan reported, `E_PROJECT_UNAVAILABLE` for a folder that
/// vanished since the scan, `E_NOT_A_REPO` for one that is no longer
/// a root, the `E_VALIDATION` envelope of a §6.9 field, the store
/// fallbacks of the one-transaction batch (busy exhaustion, a corrupt
/// cell, a rolled-back row) and the blocking-pool joins — flows
/// through here with the error's own §7.10 vocabulary, in the
/// `project_add_failed` shape. `error` level per §10's every-error
/// rule. The typed non-add outcomes (`already_added`, `archived`,
/// `excluded`) and the empty selection are success-shaped answers and
/// log nothing; a successful commit is its own `project.added` journal
/// entries, written inside the operation's transaction (D-051) — never
/// duplicated into the file log (the info+user+ok unmirrored shape).
pub(crate) fn scan_commit_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::scan",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "adding the selected scanned repositories failed"
    );
}

/// A scan stopped by its user (T-076, ERR-002): the cancel token
/// latched and a walker thread observed it — everything the walk
/// accumulated was dropped and NOTHING was saved (the command wrote
/// nothing anywhere by construction; the answer's `Cancelled` arm
/// carries no payload). `info` + the class-only count of folders
/// the walk had decided by then: the stop is a decision, not a
/// failure (the `sync_run_aborted` posture), and the count is the
/// triage fact (SEC-008 — no paths ride the line).
pub(crate) fn scan_cancelled(checked: u32) {
    tracing::info!(
        target: "eunomia::scan",
        checked,
        "the repository scan was cancelled; nothing was saved"
    );
}

/// Emitting a `scan://progress` event failed (T-076): the walk
/// itself is unaffected — the emission is display-only, throttled
/// to at most ten per second, and the command's ANSWER carries the
/// terminal state the dropped event would have previewed.
/// `warn`, not `error`, for exactly the `availability_emit_failed`
/// reason: a missed UI update, not lost state. The tauri error is
/// the whole payload — it carries no user paths.
pub(crate) fn scan_progress_emit_failed(error: &tauri::Error) {
    tracing::warn!(
        target: "eunomia::scan",
        error = %error,
        "emitting a scan progress event failed; the scan continues"
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

/// The watch service's backend failed to START (D-137's follow-up 1,
/// D-155): `file_watch::spawn_service` refused to construct the
/// watcher (the instances limit at `inotify_init`, a resource error)
/// and nothing is spawned — no watching this session, the bus's
/// receiving half dropped with the refusal, later `try_send`s fail
/// harmlessly, and the app runs without external-change detection
/// until restart. `warn`, not `error`: the launch deliberately
/// continues (SYS-010's no-crash posture — an `error` line would
/// read as a failed launch in log triage, the
/// `db_backup_daily_failed` reasoning). The payload is the error's
/// own Display — which itself states the start failure, so the
/// message states only the consequence and never repeats it (the
/// round-1 P3-6 fix); the text is the backend's machine-generated
/// diagnostic, never file content (SEC-008).
pub(crate) fn file_watch_init_failed(error: &eunomia_fs::watcher::WatcherInitError) {
    tracing::warn!(
        target: "eunomia::file_watch",
        error = %error,
        "watching is disabled until the next launch"
    );
}

/// One pass of the watch service FAILED (D-137's follow-up 1, D-155)
/// — the §7.10 every-error rule at the fire-and-forget trigger that
/// has no caller to return to (startup, an availability change, a
/// debounced batch, the backend's lost-events re-check): the pass
/// recorded nothing and the next trigger re-runs it (at-least-once),
/// so the state is never lost, only late. The payload is the
/// error's own §7.10 vocabulary — the code literal, its
/// interpolation params, the OS facts — in the
/// `availability_check_failed` shape. An AWAITED pass failure is
/// deliberately NOT logged here, for either awaited seam
/// (`refresh_after_sync`, or its bounded mutation twin
/// `refresh_after_mutation`): the awaiting caller owns the durable
/// half — the sync pipeline's `sync_run_degraded` watch arm answers
/// for its calls, and the add flow's line is
/// [`file_watch_mutation_refresh_degraded`] (T-278) — so this edge
/// carries the fire-and-forget triggers' failures and every class the
/// caller OBSERVES is logged exactly once (the one window: a pass
/// failure the mutation budget outlives answers the caller
/// `FileWatchTimeout` and is logged nowhere else — the D-156
/// addendum's recorded boundary).
pub(crate) fn file_watch_pass_failed(error: &AppError) {
    tracing::error!(
        target: "eunomia::file_watch",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "a file-watch pass failed; nothing was recorded and the next trigger retries"
    );
}

/// Emitting the `state://file-changed` event failed (D-137's
/// follow-up 1, D-155): the pass itself succeeded — its diff was
/// computed — but the UI update did not leave the process, and the
/// cache deliberately stays unrecorded so the next trigger
/// re-detects and re-emits it (at-least-once, the diff contract in
/// `file_watch.rs`). `warn`, not `error`, for exactly the
/// `availability_emit_failed` reason: a missed UI update, not lost
/// state. The tauri error is the whole payload — it carries no user
/// paths.
pub(crate) fn file_watch_emit_failed(error: &tauri::Error) {
    tracing::warn!(
        target: "eunomia::file_watch",
        error = %error,
        "emitting the file-changed event failed; the cache stays stale and the \
         next pass re-emits"
    );
}

/// A full pass found target files it could not put under watch
/// (SYS-010, D-137's follow-up 1, D-155): the backend refused their
/// parent directories — `gone` of them because the directory is not
/// there, `limit` of them because the watch budget refused the arm,
/// `other` for refusals that prove nothing — so those files'
/// external edits are invisible until the backend arms them again
/// (every full pass retries unconditionally). ONE line per
/// degradation episode, at `warn`: the pass compares the fresh
/// refused set with the previously recorded one and speaks only when
/// a NON-EMPTY answer differs (the `log_rotation_failed` gate) — a
/// standing refusal repeats nothing, and a recovered pass (the set
/// emptied) logs nothing at all: the no-line policy for converged
/// passes, the healed convergence speaks for itself. The counts are
/// the whole payload — class-only, SEC-008: which files were lost is
/// the non-blocking notice's fact (the `projects_list` carrier),
/// not the log's.
pub(crate) fn file_watch_refused(gone: usize, limit: usize, other: usize) {
    tracing::warn!(
        target: "eunomia::file_watch",
        gone,
        limit,
        other,
        "target files are not being watched; their external edits are \
         invisible until the backend arms them again"
    );
}

/// The mutation-initiated watch refresh degraded (T-278, D-144's
/// silent-degrade gap; the D-156 addendum records the class set): the
/// awaiting command's own durable half for the seam it called — the
/// caller-side complement of [`file_watch_pass_failed`], whose
/// fire-and-forget gate (`done: None`) deliberately leaves every
/// awaited failure to its caller. Exactly three refusal classes reach
/// the add flow's committed arm, and all three warn here:
/// `FileWatchGone` (the bus gone — the backend refused to start; the
/// init warn at launch was a different episode, so this line is the
/// only record that the user's new project joins no watch this
/// session), the pass's own §10 failure answered within the budget
/// (the sync pipeline carries its equivalent through
/// `sync_run_degraded`'s watch arm), and `FileWatchTimeout` (the
/// budget's expiry — the most likely real-world refusal, a stuck
/// mount). Recovery mode's `NoDatabase` is not in the set: the
/// committed arm runs after the flow's own transaction, a database is
/// proven (D-144), so the class cannot occur here. `warn`, not
/// `error`: the project IS added, the verdict stands, and the next
/// watch anchor re-derives the set (the `db_backup_daily_failed`
/// reasoning — an `error` line would read as a failed add in log
/// triage). ONE line per command whose refresh refused — the command
/// is the episode (the `sync_run_degraded` per-run precedent for
/// awaited callers; the `log_rotation_failed` episode gates target
/// repeating producers, and the awaited seam answers at most once per
/// call) — so repeat adds in one session re-warn: each is a distinct
/// user answer that degraded. A healthy refresh logs nothing (the
/// no-line policy). The payload is the error's own §10 vocabulary —
/// the class rides `os_kind` for the seam's two class-only answers
/// and the code literal for the pass failure; nothing SEC-008-
/// protected (instruction text, file content) rides the line —
/// `params` is the error contract's curated user-visible set, empty
/// for today's pass errors (SEC-008) — and the borrowed `DbLocked`
/// code stays the D-037 family convention (the dedicated timeout-code
/// rule is the first surfacing consumer's follow-up — a log line
/// surfaces nothing). The exactly-once property is the CALLER'S
/// observed class: a pass failure the budget outlives answers
/// `FileWatchTimeout` here and is logged nowhere else (the service's
/// awaited gate keeps its silence, `file_watch.rs`'s `done.is_none()`
/// arm) — the one window the D-155 chain leaves open, recorded in the
/// D-156 addendum.
pub(crate) fn file_watch_mutation_refresh_degraded(error: &AppError) {
    tracing::warn!(
        target: "eunomia::file_watch",
        code = error.code.as_str(),
        params = ?error.params,
        os_code = ?error.details.as_ref().and_then(|details| details.os_code),
        os_kind = ?error.details.as_ref().and_then(|details| details.os_kind.as_deref()),
        os_message = ?error.details.as_ref().and_then(|details| details.os_message.as_deref()),
        "the watch refresh after the project mutation degraded; the \
         committed state stands and the next watch anchor re-derives \
         the set"
    );
}

/// A file operation on a project's library file — the SEC-008 core
/// event. It takes the operation code, the file's path and the numeric
/// ID of the entity involved (variant ID, project ID — whatever the
/// operation is about), and *nothing else*. There is no content
/// parameter, so the instruction text being saved or synced cannot leak
/// into the log — the module tests pin exactly that contract.
///
/// A tested, still-unused facet (the keep ruling D-163): the sync and
/// backup tasks landed without it — sync journals its file-write
/// outcomes in the `action_log` journal mirrored by [`action_logged`]
/// plus the `sync_*` diagnostic facets, and the phase-2 backups record
/// theirs in `backup_records` rows — so the prospective consumers are
/// the pending file-operation producers outside the sync journal (the
/// variant library tasks, `backup_restore`/BKP-007). The first such
/// producer either adopts this helper (removing the `allow`) or
/// consciously retires the primitive by its own ruling.
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
/// Same status as [`file_event`]: tested today, unused in production
/// (D-163). The landed sync/backup producers routed around it (the
/// journal and `backup_records`), leaving the `Synced`/`BackedUp`
/// members already superseded — the adopt-or-retire ruling decides
/// their fate along with the enum's.
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

/// The steps of the rotation writer's file switch (T-255), for
/// [`log_rotation_failed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RotationStep {
    /// The retention prune could not read the directory or remove an
    /// aged file.
    Prune,
    /// The next part file could not be opened.
    Open,
}

impl RotationStep {
    /// The word the log line carries (`step=…`).
    fn as_str(self) -> &'static str {
        match self {
            RotationStep::Prune => "prune",
            RotationStep::Open => "open",
        }
    }
}

/// The file log's own rotation degraded (T-255): the prune could not
/// read or delete, or the next part file could not be opened. `warn`,
/// emitted at most once per degradation episode — the writer keeps
/// writing into the current file (the 10 MB cap is exceeded, never the
/// launch) and retries the rotation on every following write, and a
/// successful rotation re-arms the gate — so the line marks the
/// degradation, not every refusal; a healed disk produces no second
/// line, the healed rotation speaks for itself. The payload is
/// the step code, the path and the OS error — paths and codes, the
/// SEC-008 vocabulary; no content exists at this seam to keep out.
/// Private to this module: the writer is the only producer, and the
/// event fires on `non_blocking`'s worker thread (see the module docs
/// for the testing boundary that follows from that).
fn log_rotation_failed(step: RotationStep, path: &Path, error: &io::Error) {
    tracing::warn!(
        target: "eunomia::log",
        step = step.as_str(),
        path = %path.display(),
        error = %error,
        "the file log could not rotate; writing continues into the current file"
    );
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
        // variant's database ID. The SEC-008 criterion, pinned at the
        // facade: this test stands in for a producer and drives
        // `file_event` the way one would (the content stays in the
        // caller's local scope; only the path, the operation code and
        // the ID cross into the logging API — there is no parameter
        // that could carry the content).
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

        // Act: a variant save and a sync run as `file_event` would
        // log them — the text exists in scope the whole time and
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
        // portable stand-in for permission failures (the writer's open
        // also tries to create the directory itself, as the crate's
        // appender did, so this covers that arm too).
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
    fn interrupted_recovery_lines_carry_the_count_and_the_code() {
        // The §9.4 crash-recovery pair (T-258): the sweep's record is
        // a `warn` carrying the COUNT alone (class-only, SEC-008 — no
        // run ids, no content), and its refusal is an `error` carrying
        // the §10 code literal that survives the strictest config,
        // saying the rows stay running and the next restart retries.
        let (_temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Warn)
            .expect("building the subscriber must work");

        tracing::subscriber::with_default(subscriber, || {
            sync_runs_interrupted(2);
            sync_runs_interrupt_failed("E_DB_LOCKED");
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains(" WARN ")
                && log.contains("count=2")
                && log.contains("were marked interrupted"),
            "the record line must be a warn with the count, got: {log}"
        );
        assert!(
            log.contains(" ERROR ")
                && log.contains("code=\"E_DB_LOCKED\"")
                && log.contains("the rows stay running and the next restart retries"),
            "the refusal line must be an error with the code literal, got: {log}"
        );
    }

    #[test]
    fn file_watch_lines_carry_levels_codes_and_class_counts() {
        // The watcher's facade quartet (D-137's follow-up 1, D-155),
        // each driven the way its producer calls it: the init degrade
        // and the emit degrade are warns (the launch and the pass
        // deliberately continue), the pass failure is the §10
        // every-error line with the code literal, and the
        // refused-watch warn carries the per-class COUNTS alone —
        // WHICH files were lost is the non-blocking notice's fact
        // (the `projects_list` carrier), not the log's (SEC-008:
        // counts and codes).
        let (_temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Warn)
            .expect("building the subscriber must work");

        tracing::subscriber::with_default(subscriber, || {
            file_watch_init_failed(&eunomia_fs::watcher::WatcherInitError::new(
                "too many open inotify instances (EMFILE-class probe)".to_owned(),
            ));
            let mut error = AppError::new(crate::errors::ErrorCode::DbLocked);
            error.details = Some(crate::errors::ErrorDetails {
                os_code: None,
                os_kind: Some("StorageError".to_owned()),
                os_message: Some("database is locked".to_owned()),
            });
            file_watch_pass_failed(&error);
            file_watch_emit_failed(&tauri::Error::WindowLabelAlreadyExists("main".to_owned()));
            file_watch_refused(1, 2, 0);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains(" WARN ")
                && log.contains("the file watcher backend failed to start")
                && log
                    .matches("the file watcher backend failed to start")
                    .count()
                    == 1
                && log.contains("watching is disabled until the next launch"),
            "the init degrade must be a warn whose message names the consequence \
             exactly once (the error field owns the start-failure text), got: {log}"
        );
        assert!(
            log.contains("EMFILE-class probe"),
            "the init line carries the backend's own class-only diagnostic, got: {log}"
        );
        assert!(
            log.contains(" ERROR ")
                && log.contains("code=\"E_DB_LOCKED\"")
                && log.contains("the next trigger retries"),
            "the pass failure must be an error with the §10 code literal, got: {log}"
        );
        assert!(
            log.contains(" WARN ")
                && log.contains("a window with label `main` already exists")
                && log.contains("the cache stays stale and the next pass re-emits"),
            "the emit degrade must be a warn with the tauri error's own text, got: {log}"
        );
        assert!(
            log.contains(" WARN ")
                && log.contains("gone=1")
                && log.contains("limit=2")
                && log.contains("other=0")
                && log.contains("invisible until the backend arms them again"),
            "the refused-watch warn must carry the per-class counts, got: {log}"
        );
    }

    #[test]
    fn the_mutation_refresh_degrade_line_warns_each_refusal_class() {
        // T-278 (the D-156 addendum's class set): the awaited
        // mutation refresh's refusal is the CALLER's durable half —
        // three classes reach the add flow's committed arm
        // (`FileWatchGone`, the pass's §10 failure,
        // `FileWatchTimeout`), and one warn per degraded command
        // carries the class in the §10 payload: the kind for the
        // seam's two class-only answers, the code literal plus OS
        // facts for the pass failure. The two seam errors here are
        // built in the exact shape the seams answer (the ipc seam
        // tests pin the real answers; the ipc call-site tests drive
        // the real seams through the committed arm); the pass-failure
        // error is the synthetic §10 form the D-155 logging tests use
        // — a real classified pass error is not ipc-constructible
        // (the D-139 fake-backend premise, and `file_watch.rs` is
        // frozen for T-104).
        let (_temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Warn)
            .expect("building the subscriber must work");

        tracing::subscriber::with_default(subscriber, || {
            let gone = {
                let mut error = AppError::new(crate::errors::ErrorCode::DbLocked);
                error.details = Some(crate::errors::ErrorDetails {
                    os_code: None,
                    os_kind: Some("FileWatchGone".to_owned()),
                    os_message: Some("the file-watch service is not running".to_owned()),
                });
                error
            };
            file_watch_mutation_refresh_degraded(&gone);
            let timeout = {
                let mut error = AppError::new(crate::errors::ErrorCode::DbLocked);
                error.details = Some(crate::errors::ErrorDetails {
                    os_code: None,
                    os_kind: Some("FileWatchTimeout".to_owned()),
                    os_message: Some(
                        "the watch pass did not answer within the mutation budget".to_owned(),
                    ),
                });
                error
            };
            file_watch_mutation_refresh_degraded(&timeout);
            let pass = {
                let mut error = AppError::new(crate::errors::ErrorCode::DbCorrupt);
                error
                    .params
                    .insert("path".to_owned(), "/db/here".to_owned());
                error.details = Some(crate::errors::ErrorDetails {
                    os_code: Some(11),
                    os_kind: Some("DatabaseCorrupt".to_owned()),
                    os_message: Some("database disk image is malformed".to_owned()),
                });
                error
            };
            file_watch_mutation_refresh_degraded(&pass);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert_eq!(
            log.matches(" WARN ").count(),
            3,
            "one warn per degraded command — three classes driven, got: {log}"
        );
        assert!(
            log.contains("os_kind=Some(\"FileWatchGone\")")
                && log.contains("the file-watch service is not running"),
            "the gone class rides its kind and class-only message, got: {log}"
        );
        assert!(
            log.contains("os_kind=Some(\"FileWatchTimeout\")")
                && log.contains("the watch pass did not answer within the mutation budget"),
            "the timeout class rides its kind and class-only message, got: {log}"
        );
        assert!(
            log.contains("code=\"E_DB_CORRUPT\"")
                && log.contains("os_code=Some(11)")
                && log.contains("DatabaseCorrupt")
                && log.contains("database disk image is malformed"),
            "the pass failure rides the §10 code literal and OS facts, got: {log}"
        );
        assert!(
            log.contains("params={\"path\": \"/db/here\"}"),
            "the params channel renders the error contract's curated user-visible \
             set verbatim — the payload's one user-visible field, pinned here (the \
             round-1 review's F5): SEC-008-protected content (instruction text, \
             file content) has no channel onto the line at all, got: {log}"
        );
        assert_eq!(
            log.matches("the watch refresh after the project mutation degraded")
                .count(),
            3,
            "the consequence names itself on every class's line, got: {log}"
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
    fn the_scan_commit_failure_line_carries_the_code_and_payload() {
        // The scan commit's §10 edge (T-077), in the
        // `project_add_failed` pin shape: the commit's own rejection
        // — a path no completed scan reported (`E_VALIDATION` naming
        // the path field, the SEC-003 reason) — and a folder that
        // vanished since the scan (`E_PROJECT_UNAVAILABLE` with the
        // path in params and the OS refusal in details) both reach the
        // log line with their code literals. The typed outcomes and
        // the empty selection log nothing; that half is structural (no
        // tracing call exists on those paths — they return `Ok`
        // before the logging edge).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Error)
            .expect("building the subscriber must work");
        let mut unreported = AppError::new(crate::errors::ErrorCode::Validation);
        unreported
            .params
            .insert("field".to_owned(), "path".to_owned());
        unreported
            .params
            .insert("value".to_owned(), "/repos/never-scanned".to_owned());
        unreported.params.insert(
            "reason".to_owned(),
            "must be a repository the last completed scan reported (SEC-003)".to_owned(),
        );
        let mut vanished = AppError::new(crate::errors::ErrorCode::ProjectUnavailable);
        vanished
            .params
            .insert("path".to_owned(), "/repos/gone".to_owned());
        let vanished = vanished.with_os_error(&std::io::Error::from_raw_os_error(2));

        tracing::subscriber::with_default(subscriber, || {
            scan_commit_failed(&unreported);
            scan_commit_failed(&vanished);
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("code=\"E_VALIDATION\"")
                && log.contains("/repos/never-scanned")
                && log.contains("SEC-003"),
            "the SEC-003 rejection's line carries its code, the path and the reason, got: {log}"
        );
        assert!(
            log.contains("code=\"E_PROJECT_UNAVAILABLE\"")
                && log.contains("/repos/gone")
                && log.contains("os_code=Some(2)"),
            "the vanished folder's line carries its code, the path and the OS refusal, got: {log}"
        );
        assert_eq!(
            log.matches("adding the selected scanned repositories failed")
                .count(),
            2,
            "each failure says what failed, exactly once per call, got: {log}"
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

    #[test]
    fn protective_copy_retention_lines_carry_counts_and_path_facts() {
        // The T-253 facade pair, driven the way the pass calls them:
        // the success line carries the per-kind counts, the failure
        // line the path and OS cause (SEC-008: numbers and paths — a
        // housekeeping pass has no content to begin with).
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");

        tracing::subscriber::with_default(subscriber, || {
            db_backup_retention(&crate::retention::BackupRetentionCounts {
                pre_migration: 2,
                pre_recovery: 1,
            });
            let failing = root
                .join("db-backups")
                .join("pre-recovery-20261121T120000000Z.sqlite3");
            db_backup_retention_failed(
                &failing,
                &std::io::Error::new(std::io::ErrorKind::PermissionDenied, "probe"),
            );
        });
        drop(guard);

        let log = read_log_file(&root);
        assert!(
            log.contains("pre_migration=2") && log.contains("pre_recovery=1"),
            "the success line must carry the per-kind removal counts, got: {log}"
        );
        assert!(
            log.contains("protective-copy retention removed"),
            "the success line must say what happened, got: {log}"
        );
        assert!(
            log.contains("db-backups") && log.contains("probe"),
            "the failure line must carry the path and the OS cause, got: {log}"
        );
        assert!(
            log.contains("WARN") && log.contains("protective-copy retention failed"),
            "the failure line must be a warn naming the failed pass, got: {log}"
        );
        drop(temp);
    }

    #[test]
    fn building_the_appender_prunes_dated_logs_beyond_five_files() {
        // The startup half of the count cap (T-253): the prune runs
        // when the writer is built, so a directory holding seven dated
        // files must come down to the cap before today's file is
        // appended to. The prune orders by the parsed (date, part)
        // stamps — fully deterministic, unlike the crate's
        // creation-time tie-breaking — so the survivors are pinned by
        // name: the four newest seeds plus today's live file. The
        // foreign seeds (a note, a `.bak`, a leading-zero part, the
        // basic-format date) are not the writer's grammar and must
        // survive untouched (D-129's rule: a file whose age cannot be
        // established has no age to judge).
        let (temp, root) = logs_dir();
        for day in 1..=7 {
            fs::write(
                root.join(format!("eunomiasync.2020-01-0{day}.log")),
                b"an old day's log",
            )
            .expect("seeding an old dated log must work");
        }
        let foreign = [
            root.join("notes.txt"),
            root.join("eunomiasync.2026-11-22.log.bak"),
            root.join("eunomiasync.2026-11-22.01.log"),
            root.join("eunomiasync.20261122.log"),
        ];
        for path in &foreign {
            fs::write(path, b"a foreign file").expect("seeding a foreign file must work");
        }

        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("rotation probe event");
        });
        drop(guard);

        let files = dated_log_files(&root);
        assert_eq!(
            files.len(),
            5,
            "seven seeded files plus today's must leave exactly five dated files"
        );
        let names: Vec<String> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert!(
            names.contains(&"eunomiasync.2020-01-04.log".to_owned())
                && names.contains(&"eunomiasync.2020-01-07.log".to_owned())
                && !names.contains(&"eunomiasync.2020-01-01.log".to_owned()),
            "the newest four seeds survive, the oldest three go — the prune is \
             deterministic by name stamp, got: {names:?}"
        );
        for path in &foreign {
            assert_eq!(
                fs::read_to_string(path).expect("the foreign file must survive untouched"),
                "a foreign file",
                "a foreign name is never the prune's to judge: {}",
                path.display()
            );
        }
        let live = files
            .iter()
            .find(|path| {
                fs::read_to_string(path)
                    .map(|content| content.contains("rotation probe event"))
                    .unwrap_or(false)
            })
            .expect("the file the subscriber wrote to must be among the survivors");
        // The live file is today's, and it is the one pruning can never
        // remove: it is created after the prune, newest by construction.
        assert!(
            live.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("eunomiasync.")),
            "the live file keeps the §5.5 name shape"
        );
        drop(temp);
    }

    #[test]
    fn a_restart_at_a_full_cap_deletes_nothing() {
        // The round-1 review's P2-2 edge: a restart over a directory
        // already at the cap whose newest file is today's creates no
        // replacement file, so the construction prune must not delete
        // one either — the "keep n, then create" reasoning applies only
        // when a file is about to be created.
        let (temp, root) = logs_dir();
        let today = today_stamp();
        for day in 1..=4 {
            fs::write(
                root.join(format!("eunomiasync.2020-01-0{day}.log")),
                b"an old day's log",
            )
            .expect("seeding an old dated log must work");
        }
        fs::write(
            root.join(format!("eunomiasync.{today}.log")),
            "today's existing tail",
        )
        .expect("seeding today's part must work");

        let mut writer = CappedDailyWriter::new(&root, MAX_LOG_FILES, 100)
            .expect("the writer must build over the full directory");
        writer
            .write_all(b"a line that still fits\n")
            .expect("the write must land");
        writer.flush().expect("flush must work");

        let names: Vec<String> = part_files(&root)
            .into_iter()
            .map(|(_, _, path)| path.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            names.len(),
            5,
            "a restart over a full cap keeps all five files, got: {names:?}"
        );
        for day in 1..=4 {
            assert!(
                names.contains(&format!("eunomiasync.2020-01-0{day}.log")),
                "no old day is deleted by a mere restart"
            );
        }
        let tail = fs::read_to_string(root.join(format!("eunomiasync.{today}.log")))
            .expect("today's part must be readable");
        assert!(
            tail.contains("today's existing tail") && tail.contains("a line that still fits"),
            "the restart continues today's part in order, got: {tail:?}"
        );
        drop(temp);
    }

    /// The log files of `dir` split by part, matched **by hand** — an
    /// oracle independent of the production name code: the §5.5 shape
    /// `eunomiasync.<YYYY-MM-DD>.log` (part 1) and the same-day part
    /// shape `eunomiasync.<YYYY-MM-DD>.<N>.log` where `N` is ≥ 2 in
    /// decimal without leading zeros. Anything else in the directory is
    /// not a part and stays invisible to this helper.
    fn part_files(dir: &Path) -> Vec<(String, Option<u32>, PathBuf)> {
        let mut files = Vec::new();
        for entry in fs::read_dir(dir).expect("the logs directory must be readable") {
            let path = entry.expect("every logs entry must be readable").path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(rest) = name
                .strip_prefix("eunomiasync.")
                .and_then(|rest| rest.strip_suffix(".log"))
            else {
                continue;
            };
            // `<date>` alone is part 1; `<date>.<N>` (N ≥ 2, no leading
            // zeros) is a same-day part — matched by hand so the oracle
            // stays independent of the production name code.
            let (date, part) = match rest.rsplit_once('.') {
                None => (rest, None),
                Some((date, digits)) => {
                    let canonical = !digits.starts_with('0')
                        && digits != "1"
                        && digits.bytes().all(|b| b.is_ascii_digit());
                    if !canonical {
                        continue;
                    }
                    let Ok(part) = digits.parse::<u32>() else {
                        continue;
                    };
                    (date, Some(part))
                }
            };
            let bytes = date.as_bytes();
            let shaped = bytes.len() == 10
                && bytes[4] == b'-'
                && bytes[7] == b'-'
                && bytes
                    .iter()
                    .enumerate()
                    .all(|(i, b)| b.is_ascii_digit() || ((i == 4 || i == 7) && *b == b'-'));
            if !shaped {
                continue;
            }
            files.push((date.to_owned(), part, path));
        }
        files.sort();
        files
    }

    #[test]
    fn writing_past_ten_megabytes_rolls_into_a_same_day_part() {
        // The owner ruling's per-file half (T-255, D-129: "5 файлов ×
        // 10 МБ"): one UTC day whose volume passes 10 MB must roll into
        // a second file named `eunomiasync.<date>.2.log`, and no file
        // of the day may exceed the 10 MB cap (beyond a single line
        // that is oversized on its own). Driven end to end through the
        // production seam with the real constants — the small-cap
        // suite covers the arithmetic; this one pins that the shipped
        // cap is the ruling's ten megabytes. The 10 MB literal comes
        // from the ruling, not from the code's constant, so a drifted
        // constant fails here.
        const RULING_CAP: u64 = 10 * 1024 * 1024;
        let (temp, root) = logs_dir();
        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");

        // ~1 KiB per event message; 10 MiB + 64 events of headroom
        // crosses the cap with margin for the fmt layer's own bytes.
        let line = "0123456789abcdef".repeat(64);
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..(10 * 1024 + 64) {
                tracing::info!("{line}");
            }
        });
        drop(guard);

        let parts = part_files(&root);
        let dates: Vec<&str> = parts.iter().map(|(date, _, _)| date.as_str()).collect();
        assert!(
            dates.len() >= 2 && dates.iter().all(|date| *date == dates[0]),
            "a day past 10 MB must roll into same-day parts of one date, \
             found: {parts:?}"
        );
        let first = fs::metadata(&parts[0].2).expect("part 1 must exist").len();
        assert!(
            first <= RULING_CAP,
            "part 1 must not exceed the 10 MB cap (got {} bytes)",
            first
        );
        assert!(
            first >= RULING_CAP - 1024 * 1024,
            "part 1 must have filled to near the cap before rolling \
             (got {first} bytes) — an early split means the cap is not \
             the ruling's ten megabytes"
        );
        for (_, _, path) in &parts {
            let size = fs::metadata(path).expect("every part must exist").len();
            assert!(
                size <= RULING_CAP,
                "no part may exceed the 10 MB cap: {path:?} is {size} bytes"
            );
        }
        drop(temp);
    }

    /// Today's UTC stamp, computed test-side straight from jiff — the
    /// independent calendar oracle for seeding and asserting on
    /// today-named files.
    fn today_stamp() -> String {
        jiff::Timestamp::now()
            .to_zoned_datetime(jiff::tz::TimeZone::UTC)
            .date()
            .to_string()
    }

    /// A [`FileKey`] built from a literal — the pure suites' input.
    fn key(date: &str, part: u32) -> FileKey {
        FileKey {
            date: date.to_owned(),
            part,
        }
    }

    #[test]
    fn file_names_follow_the_spec_shape_and_the_part_scheme() {
        // Part 1 is the spec §5.5 literal — pinned as a literal, so a
        // mutated prefix/suffix or date placement fails here; parts ≥ 2
        // are the T-255 same-day scheme (the ruling's reading, D-entry).
        assert_eq!(
            log_file_name(&key("2026-11-22", 1)),
            "eunomiasync.2026-11-22.log"
        );
        assert_eq!(
            log_file_name(&key("2026-11-22", 2)),
            "eunomiasync.2026-11-22.2.log"
        );
        assert_eq!(
            log_file_name(&key("2026-11-22", 10)),
            "eunomiasync.2026-11-22.10.log"
        );
    }

    #[test]
    fn name_parsing_accepts_only_the_canonical_grammar() {
        // The prune trusts exactly this grammar (D-129's rule: a name
        // whose age cannot be established is never deleted), so the
        // accept/reject table is the safety net of the whole retention.
        let accepted = [
            ("eunomiasync.2026-11-22.log", key("2026-11-22", 1)),
            ("eunomiasync.2024-02-29.log", key("2024-02-29", 1)), // a real leap day
            ("eunomiasync.2026-11-22.2.log", key("2026-11-22", 2)),
            ("eunomiasync.2026-11-22.10.log", key("2026-11-22", 10)),
        ];
        for (name, expected) in accepted {
            assert_eq!(parse_log_file_name(name), Some(expected), "{name}");
        }
        let rejected = [
            // Impossible calendar dates.
            "eunomiasync.2026-02-30.log",
            "eunomiasync.2026-13-01.log",
            // Non-canonical shapes.
            "eunomiasync.2026-11-2.log",    // unpadded day
            "eunomiasync.2026-1-22.log",    // unpadded month
            "eunomiasync.20261122.log",     // no dashes
            "eunomiasync.2026-11-22.1.log", // part 1 is the unsuffixed name
            "eunomiasync.2026-11-22.02.log",
            "eunomiasync.2026-11-22.0.log",
            "eunomiasync.2026-11-22.2g.log",
            "eunomiasync.2026-11-22..log",
            "eunomiasync.2026-11-22.log.bak",
            "eunomiasync.2026-11-22.txt",
            "eunomiasync..log",
            "app.2026-11-22.log",
            "2026-11-22.log",
        ];
        for name in rejected {
            assert_eq!(
                parse_log_file_name(name),
                None,
                "`{name}` is not a canonical log file name"
            );
        }
    }

    #[test]
    fn the_size_decision_boundary_is_exact() {
        // The ruling's arithmetic, driven as a table over the pure
        // decision: a write landing exactly at the cap stays (the file
        // reaches it precisely; the NEXT write rolls), a lone
        // oversized write on an empty file stays (no empty-part spin),
        // and the date change wins over size.
        let current = key("2026-11-22", 3);
        let cases = [
            // (current_len, incoming, now_date) → expected target
            (0_u64, 10_usize, "2026-11-22", key("2026-11-22", 3)), // fresh file, fits
            (10, 10, "2026-11-22", key("2026-11-22", 3)),          // fits
            (90, 10, "2026-11-22", key("2026-11-22", 3)),          // lands exactly at the cap
            (91, 10, "2026-11-22", key("2026-11-22", 4)),          // would pass the cap
            (100, 1, "2026-11-22", key("2026-11-22", 4)), // at the cap: the next write rolls
            (0, 1_000, "2026-11-22", key("2026-11-22", 3)), // lone oversized line
            (50, 1_000, "2026-11-22", key("2026-11-22", 4)), // rotate first, land alone there
            (10, 10, "2026-11-23", key("2026-11-23", 1)), // the day changed
            (100, 10, "2026-11-23", key("2026-11-23", 1)), // and it wins over size
        ];
        for (current_len, incoming, now_date, expected) in cases {
            assert_eq!(
                next_write_target(&current, current_len, now_date, incoming, 100),
                expected,
                "len={current_len} incoming={incoming} now={now_date}"
            );
        }
    }

    #[test]
    fn the_prune_plan_keeps_the_newest_and_the_protected() {
        // The count half's verdict as a table: oldest beyond `keep` go,
        // protected keys never do, and same-day parts order numerically
        // (`.10` is newer than `.9`, not lexicographically older).
        let d1 = key("2020-01-01", 1);
        let p1 = key("2020-01-02", 1);
        let p2 = key("2020-01-02", 2);
        let p9 = key("2020-01-02", 9);
        let p10 = key("2020-01-02", 10);
        assert_eq!(
            prune_plan(&[d1.clone(), p1.clone(), p9.clone()], 2, &[]),
            vec![d1.clone()],
            "the oldest beyond the keep count goes"
        );
        assert_eq!(
            prune_plan(
                &[d1.clone(), p1.clone(), p2.clone(), p9.clone(), p10.clone()],
                3,
                &[]
            ),
            vec![d1.clone(), p1.clone()],
            "parts order numerically: .2 is older than .10, so d1 and \
             part 1 go — a lexicographic sort would pick .10 instead"
        );
        assert_eq!(
            prune_plan(
                &[d1.clone(), p1.clone(), p9.clone()],
                2,
                std::slice::from_ref(&d1)
            ),
            vec![p1.clone()],
            "protection wins over age: the protected oldest stays and the \
             oldest *deletable* goes, so the count still lands on `keep` — \
             the realistic shape is the open file being protected while \
             being the newest, this pins the tie-break under clock skew"
        );
        assert_eq!(
            prune_plan(&[d1.clone(), p1.clone()], 1, &[d1.clone(), p1.clone()]),
            Vec::<FileKey>::new(),
            "nothing deletable: the next prune converges instead"
        );
        assert_eq!(
            prune_plan(&[p1.clone(), p2.clone()], 5, &[]),
            Vec::<FileKey>::new(),
            "below the keep count nothing is deleted"
        );
        assert_eq!(
            prune_plan(&[p1.clone(), p1.clone(), p2.clone()], 2, &[]),
            Vec::<FileKey>::new(),
            "a duplicated key is the same file: two unique files against \
             keep 2 delete nothing (counting duplicates would phantom-delete)"
        );
    }

    #[test]
    fn a_chatty_day_through_the_subscriber_rolls_within_a_small_cap() {
        // The small-cap twin of the ten-megabyte pin: the same
        // production seam with a tiny cap, so the roll arithmetic runs
        // many times in one test. What any reader of the directory
        // sees: same-day parts of one date, no part above the cap, the
        // earliest line in part 1 and the latest in the last part.
        let cap = 4_000u64;
        let (temp, root) = logs_dir();
        let (subscriber, guard) =
            build_file_subscriber_capped(&root, LogLevel::Info, MAX_LOG_FILES, cap)
                .expect("building the subscriber must work");

        tracing::subscriber::with_default(subscriber, || {
            for i in 0..120 {
                tracing::info!("roll probe {i:04} {}", "-".repeat(64));
            }
        });
        drop(guard);

        let parts = part_files(&root);
        assert!(
            parts.len() >= 2,
            "120 events past a 4 KB cap must roll into parts, found: {parts:?}"
        );
        assert!(
            parts.len() <= 10,
            "a 4 KB cap over ~130-byte lines yields a handful of parts — a \
             writer that rotated on every write would produce ~120 (found {})",
            parts.len()
        );
        assert!(
            parts
                .iter()
                .all(|(_, _, path)| fs::metadata(path).expect("part must exist").len() <= cap),
            "no part may exceed the cap, found: {parts:?}"
        );
        assert!(
            parts.iter().all(|(date, _, _)| *date == parts[0].0),
            "all parts of one run belong to one date, found: {parts:?}"
        );
        let first = fs::read_to_string(&parts[0].2).expect("part 1 must be readable");
        let last =
            fs::read_to_string(&parts[parts.len() - 1].2).expect("the last part must be readable");
        assert!(
            first.contains("roll probe 0000") && !first.contains("roll probe 0119"),
            "part 1 carries the day's earliest lines, got: {} bytes",
            first.len()
        );
        assert!(
            last.contains("roll probe 0119"),
            "the last part carries the latest lines"
        );
        drop(temp);
    }

    #[test]
    fn a_restart_continues_todays_highest_part() {
        // Idempotency across restarts (the make-operations-idempotent
        // rule): a second writer over the same directory appends to
        // today's highest part at its true length — no new file below
        // the cap, no data loss — and rolls only after continuing past
        // the cap. Driven directly through `io::Write` for exact byte
        // accounting.
        let (temp, root) = logs_dir();
        let cap = 100u64;
        {
            let mut writer = CappedDailyWriter::new(&root, MAX_LOG_FILES, cap)
                .expect("the first writer must build");
            for i in 0..25 {
                writer
                    .write_all(format!("run-1-{i:03}\n").as_bytes())
                    .expect("the first run's writes must land");
            }
            writer.flush().expect("flush must work");
        }
        let sizes_after_run1: Vec<u64> = part_files(&root)
            .iter()
            .map(|(_, _, path)| fs::metadata(path).unwrap().len())
            .collect();
        assert_eq!(
            sizes_after_run1,
            vec![100, 100, 50],
            "25 ten-byte lines at a 100-byte cap: two full parts and a \
             50-byte tail, found: {sizes_after_run1:?}"
        );

        let mut writer = CappedDailyWriter::new(&root, MAX_LOG_FILES, cap)
            .expect("the second writer must build over the same directory");
        writer
            .write_all(b"run-2-000\n")
            .expect("the restart's write must land");
        writer.flush().expect("flush must work");
        let parts = part_files(&root);
        assert_eq!(
            parts.len(),
            3,
            "one line below the cap continues the tail part — no new file, \
             found: {parts:?}"
        );
        let tail = fs::read_to_string(&parts[2].2).expect("the tail part must be readable");
        assert!(
            tail.contains("run-1-024\nrun-2-000\n"),
            "the restart appends after the first run's last line, got: {tail:?}"
        );

        // Continuing past the cap rolls — the restarted writer obeys
        // the same arithmetic from the part's true length.
        for i in 1..6 {
            writer
                .write_all(format!("run-2-{i:03}\n").as_bytes())
                .expect("the second run's writes must land");
        }
        writer.flush().expect("flush must work");
        let parts = part_files(&root);
        assert_eq!(
            parts.len(),
            4,
            "crossing the cap after the restart rolls into the next part, \
             found: {parts:?}"
        );
        let last = fs::read_to_string(&parts[3].2).expect("the new part must be readable");
        assert!(
            last.contains("run-2-005"),
            "the post-cap lines land in the new part, got: {last:?}"
        );
        drop(temp);
    }

    #[test]
    fn a_restart_over_a_full_day_rolls_on_its_first_write() {
        // The crash-recovery edge of the same idempotency: a previous
        // run left today's part exactly at the cap (or an interrupted
        // rotation left it full), and the restart must not append past
        // the cap — the first write rolls into the next part and the
        // full part is left byte-identical.
        let (temp, root) = logs_dir();
        let today = today_stamp();
        let full = root.join(format!("eunomiasync.{today}.log"));
        fs::write(&full, "x".repeat(100)).expect("seeding a full part must work");

        let mut writer = CappedDailyWriter::new(&root, MAX_LOG_FILES, 100)
            .expect("the writer must build over the full part");
        writer
            .write_all(b"the first line after the restart\n")
            .expect("the write must land");
        writer.flush().expect("flush must work");

        assert_eq!(
            fs::metadata(&full)
                .expect("the full part must survive")
                .len(),
            100,
            "a part already at the cap is never appended past it"
        );
        assert_eq!(
            fs::read_to_string(&full).expect("the full part must be readable"),
            "x".repeat(100),
            "and its content is untouched"
        );
        let second = root.join(format!("eunomiasync.{today}.2.log"));
        assert!(
            fs::read_to_string(&second)
                .expect("part 2 must exist")
                .contains("the first line after the restart"),
            "the restart's first line lands in the next part"
        );
        drop(temp);
    }

    #[test]
    fn parts_count_toward_the_five_file_cap() {
        // The count half over the writer's own rotation (T-253's seed
        // pattern, now with a same-day part in the set): four old dated
        // files plus today's full part is the five-file status quo; the
        // rotation that opens part 2 prunes the oldest day first and
        // lands the directory back at exactly five files.
        let (temp, root) = logs_dir();
        let today = today_stamp();
        for day in 1..=4 {
            fs::write(
                root.join(format!("eunomiasync.2020-01-0{day}.log")),
                b"an old day's log",
            )
            .expect("seeding an old dated log must work");
        }
        fs::write(
            root.join(format!("eunomiasync.{today}.log")),
            "x".repeat(100),
        )
        .expect("seeding today's full part must work");

        let mut writer = CappedDailyWriter::new(&root, MAX_LOG_FILES, 100)
            .expect("the writer must build over the seeded directory");
        writer
            .write_all(b"the line that tips the day over the cap\n")
            .expect("the write must land");
        writer.flush().expect("flush must work");

        let mut names: Vec<String> = part_files(&root)
            .into_iter()
            .map(|(_, _, path)| path.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "eunomiasync.2020-01-02.log".to_owned(),
                "eunomiasync.2020-01-03.log".to_owned(),
                "eunomiasync.2020-01-04.log".to_owned(),
                format!("eunomiasync.{today}.2.log"),
                format!("eunomiasync.{today}.log"),
            ],
            "the oldest day goes, the four newest plus the new part stay — \
             exactly five files (lexicographic name order: `.2.log` sorts \
             before the part-1 name)"
        );
        drop(temp);
    }

    #[test]
    fn a_blocked_rotation_degrades_reports_once_and_loses_no_lines() {
        // The never-fatal half (D-032's policy applied to rotation): a
        // next part file that cannot be opened — here a directory sits
        // on its name, the portable stand-in for a refused create —
        // must not lose the line, abort, or stop the log: the writer
        // keeps writing into the current file (past the cap — the
        // honest overrun), reports the degradation exactly once through
        // the facade, and retries silently afterwards. The warn is
        // observed through a real subscriber over the same directory
        // (the writer under test is driven directly, so its facade
        // event fires on this thread — see the module docs for the
        // worker-thread boundary of the production path).
        let (temp, root) = logs_dir();
        let today = today_stamp();
        let blocked = root.join(format!("eunomiasync.{today}.2.log"));
        fs::create_dir(&blocked).expect("blocking the part-2 name must work");

        // The observing subscriber: the production seam with a 1 MB cap
        // so it never rotates itself and its own lines stay in part 1.
        let (subscriber, guard) =
            build_file_subscriber_capped(&root, LogLevel::Info, MAX_LOG_FILES, 1024 * 1024)
                .expect("building the observing subscriber must work");

        let mut writer =
            CappedDailyWriter::new(&root, MAX_LOG_FILES, 100).expect("the writer must build");
        tracing::subscriber::with_default(subscriber, || {
            for i in 0..40 {
                writer
                    .write_all(format!("blocked-rotation-{i:03}\n").as_bytes())
                    .expect("every line must land even past the blocked rotation");
            }
            writer.flush().expect("flush must work");
        });
        drop(guard);

        // The directory is still a directory — no part-2 file was
        // created — and every driven line survived in part 1.
        assert!(
            blocked.is_dir(),
            "the blocked name must still be the blocker, not a log file"
        );
        let part1 = root.join(format!("eunomiasync.{today}.log"));
        let content = fs::read_to_string(&part1).expect("part 1 must be readable");
        assert!(
            content.contains("blocked-rotation-000\n")
                && content.contains("blocked-rotation-039\n"),
            "every line past the blocked rotation lands in the current file, \
             got {} bytes",
            content.len()
        );
        assert!(
            fs::metadata(&part1).unwrap().len() > 100,
            "the honest overrun: the current file passes the cap rather than \
             losing lines"
        );
        // The degradation is reported exactly once — the facade warn
        // landed through the observing subscriber into the same file.
        assert_eq!(
            content.matches("the file log could not rotate").count(),
            1,
            "one warn per process, no storm — got: {content:?}"
        );
        assert!(
            content.contains("step=\"open\""),
            "the warn names the failed step, got: {content:?}"
        );
        drop(temp);
    }

    #[test]
    fn a_restart_never_refills_a_gap_in_part_numbers() {
        // The "gaps never refilled" half of the idempotency rule (the
        // round-2 review's coverage ask): with today's parts 1 and 3
        // on disk and part 2 missing (a manual deletion or a crash
        // window), the writer must continue at the highest part — a
        // first-free-slot writer would recreate part 2 and write
        // today's newest lines into a name that sorts *before* part 3.
        let (temp, root) = logs_dir();
        let today = today_stamp();
        fs::write(root.join(format!("eunomiasync.{today}.log")), b"part one").expect("seed 1");
        fs::write(
            root.join(format!("eunomiasync.{today}.3.log")),
            b"part three",
        )
        .expect("seed 3");

        let mut writer = CappedDailyWriter::new(&root, MAX_LOG_FILES, 100)
            .expect("the writer must build over the gapped directory");
        writer
            .write_all(b"the newest line\n")
            .expect("the write must land");
        writer.flush().expect("flush must work");

        assert_eq!(
            fs::read_to_string(root.join(format!("eunomiasync.{today}.3.log")))
                .expect("part 3 must be readable"),
            "part threethe newest line\n",
            "the writer continues the highest part in place"
        );
        assert!(
            !root.join(format!("eunomiasync.{today}.2.log")).exists(),
            "the gap is never refilled — no part 2 is created"
        );
        drop(temp);
    }

    #[test]
    fn building_the_appender_below_the_cap_deletes_nothing() {
        // The non-eager half: with fewer files than the cap, building
        // the appender must not remove any of them (a prune that fired
        // below its threshold would eat history the cap exists to keep).
        let (temp, root) = logs_dir();
        let survivors = [
            root.join("eunomiasync.2020-01-06.log"),
            root.join("eunomiasync.2020-01-07.log"),
        ];
        for path in &survivors {
            fs::write(path, b"an old day's log").expect("seeding an old dated log must work");
        }

        let (subscriber, guard) = build_file_subscriber(&root, LogLevel::Info)
            .expect("building the subscriber must work");
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("quiet probe event");
        });
        drop(guard);

        for path in &survivors {
            assert!(
                path.is_file(),
                "a below-cap seed must survive: {}",
                path.display()
            );
        }
        assert_eq!(
            dated_log_files(&root).len(),
            3,
            "two seeds plus today's live file"
        );
        drop(temp);
    }
}
