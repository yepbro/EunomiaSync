//! §12.2's assignment-toggle row: "Saving an assignment toggle (p95,
//! set L) — no more than 50 ms".
//!
//! The measured iteration is **one real save** of the assignment
//! toggle: the store's `library::reassign` on the fixed pair — the
//! read of the current row, DB-012's active-sibling verification, the
//! `UPDATE project_instructions` variant flip with its `updated_at`
//! restamp, the stored row read back — one autocommit transaction
//! over the fixture's WAL/`synchronous=FULL` connection (the
//! production DB-001 commit semantics, NFR-007's fsync included).
//! That function is §7.10 `assignment_set`'s store half (MAIN-005);
//! the command's `action_log` entry and event emit belong to the IPC
//! layer (not landed) and are cut here, the same seam D-135 cut for
//! the two read-side rows. The alternation
//! ([`AssignmentToggle::save`]) keeps every iteration a real write:
//! A→B, B→A, … — the no-op path is never measured by this bench.
//!
//! The budget's p95 framing is read honestly: criterion's report
//! gives the mean and median with confidence intervals, not p95 —
//! the row is measured here as the whole distribution sitting far
//! below the budget, and the p95 confirmation on the actual 4-core
//! reference machine stays §15.5's open rerun gate (D-135's posture).
//!
//! Two benchmarks share one set-L fixture: the budget row itself
//! (`save_set_L`) and the no-op path alone
//! (`noop_same_variant_set_L`) — the current-row read and the
//! no-write early return (the store checks DB-012 only after that
//! return), the breakdown that attributes the save cost between
//! that read and everything the real save adds on top: DB-012's
//! verify read, the UPDATE, the commit, the read-back.
//!
//! The budget is a criterion report on the machine running
//! `cargo bench` (the reference machine), not a CI gate — CI does
//! not run benchmarks; the numbers land in this task's
//! DECISIONS.md entry.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};

use eunomia_load::LoadFixture;
use eunomia_store::data_sets;

/// Fast iterations: one save is microseconds-to-milliseconds scale,
/// so the default-class sample size (100) with a 1 s warm-up and
/// 20 s of measurement time per benchmark gives criterion its
/// distribution; the two benches together stay under a minute of
/// wall clock. No plots are generated; the console report is the
/// deliverable.
fn assignment_toggle_set_l(c: &mut Criterion) {
    let mut fixture = LoadFixture::create(&data_sets::L, "eunomia-bench-toggle")
        .expect("the §12.2 set-L fixture builds");
    let mut toggle = eunomia_load::AssignmentToggle::new(&mut fixture)
        .expect("the generator's first pair matches the toggle premise");
    let mut group = c.benchmark_group("assignment_toggle");
    group.sample_size(100);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(20));
    group.bench_function("save_set_L", |b| {
        b.iter(|| {
            let stored = toggle
                .save(fixture.connection())
                .expect("the toggle save cannot fail on the bench fixture");
            black_box(stored)
        })
    });
    group.bench_function("noop_same_variant_set_L", |b| {
        b.iter(|| {
            let unchanged = toggle
                .noop_save(fixture.connection())
                .expect("the no-op save cannot fail on the bench fixture");
            black_box(unchanged)
        })
    });
    group.finish();
}

criterion_group!(benches, assignment_toggle_set_l);
criterion_main!(benches);
