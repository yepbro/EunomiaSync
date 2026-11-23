//! The scan-walk bench's pins: the bench calls the walk engine
//! itself (`eunomia_fs::scan::scan_repositories`), so there is no
//! mirrored code to tripwire — the pins guard the bench's own
//! fixture and its arithmetic, against oracles that do not run
//! through the engine or the harness's counting.
//!
//! * The builder's arithmetic equals an independent `std::fs`
//!   directory census (every directory on disk, counted by
//!   recursion, equals the spec's `dir_count` by hand).
//! * The real scan of a small tree runs to completion, finds exactly
//!   the planted repositories, and reports exactly the arithmetic's
//!   event count (every directory — plain, repository, or excluded
//!   `.git` — is one event).
//! * The row's tree meets §12.2's 100 000-folder floor at the
//!   documented shape, and the walk's settings are §6.5's default
//!   literals.

use std::fs;
use std::path::Path;

use eunomia_fs::scan::ScanOutcome;
use eunomia_load::{SCAN_TREE, ScanTree, ScanTreeSpec, default_scan_options, scan_counting};

/// The small tree the empirical pins run on: 3 × 4 × 2 × 2 — by hand:
/// 1 root + 3 groups + 12 repositories + 12 `.git` + 24 leaves +
/// 48 children = 100 directories, 100 events, 12 repositories.
const SMALL: ScanTreeSpec = ScanTreeSpec {
    groups: 3,
    subgroups: 4,
    leaves: 2,
    leaf_children: 2,
};

/// Counts the root and every directory under it by `std::fs`
/// recursion — the independent census (never through the scan
/// engine, never through the harness's arithmetic).
fn count_dirs(root: &Path) -> usize {
    let mut count = 1; // the root itself
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)
            .expect("the fixture tree is listable")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                count += 1;
                stack.push(path);
            }
        }
    }
    count
}

/// The builder lays out exactly the arithmetic it promises: the
/// independent census of the small tree equals the hand-computed
/// directory count.
#[test]
fn the_builders_arithmetic_matches_an_independent_census() {
    let tree = ScanTree::create(&SMALL).expect("the small tree builds");
    assert_eq!(count_dirs(tree.root()), SMALL.dir_count());
    assert_eq!(SMALL.dir_count(), 100, "the small tree's count by hand");
    assert_eq!(
        SMALL.repository_count(),
        12,
        "the small tree's repositories by hand"
    );
}

/// The real engine over the small tree: completion, every planted
/// repository found, and the event arithmetic exact — every
/// directory produces exactly one event (the 12 `.git` folders as
/// excluded skips, everything else as directories or repositories).
#[test]
fn the_scan_finds_every_repository_and_counts_every_folder() {
    let tree = ScanTree::create(&SMALL).expect("the small tree builds");
    let (outcome, checked, found) = scan_counting(tree.root(), &default_scan_options());
    assert_eq!(outcome, ScanOutcome::Completed);
    assert_eq!(
        found,
        SMALL.repository_count(),
        "every planted repository is found"
    );
    assert_eq!(
        checked,
        SMALL.event_count(),
        "every directory is one event (the .git skips included)"
    );
}

/// The row's tree: the §12.2 floor is met at the documented shape,
/// within the default depth, with the default settings the walk
/// runs under — and those settings ARE the spec table's literals:
/// the harness wires them from the settings repository's own
/// defaults (round-1 P2-1's fix), so this pin checks the wiring
/// against the §6.5 table spelled here, independently of both the
/// harness and the store.
#[test]
fn the_row_tree_meets_the_hundred_thousand_folder_floor() {
    let below_root = SCAN_TREE.folder_count() - 1;
    assert_eq!(
        below_root, 101_100,
        "folders below the root, by hand: 100 + 1 000 + 10 000 + 90 000"
    );
    assert!(
        below_root >= 100_000,
        "§12.2's row names 100 000 folders — the tree must meet the floor"
    );
    assert_eq!(SCAN_TREE.repository_count(), 1_000);
    // The tree's depth is 4 (groups, subgroups, leaves, children),
    // within the default max_depth pinned just below — the walk never
    // sees the depth bound bite, exactly like the row's premise.
    let options = default_scan_options();
    assert_eq!(
        options.max_depth.get(),
        6,
        "§6.5's default scan depth, through the store's own default"
    );
    assert_eq!(
        options.exclude_names,
        [
            "node_modules",
            "vendor",
            ".git",
            "dist",
            "build",
            "target",
            ".venv"
        ],
        "§6.5's default exclusion list, verbatim and complete"
    );
}
