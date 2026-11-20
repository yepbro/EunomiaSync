//! Platform matrix tests for the repository scan (task T-070,
//! docs/spec.md 9.6, 12.1/15.3, NFR-001…003): the ALG-050..053 walk
//! and classification driven over each platform's real filesystem
//! flavor of a "link" or a "name spelling".
//!
//! Groups:
//!
//! * Linux (runs locally on the Linux leg): NFC- and NFD-named
//!   repositories are TWO projects there — the filesystem keeps the
//!   bytes, the counter-truth that makes the macOS folding row
//!   meaningful; and the unreadable-folder traversal of §15.3's
//!   "rights and owners" row re-pinned on the matrix leg in its
//!   pipeline shape — the exact single access skip (EACCES riding it
//!   for the §10 log line), no repository reported from behind the
//!   refusal, the sibling still found, the walk completed.
//! * Windows: a junction inside the scan root is not followed
//!   (ALG-050's reparse-point rule in its native flavor — no
//!   repository may be seen through it), and a repository beyond the
//!   260-character MAX_PATH bound is still found.
//! * macOS: a repository created with an NFD name classifies `known`
//!   against a registry keyed by the NFC spelling — §15.3's "NFD
//!   names" row end to end: one identity, no duplicate "new" entity.
//!
//! Symlink non-following for the Unix flavors is pinned by the inline
//! `scan` tests (they run on the Linux and macOS legs); the Windows
//! flavor of the same rule is the junction test here. The Windows and
//! macOS groups compile on the other targets through the cross-target
//! checks (the D-099 precedent) and run on their CI matrix legs.

mod common;

use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use eunomia_fs::paths::canonicalize;
use eunomia_fs::scan::{CancelToken, ScanEvent, ScanOptions, ScanOutcome, scan_repositories};
use tempfile::TempDir;

/// What a walk collected: the found repositories and the skips with
/// their OS codes (`SkipReason::os_code`); plain directories do not
/// interest these tests and are counted nowhere.
#[derive(Default)]
struct WalkLog {
    repositories: Vec<PathBuf>,
    skipped: Vec<(PathBuf, Option<i32>)>,
}

/// A minimal scan — depth 4, no exclusions, a fresh cancel token —
/// collecting every repository and skip event. The log rides a
/// `Mutex` because the callback is `Fn` (several walker threads share
/// it); the root is passed CANONICAL by every caller (the D-098
/// caller obligation), the way the command layer will pass it.
fn scan_collecting(root: &Path) -> (WalkLog, ScanOutcome) {
    let log = Mutex::new(WalkLog::default());
    let outcome = scan_repositories(
        root,
        &ScanOptions {
            max_depth: NonZeroUsize::new(4).expect("4 is non-zero"),
            exclude_names: Vec::new(),
        },
        &CancelToken::new(),
        |event| match event {
            ScanEvent::Repository { path } => {
                let mut log = log
                    .lock()
                    .expect("the walker threads must not poison the log");
                log.repositories.push(path.clone());
            }
            ScanEvent::Skipped { path, reason } => {
                let code = reason.os_code();
                let mut log = log
                    .lock()
                    .expect("the walker threads must not poison the log");
                log.skipped.push((path.clone(), code));
            }
            ScanEvent::Directory { .. } => {}
        },
    );
    (
        log.into_inner()
            .expect("the walk ended with the log intact"),
        outcome,
    )
}

/// A fixture folder named `name` with a `.git` directory inside: a
/// repository the walk must find (ALG-052).
fn repository(parent: &Path, name: &str) -> PathBuf {
    let repo = parent.join(name);
    fs::create_dir_all(repo.join(".git")).expect("creating the fixture repository must work");
    repo
}

