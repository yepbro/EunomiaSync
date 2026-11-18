//! The data directory layout (spec §5.5).
//!
//! The data directory is Tauri's default `app_local_data_dir()` —
//! `local_data_dir/pro.yepbro.eunomiasync` on every OS. The per-OS path
//! table is pinned by the `config_tests.rs` identifier and
//! `appDirectoriesOverride` pins, so this module never hardcodes a
//! platform path; it works on whatever root the path resolver hands it.
//! At startup the directory must hold exactly four subdirectories —
//! `db/` (the SQLite file and its WAL/-shm siblings, DB-001),
//! `db-backups/` (pre-migration copies, DB-004), `backups/` (project
//! file backups) and `logs/` (rotated trace logs) — and
//! [`ensure_data_layout`] creates whatever is missing, whatever the
//! reason it is missing: a clean first run, a previous run that crashed
//! halfway through the creation loop, or a user who deleted part of the
//! tree (spec 5.5 tip: deleted data means a fresh, empty app).
//! [`std::fs::create_dir_all`] is the whole mechanism: it no-ops on
//! existing directories, creates missing ancestors, and fails only when
//! a path is truly uncreatable (a regular file occupies it, no
//! permission), which makes the operation idempotent by construction —
//! running it at every startup converges to the full layout instead of
//! depending on what a previous run left behind.
//!
//! Two things are deliberately *not* done here. `config.toml` is not
//! created: SYS-011 has it read (and created with defaults when missing)
//! before logging and the database are set up, which is the config
//! task's job — it only needs this layout to exist first. And no log
//! path is derived from Tauri's `app_log_dir()`: logs live in `logs/`
//! inside the data directory on every OS (spec 5.5). On Windows and
//! Linux that happens to match the Tauri default, but on macOS
//! `app_log_dir()` would point at `~/Library/Logs/pro.yepbro.eunomiasync/`,
//! so the logging module (`logging.rs`) points the appender at
//! `<data dir>/logs/` explicitly — pinned against an `app_log_dir()`
//! regression by a `config_tests.rs` absence pin (spec §5.5).
//!
//! [`ensure_data_layout`] is the testable seam; the startup wiring —
//! resolving `app_local_data_dir()` through the path resolver and
//! failing the launch when the layout cannot be created — lives in the
//! `setup` hook in `lib.rs` and is pinned there by a `config_tests.rs`
//! text pin (the same honesty boundary as the plugin-registration pin:
//! the pin proves the wiring exists and stays wired, the runtime effect
//! is covered by the manual smoke runs and the first launch on a clean
//! system). The failure policy: a layout that cannot be created means no
//! database, no logs and no config, so there is nothing meaningful to
//! run — and no diagnostics channel exists at that point either: SYS-011
//! orders the layout *before* the config, and the config *before*
//! logging, so the tracing subscriber is initialized only after this
//! step has already succeeded. The error therefore propagates out of
//! `setup`. Tauri wraps
//! it in `Error::Setup` and panics in the event-loop `Ready` handler
//! with `Failed to setup app: error encountered during setup hook: …`
//! (tauri 2.12.6 `src/app.rs`); the panic message prints the `Display`
//! form of that chain, so the offending path and the OS cause reach the
//! console (DECISIONS.md D-027).

use std::fmt;
use std::path::{Path, PathBuf};

/// The layout subdirectories, exactly the structure block of spec §5.5
/// (`config.toml` completes the block as a *file* and lands with the
/// config task — creating it here would be both a directory/file
/// confusion and a SYS-011 ordering violation).
///
/// Tests must not derive their expectations from this constant: they pin
/// the literal four names, so a mutated layout fails its own test.
const SUBDIRECTORIES: &[&str] = &[
    DB_SUBDIRECTORY,
    DB_BACKUPS_SUBDIRECTORY,
    BACKUPS_SUBDIRECTORY,
    LOGS_SUBDIRECTORY,
];

