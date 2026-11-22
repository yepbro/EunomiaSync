//! The mirror pins: the harness's third spelling of D-064/D-121's
//! mechanics cannot drift, because every mirrored half is checked
//! against an oracle that does not run through this crate.
//!
//! * [`managed_text`] is pinned against `build_content`'s real output
//!   (the file minus its marker tail) for schemes A and C, and against
//!   the body for scheme B — the same oracles `dirty_recalc.rs` and
//!   `sync_phase0.rs` pin their own copies with (D-121/D-130).
//! * The chunked mini-snapshot scoping is pinned against an
//!   independent full-snapshot build on a boundary-crossing fixture:
//!   every project's chunked bytes must equal the bytes
//!   `build_body` produces over the whole snapshot (D-121's
//!   body-equivalence, executable).
//! * The parallel verdict pass is pinned against the sequential one
//!   id-for-id, and both against DB-021's semantics (never-synced
//!   dirty, synced-hash clean) on the same fixture.

use eunomia_core::body::build_body;
use eunomia_core::extra_files::{ExtraFileLines, extra_file_lines};
use eunomia_core::project::{BaseName, ExtraFile, ExtraFilePosition, Scheme};
use eunomia_core::target_file::build_content;

use eunomia_load::{
    BuiltProject, LoadError, LoadFixture, StoredHashes, build_projects_sequential,
    compute_verdicts_parallel, compute_verdicts_sequential, managed_text, read_snapshot,
    read_snapshot_and_hashes,
};
use eunomia_store::data_sets::DataSetSpec;

/// The house marker fixture (single line, DB-016's shape).
const MARKER: &str = "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";

/// A body without framing line breaks (`build_body`'s output shape,
/// `build_content`'s input contract).
const BODY: &str = "## Style\n\nBe brief.";

/// The off extra-file option.
fn off() -> ExtraFile {
    ExtraFile {
        enabled: false,
        position: ExtraFilePosition::End,
    }
}

/// Both reading lines enabled: `.project.md` at the start, `.local.md`
/// at the end (§7.5's positions).
fn both_lines() -> ExtraFileLines {
    extra_file_lines(
        BaseName::AgentsMd,
        ExtraFile {
            enabled: true,
            position: ExtraFilePosition::Start,
        },
        ExtraFile {
            enabled: true,
            position: ExtraFilePosition::End,
        },
    )
}

/// No reading lines (both options off).
fn no_lines() -> ExtraFileLines {
    extra_file_lines(BaseName::AgentsMd, off(), off())
}

/// The empty-managed edge is spelled by the truth itself: an empty
/// managed block makes the file the marker alone (ALG-030).
#[test]
fn managed_text_matches_build_content_for_scheme_a() {
    let lines = both_lines();
    let managed = managed_text(Scheme::A, BODY, &lines);
    assert_eq!(
        build_content(Scheme::A, BODY, &lines, MARKER),
        format!("{managed}\n\n{MARKER}\n"),
        "scheme A's real file is the mirror's managed text plus the \
         marker tail"
    );
    // The reading lines splice around the body (ALG-030's "(A only)")
    // — D-21's sentence with each file bound, the first at the start,
    // the second at the end.
    assert!(
        managed.starts_with("If AGENTS.project.md exists in the repository root,"),
        "the project file's reading line leads the managed block: {managed:?}"
    );
    assert!(
        managed.ends_with(
            "If AGENTS.local.md exists in the repository root, read it before \
                           starting work and follow its instructions."
        ),
        "the local file's reading line closes the managed block: {managed:?}"
    );
    // An empty body leaves the lines alone as the managed block.
    let lines_only = managed_text(Scheme::A, "", &lines);
    assert_eq!(
        build_content(Scheme::A, "", &lines, MARKER),
        format!("{lines_only}\n\n{MARKER}\n")
    );
    // And the empty-managed edge: no lines, no body — the file is the
    // marker alone, the mirror is the empty string.
    assert_eq!(managed_text(Scheme::A, "", &no_lines()), "");
    assert_eq!(
        build_content(Scheme::A, "", &no_lines(), MARKER),
        format!("{MARKER}\n")
    );
}

/// Scheme C takes the body alone (ALG-030's gate): the real file
/// ignores the reading lines, and so must the mirror.
#[test]
fn managed_text_matches_build_content_for_scheme_c() {
    let lines = both_lines();
    let managed = managed_text(Scheme::C, BODY, &lines);
    assert_eq!(managed, BODY, "scheme C's managed block is the body alone");
    assert_eq!(
        build_content(Scheme::C, BODY, &lines, MARKER),
        format!("{managed}\n\n{MARKER}\n")
    );
    assert_eq!(managed_text(Scheme::C, "", &no_lines()), "");
}

