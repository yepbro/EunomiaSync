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
//! [`std::fs::create_dir_all`] is the whole creation mechanism: it
//! no-ops on existing directories, creates missing ancestors, and
//! fails only when a path is truly uncreatable (a regular file
//! occupies it, no permission), which makes the operation idempotent
//! by construction — running it at every startup converges to the
//! full layout instead of depending on what a previous run left
//! behind. On top of the creation, the same run enforces SEC-007's
//! directory modes on the two backup directories (see the paragraph
//! below): two unconditional chmods, re-asserted at every startup,
//! which can fail like a creation step can.
//!
//! Three things are deliberately *not* done here. `config.toml` is not
//! created: SYS-011 has it read (and created with defaults when missing)
//! before logging and the database are set up, which is the config
//! task's job — it only needs this layout to exist first. No log
//! path is derived from Tauri's `app_log_dir()`: logs live in `logs/`
//! inside the data directory on every OS (spec 5.5). On Windows and
//! Linux that happens to match the Tauri default, but on macOS
//! `app_log_dir()` would point at `~/Library/Logs/pro.yepbro.eunomiasync/`,
//! so the logging module (`logging.rs`) points the appender at
//! `<data dir>/logs/` explicitly — pinned against an `app_log_dir()`
//! regression by a `config_tests.rs` absence pin (spec §5.5). And the
//! database file's `0600` is not enforced here either: the store
//! creates the file, so the store sets its mode at the open (see the
//! SEC-007 paragraph below).
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
//!
//! SEC-007 (spec §13) sharpens the layout's two backup directories on
//! Unix: `backups/` and `db-backups/` are held at `0700` — no group or
//! other access — and the mode is *re-asserted* at every startup, not
//! only set at creation, so a pre-existing directory with wider bits
//! (restored from a copy, loosened by a tool) converges back on the
//! next run (the same idempotency as the creation itself). The exact
//! mode is written, not a umask-filtered `mkdir` mode, and a chmod
//! failure aborts the launch like a creation failure: a backups
//! directory that cannot be closed off fails the requirement it exists
//! for. The other layout directories (`db/`, `logs/`, the root) keep
//! the OS defaults — SEC-007 names the backups folders and the DB file
//! only. On Windows nothing is chmod'ed: the rights are inherited from
//! the user profile (SEC-007). The store's own `0600` enforcement —
//! the database file and every copy of it — is recorded together with
//! this layer split in DECISIONS.md D-048.

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
/// `db-backups/` — pre-migration copies (DB-004) and the ERR-001
/// daily startup copies (spec §10).
const DB_BACKUPS_SUBDIRECTORY: &str = "db-backups";
/// `backups/` — project file backups.
const BACKUPS_SUBDIRECTORY: &str = "backups";
/// `logs/` — the rotated trace logs (spec §5.5). Exposed for the
/// logging module, which points its appender at
/// `<data dir>/<LOGS_SUBDIRECTORY>/eunomiasync.<date>.log` explicitly
/// instead of Tauri's `app_log_dir()` (see the module docs).
pub(crate) const LOGS_SUBDIRECTORY: &str = "logs";

/// The SQLite database file name (spec §5.5 structure block:
/// `db/eunomiasync.sqlite3` plus its `-wal`/`-shm` siblings, which
/// SQLite creates and names itself). One file, one connection for the
/// whole app (SYS-002).
const DB_FILE_NAME: &str = "eunomiasync.sqlite3";

/// Returns the database file path inside the layout:
/// `<data dir>/db/eunomiasync.sqlite3` (spec §5.5). The `db/` directory
/// must already exist — [`ensure_data_layout`] creates it, and
/// `eunomia_store::storage::open_db` deliberately does not create
/// parent directories (DECISIONS.md D-030).
pub(crate) fn db_file_path(root: &Path) -> PathBuf {
    root.join(DB_SUBDIRECTORY).join(DB_FILE_NAME)
}

/// Returns the backup directory inside the layout:
/// `<data dir>/db-backups` (spec §5.5) — where `run_migrations`
/// writes its `VACUUM INTO` safety copies (DB-004, DECISIONS.md
/// D-038), where the ERR-001 daily startup copies live
/// (`daily-YYYYMMDD.sqlite3`, spec §10, DECISIONS.md D-046), and where
/// the recovery screen's `db_recreate`/`db_restore_backup` keep the
/// damaged database as `pre-recovery-<UTC>.sqlite3` (§7.10, T-024).
/// Like the
/// database file's parent, the directory must already exist: the
/// store creates no directories, and an upgrade run that needs a copy
/// fails before applying anything when it is missing (the layout step
/// creates it).
pub(crate) fn db_backups_dir(root: &Path) -> PathBuf {
    root.join(DB_BACKUPS_SUBDIRECTORY)
}

