//! §12.2's scan row: "Scan of 100,000 folders on an SSD — no more
//! than 30 s; UI input response no more than 100 ms".
//!
//! The measured iteration is the scan ENGINE itself:
//! `eunomia_fs::scan::scan_repositories` over the generated
//! 101 100-folder tree (plus the root and the planted `.git`
//! folders), under the §6.5 default settings (`max_depth` 6, the
//! default exclude list with `.git` on it) — the ALG-050..054 walk
//! with its cancel checks, its `.git` probe per folder, its
//! exclude-name skips, `min(4, cores)` walker threads. Nothing is
//! mirrored and nothing is cut from the walk: the command layer's
//! surroundings the row's walk half does not include are the
//! progress throttle (its per-event mutex and clock check), the
//! cancel slot, the SEC-003 root gate, and the ALG-053
//! classification of found repositories
//! (`scan_classify::classify_entry`, T-075's module) —
//! `src-tauri/src/scan.rs`'s layer, D-153, with the complete list
//! in the harness crate's docs. The row's OTHER half — "UI input
//! response no more than 100 ms" — is a UI-thread claim measured by
//! the e2e scripts, not by a criterion bench (the boundary is
//! recorded with the task's DECISIONS.md entry).
//!
//! The walk is read-only, so every iteration does identical full
//! work over an unchanged tree: there is no no-op path (the honest
//! difference from the sync pass, which needs its alternation), and
//! the callback only counts events — the `checked`/`found`
//! arithmetic of the command layer's tracker (every event is one
//! folder the walk made a decision about), without the tracker's
//! per-event locking or the classification the counting omits.
//!
//! The budget is a criterion report on the machine running
//! `cargo bench` (the reference machine), not a CI gate — CI does
//! not run benchmarks; the numbers land in this task's
//! DECISIONS.md entry. The programmatic settings (sample size 10,
//! 1 s warm-up, 20 s of measurement time) override criterion's CLI
//! flags, so a plain `cargo bench -p eunomia-load -- scan_walk` is
//! the full measurement (the D-157 bounded-check-equals-full-
//! measurement posture).

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};

use eunomia_load::{SCAN_TREE, ScanTree, default_scan_options, scan_counting};

/// One iteration is one complete walk (hundreds of milliseconds):
/// 10 samples (criterion's minimum) with a 1 s warm-up and 20 s of
/// measurement time. The fixture setup (102 101 directory creations)
/// and the verification scan run before the group, unmeasured. No
/// plots are generated; the console report is the deliverable.
fn scan_walk_100k_folders(c: &mut Criterion) {
    let tree = ScanTree::create(&SCAN_TREE).expect("the §12.2 scan tree builds");
    let options = default_scan_options();
    // The setup's own verification (unmeasured): the walk sees every
    // folder and finds every planted repository — the same census
    // `tests/scan_walk_pins.rs` pins on the small tree, at the row's
    // scale.
    let (outcome, checked, found) = scan_counting(tree.root(), &options);
    assert_eq!(outcome, eunomia_fs::scan::ScanOutcome::Completed);
    assert_eq!(
        checked,
        SCAN_TREE.event_count(),
        "every folder is one event"
    );
    assert_eq!(
        found,
        SCAN_TREE.repository_count(),
        "every planted repository is found"
    );

    let mut group = c.benchmark_group("scan_walk");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(20));
    group.bench_function("walk_100k_folders", |b| {
        b.iter(|| {
            let counts = scan_counting(tree.root(), &options);
            black_box(counts)
        })
    });
    group.finish();
}

criterion_group!(benches, scan_walk_100k_folders);
criterion_main!(benches);
