//! The harness pins: the benchmark substrate cannot drift, because
//! every measured half is checked against an oracle that does not run
//! through this crate's plumbing.
//!
//! * The chunked mini-snapshot scoping is pinned against an
//!   independent full-snapshot build on a boundary-crossing fixture:
//!   every project's chunked bytes must equal the bytes
//!   `build_body` produces over the whole snapshot (D-121's
//!   body-equivalence, executable). The chunked pass itself now lives
//!   in `eunomia_core::content_build` (T-265/D-145) — this pin runs
//!   the shared pass and checks it from outside, exactly as before.
//! * The parallel verdict pass (this crate's `std::thread::scope`
//!   substrate) is pinned against core's sequential one id-for-id,
//!   and both against DB-021's semantics (never-synced dirty,
//!   synced-hash clean) on the same fixture.
//! * The tripwire pins the shared chunk size by value: the §12.2
//!   budgets were measured at 25, and a core change goes red here
//!   until the budgets are re-measured.
//!
//! The managed-text mirror this suite once pinned against
//! `build_content`'s output retired with the mirror (T-265/D-145):
//! `managed_text` is now `eunomia_core::target_file`'s own public
//! seam, `build_content` delegates its join to it, and an
//! equality pin between the two would mirror the implementation
//! (rule 4) — the seam is pinned where it lives, against the D-21
//! and ALG-030 spec literals.

use eunomia_core::body::build_body;
use eunomia_core::content_build::{
    BuiltProject, MINI_SNAPSHOT_PROJECTS, StoredHashes, build_projects, compute_dirty_verdicts,
};
use eunomia_core::hash::build_hash;
use eunomia_core::target_file::managed_text;
use eunomia_store::data_sets::DataSetSpec;

use eunomia_load::{
    LoadError, LoadFixture, compute_verdicts_parallel, read_snapshot, read_snapshot_and_hashes,
};

/// The D-21 reading-line literals, spelled from the spec — the
/// oracle for the managed-text seam the shared pass splices (the
/// sentences' exact bytes, independent of any builder).
const PROJECT_LINE: &str = "If AGENTS.project.md exists in the repository root, read it before starting work and follow its instructions.";
const LOCAL_LINE: &str = "If AGENTS.local.md exists in the repository root, read it before starting work and follow its instructions.";

/// The reading lines enter the built managed text of an extras-on
/// project: the generator's write-settings cycle (ordinals `i % 6 ==
/// 0`: both options enabled, and `i % 3 == 0` makes those scheme A)
/// produces projects whose built managed text is ALG-030's full
/// splice — the `.local.md` sentence at start (the local option's
/// generator-fixed position), the body, the `.project.md` sentence at
/// end, exactly one empty line between neighbours. Spelled here from
/// the D-21 literals, never from a builder: a harness or pass that
/// dropped the reading lines (or joined them wrong) would measure a
/// cheaper, wrong pass.
#[test]
fn extras_on_projects_carry_the_d21_reading_lines() {
    let mut fixture = LoadFixture::create(&crossing_spec(), "eunomia-load-d21")
        .expect("the crossing fixture builds");
    let snapshot = read_snapshot(fixture.connection()).expect("the snapshot read succeeds");
    let built = build_projects(&snapshot);
    let with_lines: Vec<&BuiltProject> = built
        .iter()
        .filter(|project| {
            project.managed.contains(PROJECT_LINE) && project.managed.contains(LOCAL_LINE)
        })
        .collect();
    assert!(
        !with_lines.is_empty(),
        "the generator's settings cycle includes extras-on projects — \
         at least one built project must carry both D-21 sentences"
    );
    for project in &with_lines {
        assert!(
            project.managed.starts_with(LOCAL_LINE),
            "the local line's position is Start: it must lead the block — {:?}",
            &project.managed[..LOCAL_LINE.len().min(project.managed.len())]
        );
        assert!(
            project.managed.ends_with(PROJECT_LINE),
            "the project line's position is End: it must close the block"
        );
        let middle = &project.managed[LOCAL_LINE.len()..project.managed.len() - PROJECT_LINE.len()];
        assert!(
            middle.starts_with("\n\n")
                && !middle.starts_with("\n\n\n")
                && middle.ends_with("\n\n")
                && !middle.ends_with("\n\n\n"),
            "exactly one empty line between each neighbouring part — \
             never two, never none: {middle:?}"
        );
        assert!(
            !middle[2..middle.len() - 2].is_empty(),
            "the body part is non-empty (the crossing fixture assigns \
             sixty instructions to every project)"
        );
    }
}

