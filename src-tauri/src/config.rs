//! The machine-level configuration file `config.toml` (spec §5.8,
//! SYS-011…013).
//!
//! The file lives in the data directory (spec §5.5 — the layout task
//! guarantees the directory exists before this module runs) and holds
//! exactly one key today: `log_level` (SYS-012). It is read at startup
//! before logging and the database are set up, which is why this module
//! cannot log: warnings come back as a [`Vec<String>`] from the
//! [`load_or_create`] seam, and the caller re-emits them through the
//! logging facade after the tracing subscriber is initialized (SYS-011
//! order — the warnings then land in the file log; see `logging.rs`).
//!
//! Behavior, per SYS-011/SYS-012:
//!
//! - **Missing file** — created with the default content
//!   ([`DEFAULT_CONFIG_TOML`]) and the defaults are returned. This is the
//!   only write the module ever performs, and it goes to a `.tmp` sibling
//!   renamed into place, so a crash mid-write cannot leave a torn file
//!   that a later run would preserve forever (a stale `.tmp` is simply
//!   overwritten by the next attempt).
//! - **Parseable file** — parsed with serde + `toml` (SYS-013,
//!   DECISIONS.md D-001). Unknown top-level keys are ignored with a
//!   warning naming the key (SYS-012); a missing `log_level` key falls
//!   back to the default; the file is never rewritten.
//! - **Unparseable content** — invalid UTF-8 bytes, unparseable TOML or
//!   an invalid `log_level` value: defaults in memory plus a warning,
//!   and the app still starts (SYS-012 — "a file that cannot be parsed"
//!   covers everything wrong with the *content*). The file is
//!   deliberately *not* rewritten: it is the user's data, a typo must
//!   not cost them the rest of the file, and rewriting it would hide the
//!   problem from whoever comes to fix it (the warning points at the
//!   exact bytes that failed).
//! - **Unreadable or uncreatable file** — [`ConfigError`], and the launch
//!   aborts like the layout step does (DECISIONS.md D-027): a data
//!   directory that cannot hand over its config file is broken in the
//!   same way as one that cannot hold the database, and no diagnostics
//!   channel exists at that point — SYS-011 orders the config *before*
//!   logging, so the tracing subscriber is not initialized yet when this
//!   failure can happen.
//!
//! Unknown-key detection is a manual diff of the parsed [`toml::Value`]'s
//! top-level keys against [`KNOWN_KEYS`], not the `serde_ignored` crate:
//! the config is flat, the diff is a handful of lines, and the project
//! adds crates only through a task decision (DECISIONS.md D-012 policy).
//! Only top-level keys are named in warnings — the content of an unknown
//! table is not enumerated.
//!
//! [`load_or_create`] is the testable seam; the startup wiring — calling
//! it after `ensure_data_layout`, re-emitting the warnings into the
//! tracing file log after `logging::init`, and publishing the config
//! with `app.manage` — lives in the `setup` hook in `lib.rs`
//! and is pinned there by a `config_tests.rs` text pin (the same honesty
//! boundary as the layout and plugin pins: the pin proves the wiring
//! exists and stays wired and ordered, the runtime effect is covered by
//! the seam tests and the manual first launch on a clean system). The
//! logging step reads the config back with
//! `app.state::<AppConfig>()` and applies `log_level` to the log output
//! (DECISIONS.md D-029).

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The config file name inside the data directory (spec §5.8). Tests pin
/// the literal `"config.toml"` instead of this constant, so a renamed
/// file fails its own test.
const CONFIG_FILE_NAME: &str = "config.toml";

/// The staging name for the missing-file write: the default content is
/// written here first and then renamed onto [`CONFIG_FILE_NAME`], so a
/// crash mid-write leaves a stale `.tmp` (overwritten on the next
/// attempt) instead of a torn config file the never-rewrite policy
/// would preserve forever. Same directory as the target, so the rename
/// stays on one filesystem and is atomic.
const CONFIG_TMP_NAME: &str = ".config.toml.tmp";

