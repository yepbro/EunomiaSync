//! Behavior tests of the temporary Git repository fixture
//! (`eunomia_fs::temp_repos`, task T-136): the minimal `.git` layout
//! a real repository needs, canonical paths with identity keys ready
//! for the store-side generator, the ALG-052 scan finding every root,
//! and the self-cleaning teardown.

use std::collections::HashSet;
use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Mutex;

use eunomia_fs::paths::canonicalize;
use eunomia_fs::scan::{CancelToken, ScanEvent, ScanOptions, ScanOutcome, scan_repositories};
use eunomia_fs::temp_repos::TempRepos;

/// Runs the repository scan over `root` (canonical by the caller, the
/// D-098 obligation) and returns the found repository paths.
fn scan_repositories_under(root: &std::path::Path) -> (Vec<PathBuf>, ScanOutcome) {
    let found = Mutex::new(Vec::new());
    let outcome = scan_repositories(
        root,
        &ScanOptions {
            max_depth: NonZeroUsize::new(4).expect("4 is non-zero"),
            exclude_names: Vec::new(),
        },
        &CancelToken::new(),
        |event| {
            if let ScanEvent::Repository { path } = event {
                found
                    .lock()
                    .expect("the walker threads must not poison the log")
                    .push(path.clone());
            }
        },
    );
    let found = found
        .into_inner()
        .expect("the walk ended with the log intact");
    (found, outcome)
}

/// Every root carries git's own minimal repository layout, and the
/// paths/keys are the canonical, unique, ready-to-store pairs.
#[test]
fn roots_carry_the_minimal_git_layout() {
    let repos = TempRepos::create(5, "layout").expect("the fixture should create");
    assert_eq!(repos.roots().len(), 5);
    for root in repos.roots() {
        let git = root.path.join(".git");
        assert!(
            git.is_dir(),
            "{} must hold a .git directory",
            root.path.display()
        );
        assert_eq!(
            fs::read_to_string(git.join("HEAD")).expect("HEAD should read"),
            "ref: refs/heads/main\n"
        );
        assert!(git.join("config").is_file(), "the config file");
        assert!(git.join("objects").is_dir(), "the objects directory");
        assert!(
            git.join("refs").join("heads").is_dir(),
            "the refs directory"
        );
        // Canonical path: resolves to itself (no symlinked spelling
        // left in), and the key is the DB-030 key of that spelling.
        assert_eq!(
            canonicalize(&root.path).expect("the root canonicalizes"),
            root.path
        );
        assert!(!root.path_key.is_empty());
    }
    let keys: HashSet<&str> = repos
        .roots()
        .iter()
        .map(|root| root.path_key.as_str())
        .collect();
    assert_eq!(keys.len(), 5, "every root keys differently");
    // The group directory is the canonical parent holding them all.
    assert_eq!(
        repos.roots()[0].path.parent(),
        Some(repos.group_dir()),
        "the roots sit directly in the group directory"
    );
}

/// The ALG-052 scan finds every root and nothing else: the fixture's
/// folders look like repositories to the very walk that will feed
/// PRJ-002 in the benchmarks.
#[test]
fn the_repository_scan_finds_every_root() {
    let repos = TempRepos::create(3, "scan-check").expect("the fixture should create");
    let scan_root = canonicalize(repos.group_dir()).expect("the group directory canonicalizes");
    let (found, outcome) = scan_repositories_under(&scan_root);
    assert_eq!(outcome, ScanOutcome::Completed);
    let expected: HashSet<PathBuf> = repos.roots().iter().map(|root| root.path.clone()).collect();
    let found: HashSet<PathBuf> = found.into_iter().collect();
    assert_eq!(found, expected);
}

/// The fixture cleans up after itself: dropping the guard removes the
/// whole base directory, roots and `.git` layouts included.
#[test]
fn dropping_the_guard_removes_everything() {
    let (group_dir, root_path) = {
        let repos = TempRepos::create(2, "cleanup").expect("the fixture should create");
        (
            repos.group_dir().to_path_buf(),
            repos.roots()[0].path.clone(),
        )
    };
    assert!(!root_path.exists(), "the root must be gone after the drop");
    assert!(!group_dir.exists(), "the group directory must be gone too");
}

/// `count = 0` is a valid empty fixture — the group directory exists,
/// no roots do.
#[test]
fn zero_roots_is_an_empty_fixture() {
    let repos = TempRepos::create(0, "empty").expect("the fixture should create");
    assert!(repos.roots().is_empty());
    assert!(repos.group_dir().is_dir());
}
