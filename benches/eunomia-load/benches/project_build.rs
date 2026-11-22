//! §12.2's single-project row: "Building one project with up to 1000
//! items — no more than 20 ms".
//!
//! The measured iteration is the whole production slice of one
//! project's build (the T-101/T-112 mechanics, the shared core pass
//! of `eunomia_core::content_build`): one read transaction for the
//! ALG-001 snapshot (SYS-005's shape), then the §9.2 body over the
//! scoped chunk, the ALG-030 managed text, the §9.4 target path and
//! the DB-020 hash. On the [`ONE_PROJECT_1000_ITEMS`] fixture that is
//! exactly "build one project with 1000 items from the live
//! database".
//!
//! The budget is read as §15.1's Load row pins it: a criterion report
//! on the machine running `cargo bench` (the reference machine), not a
//! CI gate — CI does not run benchmarks (verified against
//! `.github/workflows/ci.yml`); the numbers land in this task's
//! DECISIONS.md entry.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};

use eunomia_core::content_build::build_projects;
use eunomia_load::{LoadFixture, ONE_PROJECT_1000_ITEMS, read_snapshot};

/// Criterion applies the measurement time per benchmark: 50 samples
/// over 20 s each with a 1 s warm-up — roughly 30 s of wall clock for
/// this one bench (the card's "keep the run fast"; no plots are
/// generated, the console report is the deliverable).
fn build_one_project_1000_items(c: &mut Criterion) {
    let mut fixture = LoadFixture::create(&ONE_PROJECT_1000_ITEMS, "eunomia-bench-build")
        .expect("the §12.2 single-project fixture builds");
    let mut group = c.benchmark_group("project_build");
    group.sample_size(50);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(20));
    group.bench_function("one_project_1000_items", |b| {
        b.iter(|| {
            let snapshot = read_snapshot(fixture.connection())
                .expect("the snapshot read cannot fail on the bench fixture");
            let built = build_projects(&snapshot);
            assert_eq!(built.len(), 1, "the fixture holds one project");
            black_box(built)
        })
    });
    group.finish();
}

criterion_group!(benches, build_one_project_1000_items);
criterion_main!(benches);
