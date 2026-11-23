//! §12.2's sync row: "Sync of 100 projects, files up to 10 KB, SSD —
//! no more than 10 s, not counting antivirus locks".
//!
//! One measured iteration of `full_pass_100_projects` is the
//! pipeline's measurable core in §9.4's phase order over the
//! 100-project fixture: the ALG-001 snapshot read (one transaction,
//! SYS-005) and `build_projects` (phase 0's pure half), then per
//! project the seven preflight steps (availability, target-path
//! resolution, the regular-file gate, write access, the existing
//! file's read, the ALG-033 unchanged check, the stale-tmp sweep),
//! the free-space tail, phase 2's verified backup copies, and phase
//! 3's §9.5 atomic writes (temp file, fsync, rename, directory
//! fsync, re-read, digest verify). Every step is the production seam
//! function the src-tauri phases call; the run bookkeeping cut
//! around them (run rows, per-project store commits, backup
//! records, phase 4) is enumerated in the harness crate's docs —
//! `cargo bench` cannot depend on the app crate (D-145).
//!
//! The iteration has a no-op path, and the setup keeps it out of
//! the measurement: after a pass every file matches its build, so
//! an unflipped second pass would be all-`unchanged` (D-24) and
//! measure the cheap branch from iteration two on. The unmeasured
//! per-iteration setup (`iter_batched` — criterion excludes it from
//! timing) flips one assignment per project (the D-157 alternation,
//! scaled to every project) and sweeps the previous pass's backup
//! copies (phase 4's retention stand-in, so copies do not
//! accumulate): every measured pass is the row's worst case — all
//! 100 projects ready, backed up, and rewritten.
//!
//! `fs_phases_100_projects` is the diagnostic: the same pass over
//! content the setup built, isolating the fs half (preflight +
//! backups + writes). The row's budget attributes between the two:
//! the full pass minus this is the snapshot read plus the build.
//!
//! The budget is a criterion report on the machine running
//! `cargo bench` (the reference machine), not a CI gate — CI does
//! not run benchmarks; the numbers land in this task's
//! DECISIONS.md entry. The programmatic settings (sample size 10,
//! 1 s warm-up, 20 s of measurement time per benchmark) override
//! criterion's CLI flags, so a plain `cargo bench -p eunomia-load
//! -- sync_pipeline` is the full measurement (the D-157 bounded-
//! check-equals-full-measurement posture).

use std::cell::RefCell;
use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

use eunomia_load::{SYNC_100_PROJECTS_10KB, SyncFixture};

/// Heavy iterations (hundreds of milliseconds): 10 samples
/// (criterion's minimum) with a 1 s warm-up; 20 s of measurement
/// time per benchmark, so the group's two benches take under a
/// minute of wall clock together plus the fixture setup. No plots
/// are generated; the console report is the deliverable.
fn sync_pipeline_100_projects(c: &mut Criterion) {
    let mut fixture = SyncFixture::create(&SYNC_100_PROJECTS_10KB, "eunomia-bench-sync")
        .expect("the §12.2 sync fixture builds");
    fixture
        .create_scheme_b_targets()
        .expect("the scheme-B seeds land");
    // The priming pass (unmeasured): A/C targets are created, the
    // marked B targets are spliced into — from here on every target
    // file exists, which every measured iteration's premise needs.
    let built = fixture.build_content().expect("the priming build succeeds");
    let primed = fixture.run_pass(&built).expect("the priming pass succeeds");
    assert_eq!(
        primed.written, 100,
        "the priming pass writes every project's target: {primed:?}"
    );
    let fixture = RefCell::new(fixture);

    let mut group = c.benchmark_group("sync_pipeline");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(20));

    group.bench_function("full_pass_100_projects", |b| {
        b.iter_batched(
            || {
                // The unmeasured setup: flip every project's toggle
                // (every measured pass becomes a full write pass) and
                // sweep the previous pass's copies (the retention
                // stand-in).
                let mut fixture = fixture.borrow_mut();
                fixture.flip_assignments().expect("the alternation flips");
                fixture.sweep_backups().expect("the backups sweep");
            },
            |_| {
                let mut fixture = fixture.borrow_mut();
                let built = fixture
                    .build_content()
                    .expect("the snapshot read and build succeed");
                let stats = fixture.run_pass(&built).expect("the pass succeeds");
                black_box(stats)
            },
            BatchSize::PerIteration,
        )
    });

    group.bench_function("fs_phases_100_projects", |b| {
        b.iter_batched(
            || {
                let mut fixture = fixture.borrow_mut();
                fixture.flip_assignments().expect("the alternation flips");
                fixture.sweep_backups().expect("the backups sweep");
                // The unmeasured content build this diagnostic cuts:
                // the routine measures the fs phases alone.
                fixture.build_content().expect("the content build succeeds")
            },
            |built| {
                let mut fixture = fixture.borrow_mut();
                let stats = fixture.run_pass(&built).expect("the pass succeeds");
                black_box(stats)
            },
            BatchSize::PerIteration,
        )
    });
    group.finish();
}

criterion_group!(benches, sync_pipeline_100_projects);
criterion_main!(benches);
