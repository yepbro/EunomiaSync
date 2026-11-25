//! The sync-pipeline bench's pins: the pass composes the REAL per-step
//! seams (the fs probes, the ALG-033 comparison, the backup copy, the
//! §9.5 atomic write, the ALG-031 splice), so there are no mirrored
//! mechanics to tripwire — the pins guard the bench's own premises,
//! against oracles that do not run through the pass's plumbing.
//!
//! * The premise: every project's `k = 0` pair exists and holds the
//!   variant D-109's closed form promises — recomputed here from the
//!   generated ids, verified through the store's own read.
//! * The size premise: the row profile's built files carry the "files
//!   up to 10 KB" shape (the generator's §12.2 sizing).
//! * The alternation: a flip changes EVERY project's build, a flip
//!   back restores it — so no measured pass can land on the
//!   unchanged path.
//! * D-24's acceptance through the composed pass: a second pass
//!   without a flip writes nothing and copies nothing.
//! * A flipped pass backs up and rewrites every project, and the
//!   written bytes match the REAL builders — `build_content`'s own
//!   output for A/C (which transitively pins the harness's
//!   three-line whole-file tail), the seed literal plus the built
//!   managed text for B.
//! * SYS-009: a missing scheme-B target is skipped, not created.

use std::fs;

use eunomia_core::body::build_body;
use eunomia_core::extra_files::extra_file_lines;
use eunomia_core::markers;
use eunomia_core::project::Scheme;
use eunomia_core::target_file::build_content;
use eunomia_load::{LoadError, PassStats, SYNC_100_PROJECTS_10KB, SyncFixture, read_snapshot};
use eunomia_store::data_sets::DataSetSpec;
use eunomia_store::library::{AssignmentTarget, get_assignment};

/// The scheme-B seed the bench's fixture plants, spelled here
/// independently (the oracle the B pins compare the preserved
/// outside against — never read back through the harness).
const SCHEME_B_SEED: &str = "Project notes live outside the managed block.\n\
                             <!-- EUNOMIASYNC:BEGIN -->\n\
                             <!-- EUNOMIASYNC:END -->\n";

/// A small mixed-scheme profile: six projects cover the generator's
/// scheme cycle (`ordinal % 3` — two of each scheme, pinned below),
/// dense enough that every project's `k = 0` assignment exists. The
/// closed form is scale-independent (D-109 at the origin), so the
/// pins run on this while the bench runs the 100-project profile.
fn sync_spec() -> DataSetSpec {
    DataSetSpec {
        projects: 6,
        headings: 2,
        instructions: 24,
        variants_per_instruction: 3,
        assignments_per_project: 8,
    }
}

/// The seed step every premise-holding pin starts from: a fixture
/// with the scheme-B targets planted and primed by one pass, so
/// every target file exists.
fn primed_fixture(label: &str) -> Result<SyncFixture, LoadError> {
    let mut fixture = SyncFixture::create(&sync_spec(), label)?;
    assert_eq!(
        fixture
            .metas()
            .iter()
            .filter(|meta| meta.scheme == Scheme::A)
            .count(),
        2,
        "the six-project profile holds two scheme-A projects"
    );
    assert_eq!(
        fixture
            .metas()
            .iter()
            .filter(|meta| meta.scheme == Scheme::B)
            .count(),
        2,
        "the six-project profile holds two scheme-B projects"
    );
    assert_eq!(
        fixture
            .metas()
            .iter()
            .filter(|meta| meta.scheme == Scheme::C)
            .count(),
        2,
        "the six-project profile holds two scheme-C projects"
    );
    fixture.create_scheme_b_targets()?;
    let built = fixture.build_content()?;
    let stats = fixture.run_pass(&built)?;
    assert_eq!(stats.written, 6, "the priming pass writes every project");
    Ok(fixture)
}