/// Every key [`AppConfig`] recognizes, used by the unknown-key diff.
/// Keep this in sync with the struct's fields: a new field without an
/// entry here would be reported as "unknown" to the user (the
/// `every_known_key_warns_nothing` test keeps the two sides honest).
const KNOWN_KEYS: &[&str] = &["log_level"];

/// The exact file content written on the first run (SYS-011: a missing
/// config is created with defaults). Pinned verbatim by a test — a
/// mutated default must fail that test, not silently ship.
const DEFAULT_CONFIG_TOML: &str = r#"# EunomiaSync machine-level configuration (spec 5.8).
#
# log_level — one of: trace, debug, info, warn, error (default: info).
# Unknown keys are ignored with a warning. User-managed settings live in
# the app database, not in this file.
log_level = "info"
"#;

/// The logging verbosity (SYS-012): one of `trace`, `debug`, `info`,
/// `warn`, `error`, defaulting to `info`. The on-disk spelling is the
/// lowercase word (`#[serde(rename_all = "lowercase")]`); anything else
/// — an unknown word, a non-string value — fails deserialization and is
/// handled as the SYS-012 fallback (defaults plus a warning), never as
/// a startup error. `Serialize` is derived only so the
/// `every_known_key_warns_nothing` test can enumerate the field names
/// the struct actually has and pin them against [`KNOWN_KEYS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// The SYS-012 spelling of the level — the word as it appears in
    /// `config.toml` and in log lines (the logging module emits the
    /// effective level into the startup event, so a log file
    /// self-documents how verbose it is).
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The machine-level app configuration (spec §5.8). Deliberately small:
/// SYS-011 reserves this file for machine-level settings only —
/// user-managed settings stay in the database (spec 6.5) and no library
/// data is ever written here.
///
/// The struct key is the spec's literal `log_level` spelling, so no
/// rename is applied at this level. Fields default individually
/// (`#[serde(default)]`): a config file that carries only some keys — or
/// none at all — is valid and completes with defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct AppConfig {
    /// The only key SYS-012 defines today.
    #[serde(default)]
    pub(crate) log_level: LogLevel,
}