/// §15.3's Linux "rights and owners" and macOS "NFD names" rows, on
/// the leg whose filesystem keeps name bytes: ext4 and friends make
/// every spelling a distinct folder, so identity rules that FOLD must
/// not be applied here (DB-030's last clause).
#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    use eunomia_fs::paths::canonical_path_key;
    use eunomia_fs::scan_classify::{ScanClass, ScanRegistry, classify_repository};

    #[test]
    fn nfc_and_nfd_named_repositories_are_two_projects() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        // The two spellings of café: precomposed and decomposed. On
        // Linux both folders coexist — two real names.
        let nfc = "caf\u{e9}";
        let nfd = "caf\u{65}\u{301}";
        assert_ne!(nfc, nfd);
        repository(dir.path(), nfc);
        repository(dir.path(), nfd);

        let root = canonicalize(dir.path()).expect("the scan root must canonicalize");
        let (log, outcome) = scan_collecting(&root);
        assert_eq!(outcome, ScanOutcome::Completed);

        let mut found: Vec<String> = log
            .repositories
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("a repository path has a file name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        found.sort_unstable();
        assert_eq!(found, vec![nfd.to_owned(), nfc.to_owned()]);

        // Two identities (DB-030: Linux keeps the bytes): distinct
        // keys, and a registry built from the first classifies it
        // `known` while the other stays `new` — two projects, one
        // scan, no false merge.
        let key_nfc = canonical_path_key(&root.join(nfc)).expect("the NFC folder must key");
        let key_nfd = canonical_path_key(&root.join(nfd)).expect("the NFD folder must key");
        assert_ne!(key_nfc, key_nfd);
        let registry = ScanRegistry::new(std::slice::from_ref(&key_nfc), &[], &[]);
        for path in &log.repositories {
            let is_nfc = path
                .file_name()
                .expect("a repository path has a file name")
                .to_string_lossy()
                == nfc;
            let class = classify_repository(path, &registry).class();
            assert_eq!(
                class,
                if is_nfc {
                    ScanClass::Known
                } else {
                    ScanClass::New
                },
                "the verdict must follow the folder's own key, not a folded \
                 merge: {}",
                path.display()
            );
        }
    }

    #[test]
    fn a_repository_inside_an_unreadable_folder_is_skipped_with_eacces() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = canonicalize(dir.path()).expect("the scan root must canonicalize");
        let sealed = repository(&root, "sealed");
        repository(&root, "open-repo");
        // Root-run mode bits are not enforced, so the unreadable-folder
        // precondition cannot be set up (the house skip pattern).
        if running_as_root(&root) {
            eprintln!(
                "skipping a_repository_inside_an_unreadable_folder_is_skipped_with_eacces: \
                 running as root — mode bits are not enforced, the \
                 unreadable-folder precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000))
            .expect("removing all mode bits must work");
        let (log, outcome) = scan_collecting(&root);
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        assert_eq!(outcome, ScanOutcome::Completed);
        // The pipeline shape over the inline traversal's facts: the
        // refusal arrives as EXACTLY one access skip for the sealed
        // folder — not descended (no listing event), not probed into
        // a repository, carrying EACCES for the §10 log line.
        assert_eq!(
            log.skipped,
            vec![(sealed.clone(), Some(13))],
            "the sealed folder yields exactly one access skip with EACCES"
        );
        assert!(
            log.repositories
                .iter()
                .all(|path| !path.starts_with(&sealed)),
            "no repository may be reported from behind the unreadable folder: \
             {:?}",
            log.repositories
        );
        assert!(
            log.repositories
                .iter()
                .any(|path| path.ends_with("open-repo")),
            "the refusal must not stop the walk: the sibling repository is found"
        );
    }

    /// The house root probe (the `running_as_root_is_untestable`
    /// idiom of the inline suites): mode-bit fixtures are vacuous
    /// under root.
    fn running_as_root(temp: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;

        fs::metadata(temp)
            .expect("the fixture directory must be statable")
            .uid()
            == 0
    }
}

/// §15.3's Windows rows for the walk: junctions and long paths.
#[cfg(windows)]
mod windows_reparse_and_long_path {
    use super::*;

    use common::{VERBATIM_PREFIX, deep_verbatim, make_junction};
    use std::path::PathBuf;

