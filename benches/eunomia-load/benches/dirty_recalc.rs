//! §12.2's dirty row: "Dirty recalculation for all projects (set L) —
//! no more than 500 ms, in the background".
//!
//! The measured iteration is the full pass of the T-101 service minus
//! its UI plumbing (the card's seam): **one read transaction** for the
//! ALG-001 snapshot and the stored hashes together (SYS-005/D-105),
//! then every project's build slice — §9.2 body over the scoped
//! mini-snapshot, ALG-030 managed text, DB-020 hash — and DB-021's
//! `is_dirty` comparison, the chunks of 25 spread over parallel
//! threads and collected in chunk order (the `spawn_blocking` shape,
//! mirrored through `std::thread::scope` over the shared core pass's
//! own per-chunk call). No event is emitted, no cache is diffed:
//! those are the pass's surroundings, not its cost.
//!
//! The stored hashes stay `NULL` in the fixture — the honest startup
//! pass, where every project with assignments reads dirty (D-121's L
//! probe measured the same state; its precedent numbers: the whole
//! release pass ~430–545 ms on its 8-core box, ~310 ms of it the read
//! transaction).
//!
//! Two benchmarks share one fixture: the budget row itself
//! (`full_pass_set_L`) and the read-transaction half alone
//! (`snapshot_read_set_L`) — the breakdown D-121's entry used to
//! attribute the cost between the store's read and the compute.
//!
//! The budget is a criterion report on the machine running
//! `cargo bench` (the reference machine), not a CI gate — CI does not
//! run benchmarks; the numbers land in this task's DECISIONS.md entry.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};

use eunomia_load::{LoadFixture, compute_verdicts_parallel, read_snapshot_and_hashes};
use eunomia_store::data_sets;

/// Heavy iterations: the pass is hundreds of milliseconds, so 10
/// samples (criterion's minimum) with a 1 s warm-up. The 35 s
/// measurement time is applied per benchmark, so the group's two
/// benches take ~1.5 minutes of wall clock together — still
/// minutes-scale for the whole `cargo bench` run. No plots are
/// generated; the console report is the deliverable.
fn dirty_recalc_set_l(c: &mut Criterion) {
    let mut fixture = LoadFixture::create(&data_sets::L, "eunomia-bench-dirty")
        .expect("the §12.2 set-L fixture builds");
    let mut group = c.benchmark_group("dirty_recalc");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(35));
    group.bench_function("full_pass_set_L", |b| {
        b.iter(|| {
            let (snapshot, stored_hashes) = read_snapshot_and_hashes(fixture.connection())
                .expect("the pass read cannot fail on the bench fixture");
            let verdicts = compute_verdicts_parallel(&snapshot, &stored_hashes);
            black_box(verdicts)
        })
    });
    group.bench_function("snapshot_read_set_L", |b| {
        b.iter(|| {
            black_box(
                read_snapshot_and_hashes(fixture.connection())
                    .expect("the pass read cannot fail on the bench fixture"),
            )
        })
    });
    group.finish();
}

criterion_group!(benches, dirty_recalc_set_l);
criterion_main!(benches);