/// Why [`load_or_create`] gave up: the config file could not be read or
/// created. Parse failures are *not* this error — they fall back to
/// defaults with a warning (SYS-012) — this is strictly the OS-level
/// failure, which aborts the launch like the layout step does
/// (DECISIONS.md D-027). The path is kept in the message (not only the
/// chain) because the startup failure surfaces as tauri's
/// `Failed to setup app: …` panic, which prints the `Display` form of
/// the error chain — a bare "Permission denied" without the path would
/// send the user looking at the wrong thing.
#[derive(Debug)]
pub(crate) struct ConfigError {
    /// The config file the OS call failed on.
    path: PathBuf,
    source: std::io::Error,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to read or create the config file `{}`: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Loads `config.toml` from the data directory, creating it with
/// [`DEFAULT_CONFIG_TOML`] when missing (SYS-011).
///
/// Returns the effective configuration plus every warning produced on
/// the way — unknown keys ignored (SYS-012), unparseable content
/// falling back to defaults. An empty warning list means the file was
/// absent (now created with defaults) or parsed cleanly. The returned
/// configuration is what the app runs with; the file itself is only
/// ever written when it does not exist.
///
/// Safe to run at every startup: the missing-file write happens once
/// (via a `.tmp` sibling renamed into place, so it cannot tear) and an
/// existing file is never modified, whatever it contains — a second
/// run, a broken run and a hand-edited file all leave the bytes on disk
/// alone.
///
/// The data directory itself must already exist: the layout step
/// (`ensure_data_layout`) runs first and guarantees that — the order is
/// pinned in `config_tests.rs`. This module deliberately does not create
/// parent directories, so a wiring regression that calls it before the
/// layout fails loudly instead of silently re-deriving the layout's
/// responsibility.
///
/// # Errors
///
/// Fails with [`ConfigError`] when the existing file cannot be read or a
/// missing one cannot be written (which includes a missing parent
/// directory) — the data directory itself is broken, and the launch
/// aborts (DECISIONS.md D-027), like the layout step before it.
/// Content problems are not errors: invalid UTF-8, unparseable TOML and
/// an invalid `log_level` all return defaults plus a warning and the
/// app starts (SYS-012).
pub(crate) fn load_or_create(data_dir: &Path) -> Result<(AppConfig, Vec<String>), ConfigError> {
    let path = data_dir.join(CONFIG_FILE_NAME);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // SYS-011: the first run creates the file. The bytes are
            // staged in a `.tmp` sibling and renamed into place so a
            // crash mid-write cannot leave a torn config file that the
            // never-rewrite policy would keep forever; a stale `.tmp`
            // from such a crash is simply overwritten here.
            let staging = data_dir.join(CONFIG_TMP_NAME);
            if let Err(source) = std::fs::write(&staging, DEFAULT_CONFIG_TOML) {
                let _ = std::fs::remove_file(&staging);
                return Err(ConfigError {
                    path: staging,
                    source,
                });
            }
            if let Err(source) = std::fs::rename(&staging, &path) {
                let _ = std::fs::remove_file(&staging);
                return Err(ConfigError { path, source });
            }
            return Ok((AppConfig::default(), Vec::new()));
        }
        Err(source) => return Err(ConfigError { path, source }),
    };

    // The whole file must decode before it can parse: bytes that are
    // not UTF-8 (e.g. a UTF-16 file written by an editor or a Windows
    // shell redirect) are a content problem and take the SYS-012
    // fallback — defaults plus a warning, bytes untouched — exactly
    // like a TOML parse failure below.
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => return Ok(fallback_with_warning(&path, "not valid UTF-8")),
    };

    // Parse once into an untyped table: the top-level keys feed the
    // unknown-key warnings (SYS-012) and the same value then
    // deserializes into AppConfig, so an invalid `log_level` surfaces
    // here as the same fallback-with-warning path (SYS-012) instead of
    // a startup error.
    let value: toml::Value = match toml::from_str(&text) {
        Ok(value) => value,
        Err(error) => return Ok(fallback_with_warning(&path, &error.to_string())),
    };

    let mut warnings = Vec::new();
    if let Some(table) = value.as_table() {
        for key in table.keys() {
            if !KNOWN_KEYS.contains(&key.as_str()) {
                warnings.push(format!(
                    "config: ignoring unknown key `{key}` in {}",
                    path.display()
                ));
            }
        }
    }

    let config = match AppConfig::deserialize(value) {
        Ok(config) => config,
        Err(error) => {
            warnings.push(format!(
                "config: failed to parse {} ({error}); using defaults",
                path.display()
            ));
            AppConfig::default()
        }
    };
    Ok((config, warnings))
}

/// The SYS-012 fallback for content the file cannot be parsed from:
/// defaults in memory plus one warning naming the file and the reason.
/// The file's bytes are never touched — the warning exists precisely so
/// the user can go fix them.
fn fallback_with_warning(path: &Path, reason: &str) -> (AppConfig, Vec<String>) {
    (
        AppConfig::default(),
        vec![format!(
            "config: failed to parse {} ({reason}); using defaults",
            path.display()
        )],
    )
}

/// Re-emits each startup warning into the tracing file log as a `warn`
/// event through the logging facade (`logging::config_load_warning` —
/// the SEC-008 event surface, see that module). Called by the `setup`
/// hook **after** `logging::init`, so the warnings land in the log file
/// the subscriber of that call writes; the warnings are data from
/// [`load_or_create`] precisely so they stay testable on the way (the
/// logging module's tests cover the emission end to end).
pub(crate) fn emit_warnings(warnings: &[String]) {
    for warning in warnings {
        crate::logging::config_load_warning(warning);
    }
}

#[cfg(test)]
mod tests {
    //! Tests of the [`load_or_create`] seam against real files under
    //! `tempfile` roots — the seam is plain `std::fs` + `toml` work, so
    //! there is no mock boundary to fake and the observable behavior
    //! (which file appeared with which bytes, which configuration and
    //! warnings came back, which bytes were left untouched) is asserted
    //! directly. Expected names and the default content are pinned as
    //! literals in the tests, never derived from
    //! [`super::DEFAULT_CONFIG_TOML`] or [`super::CONFIG_FILE_NAME`]:
    //! a mutated default must fail its own test (the same trick as the
    //! `data_dirs.rs` layout tests). The startup wiring around the seam
    //! (call order after the layout, warning re-emission after logging
    //! init, `app.manage`) is covered by the `config_tests.rs` text pin
    //! and the manual first launch on a clean system.

