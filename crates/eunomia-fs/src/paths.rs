//! Path normalization and validation.
//!
//! Planned responsibility (docs/spec.md 5.2): canonicalize and normalize
//! repository paths so that they compare equal across platforms and app
//! restarts — `dunce` keeps canonical Windows paths free of verbatim
//! (`\\?\`) prefixes — and provide the path checks used when validating
//! paths that arrive from the UI (spec 5.6, SEC-003: every UI-supplied
//! path is checked in Rust). The registered-project and session-picked
//! allowlists that feed those checks live in the app and database layers;
//! this module supplies the normalization primitives and checks, not the
//! allowlist policy.
//!
//! Implemented so far: the project-folder availability check
//! ([`check_project_available`], spec 6.9's project-path row, 9.4
//! preflight step 1). Still planned: canonicalization (`dunce`, DB-030's
//! `path_key` shape) and the SEC-003 allowlist path checks.
//!
//! # Testing plan
//!
//! Availability is tested through the public function against real
//! temporary fixtures: the happy `.git`-directory and `.git`-file
//! forms, a plain file and a missing path rejected, a vanished `.git`
//! flipping the verdict, permission-denied folders (skipped loudly on
//! a root account, where mode bits are not enforced), the OS error
//! code riding on the typed error, and the DB-032 contract that a
//! folder on another filesystem (and a path shaped like a
//! scan-excluded name) passes like any local folder. Canonicalization
//! and the SEC-003 checks keep their planned coverage: relative,
//! absolute, and symlinked paths, the Windows verbatim-prefix case,
//! and rejection of paths outside the registered or session-allowed
//! set.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// Checks that a registered project's folder is available: it exists,
/// is a directory, is readable, and contains a `.git` entry.
///
/// Spec 6.9's project-path row ("Exists, is a folder with `.git`
/// (folder or file), is readable") and 9.4 preflight step 1 ("Check
/// that the folder is available. No access: `skipped`,
/// `E_PROJECT_UNAVAILABLE`"): the startup pass, the window-open pass
/// and the sync pipeline (DB-032) all reduce to this one verdict, and
/// D-09's "unavailable" project is the failure half of it.
///
/// # The verdict is four probes, nothing else
///
/// 1. `stat` of the root (symlinks followed: a project root that is
///    itself a link to the real repository checks the repository).
///    `NotFound` → [`ProjectUnavailableError::Missing`]; any other OS
///    refusal — denied access to the folder or an ancestor, a
///    disconnected drive's EIO/ENODEV, a symlink loop, a name too
///    long — → [`ProjectUnavailableError::NotReadable`]: the folder is
///    not plainly gone, it cannot be inspected, and the carried
///    [`io::Error`] keeps the exact OS code.
/// 2. `is_dir()` — anything else on the path (a plain file, or a
///    symlink to one) is [`ProjectUnavailableError::NotADirectory`].
/// 3. One probe read: `read_dir` opens the directory, which is what
///    "is readable" means for a directory (listability). Permission
///    errors surface here with their OS code. A search-only folder
///    (Unix `x` without `r`) is therefore reported unavailable even
///    though named paths inside could still be opened — the stricter
///    of the two readings, taken because the app itself lists the
///    folder (the repository scan reads entry names, ALG-054) rather
///    than poking fixed paths.
/// 4. `symlink_metadata(<root>/.git)` — ANY entry named `.git` counts:
///    a directory (a normal repository) or a file (a worktree or
///    submodule gitdir pointer — ALG-052's other form). The scan
///    distinguishes the two (ALG-052 skips the file form); availability
///    only needs "there is a `.git` entry", so the file form is
///    deliberately not rejected. `symlink_metadata` rather than
///    `metadata`: presence, not validity, is the contract — a `.git`
///    symlink is still an entry, and whether it points at a healthy
///    gitdir is git's business, not this check's. `NotFound` here is
///    [`ProjectUnavailableError::NoGitEntry`]; any other error is
///    [`ProjectUnavailableError::NotReadable`].
///
/// # Network and removable drives are allowed (DB-032)
///
/// The check is purely metadata: one `stat`, one directory open, one
/// entry `stat`. It reads no device numbers, no mount tables, no
/// volume flags, and sniffs no path shapes (`/mnt/…`, `\\UNC\…`) —
/// a repository on an NFS share, a USB drive, a `\\server\share` UNC
/// path or a tmpfs mount passes exactly like a local one. There is
/// deliberately no seam for a drive-type heuristic to plug into: the
/// four probes above are the entire decision. The tests pin the
/// contract with a cross-filesystem fixture and with paths shaped
/// like names the repository scan excludes.
///
/// # Races are honest (TOCTOU)
///
/// The probes are separate OS calls over a live filesystem: the folder
/// can lose its `.git` between them, a drive can drop mid-check, a
/// plain file can replace the folder between `stat` and the probe
/// read (that surfaces as [`ProjectUnavailableError::NotReadable`]
/// with the OS code for "not a directory"). The function reports what
/// the probes observed at their moment — no snapshot is taken and no
/// atomicity is pretended. The availability verdict is recomputed at
/// every startup, window open and sync (DB-032), so a flip between two
/// checks is simply the next check's verdict.
///
/// # Errors
///
/// `Ok(())` means the folder is available. Every failure maps, at the
/// command layer, to `E_PROJECT_UNAVAILABLE` (§10: folder moved or
/// deleted, drive disconnected, no read rights) with the OS code as
/// detail — [`ProjectUnavailableError::io_error`] and
/// [`ProjectUnavailableError::os_code`] feed that detail. The reasons
/// stay distinguishable in the enum because the add-project flow's
/// `E_NOT_A_REPO` (a folder without `.git`) is a different consumer
/// that may reuse the same check.
pub fn check_project_available(root: &Path) -> Result<(), ProjectUnavailableError> {
    let metadata = fs::metadata(root).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            ProjectUnavailableError::Missing { source }
        } else {
            ProjectUnavailableError::NotReadable { source }
        }
    })?;
    if !metadata.is_dir() {
        return Err(ProjectUnavailableError::NotADirectory);
    }
    // The open is the probe read: `read_dir` opens the directory
    // eagerly, so a permission or I/O failure surfaces on this call;
    // the entries themselves are not needed.
    fs::read_dir(root).map_err(|source| ProjectUnavailableError::NotReadable { source })?;
    match fs::symlink_metadata(root.join(".git")) {
        Ok(_) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            Err(ProjectUnavailableError::NoGitEntry)
        }
        Err(source) => Err(ProjectUnavailableError::NotReadable { source }),
    }
}