/// The resolved data-directory root, published as managed state by the
/// `setup` hook right after [`ensure_data_layout`] (T-024).
///
/// The recovery commands operate exactly when the database is *not*
/// managed — the startup open failed as `E_DB_CORRUPT` — so they cannot
/// derive their paths from any DB-owned state; this handle is the one
/// path fact that survives a failed open. Publishing the resolved root
/// also keeps the commands off `app.path().app_local_data_dir()` at
/// call time: the resolver would answer with the real user data
/// directory on every platform, including the mock runtime of the IPC
/// tests, while this state is a plain value the tests (and only they)
/// point at a temp root.
#[derive(Debug, Clone)]
pub(crate) struct DataRoot(pub(crate) PathBuf);

/// Why [`ensure_data_layout`] gave up: the path it could not create or
/// secure, plus the OS cause. The path is kept in the message (not only
/// the chain) because the startup failure surfaces as tauri's
/// `Failed to setup app: …` panic, which prints the `Display` form of
/// the error chain — a bare "Permission denied" without the path would
/// send the user looking at the wrong thing.
#[derive(Debug)]
pub(crate) struct DataLayoutError {
    /// The directory `create_dir_all` was asked for when it failed —
    /// the root itself or one of the four layout subdirectories — or
    /// the backups directory whose SEC-007 mode could not be set.
    path: PathBuf,
    source: std::io::Error,
}

impl fmt::Display for DataLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to create or secure the `{}` directory: {}",
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
/// previous run stopped. On Unix the two backup directories are
/// additionally held at `0700` (SEC-007) — the mode is re-asserted on
/// every run, so a directory that already exists with wider bits
/// converges to `0700` too, not just a freshly created one (see
/// [`secure_backups_dir`]). Returns the first failure as
/// [`DataLayoutError`] — the app has no meaningful startup without the
/// layout (no database, no logs, no config), so callers abort the
/// launch instead of degrading silently.
///
/// # Errors
///
/// Fails when `root` or any subdirectory cannot be created — the path
/// is occupied by a regular file, an ancestor is not a directory, or
/// the OS denies the write — or when a backup directory's `0700` mode
/// cannot be set. The error carries the offending path and the
/// underlying [`std::io::Error`].
pub(crate) fn ensure_data_layout(root: &Path) -> Result<(), DataLayoutError> {
    create_dir(root)?;
    for name in SUBDIRECTORIES {
        create_dir(&root.join(name))?;
    }
    // SEC-007: both backup directories are owner-only (`0700`) on
    // Unix. Unconditional, so the run converges the mode on existing
    // directories as well — creation alone would leave a
    // umask-filtered mode in place for anything this run did not
    // create.
    secure_backups_dir(&root.join(BACKUPS_SUBDIRECTORY))?;
    secure_backups_dir(&root.join(DB_BACKUPS_SUBDIRECTORY))?;
    Ok(())
}

/// One `create_dir_all` step with the failing path attached.
fn create_dir(path: &Path) -> Result<(), DataLayoutError> {
    std::fs::create_dir_all(path).map_err(|source| DataLayoutError {
        path: path.to_path_buf(),
        source,
    })
}

/// Enforces SEC-007's `0700` on one backups directory: no group or
/// other access, exact mode — a chmod, not a `DirBuilder` mode, so
/// neither a pre-existing directory nor the process umask can leave
/// wider bits in place. The copies inside (DB copies in `db-backups/`,
/// project file backups in `backups/`) are the user's data, which is
/// what the directory mode shields at rest.
#[cfg(unix)]
fn secure_backups_dir(path: &Path) -> Result<(), DataLayoutError> {
    use std::os::unix::fs::PermissionsExt;
    const BACKUPS_DIR_MODE: u32 = 0o700;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(BACKUPS_DIR_MODE)).map_err(
        |source| DataLayoutError {
            path: path.to_path_buf(),
            source,
        },
    )
}