/// `db/` — the SQLite file and its WAL/-shm siblings (DB-001).
const DB_SUBDIRECTORY: &str = "db";
/// `db-backups/` — pre-migration copies (DB-004).
const DB_BACKUPS_SUBDIRECTORY: &str = "db-backups";
/// `backups/` — project file backups.
const BACKUPS_SUBDIRECTORY: &str = "backups";
/// `logs/` — the rotated trace logs (spec §5.5). Exposed for the
/// logging module, which points its appender at
/// `<data dir>/<LOGS_SUBDIRECTORY>/eunomiasync.<date>.log` explicitly
/// instead of Tauri's `app_log_dir()` (see the module docs).
pub(crate) const LOGS_SUBDIRECTORY: &str = "logs";

/// Why [`ensure_data_layout`] gave up: the path it could not create,
/// plus the OS cause. The path is kept in the message (not only the
/// chain) because the startup failure surfaces as tauri's
/// `Failed to setup app: …` panic, which prints the `Display` form of
/// the error chain — a bare "Permission denied" without the path would
/// send the user looking at the wrong thing.
#[derive(Debug)]
pub(crate) struct DataLayoutError {
    /// The directory `create_dir_all` was asked for when it failed —
    /// the root itself or one of the four layout subdirectories.
    path: PathBuf,
    source: std::io::Error,
}

impl fmt::Display for DataLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to create the `{}` directory: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for DataLayoutError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Creates the missing directories of the data directory layout:
/// `root` itself, then `db/`, `db-backups/`, `backups/` and `logs/`
/// inside it (spec §5.5).
///
/// Safe to run at every startup: existing directories are left alone
/// and a partially deleted or half-created layout is completed, so the
/// end state is the same however many times this runs and wherever a
/// previous run stopped. Returns the first failure as
/// [`DataLayoutError`] — the app has no meaningful startup without the
/// layout (no database, no logs, no config), so callers abort the
/// launch instead of degrading silently.
///
/// # Errors
///
/// Fails when `root` or any subdirectory cannot be created — the path
/// is occupied by a regular file, an ancestor is not a directory, or
/// the OS denies the write. The error carries the offending path and
/// the underlying [`std::io::Error`].
pub(crate) fn ensure_data_layout(root: &Path) -> Result<(), DataLayoutError> {
    create_dir(root)?;
    for name in SUBDIRECTORIES {
        create_dir(&root.join(name))?;
    }
    Ok(())
}