/// Why [`check_project_available`] said no.
///
/// One variant per §10 situation behind `E_PROJECT_UNAVAILABLE` (plus
/// the plain "not a folder" and "no `.git` entry" outcomes of the §6.9
/// row), so the command layer can log the reason and the OS code
/// without re-deriving them from strings. I/O-backed variants carry
/// the [`io::Error`] itself as their `source` — its `raw_os_error`
/// and `ErrorKind` are §10's first-class log facts, and
/// `ErrorDetails::from_io_error` in the app layer consumes exactly
/// that shape.
#[derive(Debug)]
pub enum ProjectUnavailableError {
    /// The folder is not there: `stat` answered ENOENT — moved,
    /// deleted, or the drive unmounted (D-09's situation list).
    Missing {
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
    /// The path exists but is not a directory: a plain file (or a
    /// symlink to one) occupies the registered path.
    NotADirectory,
    /// The folder cannot be inspected or read: access denied on the
    /// folder or an ancestor, the drive disconnected mid-check, a
    /// symlink loop — every OS refusal that is not a plain
    /// `NotFound`. The exact OS code rides in the source error.
    NotReadable {
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
    /// The folder is readable but has no `.git` entry: the directory
    /// lost its repository, or never had one.
    NoGitEntry,
}

impl ProjectUnavailableError {
    /// The I/O error behind this failure, when an OS call produced
    /// it; `None` for the verdicts that are plain facts about the
    /// path (`NotADirectory`, `NoGitEntry`) rather than OS failures.
    pub fn io_error(&self) -> Option<&io::Error> {
        match self {
            ProjectUnavailableError::Missing { source }
            | ProjectUnavailableError::NotReadable { source } => Some(source),
            ProjectUnavailableError::NotADirectory | ProjectUnavailableError::NoGitEntry => None,
        }
    }

    /// The raw OS error code (`std::io::Error::raw_os_error`): §10
    /// names it a first-class fact of every error log line. `None`
    /// where no OS call failed.
    pub fn os_code(&self) -> Option<i32> {
        self.io_error().and_then(io::Error::raw_os_error)
    }
}

impl fmt::Display for ProjectUnavailableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectUnavailableError::Missing { source } => {
                write!(f, "project folder does not exist: {source}")
            }
            ProjectUnavailableError::NotADirectory => {
                write!(f, "project path is not a directory")
            }
            ProjectUnavailableError::NotReadable { source } => {
                write!(f, "project folder cannot be read: {source}")
            }
            ProjectUnavailableError::NoGitEntry => {
                write!(f, "project folder contains no .git entry")
            }
        }
    }
}