    #[test]
    fn a_junction_inside_the_root_is_not_followed() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        // A repository OUTSIDE the scan root, and a junction to it
        // INSIDE the root: following the junction would report the
        // repository twice over (once real, once through the link).
        let external = repository(dir.path(), "external-repo");
        let root = dir.path().join("work");
        fs::create_dir(&root).expect("creating the scan root must work");
        repository(&root, "plain-repo");
        let junction: PathBuf = root.join("linked-repo");
        make_junction(&junction, &external);

        let canonical_root = canonicalize(&root).expect("the scan root must canonicalize");
        let (log, outcome) = scan_collecting(&canonical_root);
        assert_eq!(outcome, ScanOutcome::Completed);
        // ALG-050: the reparse point is not followed — no repository
        // is seen through it, and nothing is even reported spelled
        // through it (a junction entry is not a directory event).
        assert!(
            log.repositories
                .iter()
                .all(|path| !path.starts_with(canonical_root.join("linked-repo"))),
            "no repository may be seen through the junction: {:?}",
            log.repositories
        );
        assert!(
            log.skipped.is_empty(),
            "a junction yields no skip either — it is simply not descended \
             into: {:?}",
            log.skipped
        );
        // The walk itself works: the plain sibling is found.
        assert!(
            log.repositories
                .iter()
                .any(|path| path.ends_with("plain-repo")),
            "the repository beside the junction is found: {:?}",
            log.repositories
        );
    }

    #[test]
    fn a_repository_beyond_max_path_is_found() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let deep = deep_verbatim(dir.path(), 7);
        let repo = repository(&deep, "Repository");
        let simplified = repo
            .to_string_lossy()
            .trim_start_matches(VERBATIM_PREFIX)
            .len();
        assert!(
            simplified > 260,
            "precondition: the fixture must exceed MAX_PATH even simplified, \
             got {simplified} characters"
        );

        // The caller's canonical root (the D-098 obligation) keeps the
        // verbatim form past MAX_PATH (D-086); the walk runs through
        // it and finds the repository.
        let root = canonicalize(&deep).expect("the deep scan root must canonicalize");
        let (log, outcome) = scan_collecting(&root);
        assert_eq!(outcome, ScanOutcome::Completed);
        assert!(
            log.repositories
                .iter()
                .any(|path| path.ends_with("Repository")),
            "the repository beyond MAX_PATH must be found: {:?}",
            log.repositories
        );
    }
}

/// §15.3's macOS "NFD names" row, end to end through the walk and the
/// classifier: a decomposed folder name is one identity, not a
/// duplicate "new" entity beside its NFC key.
#[cfg(target_os = "macos")]
mod macos_nfd_names {
    use super::*;

    use eunomia_fs::paths::canonical_path_key;
    use eunomia_fs::scan_classify::{ScanClass, ScanRegistry, classify_repository};

    #[test]
    fn an_nfd_named_repository_classifies_known_against_its_nfc_registry_key() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let nfc = "caf\u{e9}";
        let nfd = "caf\u{65}\u{301}";
        repository(dir.path(), nfd);

        let root = canonicalize(dir.path()).expect("the scan root must canonicalize");
        let (log, outcome) = scan_collecting(&root);
        assert_eq!(outcome, ScanOutcome::Completed);
        assert_eq!(
            log.repositories.len(),
            1,
            "exactly the NFD-named repository is found: {:?}",
            log.repositories
        );

        // The registry is keyed the way the add flow would key an
        // NFC-typed spelling of the same folder: canonical_path_key
        // resolves through APFS (one folder) and folds the key. The
        // WALKED path carries the stored NFD bytes — and classifies
        // `known` against the NFC-derived key: one identity, no
        // duplicate entity.
        let registry_key =
            canonical_path_key(&root.join(nfc)).expect("the NFC spelling must resolve and key");
        let registry = ScanRegistry::new(std::slice::from_ref(&registry_key), &[], &[]);
        let walked = &log.repositories[0];
        let classified = classify_repository(walked, &registry);
        assert_eq!(classified.class(), ScanClass::Known);
    }
}