/// One `create_dir_all` step with the failing path attached.
fn create_dir(path: &Path) -> Result<(), DataLayoutError> {
    std::fs::create_dir_all(path).map_err(|source| DataLayoutError {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    //! Tests of the [`ensure_data_layout`] seam against real
    //! directories under `tempfile` roots — the seam is plain
    //! `std::fs` work, so there is no mock boundary to fake and the
    //! observable behavior (which directories exist afterwards, what
    //! the error carries) is asserted directly. The expected names are
    //! pinned as literals in each test, never read from
    //! [`super::SUBDIRECTORIES`]: a mutated layout constant must fail
    //! its own test, not satisfy it (the same trick as the
    //! `single_instance.rs` window-label test). The startup wiring
    //! around the seam (path resolution, abort-on-failure) is covered
    //! by the `config_tests.rs` text pin and the manual smoke runs.

    use super::*;
    use std::error::Error as _;
    use std::fs;

    /// The layout this module must produce, as a literal — a copy of
    /// the spec §5.5 structure block, deliberately not
    /// [`super::SUBDIRECTORIES`].
    const EXPECTED_SUBDIRECTORIES: [&str; 4] = ["db", "db-backups", "backups", "logs"];

    /// A fresh root that does not exist yet — and neither do its
    /// ancestors — mirroring the first-run state on a clean system
    /// (on Linux, `~/.local/share` itself need not exist, so the
    /// nested-ancestor creation must be part of what these tests pin).
    /// The `TempDir` is returned alongside so it stays alive (dropping
    /// it removes the tree).
    fn nonexistent_root() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("failed to create a temp directory");
        let root = temp
            .path()
            .join("home")
            .join(".local")
            .join("share")
            .join("eunomiasync");
        (temp, root)
    }

    /// Asserts the root holds exactly the pinned directories and
    /// nothing else — shared by the tests that must observe the full
    /// invariant (after creation and again after a rerun).
    fn assert_exactly_pinned_entries(root: &Path) {
        let mut entries: Vec<String> = fs::read_dir(root)
            .expect("the created root must be readable")
            .map(|entry| {
                entry
                    .expect("every root entry must be readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        entries.sort();
        assert_eq!(
            entries,
            ["backups", "db", "db-backups", "logs"],
            "the data directory must hold exactly the spec §5.5 \
             subdirectories"
        );
    }

    #[test]
    fn creates_the_root_and_all_four_subdirectories() {
        // Arrange: a clean system — the data directory does not exist.
        let (_temp, root) = nonexistent_root();

        // Act: the first startup.
        ensure_data_layout(&root).expect("the layout must be created in a temp directory");

        // Assert: every spec §5.5 directory exists and is a directory
        // (not a file, not left out).
        for name in EXPECTED_SUBDIRECTORIES {
            assert!(
                root.join(name).is_dir(),
                "`{}` must exist as a directory after the first run",
                name
            );
        }
    }

    #[test]
    fn creates_exactly_the_pinned_entries_and_nothing_else() {
        // Arrange
        let (_temp, root) = nonexistent_root();

        // Act
        ensure_data_layout(&root).expect("the layout must be created in a temp directory");

        // Assert: the root holds exactly the four directories — no
        // extra entries, and in particular no `config.toml`, which
        // SYS-011 assigns to the config task, not to this layout.
        assert_exactly_pinned_entries(&root);
    }

    #[test]
    fn a_second_run_is_a_no_op() {
        // Arrange: a completed layout from a first run.
        let (_temp, root) = nonexistent_root();
        ensure_data_layout(&root).expect("the first run must succeed");

        // Act: the next startup runs the same seam again.
        let second = ensure_data_layout(&root);

        // Assert: idempotent — no error, and the layout is unchanged:
        // still exactly the four directories, nothing duplicated or
        // added by the rerun.
        second.expect("a repeated run must treat existing directories as done");
        assert_exactly_pinned_entries(&root);
    }

    #[test]
    fn a_partially_deleted_layout_is_completed() {
        // Arrange: a layout from an earlier run, then the user (or a
        // crash of a hypothetical future writer) removes part of it —
        // the state the spec 5.5 "user deletes the data" tip describes.
        let (_temp, root) = nonexistent_root();
        ensure_data_layout(&root).expect("the first run must succeed");
        fs::remove_dir(root.join("logs")).expect("removing a temp directory must work");
        fs::remove_dir(root.join("db-backups")).expect("removing a temp directory must work");

        // Act
        ensure_data_layout(&root).expect("a rerun over a partial layout must succeed");

        // Assert: converged — all four directories exist again,
        // including the survivors.
        for name in EXPECTED_SUBDIRECTORIES {
            assert!(
                root.join(name).is_dir(),
                "`{name}` must exist after the layout is completed"
            );
        }
    }

    #[test]
    fn an_uncreatable_layout_fails_and_names_the_path() {
        // Arrange: the would-be root sits under a regular file, so no
        // directory can be created anywhere beneath it — a portable
        // stand-in for a permission-denied or occupied path on a real
        // system (the specific OS errno is deliberately not pinned).
        let temp = tempfile::tempdir().expect("failed to create a temp directory");
        let blocker = temp.path().join("blocker");
        fs::write(&blocker, b"not a directory").expect("writing a temp file must work");
        let root = blocker.join("eunomiasync");

        // Act
        let result = ensure_data_layout(&root);

        // Assert: the failure is reported (never swallowed, never a
        // panic) and the message carries the full offending path —
        // what an operator needs when the startup aborts.
        let error = result.expect_err("creation under a regular file must fail");
        let expected_path = root.display().to_string();
        assert!(
            error.to_string().contains(&expected_path),
            "the error must name the full path it could not create \
             (`{expected_path}`), got: {error}"
        );
        assert!(
            error.source().is_some(),
            "the OS cause must stay reachable through the error chain"
        );
    }
}