impl std::error::Error for ProjectUnavailableError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.io_error()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use tempfile::TempDir;

    /// The fixture's owner uid is this process's effective uid (the
    /// fixture was created by it): uid 0 means the tests run as root,
    /// where mode bits are not enforced and a chmod-000 directory
    /// stays listable — the precondition of a permission test cannot
    /// be set up. The caller skips loudly instead of passing
    /// vacuously (precedent: `running_as_root_is_untestable` in
    /// eunomia-store).
    #[cfg(unix)]
    fn running_as_root_is_untestable(temp: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(temp)
            .expect("the fixture directory must be statable")
            .uid()
            == 0
    }

    fn repo_with_git_entry(dir: &Path, name: &str) -> std::path::PathBuf {
        let repo = dir.join(name);
        fs::create_dir_all(repo.join(".git")).expect("creating the fixture repository must work");
        repo
    }

    #[test]
    fn a_folder_with_a_git_directory_is_available() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");

        check_project_available(&repo)
            .expect("a folder with a .git directory is the happy path of the §6.9 row");
    }

    #[test]
    fn a_folder_with_a_git_file_is_available() {
        // The worktree/submodule form: `.git` is a FILE holding a
        // `gitdir:` pointer (ALG-052's other form). Availability needs
        // only "there is a .git entry" — the scan's skip of the file
        // form is a scan rule, not an availability rule.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("worktree");
        fs::create_dir(&repo).expect("creating the fixture repository must work");
        fs::write(repo.join(".git"), "gitdir: ../main/repo/.git\n")
            .expect("writing the gitdir pointer must work");

        check_project_available(&repo)
            .expect("a .git FILE must be accepted exactly like a .git directory");
    }

    #[test]
    fn a_plain_file_is_reported_not_a_directory() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let file = dir.path().join("plain.txt");
        fs::write(&file, b"not a folder").expect("writing the fixture file must work");

        let err = check_project_available(&file)
            .expect_err("a plain file must fail the is-a-folder half of the §6.9 row");
        assert!(
            matches!(&err, ProjectUnavailableError::NotADirectory),
            "a plain file must be NotADirectory, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            None,
            "the stat succeeded, so no OS code exists for this verdict"
        );
    }

    #[test]
    fn a_missing_path_is_reported_missing() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let gone = dir.path().join("never-created");

        let err = check_project_available(&gone)
            .expect_err("a path that does not exist must be unavailable (D-09)");
        assert_eq!(
            err.os_code().map(|_| ()),
            Some(()),
            "an io-backed verdict carries an OS code"
        );
        match &err {
            ProjectUnavailableError::Missing { source } => {
                assert_eq!(
                    source.kind(),
                    io::ErrorKind::NotFound,
                    "a vanished folder is ENOENT"
                );
                #[cfg(unix)]
                assert_eq!(
                    source.raw_os_error(),
                    Some(2),
                    "ENOENT is the OS code the §10 log line carries"
                );
            }
            other => panic!("a missing path must be Missing, got: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_without_read_permission_is_reported_not_readable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");
        if running_as_root_is_untestable(&repo) {
            eprintln!(
                "skipping a_folder_without_read_permission_is_reported_not_readable: \
                 running as root — mode bits are not enforced, the \
                 unreadable-folder precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&repo, fs::Permissions::from_mode(0o000))
            .expect("removing all mode bits must work");
        let result = check_project_available(&repo);
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(&repo, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        let err = result.expect_err("a folder that cannot be listed must be unavailable");
        assert!(
            matches!(&err, ProjectUnavailableError::NotReadable { .. }),
            "a 000-mode folder must be NotReadable, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(13),
            "EACCES is the OS code the §10 log line carries for denied access"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_readable_but_unsearchable_folder_surfaces_the_denied_git_stat() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");
        if running_as_root_is_untestable(&repo) {
            eprintln!(
                "skipping a_readable_but_unsearchable_folder_surfaces_the_denied_git_stat: \
                 running as root — mode bits are not enforced, the \
                 unsearchable-folder precondition cannot be set up"
            );
            return;
        }

        // r without x: the probe read (listing) succeeds, but the
        // `.git` entry cannot be statted — the second NotReadable
        // producer, distinct from the denied listing above.
        fs::set_permissions(&repo, fs::Permissions::from_mode(0o400))
            .expect("removing the search bits must work");
        // Prove the failure below comes from the `.git` stat and not
        // from an earlier probe: with mode 0o400 the listing itself
        // must still succeed. The check is captured while the fixture
        // is locked but asserted only after the restore, so even a
        // precondition panic cannot strand an unremovable fixture.
        let listing_still_ok = fs::read_dir(&repo).is_ok();
        let result = check_project_available(&repo);
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(&repo, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");
        assert!(
            listing_still_ok,
            "precondition: a 0o400 folder must still be listable"
        );

        let err =
            result.expect_err("a folder whose .git entry cannot be statted must be unavailable");
        assert!(
            matches!(&err, ProjectUnavailableError::NotReadable { .. }),
            "a denied .git stat must be NotReadable, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(13),
            "EACCES is the OS code the §10 log line carries for denied access"
        );
    }

    #[test]
    fn a_vanished_git_entry_flips_the_verdict() {
        // The card's TOCTOU acceptance case: the folder HAS `.git`,
        // the check passes, the entry disappears, the next check
        // reports the folder unavailable — the verdict is what the
        // probes observed, not a cached snapshot.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");
        check_project_available(&repo).expect("the repository is available while .git exists");

        fs::remove_dir_all(repo.join(".git")).expect("removing the .git entry must work");
        let err = check_project_available(&repo)
            .expect_err("after .git disappears the folder must be unavailable");
        assert!(
            matches!(&err, ProjectUnavailableError::NoGitEntry),
            "a folder without .git must be NoGitEntry, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            None,
            "the folder itself is readable; no OS call failed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_git_symlink_is_still_a_git_entry() {
        // Presence, not validity: a `.git` symlink counts as the
        // entry, and whether it points at a healthy gitdir is git's
        // business, not this check's.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("linked");
        fs::create_dir(&repo).expect("creating the fixture repository must work");
        std::os::unix::fs::symlink("/nowhere/gitdir", repo.join(".git"))
            .expect("creating the .git symlink must work");

        check_project_available(&repo).expect("a .git entry that is a symlink counts");
    }

    #[cfg(unix)]
    #[test]
    fn io_backed_failures_carry_the_raw_os_error_code() {
        // The `.git` probe under a path whose parent is a plain file:
        // ENOTDIR, not ENOENT — classified NotReadable (the path
        // cannot be a folder), with the errno riding in the typed
        // error so the §10 log line keeps its OS code even where the
        // reason bucket is coarse. Runs regardless of privileges,
        // unlike the permission tests.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let file = dir.path().join("plain.txt");
        fs::write(&file, b"not a folder").expect("writing the fixture file must work");

        let err = check_project_available(&file.join(".git"))
            .expect_err("a .git probe under a plain file must fail");
        assert!(
            matches!(&err, ProjectUnavailableError::NotReadable { .. }),
            "ENOTDIR must be NotReadable, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(20),
            "ENOTDIR = 20 must ride into the typed error"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_project_root_checks_through_to_the_repository() {
        // The root stat follows symlinks: a project registered as a
        // link to the real repository checks the repository. (DB-031's
        // aliasing of two canonical paths is the canonicalization
        // task's concern; availability simply follows the link.)
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = repo_with_git_entry(dir.path(), "repo");
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&repo, &alias).expect("creating the root symlink must work");

        check_project_available(&alias)
            .expect("a symlinked project root checks through to the repository");

        // A link whose target is gone is the "vanished folder" shape:
        // ENOENT at the stat, reported as Missing.
        let dangling = dir.path().join("dangling");
        std::os::unix::fs::symlink(dir.path().join("never-created"), &dangling)
            .expect("creating the dangling symlink must work");
        let err = check_project_available(&dangling)
            .expect_err("a dangling root link must be unavailable");
        assert!(
            matches!(&err, ProjectUnavailableError::Missing { .. }),
            "a dangling root link must be Missing, got: {err:?}"
        );
    }

    #[test]
    fn display_texts_and_the_source_chain_follow_the_contract() {
        // The command layer branches on the variants and logs the
        // Display text; these assertions pin the wording (lowercase,
        // no trailing punctuation — the house style) and the source()
        // chain without pinning std's strerror table.
        let not_a_directory = ProjectUnavailableError::NotADirectory;
        assert_eq!(
            not_a_directory.to_string(),
            "project path is not a directory"
        );
        let no_git_entry = ProjectUnavailableError::NoGitEntry;
        assert_eq!(
            no_git_entry.to_string(),
            "project folder contains no .git entry"
        );

        let missing = ProjectUnavailableError::Missing {
            source: io::Error::from_raw_os_error(13),
        };
        assert!(
            missing
                .to_string()
                .starts_with("project folder does not exist: "),
            "the Missing text must state the fact, then the OS text: {}",
            missing
        );
        let not_readable = ProjectUnavailableError::NotReadable {
            source: io::Error::from_raw_os_error(13),
        };
        assert!(
            not_readable
                .to_string()
                .starts_with("project folder cannot be read: "),
            "the NotReadable text must state the fact, then the OS text: {}",
            not_readable
        );

        // source() chains exactly to the carried io error on the
        // io-backed variants, and is None where no OS call failed.
        assert_eq!(
            missing.source().map(|source| source.to_string()),
            Some(io::Error::from_raw_os_error(13).to_string())
        );
        assert!(not_readable.source().is_some());
        assert!(not_a_directory.source().is_none());
        assert!(no_git_entry.source().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_on_another_filesystem_is_available() {
        use std::os::unix::fs::MetadataExt;

        // DB-032: network and removable drives are allowed. A real
        // network share cannot be mounted from a unit test; the honest
        // stand-in is a fixture on a filesystem DIFFERENT from the
        // test's own temp dir: `/dev/shm` is a separate tmpfs mount
        // with its own st_dev. A drive-type heuristic — the "reject
        // non-local volumes" shortcut this contract forbids — keys on
        // exactly that mount identity, so a heuristic would reject
        // this fixture; the four-probe check must accept it.
        let shm = Path::new("/dev/shm");
        let Ok(shm_metadata) = fs::metadata(shm) else {
            eprintln!(
                "skipping a_folder_on_another_filesystem_is_available: \
                 no /dev/shm on this system, no cross-filesystem fixture possible"
            );
            return;
        };
        let temp_dev = fs::metadata(std::env::temp_dir())
            .expect("the temp directory must be statable")
            .dev();
        if shm_metadata.dev() == temp_dev {
            eprintln!(
                "skipping a_folder_on_another_filesystem_is_available: \
                 /dev/shm sits on the same filesystem as the temp dir, \
                 no cross-filesystem fixture possible"
            );
            return;
        }

        let mount = match tempfile::Builder::new()
            .prefix("eunomia-fs-t060-crossfs-")
            .tempdir_in(shm)
        {
            Ok(mount) => mount,
            Err(error) => {
                eprintln!(
                    "skipping a_folder_on_another_filesystem_is_available: \
                     a fixture cannot be created under /dev/shm ({error}), \
                     no cross-filesystem fixture possible"
                );
                return;
            }
        };
        let repo = mount.path().join("repo");
        fs::create_dir_all(repo.join(".git")).expect("creating the fixture repository must work");

        check_project_available(&repo).expect(
            "a repository on another filesystem must be available: \
             DB-032 allows non-local drives, and the check consults \
             only existence, type, readability and the .git entry",
        );
    }

    #[test]
    fn a_path_shaped_like_a_scan_excluded_name_is_available() {
        // ALG-051's exclusion names belong to the repository SCAN; a
        // "reject mount-point-shaped paths" shortcut would match the
        // `mnt` component. Availability's contract is only the four
        // probes, so a repository stored under either shape is checked
        // like any other folder — the same DB-032 "no path-shape
        // sniffing" promise as the cross-filesystem test, exercised on
        // path TEXT instead of mount identity.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let recycled = dir.path().join("$RECYCLE.BIN").join("repo");
        fs::create_dir_all(recycled.join(".git"))
            .expect("creating the fixture repository must work");
        let mnt = dir.path().join("mnt").join("repo");
        fs::create_dir_all(mnt.join(".git")).expect("creating the fixture repository must work");

        check_project_available(&recycled)
            .expect("availability must not reuse the scan's exclusion names");
        check_project_available(&mnt).expect("availability must not sniff path shapes");
    }
}