/// The premise, recomputed by hand: for every project `p`, the
/// `k = 0` assignment points at instruction ordinal `(37·p) mod I`
/// and holds variant ordinal `p mod V` — through the store's own
/// read, never through the toggle code.
#[test]
fn every_k0_pair_holds_the_closed_forms_variant() -> Result<(), LoadError> {
    let spec = sync_spec();
    let mut fixture = SyncFixture::create(&spec, "eunomia-load-sync-premise")?;
    // The metas' identity half, collected before the store reads (the
    // connection borrow is mutable, the metas borrow immutable).
    let project_ids: Vec<String> = fixture
        .metas()
        .iter()
        .map(|meta| meta.project_id.as_str().to_owned())
        .collect();
    for (ordinal, project_id) in project_ids.iter().enumerate() {
        let instruction_ordinal = (37 * ordinal) % spec.instructions;
        let instruction = fixture.generated().instruction_ids[instruction_ordinal].clone();
        let expected = fixture.generated().variant_ids[instruction_ordinal]
            [ordinal % spec.variants_per_instruction]
            .clone();
        let row = get_assignment(fixture.connection(), project_id, &instruction)
            .map_err(LoadError::Library)?
            .unwrap_or_else(|| {
                panic!("project {ordinal}: the k = 0 assignment exists");
            });
        assert_eq!(
            row.target,
            AssignmentTarget::Variant(expected),
            "project {ordinal}: D-109's closed form at k = 0"
        );
    }
    Ok(())
}

/// The row's size premise on the real profile: 100 projects, every
/// one building ≈10 KB of managed text (the generator's §12.2
/// variant-text sizing — "a fully assigned set-L project builds
/// ≈10 KB", and 200 of the 1000 instructions is that density). The
/// band is deliberately ±20 %: the pin guards the ORDER of the size
/// (not 1 KB, not 100 KB), the exact number is recorded with the
/// task's DECISIONS.md entry.
#[test]
fn the_row_profile_builds_ten_kb_files() -> Result<(), LoadError> {
    let mut fixture = SyncFixture::create(&SYNC_100_PROJECTS_10KB, "eunomia-load-sync-size")?;
    let built = fixture.build_content()?;
    assert_eq!(built.len(), 100, "the row names 100 projects");
    let mut sizes: Vec<usize> = built.iter().map(|project| project.managed.len()).collect();
    sizes.sort_unstable();
    for size in &sizes {
        assert!(
            (8_000..=12_000).contains(size),
            "the row's 'files up to 10 KB' premise: built {size} bytes"
        );
    }
    Ok(())
}

/// The alternation: one flip changes EVERY project's build (every
/// measured pass is a full write pass), and a flip back restores the
/// original builds — the A↔B content pair the bench cycles.
#[test]
fn the_alternation_flips_and_restores_every_build() -> Result<(), LoadError> {
    let mut fixture = SyncFixture::create(&sync_spec(), "eunomia-load-sync-alternates")?;
    let base = fixture.build_content()?;
    fixture.flip_assignments()?;
    let flipped = fixture.build_content()?;
    for (ordinal, (before, after)) in base.iter().zip(&flipped).enumerate() {
        assert_ne!(
            before.build_hash, after.build_hash,
            "project {ordinal}: the flip changes the build"
        );
    }
    fixture.flip_assignments()?;
    let restored = fixture.build_content()?;
    for (before, after) in base.iter().zip(&restored) {
        assert_eq!(before.build_hash, after.build_hash);
    }
    Ok(())
}

/// D-24's acceptance through the composed pass: with no flip, the
/// second pass changes no files and creates no copies — and that is
/// the no-op path the bench's alternation exists to keep OUT of the
/// measurement.
#[test]
fn a_second_pass_without_a_flip_writes_nothing() -> Result<(), LoadError> {
    let mut fixture = primed_fixture("eunomia-load-sync-unchanged")?;
    let copies_before = count_backup_copies(fixture.data_dir());
    let built = fixture.build_content()?;
    let repeat = fixture.run_pass(&built)?;
    assert_eq!(
        repeat,
        PassStats {
            ready: 0,
            unchanged: 6,
            skipped: 0,
            backed_up: 0,
            written: 0
        },
        "a no-change pass is all-unchanged (D-24)"
    );
    assert_eq!(
        count_backup_copies(fixture.data_dir()),
        copies_before,
        "an unchanged pass copies nothing"
    );
    Ok(())
}