/// A boundary-crossing profile: 57 projects make chunks of 25 + 25 + 7
/// (a boundary crossed, a partial last chunk — D-121's clamped-end
/// catch), with headings, two variants and enough assignments to make
/// every project's body non-trivial.
fn crossing_spec() -> DataSetSpec {
    DataSetSpec {
        projects: 57,
        headings: 5,
        instructions: 120,
        variants_per_instruction: 2,
        assignments_per_project: 60,
    }
}

/// The chunked build must be body-equivalent to the independent
/// full-snapshot spelling (D-121's construction, made falsifiable):
/// for every project, `build_body` over the **whole** snapshot —
/// through core's public API, never through the chunk plumbing —
/// produces the same managed text and hash the chunked pass
/// produced. A scoping leak (another chunk's assignments in, or this
/// chunk's own dropped) breaks the equality.
#[test]
fn chunked_builds_equal_the_full_snapshot_builds() {
    let mut fixture = LoadFixture::create(&crossing_spec(), "eunomia-load-pins")
        .expect("the crossing fixture builds");
    let snapshot = read_snapshot(fixture.connection()).expect("the snapshot read succeeds");
    let chunked = build_projects(&snapshot);

    // The oracle: each project rebuilt over the full snapshot, the
    // same core seams the pass composes, spelled independently here.
    let expected: Vec<BuiltProject> = snapshot
        .projects()
        .iter()
        .map(|project| {
            let body = build_body(&snapshot, &project.id)
                .expect("the full snapshot holds every project it lists");
            let settings = &project.settings;
            let lines = eunomia_core::extra_files::extra_file_lines(
                settings.base_name,
                settings.extra_project,
                settings.extra_local,
            );
            let managed = managed_text(settings.scheme, &body, &lines);
            let target_path = project
                .root
                .join(eunomia_core::backup_name::target_file_name(
                    settings.scheme,
                    settings.base_name,
                ));
            let hash = build_hash(
                settings.scheme,
                &target_path,
                settings.extra_local,
                settings.extra_project,
                &managed,
            );
            BuiltProject {
                project_id: project.id.as_str().to_owned(),
                managed,
                target_path,
                build_hash: hash,
            }
        })
        .collect();

    assert_eq!(
        chunked, expected,
        "every project's chunked bytes, paths and hashes"
    );
    assert_eq!(chunked.len(), 57, "all three chunks' projects are built");
}

/// The dirty pass's read half is pinned against the database itself:
/// the stored-hash map's keys are exactly the snapshot's (active)
/// project ids, every generated value is `None` (never synced —
/// DB-024's NULL), and archiving one project through the projects
/// repository drops it from **both** halves of the same transaction's
/// read (the `ListScope::Active` scope and the snapshot's active
/// filter, ALG-001/DB-014). A wrong column, scope or keying here
/// would leave every verdict test green while the benchmark measured
/// a pass production never runs (round-1 review's P3-2).
#[test]
fn read_snapshot_and_hashes_pins_the_stored_hash_map() -> Result<(), LoadError> {
    let mut fixture = LoadFixture::create(&crossing_spec(), "eunomia-load-read-pin")?;
    let (snapshot, stored) = read_snapshot_and_hashes(fixture.connection())?;
    let ids: Vec<&str> = snapshot
        .projects()
        .iter()
        .map(|project| project.id.as_str())
        .collect();
    let mut stored_ids: Vec<&str> = stored.keys().map(String::as_str).collect();
    stored_ids.sort_unstable();
    let mut sorted_ids = ids.clone();
    sorted_ids.sort_unstable();
    assert_eq!(stored_ids, sorted_ids, "the map keys are the project ids");
    assert!(
        stored.values().all(Option::is_none),
        "the generated rows are never synced: every stored hash is NULL"
    );

    // The scope half: an archived project leaves both collections.
    let archived_id = fixture.generated().project_ids[3].clone();
    eunomia_store::projects::archive(fixture.connection(), &archived_id)
        .map_err(LoadError::Projects)?;
    let (after, stored_after) = read_snapshot_and_hashes(fixture.connection())?;
    assert!(
        after
            .projects()
            .iter()
            .all(|project| project.id.as_str() != archived_id),
        "the snapshot drops the archived project (ALG-001/DB-014)"
    );
    assert!(
        !stored_after.contains_key(&archived_id),
        "the stored-hash read drops the archived project (ListScope::Active)"
    );
    Ok(())
}