/// The non-Unix half of [`secure_backups_dir`]: SEC-007 inherits the
/// user profile's rights on Windows, so nothing is chmod'ed.
#[cfg(not(unix))]
fn secure_backups_dir(_path: &Path) -> Result<(), DataLayoutError> {
    Ok(())
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

    // ---- SEC-007 directory modes (Unix only) --------------------------------
    //
    // Spec §13 SEC-007: the backups folders are `0700`. Both of them:
    // `backups/` (project file backups) and `db-backups/` (the DB
    // copies, D-046's assignment to this requirement). Windows inherits
    // the profile's rights, so these tests — and the enforcement — are
    // Unix-gated; the app-layer end-to-end mode test lives in
    // `db_tests.rs`.

    /// Reads a directory's permission bits masked to the standard
    /// `0o777`.
    #[cfg(unix)]
    fn dir_mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .unwrap_or_else(|err| panic!("stat of {} must work: {err}", path.display()))
            .permissions()
            .mode()
            & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn the_backup_directories_are_owner_only_after_the_first_run() {
        // Arrange: a clean system — the data directory does not exist.
        let (_temp, root) = nonexistent_root();

        // Act: the first startup.
        ensure_data_layout(&root).expect("the layout must be created in a temp directory");

        // Assert: both backups directories are exactly `0700` — the
        // exact mode, so a umask-filtered `0755` default fails as
        // loudly as a dropped enforcement. The literal names, like the
        // layout pins above (never `super::BACKUPS_SUBDIRECTORY`).
        assert_eq!(
            dir_mode(&root.join("backups")),
            0o700,
            "`backups/` must be owner-only (SEC-007)"
        );
        assert_eq!(
            dir_mode(&root.join("db-backups")),
            0o700,
            "`db-backups/` must be owner-only (SEC-007)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_loosened_backups_directory_converges_back_on_the_next_run() {
        // Arrange: a completed layout, then one backups directory
        // loosened to the OS default — the state a restore from a
        // copy or a tool with its own mode policy leaves behind.
        use std::os::unix::fs::PermissionsExt;
        let (_temp, root) = nonexistent_root();
        ensure_data_layout(&root).expect("the first run must succeed");
        fs::set_permissions(root.join("db-backups"), fs::Permissions::from_mode(0o755))
            .expect("loosening a temp directory's mode must work");

        // Act: the next startup runs the same seam again.
        ensure_data_layout(&root).expect("the rerun must succeed");

        // Assert: converged back to `0700` — the mode is a property
        // the layout enforces, not a birth right of the first run.
        assert_eq!(
            dir_mode(&root.join("db-backups")),
            0o700,
            "a loosened backups directory must be re-tightened (SEC-007)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_rerun_leaves_the_backup_directories_at_the_required_mode() {
        // The idempotency half of the enforcement: the second run of
        // an already-correct layout changes nothing about the modes.
        let (_temp, root) = nonexistent_root();
        ensure_data_layout(&root).expect("the first run must succeed");

        ensure_data_layout(&root).expect("the second run must succeed");

        assert_eq!(
            dir_mode(&root.join("backups")),
            0o700,
            "`backups/` stays owner-only across reruns (SEC-007)"
        );
        assert_eq!(
            dir_mode(&root.join("db-backups")),
            0o700,
            "`db-backups/` stays owner-only across reruns (SEC-007)"
        );
    }

    #[test]
    fn the_database_file_path_is_the_spec_five_five_name() {
        // Arrange & Act: the path the DB executor opens, built from a
        // root whose name does not collide with the expected segments
        // (so a wrongly joined path cannot satisfy the pin by accident).
        let root = Path::new("/data");
        let path = db_file_path(root);

        // Assert: the spec §5.5 structure block pins
        // `db/eunomiasync.sqlite3` — as a literal, never derived from
        // the module's own constants (the same mutation honesty as the
        // layout pins above). The `-wal`/`-shm` siblings are SQLite's
        // business, not part of the name this function returns.
        assert_eq!(
            path,
            Path::new("/data/db/eunomiasync.sqlite3"),
            "the database file must live at `db/eunomiasync.sqlite3` \
             inside the data directory (spec §5.5)"
        );
    }

    #[test]
    fn the_backup_directory_is_the_spec_five_five_name() {
        // Arrange & Act: the directory the DB executor hands to
        // `run_migrations` for its pre-migration safety copies
        // (DB-004, D-038), from the same non-colliding root.
        let root = Path::new("/data");
        let dir = db_backups_dir(root);

        // Assert: the spec §5.5 structure block pins `db-backups/` —
        // literal, like the database file pin beside it.
        assert_eq!(
            dir,
            Path::new("/data/db-backups"),
            "the pre-migration backups must live in `db-backups/` \
             inside the data directory (spec §5.5, D-038)"
        );
    }
}