/// Scheme B is ALG-031's business — `build_content` refuses it by
/// design, so the mirror's oracle is the documented rule itself
/// (`assembly_b`'s "the managed block is the body alone"), the same
/// pin `dirty_recalc.rs` keeps for its copy.
#[test]
fn managed_text_takes_the_body_alone_for_scheme_b() {
    let lines = both_lines();
    assert_eq!(managed_text(Scheme::B, BODY, &lines), BODY);
    assert_eq!(managed_text(Scheme::B, "", &no_lines()), "");
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
/// through core's public API, never through this crate's chunk
/// plumbing — produces the same managed text and hash the chunked pass
/// produced. A scoping leak (another chunk's assignments in, or this
/// chunk's own dropped) breaks the equality.
#[test]
fn chunked_builds_equal_the_full_snapshot_builds() {
    let mut fixture = LoadFixture::create(&crossing_spec(), "eunomia-load-pins")
        .expect("the crossing fixture builds");
    let snapshot = read_snapshot(fixture.connection()).expect("the snapshot read succeeds");
    let chunked = build_projects_sequential(&snapshot);

    // The oracle: each project rebuilt over the full snapshot, the
    // same core seams the mirrors compose, spelled independently here.
    let expected: Vec<BuiltProject> = snapshot
        .projects()
        .iter()
        .map(|project| {
            let body = build_body(&snapshot, &project.id)
                .expect("the full snapshot holds every project it lists");
            let settings = &project.settings;
            let lines = extra_file_lines(
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
            let hash = eunomia_core::hash::build_hash(
                settings.scheme,
                &target_path,
                settings.extra_local,
                settings.extra_project,
                &managed,
            );
            BuiltProject {
                project_id: project.id.as_str().to_owned(),
                managed,
                build_hash: hash,
            }
        })
        .collect();

    assert_eq!(
        chunked, expected,
        "every project's chunked bytes and hashes"
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
/// mirror that passed `has_assignments = true` unconditionally would
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
    let built = build_projects_sequential(&snapshot);
    assert!(
        built.iter().any(|project| !project.managed.is_empty()),
        "the fixture carries projects whose managed text is non-empty"
    );
    let never_synced: StoredHashes = snapshot
        .projects()
        .iter()
        .map(|project| (project.id.as_str().to_owned(), None))
        .collect();
    let verdicts = compute_verdicts_sequential(&snapshot, &never_synced);
    assert!(
        verdicts.iter().all(|&(_, dirty)| !dirty),
        "no assignments: every project reads clean despite the non-empty \
         managed text (DB-021's exception sentence)"
    );
    Ok(())
}

/// The tripwire against the mirrors measuring a stale pass: the two
/// production spellings still chunk at 25. `MINI_SNAPSHOT_PROJECTS`
/// lives in frozen zones this crate cannot import, so the pin is a
/// source scan (the `dirty_recalc_tests.rs` precedent) — a task that
/// changes either production constant goes red here until this
/// harness is re-mirrored. The structural fix is D-130's consolidation
/// card, which this third copy joins (round-1 review's P3-7).
#[test]
fn the_production_chunk_size_is_still_twenty_five() {
    for source in [
        include_str!("../../../src-tauri/src/dirty_recalc.rs"),
        include_str!("../../../src-tauri/src/sync_phase0.rs"),
    ] {
        assert!(
            source.contains("const MINI_SNAPSHOT_PROJECTS: usize = 25;"),
            "a production pass changed its chunk size: re-mirror the \
             harness (D-130's consolidation card is the structural fix)"
        );
    }
}

/// DB-021 through the mirrored pass, both directions, and the parallel
/// spelling equal to the sequential one id-for-id (D-121's L-test
/// pattern, on a small fixture): never-synced projects with
/// assignments read dirty; the same pass over the just-built hashes
/// (what a sync would have stored, DB-024) reads clean; the order is
/// the snapshot's.
#[test]
fn verdicts_follow_the_stored_hashes_and_stay_in_order() -> Result<(), LoadError> {
    let mut fixture = LoadFixture::create(&crossing_spec(), "eunomia-load-verdicts")?;
    let snapshot = read_snapshot(fixture.connection())?;
    let never_synced: StoredHashes = snapshot
        .projects()
        .iter()
        .map(|project| (project.id.as_str().to_owned(), None))
        .collect();

    let sequential = compute_verdicts_sequential(&snapshot, &never_synced);
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
    let built = build_projects_sequential(&snapshot);
    let synced: StoredHashes = built
        .into_iter()
        .map(|project| (project.project_id, Some(project.build_hash)))
        .collect();
    let clean = compute_verdicts_sequential(&snapshot, &synced);
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