/// DB-021's exception sentence, unpinned by the assignment-carrying
/// fixtures: a project with **no** assignments reads clean even when
/// never synced and its managed text is non-empty (ordinal 0 is scheme
/// A with both reading lines enabled, so its build carries text). A
/// pass that answered `has_assignments = true` unconditionally would
/// fail here (round-1 review's P3-3).
#[test]
fn a_project_without_assignments_reads_clean() -> Result<(), LoadError> {
    let zero_assignments = DataSetSpec {
        projects: 3,
        headings: 1,
        instructions: 8,
        variants_per_instruction: 1,
        assignments_per_project: 0,
    };
    let mut fixture = LoadFixture::create(&zero_assignments, "eunomia-load-zero")?;
    let snapshot = read_snapshot(fixture.connection())?;
    let built = build_projects(&snapshot);
    assert!(
        built.iter().any(|project| !project.managed.is_empty()),
        "the fixture carries projects whose managed text is non-empty"
    );
    let never_synced: StoredHashes = snapshot
        .projects()
        .iter()
        .map(|project| (project.id.as_str().to_owned(), None))
        .collect();
    let verdicts = compute_dirty_verdicts(&snapshot, &never_synced);
    assert!(
        verdicts.iter().all(|&(_, dirty)| !dirty),
        "no assignments: every project reads clean despite the non-empty \
         managed text (DB-021's exception sentence)"
    );
    Ok(())
}

/// The tripwire against measuring a stale pass: the shared chunk
/// size is still 25. The constant lives in `eunomia-core` since
/// T-265/D-145 (this crate's and the two src-tauri mirrors' own
/// spellings dissolved into it), so the pin is by value: the §12.2
/// budget rows were measured at 25, and a core change turns this
/// harness's suite red until the budgets are re-measured on the new
/// chunk size.
#[test]
fn the_production_chunk_size_is_still_twenty_five() {
    assert_eq!(
        MINI_SNAPSHOT_PROJECTS, 25,
        "the shared pass changed its chunk size: re-measure the §12.2 \
         budget rows before this harness reports them"
    );
}

/// DB-021 through the parallel substrate, both directions, and the
/// parallel spelling equal to core's sequential one id-for-id
/// (D-121's L-test pattern, on a small fixture): never-synced
/// projects with assignments read dirty; the same pass over the
/// just-built hashes (what a sync would have stored, DB-024) reads
/// clean; the order is the snapshot's.
#[test]
fn verdicts_follow_the_stored_hashes_and_stay_in_order() -> Result<(), LoadError> {
    let mut fixture = LoadFixture::create(&crossing_spec(), "eunomia-load-verdicts")?;
    let snapshot = read_snapshot(fixture.connection())?;
    let never_synced: StoredHashes = snapshot
        .projects()
        .iter()
        .map(|project| (project.id.as_str().to_owned(), None))
        .collect();

    let sequential = compute_dirty_verdicts(&snapshot, &never_synced);
    let parallel = compute_verdicts_parallel(&snapshot, &never_synced);
    assert_eq!(
        parallel, sequential,
        "the parallel pass is the sequential one"
    );

    let ids: Vec<&str> = sequential.iter().map(|(id, _)| id.as_str()).collect();
    let expected_ids: Vec<&str> = snapshot
        .projects()
        .iter()
        .map(|project| project.id.as_str())
        .collect();
    assert_eq!(ids, expected_ids, "the verdicts keep the snapshot order");
    assert!(
        sequential.iter().all(|&(_, dirty)| dirty),
        "never synced with assignments: every project is dirty (DB-021's first clause)"
    );

    // The synced half: stored = the current build hash (DB-024 after a
    // sync) — every verdict flips to clean.
    let built = build_projects(&snapshot);
    let synced: StoredHashes = built
        .into_iter()
        .map(|project| (project.project_id, Some(project.build_hash)))
        .collect();
    let clean = compute_dirty_verdicts(&snapshot, &synced);
    assert_eq!(
        clean,
        expected_ids
            .iter()
            .map(|&id| (id.to_owned(), false))
            .collect::<Vec<_>>(),
        "a stored matching hash reads clean (DB-021's second clause)"
    );
    Ok(())
}