/// The measured iteration's shape: after a flip, every project is
/// ready, every existing file is backed up, every file is rewritten
/// — and the written bytes are the REAL builders' output:
/// `build_content`'s own whole file for A/C (which pins the
/// harness's three-line whole-file tail against the builder it
/// mirrors), and the seed's preserved outside around the built
/// managed text for B.
#[test]
fn a_flipped_pass_backs_up_and_rewrites_with_the_real_builders_bytes() -> Result<(), LoadError> {
    let mut fixture = primed_fixture("eunomia-load-sync-flipped")?;
    // The bench's between-iterations sweep: the priming pass's copies
    // go, so the copies counted below are this pass's alone.
    fixture.sweep_backups()?;
    fixture.flip_assignments()?;
    let built = fixture.build_content()?;
    let stats = fixture.run_pass(&built)?;
    assert_eq!(
        stats,
        PassStats {
            ready: 6,
            unchanged: 0,
            skipped: 0,
            backed_up: 6,
            written: 6
        },
        "a flipped pass is the row's worst case: everyone ready, copied, written"
    );

    // The independent oracle for A/C: the whole file rebuilt through
    // the real `build_content` over the independently built body and
    // the settings' own extra lines (the D-130 pin pattern — never
    // through the harness's whole-file tail).
    let snapshot = read_snapshot(fixture.connection())?;
    let marker = fixture.marker_line().to_owned();
    for (project, built_one) in snapshot.projects().iter().zip(&built) {
        let meta = fixture
            .metas()
            .iter()
            .find(|meta| meta.project_id == project.id)
            .expect("every snapshot project has its meta");
        let file = fs::read(meta.root.join(&meta.file_name)).map_err(LoadError::Io)?;
        match meta.scheme {
            Scheme::B => {
                // B: the seed's outside survives byte for byte, the
                // built managed text and the marker sit between the
                // markers (spelled from the seed literal, never
                // through the harness's splice inputs).
                let text = String::from_utf8(file).expect("the written B file is UTF-8");
                let span = markers::parse(&text).expect("the written B file keeps one pair");
                let seed_span = markers::parse(SCHEME_B_SEED).expect("the seed holds one pair");
                assert_eq!(
                    &text[..span.managed_start],
                    &SCHEME_B_SEED[..seed_span.managed_start],
                    "the outside before the markers is preserved byte for byte"
                );
                assert_eq!(
                    &text[span.managed_end..],
                    &SCHEME_B_SEED[seed_span.managed_end..],
                    "the outside after the markers is preserved byte for byte"
                );
                let managed_block = &text[span.managed_start..span.managed_end];
                assert_eq!(
                    managed_block,
                    format!("{}\n\n{}\n", built_one.managed, marker),
                    "the spliced block is exactly the built managed text, one blank \
                     line, and the marker"
                );
            }
            scheme => {
                let body =
                    build_body(&snapshot, &project.id).expect("the snapshot holds the project");
                let lines = extra_file_lines(
                    project.settings.base_name,
                    project.settings.extra_project,
                    project.settings.extra_local,
                );
                let expected = build_content(scheme, &body, &lines, &marker);
                assert_eq!(
                    file,
                    expected.as_bytes(),
                    "the written file is the real builder's output"
                );
            }
        }
    }
    assert_eq!(count_backup_copies(fixture.data_dir()), 6);
    Ok(())
}

/// SYS-009: a scheme-B project whose target is missing is SKIPPED,
/// not created — the pass refuses the no-markers shape the way
/// production preflight does, so the bench's B handling cannot drift
/// into a creation the spec forbids.
#[test]
fn a_missing_scheme_b_target_is_skipped_not_created() -> Result<(), LoadError> {
    let mut fixture = SyncFixture::create(&sync_spec(), "eunomia-load-sync-b-missing")?;
    // No create_scheme_b_targets: the B targets stay absent.
    let built = fixture.build_content()?;
    let stats = fixture.run_pass(&built)?;
    assert_eq!(
        stats.skipped, 2,
        "the two scheme-B projects skip: {stats:?}"
    );
    assert_eq!(
        stats.written, 4,
        "the four A/C projects are created: {stats:?}"
    );
    for meta in fixture.metas() {
        let target = meta.root.join(&meta.file_name);
        match meta.scheme {
            Scheme::B => assert!(
                !target.exists(),
                "a missing scheme-B target must not be created (SYS-009)"
            ),
            _ => assert!(target.exists(), "the A/C targets are created"),
        }
    }
    Ok(())
}

/// Counts the backup copies under the fixture's data directory — an
/// independent `std::fs` walk, never through the backup module.
fn count_backup_copies(data_dir: &std::path::Path) -> usize {
    let backups = data_dir.join("backups");
    let mut count = 0;
    let mut stack = vec![backups];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                count += 1;
            }
        }
    }
    count
}