    use super::*;
    use std::error::Error as _;
    use std::fs;

    /// The default file content this module must write, as a literal —
    /// deliberately not [`super::DEFAULT_CONFIG_TOML`].
    const EXPECTED_DEFAULT_TOML: &str = r#"# EunomiaSync machine-level configuration (spec 5.8).
#
# log_level — one of: trace, debug, info, warn, error (default: info).
# Unknown keys are ignored with a warning. User-managed settings live in
# the app database, not in this file.
log_level = "info"
"#;

    /// A data directory that exists and is empty — the state the layout
    /// step leaves behind right before the config seam runs. The
    /// `TempDir` is returned alongside so it stays alive (dropping it
    /// removes the tree).
    fn empty_data_dir() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("failed to create a temp directory");
        let root = temp.path().join("eunomiasync");
        fs::create_dir(&root).expect("creating the temp data directory must work");
        (temp, root)
    }

    /// Skips the test unless the process may fail on permission bits:
    /// root (uid 0) bypasses file permissions entirely, so chmod-based
    /// fixtures prove nothing there. The temp directory was created by
    /// this process, so its owner uid is the effective uid.
    #[cfg(unix)]
    fn running_as_root_is_untestable(temp: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(temp)
            .expect("the temp directory must be statable")
            .uid()
            == 0
    }

    /// Asserts the data directory holds exactly one entry — the config
    /// file, pinned by its literal name — so a renamed file or an extra
    /// artifact fails the test.
    fn assert_only_config_file(root: &Path) {
        let mut entries: Vec<String> = fs::read_dir(root)
            .expect("the data directory must be readable")
            .map(|entry| {
                entry
                    .expect("every data-dir entry must be readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        entries.sort();
        assert_eq!(
            entries,
            ["config.toml"],
            "the config seam must only ever produce `config.toml` in the \
             data directory"
        );
    }

    #[test]
    fn a_missing_file_is_created_with_the_pinned_defaults() {
        // Arrange: the data directory exists (the layout step guarantees
        // that — the seam runs after it, pinned in `config_tests.rs`)
        // but holds no config yet: the first-run state this seam owns.
        let (_temp, root) = empty_data_dir();
        let path = root.join("config.toml");

        // Act: the first startup.
        let (config, warnings) =
            load_or_create(&root).expect("the first run must create the config");

        // Assert: defaults in memory, no warnings, the file exists with
        // exactly the pinned bytes, and nothing else appeared.
        assert_eq!(config, AppConfig::default());
        assert_eq!(config.log_level, LogLevel::Info);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let written = fs::read_to_string(&path).expect("the config file must exist");
        assert_eq!(written, EXPECTED_DEFAULT_TOML);
        assert_only_config_file(&root);
    }

    #[test]
    fn a_second_run_is_a_no_op() {
        // Arrange: a first run created the default file.
        let (_temp, root) = empty_data_dir();
        let path = root.join("config.toml");
        load_or_create(&root).expect("the first run must create the config");
        let before = fs::read(&path).expect("the created file must be readable");

        // Act: the next startup runs the same seam again.
        let (config, warnings) =
            load_or_create(&root).expect("a repeated run must treat the file as done");

        // Assert: idempotent — the same defaults, no warnings, and the
        // bytes on disk are content-identical (the seam's contract is
        // about the file's contents, which is what the seam observes;
        // mtimes are the OS's business and are not asserted).
        assert_eq!(config, AppConfig::default());
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let after = fs::read(&path).expect("the file must stay readable");
        assert_eq!(
            before, after,
            "an existing default file must not be rewritten"
        );
    }

    #[test]
    fn every_spec_log_level_spelling_is_accepted() {
        // Arrange: SYS-012 fixes the accepted words; each must parse.
        let spellings = [
            ("trace", LogLevel::Trace),
            ("debug", LogLevel::Debug),
            ("info", LogLevel::Info),
            ("warn", LogLevel::Warn),
            ("error", LogLevel::Error),
        ];
        for (spelling, expected) in spellings {
            let (_temp, root) = empty_data_dir();
            fs::write(
                root.join("config.toml"),
                format!("log_level = \"{spelling}\"\n"),
            )
            .expect("writing a temp config must work");

            // Act
            let (config, warnings) =
                load_or_create(&root).expect("a valid log_level must never fail the seam");

            // Assert: the exact variant came back and nothing was
            // warned about or rewritten.
            assert_eq!(config.log_level, expected, "spelling `{spelling}`");
            assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
            let after =
                fs::read_to_string(root.join("config.toml")).expect("the file must stay readable");
            assert_eq!(
                after,
                format!("log_level = \"{spelling}\"\n"),
                "a valid file must not be rewritten"
            );
        }
    }

    #[test]
    fn a_partial_file_completes_with_defaults_without_being_rewritten() {
        // Arrange: a file the user trimmed down to a subset of keys —
        // valid TOML, no `log_level` at all.
        let (_temp, root) = empty_data_dir();
        let path = root.join("config.toml");
        fs::write(&path, "\n").expect("writing a temp config must work");

        // Act
        let (config, warnings) = load_or_create(&root).expect("an empty table is a valid config");

        // Assert: all defaults, and nothing to warn about — a missing
        // key is a default, not a mistake; the trimmed file stays as
        // the user left it.
        assert_eq!(config, AppConfig::default());
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let after = fs::read_to_string(&path).expect("the file must stay readable");
        assert_eq!(after, "\n", "a valid file must not be rewritten");
    }

    #[test]
    fn unknown_keys_are_ignored_with_a_warning_naming_the_key() {
        // Arrange: a config with two unknown entries — a plain key and
        // a whole table (a user guessing at the format).
        let (_temp, root) = empty_data_dir();
        let path = root.join("config.toml");
        fs::write(
            &path,
            "log_level = \"warn\"\nverbosity = 3\n[user]\nname = \"x\"\n",
        )
        .expect("writing a temp config must work");

        // Act
        let (config, warnings) =
            load_or_create(&root).expect("unknown keys must not fail the seam");

        // Assert: the known key still applies; each unknown top-level
        // key is named in a warning; the file keeps the user's bytes.
        assert_eq!(config.log_level, LogLevel::Warn);
        assert_eq!(
            warnings.len(),
            2,
            "one warning per unknown key: {warnings:?}"
        );
        assert!(
            warnings.iter().all(|w| w.contains("unknown key")),
            "warnings must name the problem: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("`verbosity`")),
            "the plain unknown key must be named: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("`user`")),
            "the unknown table must be named: {warnings:?}"
        );
        let after = fs::read_to_string(&path).expect("the file must stay readable");
        assert_eq!(
            after, "log_level = \"warn\"\nverbosity = 3\n[user]\nname = \"x\"\n",
            "unknown keys must be ignored, not pruned"
        );
    }

    #[test]
    fn an_unparseable_file_falls_back_to_defaults_and_is_left_untouched() {
        // Arrange: garbage that is not TOML at all.
        let (_temp, root) = empty_data_dir();
        let path = root.join("config.toml");
        let broken = "this is ][ not toml";
        fs::write(&path, broken).expect("writing a temp config must work");

        // Act
        let (config, warnings) =
            load_or_create(&root).expect("a broken file must not fail the seam");

        // Assert: defaults in memory (SYS-012: the app still starts),
        // a warning explaining the fallback, and the user's broken
        // bytes preserved on disk — the seam must not "fix" the file
        // and destroy whatever the user kept in it.
        assert_eq!(config, AppConfig::default());
        assert_eq!(warnings.len(), 1, "expected one warning: {warnings:?}");
        assert!(
            warnings[0].contains("failed to parse") && warnings[0].contains("config.toml"),
            "the warning must point at the file and the parse failure: {warnings:?}"
        );
        let after = fs::read_to_string(&path).expect("the file must stay readable");
        assert_eq!(after, broken, "a broken file must not be rewritten");
    }

    #[test]
    fn an_invalid_log_level_value_falls_back_to_defaults_and_is_left_untouched() {
        // Arrange: valid TOML, but `log_level` is not one of the five
        // SYS-012 words.
        let (_temp, root) = empty_data_dir();
        let path = root.join("config.toml");
        let broken = "log_level = \"loud\"\n";
        fs::write(&path, broken).expect("writing a temp config must work");

        // Act
        let (config, warnings) =
            load_or_create(&root).expect("an invalid value must not fail the seam");

        // Assert: the default level applies, the warning names the
        // offending value, and the file keeps the user's bytes.
        assert_eq!(config.log_level, LogLevel::Info);
        assert_eq!(warnings.len(), 1, "expected one warning: {warnings:?}");
        assert!(
            warnings[0].contains("loud"),
            "the warning must name the invalid value: {warnings:?}"
        );
        let after = fs::read_to_string(&path).expect("the file must stay readable");
        assert_eq!(after, broken, "the file must not be rewritten");
    }

    #[test]
    fn a_data_dir_that_is_not_a_directory_fails_and_names_the_path() {
        // Arrange: the would-be data directory sits under a regular
        // file, so reading the config inside it fails with ENOTDIR
        // before any write is attempted — the read-failure arm of the
        // seam (the write arm is covered by the read-only-parent test
        // below; the layout step would normally have aborted first, the
        // seam still surfaces its own failure instead of pretending).
        let temp = tempfile::tempdir().expect("failed to create a temp directory");
        let blocker = temp.path().join("blocker");
        fs::write(&blocker, b"not a directory").expect("writing a temp file must work");
        let root = blocker.join("eunomiasync");

        // Act
        let result = load_or_create(&root);

        // Assert: the failure is reported (never swallowed, never a
        // panic) and the message points at the data root's config
        // file — what the startup abort prints. Cross-OS routing
        // differs: Unix fails the read with ENOTDIR, and the read
        // arm's error names the final `config.toml` path; Windows
        // fails the same read with ERROR_PATH_NOT_FOUND, which is
        // `ErrorKind::NotFound`, so the seam takes the create arm and
        // the write error names the `.config.toml.tmp` sibling. Both
        // spellings live under this data root and carry the config
        // file name, so that is the asserted contract; the exact
        // final-path spelling stays asserted on the arm that ran.
        let error = result.expect_err("a read under a regular file must fail");
        let expected_root = root.display().to_string();
        let message = error.to_string();
        assert!(
            message.contains(&expected_root) && message.contains("config.toml"),
            "the error must name the config file under the data root \
             (`{expected_root}/config.toml`), got: {error}"
        );
        assert!(
            error.source().is_some(),
            "the OS cause must stay reachable through the error chain"
        );
    }

    #[test]
    fn a_non_utf8_file_falls_back_to_defaults_and_is_left_untouched() {
        // Arrange: bytes that never decode as UTF-8 — a UTF-16 file,
        // which Windows PowerShell 5.1 redirection and some editors
        // produce. Content the file cannot be parsed from (SYS-012),
        // not a broken data directory: the app must start on defaults
        // and keep the user's bytes.
        let (_temp, root) = empty_data_dir();
        let path = root.join("config.toml");
        let broken: Vec<u8> = vec![0xFF, 0xFE, b'l', 0x00, b'o', 0x00, b'g', 0x00];
        fs::write(&path, &broken).expect("writing a temp config must work");

        // Act
        let (config, warnings) =
            load_or_create(&root).expect("an undecodable file must not fail the seam");

        // Assert: defaults in memory, one warning pointing at the
        // file, and the exact bytes preserved on disk.
        assert_eq!(config, AppConfig::default());
        assert_eq!(warnings.len(), 1, "expected one warning: {warnings:?}");
        assert!(
            warnings[0].contains("UTF-8") && warnings[0].contains("config.toml"),
            "the warning must point at the file and the decoding failure: \
             {warnings:?}"
        );
        let after = fs::read(&path).expect("the file must stay readable");
        assert_eq!(after, broken, "the file must not be rewritten");
    }

    #[cfg(unix)]
    #[test]
    fn an_unwritable_data_dir_fails_on_the_first_write() {
        // Arrange: the data directory exists but is read-only, so the
        // read yields NotFound (the missing-file branch) and the
        // staging write itself fails with EACCES — the write arm the
        // not-a-directory fixture above cannot reach. Unix-only (other
        // platforms have no portable permission model), and skipped as
        // root: root bypasses permission bits entirely.
        use std::os::unix::fs::PermissionsExt;
        let (temp, root) = empty_data_dir();
        if running_as_root_is_untestable(temp.path()) {
            return;
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(0o555))
            .expect("setting temp-dir permissions must work");

        // Act
        let result = load_or_create(&root);

        // Restore before the TempDir drop: removing a tree needs write
        // access to the directory again.
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755))
            .expect("restoring temp-dir permissions must work");

        // Assert: the launch aborts with the OS cause reachable — the
        // failure kind is pinned, not the staging path's name.
        let error = result.expect_err("writing into a read-only directory must fail");
        let source = error
            .source()
            .expect("the OS cause must stay reachable through the error chain");
        let io = source
            .downcast_ref::<std::io::Error>()
            .expect("the cause must stay an io::Error");
        assert_eq!(
            io.kind(),
            std::io::ErrorKind::PermissionDenied,
            "the write arm must surface the OS permission failure, got: {io}"
        );
    }

    #[test]
    fn every_known_key_warns_nothing() {
        // Arrange: the two sides of the unknown-key diff must agree in
        // both directions. First, the struct must have exactly the
        // fields KNOWN_KEYS lists: serializing the config enumerates
        // the real field names, so a field added to AppConfig without a
        // KNOWN_KEYS entry (or an entry left over from a removed field)
        // fails here — the diff alone cannot see this drift, it would
        // merely start warning (or stop) for real user documents.
        let mut fields: Vec<String> = {
            let value = toml::Value::try_from(AppConfig::default())
                .expect("the default config must serialize to TOML");
            value
                .as_table()
                .expect("the serialized config must be a table")
                .keys()
                .cloned()
                .collect()
        };
        fields.sort();
        let mut known: Vec<String> = KNOWN_KEYS.iter().map(|key| key.to_string()).collect();
        known.sort();
        assert_eq!(
            fields, known,
            "KNOWN_KEYS must list exactly the fields AppConfig has"
        );

        // Second, a document assembled from exactly the known keys must
        // parse with zero warnings — the diff must not invent spurious
        // "unknown key" noise for the keys it exists to whitelist.
        let (_temp, root) = empty_data_dir();
        let doc = KNOWN_KEYS
            .iter()
            .map(|key| match *key {
                "log_level" => "log_level = \"info\"".to_string(),
                other => panic!("`{other}` is in KNOWN_KEYS but has no test value"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(root.join("config.toml"), doc).expect("writing a temp config must work");

        // Act
        let (config, warnings) =
            load_or_create(&root).expect("a known-keys-only document must parse");

        // Assert
        assert_eq!(config, AppConfig::default());
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_config_fails_instead_of_silently_using_defaults() {
        // Arrange: an existing file the process cannot read (mode 000).
        // Unix-only: other platforms have no portable permission model,
        // and CI runs this suite on Linux only; skipped as root, which
        // bypasses permission bits entirely.
        use std::os::unix::fs::PermissionsExt;
        let (temp, root) = empty_data_dir();
        if running_as_root_is_untestable(temp.path()) {
            return;
        }
        let path = root.join("config.toml");
        fs::write(&path, "log_level = \"debug\"\n").expect("writing a temp config must work");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000))
            .expect("setting temp-file permissions must work");

        // Act
        let result = load_or_create(&root);

        // Assert: an unreadable file is an OS-level failure — the
        // SYS-012 defaults-with-warning fallback is for parse problems,
        // not for a data directory we cannot read; the launch aborts
        // with the path in the message (never a silent default run
        // that would also skip creating the file).
        let error = result.expect_err("an unreadable config must fail the seam");
        let expected_path = path.display().to_string();
        assert!(
            error.to_string().contains(&expected_path),
            "the error must name the config path (`{expected_path}`), got: {error}"
        );
    }
}
