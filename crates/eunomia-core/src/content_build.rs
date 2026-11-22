//! Building every active project's content from one snapshot — the
//! chunked multi-project pass the dirty service (T-101/D-121), sync
//! phase 0 (T-112/D-130) and the §12.2 load benchmarks (T-137/D-135)
//! share; consolidated here from their three mirrored spellings
//! (T-265/D-145).
//!
//! One snapshot, many projects, one shape per project: the §9.2 body
//! ([`build_body`](crate::body::build_body)), the ALG-030 managed
//! text around it ([`managed_text`](crate::target_file::managed_text)
//! — scheme A's reading lines spliced in), the §9.4 phase-1-step-2
//! target path, and the DB-020 build hash of it all
//! ([`build_hash`](crate::hash::build_hash)) — [`BuiltProject`]. The
//! dirty pass adds DB-021's comparison
//! ([`is_dirty`](crate::dirty::is_dirty)) over the stored
//! `last_synced_hash` cells: [`compute_dirty_verdicts`].
//!
//! # The batching the §12.2 budget needs
//!
//! `build_body` is per-project and rescans the snapshot's whole
//! assignment list every call — its own docs assign the multi-project
//! reuse to the caller. This is that batching, on set L's numbers
//! (D-121's measurements): calling it naively per project over one
//! snapshot means 500 × 100,000 assignment comparisons (50M — the
//! quadratic the 500 ms budget cannot hold), while one singleton
//! snapshot per project means cloning and re-validating the
//! 1,100-item library 500 times (four million short-lived
//! allocations — measured at twice the budget in release). The
//! middle: **scoped mini-snapshots of [`MINI_SNAPSHOT_PROJECTS`]
//! projects** — the same items, only the chunk's assignments — so
//! `selected_variants`' internal project filter meets a mostly-scoped
//! list and both costs land in the flat middle. The scoping is
//! body-equivalent by construction: filtering the assignment list by
//! the project id is exactly what `selected_variants` does inside, so
//! every project's built bytes are the full snapshot's bytes (pinned
//! by the crossing-wires test and the full-snapshot equivalence test
//! below).
//!
//! # Boundary
//!
//! Pure — no clock, no I/O, no database, no runtime (the crate's
//! discipline, D-006): the snapshot is an immutable value, the
//! stored hashes arrive as a map, and every error path below marks a
//! broken invariant of the caller's composition, never a data
//! condition. What deliberately stays with the consumers: reading
//! the snapshot and the hashes (the store's one-transaction shape,
//! SYS-005/D-105), spreading the chunks over a pool (the app's
//! `spawn_blocking`, the benchmark's `std::thread::scope` — both
//! call [`chunk_verdicts`]/[`build_projects`] as their per-chunk /
//! sequential halves), and everything UI (the `state://dirty` diff
//! cache and emit, the sync run's records).
//!
//! # Examples
//!
//! ```
//! use eunomia_core::assignment::Assignment;
//! use eunomia_core::content_build::{BuiltProject, build_projects};
//! use eunomia_core::library::{
//!     Instruction, InstructionState, ItemId, LibraryItem, Variant, VariantId,
//! };
//! use eunomia_core::project::{
//!     BaseName, ExtraFile, ExtraFilePosition, ProjectId, ProjectWriteSettings, Scheme,
//! };
//! use eunomia_core::snapshot::{BuildSnapshot, SnapshotProject};
//! use std::path::Path;
//!
//! # let project = SnapshotProject {
//! #     id: ProjectId::new("p1".to_owned()),
//! #     name: "demo".to_owned(),
//! #     root: Path::new("/repos/demo").to_owned(),
//! #     settings: ProjectWriteSettings {
//! #         scheme: Scheme::A,
//! #         base_name: BaseName::AgentsMd,
//! #         extra_local: ExtraFile { enabled: false, position: ExtraFilePosition::End },
//! #         extra_project: ExtraFile { enabled: false, position: ExtraFilePosition::End },
//! #     },
//! # };
//! let instruction = LibraryItem::Instruction(Instruction {
//!     id: ItemId::new("a".to_owned()),
//!     title: "Be brief".to_owned(),
//!     state: InstructionState::Active,
//!     variants: vec![Variant {
//!         id: VariantId::new("v-a".to_owned()),
//!         name: "Main".to_owned(),
//!         content: "Be brief.".to_owned(),
//!     }],
//! });
//! let snapshot = BuildSnapshot::new(
//!     vec![project],
//!     vec![instruction],
//!     vec![Assignment {
//!         project_id: ProjectId::new("p1".to_owned()),
//!         instruction_id: ItemId::new("a".to_owned()),
//!         variant_id: VariantId::new("v-a".to_owned()),
//!     }],
//! )
//! .expect("the example is a valid snapshot");
//!
//! let built: Vec<BuiltProject> = build_projects(&snapshot);
//! assert_eq!(built.len(), 1);
//! assert_eq!(built[0].managed, "Be brief.");
//! ```

use std::collections::HashMap;
use std::path::PathBuf;

use crate::assignment::Assignment;
use crate::backup_name::target_file_name;
use crate::body::build_body;
use crate::dirty::{ProjectState, is_dirty};
use crate::extra_files::extra_file_lines;
use crate::hash::build_hash;
use crate::snapshot::{BuildSnapshot, SnapshotProject};
use crate::target_file::managed_text;

/// How many projects share one scoped mini-snapshot in
/// [`build_projects`] and [`compute_dirty_verdicts`] — the batching
/// knob of the §12.2 set-L budget, and the parallel unit of the
/// consumers' passes (one pool task per chunk).
///
/// 25 keeps the per-`build_body` assignment rescan at
/// `25 × (assignments / projects)` (set L: 5,000 rows — 2.5M compares
/// over the whole pass) while paying the item-list clone and the
/// constructor's validation only `500 / 25 = 20` times instead of
/// once per project. Smaller chunks trade clones for rescans and
/// larger ones the reverse; 25 sits in the flat middle for both
/// bounds (anything from ~10 to ~50 measures within noise of the
/// same total on set L, D-121). The §12.2 budget rows were measured
/// at this value; changing it re-opens the measurement.
pub const MINI_SNAPSHOT_PROJECTS: usize = 25;

/// The stored-hash half of the dirty pass: `last_synced_hash` per
/// project row id, `None` ≡ never synced (DB-024's NULL until the
/// first writer; an empty string means the same thing, D-068).
pub type StoredHashes = HashMap<String, Option<String>>;

/// The assignment half of the batching: row indices into the
/// snapshot's assignment list, grouped by project id — one pass over
/// the assignments instead of a rescan per chunk, and the grouping is
/// built once per pass (owned keys, so an `Arc` of it shares across
/// the consumers' parallel chunk tasks). The same map answers DB-021's
/// `has_assignments` exception sentence (a hidden instruction's
/// assignment counts alike, DB-013).
pub type AssignmentsByProject = HashMap<String, Vec<usize>>;

/// One active project's built content — the product of every
/// consumer's pass: what the pipeline will write, where, and under
/// which fingerprint.
///
/// The managed text is the markerless form (ALG-033's comparison
/// basis, the DB-020 hash input): the marker is a per-run wall-clock
/// line (ALG-032) the write phase stamps once for all projects, so it
/// deliberately does not exist here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltProject {
    /// The project row id (UUIDv7 TEXT, DB-002), from the snapshot.
    pub project_id: String,
    /// The ALG-030 managed text, marker excluded: scheme A's reading
    /// lines spliced around the §9.2 body, B and C the body alone.
    pub managed: String,
    /// The target file path — §9.4 phase 1 step 2's spelling:
    /// `<root>/<base_name>` (A, B) or `<root>/<STEM>.project.md` (C).
    /// Phase 1 resolves symlinks on it; a stored
    /// `sync_run_projects.target_path` keeps this snapshot-derived
    /// spelling.
    pub target_path: PathBuf,
    /// The DB-020 build hash of `managed` over this project's write
    /// settings — the value DB-024 stores into
    /// `projects.last_synced_hash` after `written`/`unchanged`, and
    /// the dirty pass compares against.
    pub build_hash: String,
}

/// The chunk boundaries of a pass over `project_count` projects —
/// every `start` a multiple of [`MINI_SNAPSHOT_PROJECTS`], the last
/// chunk possibly shorter (the scoping clamps its end). The boundary
/// arithmetic lives here, once: the consumers' substrates (the app's
/// `spawn_blocking` loop, the benchmark's `std::thread::scope`, the
/// sequential passes) iterate these starts so a loop step can never
/// drift from the constant the §12.2 budgets were measured at.
#[must_use = "the starts are the pass's whole schedule — iterating nothing silently builds no project"]
pub fn chunk_starts(project_count: usize) -> impl Iterator<Item = usize> {
    (0..project_count).step_by(MINI_SNAPSHOT_PROJECTS)
}

/// Groups the snapshot's assignment row indices by project id — one
/// pass over the assignments instead of a rescan per chunk. Built
/// once per pass and shared (as an `Arc`, by the parallel consumers)
/// with every chunk.
#[must_use]
pub fn group_assignments(snapshot: &BuildSnapshot) -> AssignmentsByProject {
    let mut grouped: AssignmentsByProject = HashMap::with_capacity(snapshot.projects().len());
    for (index, assignment) in snapshot.assignments().iter().enumerate() {
        grouped
            .entry(assignment.project_id.as_str().to_owned())
            .or_default()
            .push(index);
    }
    grouped
}

/// The chunk scoping both passes share: the chunk's project slice and
/// its fresh scoped mini-snapshot — the same items, only the chunk's
/// assignments (the body-equivalence the module docs argue). `None`
/// names a `start` past the project list's end.
fn scope_chunk<'a>(
    snapshot: &'a BuildSnapshot,
    grouped: &AssignmentsByProject,
    start: usize,
) -> Option<(&'a [SnapshotProject], BuildSnapshot)> {
    // The last chunk may be shorter — clamp the range end, or `get`
    // would answer None and silently drop the chunk's projects (the
    // release-suite catch D-121 records: every small fixture is one
    // partial chunk).
    let end = (start + MINI_SNAPSHOT_PROJECTS).min(snapshot.projects().len());
    let projects = snapshot.projects().get(start..end)?;
    // This chunk's assignments, scoped once from the grouped indices.
    let mut scoped: Vec<Assignment> = Vec::new();
    for project in projects {
        if let Some(indices) = grouped.get(project.id.as_str()) {
            scoped.extend(
                indices
                    .iter()
                    .map(|&index| snapshot.assignments()[index].clone()),
            );
        }
    }
    let mini = BuildSnapshot::new(projects.to_vec(), snapshot.items().to_vec(), scoped).expect(
        "the full snapshot passed the same validation: a subset of its \
         assignments and projects over the same items cannot fail it",
    );
    Some((projects, mini))
}

/// One project's built content over its chunk's mini-snapshot — the
/// per-project production slice every pass shares: the ALG-030 managed
/// text, the §9.4 target path and the DB-020 hash. The `expect` is
/// D-061's reasoned-panic posture: a `None` body names a broken
/// invariant of the composition, never a silent default (an
/// `unwrap_or_default` would mask a batching bug — the release suite
/// caught exactly such a drop).
fn build_one(project: &SnapshotProject, body: String) -> BuiltProject {
    let settings = &project.settings;
    // D-064's seam: the assembled managed block, marker excluded,
    // scheme A's reading lines included. The argument order is
    // project-then-local — §7.5's tie order, the reverse of the
    // settings struct's field order.
    let lines = extra_file_lines(
        settings.base_name,
        settings.extra_project,
        settings.extra_local,
    );
    let managed = managed_text(settings.scheme, &body, &lines);
    // The DB-020 target path: `<root>/<base_name>` (A, B) or
    // `<root>/<STEM>.project.md` (C) — §9.4 phase 1 step 2's
    // spelling, through the one public file-name owner.
    let target_path = project
        .root
        .join(target_file_name(settings.scheme, settings.base_name));
    let build_hash = build_hash(
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
        build_hash,
    }
}

/// Builds the projects of chunk `start..` (up to
/// [`MINI_SNAPSHOT_PROJECTS`] of them): the §9.2 body over the scoped
/// mini-snapshot, then [`build_one`], in chunk (=`sort_order`)
/// order — the per-chunk half of [`build_projects`].
///
/// # Preconditions
///
/// `start` is a boundary of [`chunk_starts`] over this snapshot's
/// project count (the scoping clamps the last chunk's end), and
/// `grouped` is [`group_assignments`] over **this** snapshot — the
/// indices it carries index into this snapshot's assignment list.
fn chunk_projects(
    snapshot: &BuildSnapshot,
    grouped: &AssignmentsByProject,
    start: usize,
) -> Vec<BuiltProject> {
    let Some((projects, mini)) = scope_chunk(snapshot, grouped, start) else {
        // `start` is always a chunk boundary of the caller's loop.
        return Vec::new();
    };
    projects
        .iter()
        .map(|project| {
            let body = build_body(&mini, &project.id)
                .expect("the mini-snapshot holds every project of its chunk");
            build_one(project, body)
        })
        .collect()
}

/// Computes the verdicts of the projects of chunk `start..`: the same
/// build slice as [`chunk_projects`], then DB-020's comparison —
/// `is_dirty` over the stored hash, the current one, the managed text
/// and the assignments fact (DB-021) — in chunk (=`sort_order`)
/// order. The per-chunk half of [`compute_dirty_verdicts`] and the
/// pool task of the parallel consumers (the dirty service's
/// `spawn_blocking` loop, the benchmark's scoped threads).
///
/// # Preconditions
///
/// `start` is a boundary of [`chunk_starts`] over this snapshot's
/// project count (the scoping clamps the last chunk's end), and
/// `grouped` is [`group_assignments`] over **this** snapshot — the
/// indices it carries index into this snapshot's assignment list.
#[must_use]
pub fn chunk_verdicts(
    snapshot: &BuildSnapshot,
    grouped: &AssignmentsByProject,
    start: usize,
    stored_hashes: &StoredHashes,
) -> Vec<(String, bool)> {
    let Some((projects, mini)) = scope_chunk(snapshot, grouped, start) else {
        // `start` is always a chunk boundary of the caller's loop.
        return Vec::new();
    };
    projects
        .iter()
        .map(|project| {
            let body = build_body(&mini, &project.id)
                .expect("the mini-snapshot holds every project of its chunk");
            let built = build_one(project, body);
            let stored = stored_hashes
                .get(project.id.as_str())
                .and_then(|hash| hash.as_deref());
            // The snapshot holds active projects only (ALG-001), so
            // the state fact is `Active` by construction — DB-014's
            // archived half is the store scope's work, already done.
            let dirty = is_dirty(
                ProjectState::Active,
                stored,
                &built.build_hash,
                &built.managed,
                grouped.contains_key(project.id.as_str()),
            );
            (built.project_id, dirty)
        })
        .collect()
}

/// Builds every active project's content from the snapshot, in
/// `sort_order` — the sequential whole-snapshot content pass (sync
/// phase 0's build step; the benchmark's single-project row).
///
/// Pure (no clock, no I/O, no database): the SYS-005 guarantee is
/// structural for every caller — this function cannot observe an edit
/// that committed after the snapshot's transaction, whatever
/// connection committed it.
#[must_use]
pub fn build_projects(snapshot: &BuildSnapshot) -> Vec<BuiltProject> {
    let grouped = group_assignments(snapshot);
    let mut built: Vec<BuiltProject> = Vec::with_capacity(snapshot.projects().len());
    for start in chunk_starts(snapshot.projects().len()) {
        built.extend(chunk_projects(snapshot, &grouped, start));
    }
    built
}

/// Computes every active project's dirty verdict (DB-021) over the
/// snapshot, in `sort_order` — the sequential whole-snapshot verdict
/// pass and the reference the parallel consumers' spelling is pinned
/// against (the dirty service's L test, the benchmark's id-for-id
/// pin).
///
/// `stored_hashes` is keyed by project row id; a missing key reads as
/// *never synced* (the column is NULL until DB-024's first writer —
/// `None` and an empty string mean the same thing, D-068).
#[must_use]
pub fn compute_dirty_verdicts(
    snapshot: &BuildSnapshot,
    stored_hashes: &StoredHashes,
) -> Vec<(String, bool)> {
    let grouped = group_assignments(snapshot);
    let mut verdicts: Vec<(String, bool)> = Vec::with_capacity(snapshot.projects().len());
    for start in chunk_starts(snapshot.projects().len()) {
        verdicts.extend(chunk_verdicts(snapshot, &grouped, start, stored_hashes));
    }
    verdicts
}

#[cfg(test)]
mod tests {
    //! The shared mechanics' own suite: the DB-021 verdict semantics
    //! through the real build (relocated from the dirty service's
    //! unit tests when its private spelling dissolved, T-265/D-145),
    //! the chunk equivalence against an independent full-snapshot
    //! oracle, and the batching's edge cases.

    use std::collections::HashMap;
    use std::path::Path;

    use super::*;
    use crate::library::{Instruction, InstructionState, ItemId, LibraryItem, Variant, VariantId};
    use crate::project::{
        BaseName, ExtraFile, ExtraFilePosition, ProjectId, ProjectWriteSettings, Scheme,
    };

    // ---- fixtures ------------------------------------------------------

    /// The extras-off option both fields default to in these fixtures.
    fn off() -> ExtraFile {
        ExtraFile {
            enabled: false,
            position: ExtraFilePosition::End,
        }
    }

    /// A canonical scheme-A project (the `body` module's fixture
    /// shape); tests vary the settings through mutation.
    fn project(id: &str) -> SnapshotProject {
        SnapshotProject {
            id: ProjectId::new(id.to_owned()),
            name: format!("project {id}"),
            root: Path::new("/repos").join(id),
            settings: ProjectWriteSettings {
                scheme: Scheme::A,
                base_name: BaseName::AgentsMd,
                extra_local: off(),
                extra_project: off(),
            },
        }
    }

    /// An active instruction with one variant `v-<id>` carrying
    /// `content`.
    fn instruction(id: &str, content: &str) -> LibraryItem {
        LibraryItem::Instruction(Instruction {
            id: ItemId::new(id.to_owned()),
            title: format!("Instruction {id}"),
            state: InstructionState::Active,
            variants: vec![Variant {
                id: VariantId::new(format!("v-{id}")),
                name: "Main".to_owned(),
                content: content.to_owned(),
            }],
        })
    }

    fn assigned(project_id: &str, instruction_id: &str) -> Assignment {
        Assignment {
            project_id: ProjectId::new(project_id.to_owned()),
            instruction_id: ItemId::new(instruction_id.to_owned()),
            variant_id: VariantId::new(format!("v-{instruction_id}")),
        }
    }

    fn snapshot(
        projects: Vec<SnapshotProject>,
        items: Vec<LibraryItem>,
        assignments: Vec<Assignment>,
    ) -> BuildSnapshot {
        BuildSnapshot::new(projects, items, assignments)
            .expect("the test fixture is a valid snapshot")
    }

    fn verdict(id: &str, dirty: bool) -> (String, bool) {
        (id.to_owned(), dirty)
    }

    /// The stored-hash side of [`compute_dirty_verdicts`]'s input.
    fn stored(entries: &[(&str, Option<&str>)]) -> StoredHashes {
        entries
            .iter()
            .map(|(id, hash)| ((*id).to_owned(), hash.map(str::to_owned)))
            .collect()
    }

    /// Computes the verdicts over a fixture snapshot — the tests
    /// speak in (id, verdict) pairs.
    fn verdicts_of(
        projects: Vec<SnapshotProject>,
        items: Vec<LibraryItem>,
        assignments: Vec<Assignment>,
        hashes: &[(&str, Option<&str>)],
    ) -> Vec<(String, bool)> {
        compute_dirty_verdicts(&snapshot(projects, items, assignments), &stored(hashes))
    }

    /// The DB-020 target of the `id` fixture, built the way the pass
    /// builds it (`root.join(file)`): on Windows the join spells `\`,
    /// so a hardcoded `/repos/p1/AGENTS.md` literal diverges from the
    /// recomputed hash (CI 44947235666 — the D-111 platform-truth
    /// rule, path-spelling half).
    fn fixture_target(id: &str, file: &str) -> std::path::PathBuf {
        Path::new("/repos").join(id).join(file)
    }

    /// The hash of the plain `Be brief.` body over the fixture
    /// project's target path — the value a sync would store for that
    /// build (DB-024), spelled directly through the public core seam,
    /// never through the code under test.
    fn synced_hash_of_plain_body() -> String {
        build_hash(
            Scheme::A,
            &fixture_target("p1", "AGENTS.md"),
            off(),
            off(),
            "Be brief.",
        )
    }

    // ---- the chunk equivalence (the §9.2/ALG-030 oracle) ---------------

    /// The chunked pass must be body-equivalent to the independent
    /// full-snapshot spelling (D-121's construction, made
    /// falsifiable at the mechanics' own home): for every project of
    /// a boundary-crossing fixture (26 = one full chunk of 25 + one
    /// project in the second), the bytes and hash the chunked pass
    /// produced must equal what the FULL snapshot produces through
    /// the same core seams spelled here, independently of the chunk
    /// plumbing. A scoping leak (another chunk's assignments in, or
    /// this chunk's own dropped) breaks the equality.
    #[test]
    fn chunked_builds_equal_the_full_snapshot_builds() {
        let ids: Vec<String> = (0..26).map(|index| format!("p{index:02}")).collect();
        let projects: Vec<SnapshotProject> = (0..26).map(|index| project(&ids[index])).collect();
        // Odd projects on `Main`, even on `Compact` — a scoping leak
        // builds the wrong variant for a neighbour.
        let shared = LibraryItem::Instruction(Instruction {
            id: ItemId::new("shared".to_owned()),
            title: "Shared".to_owned(),
            state: InstructionState::Active,
            variants: vec![
                Variant {
                    id: VariantId::new("v-main".to_owned()),
                    name: "Main".to_owned(),
                    content: "Main body.".to_owned(),
                },
                Variant {
                    id: VariantId::new("v-compact".to_owned()),
                    name: "Compact".to_owned(),
                    content: "Compact body.".to_owned(),
                },
            ],
        });
        let assignments: Vec<Assignment> = (0..26)
            .map(|index| Assignment {
                project_id: ProjectId::new(ids[index].clone()),
                instruction_id: ItemId::new("shared".to_owned()),
                variant_id: VariantId::new(if index % 2 == 0 {
                    "v-compact".to_owned()
                } else {
                    "v-main".to_owned()
                }),
            })
            .collect();
        let full = snapshot(projects, vec![shared], assignments);
        let chunked = build_projects(&full);

        // The oracle: each project rebuilt over the FULL snapshot
        // through the public core seams, spelled here independently
        // of the chunk plumbing and of `build_one` — the body over
        // the whole snapshot (`build_body`'s own project filter is
        // the equivalence's other side), the managed text, the
        // target path and the hash composed by hand.
        let expected: Vec<BuiltProject> = full
            .projects()
            .iter()
            .map(|project| {
                let body = build_body(&full, &project.id)
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
                    .join(target_file_name(settings.scheme, settings.base_name));
                BuiltProject {
                    project_id: project.id.as_str().to_owned(),
                    build_hash: build_hash(
                        settings.scheme,
                        &target_path,
                        settings.extra_local,
                        settings.extra_project,
                        &managed,
                    ),
                    managed,
                    target_path,
                }
            })
            .collect();
        assert_eq!(
            chunked, expected,
            "every project's chunked bytes, paths and hashes"
        );
        assert_eq!(chunked.len(), 26, "both chunks' projects are built");
    }

    // ---- the batching's edge cases --------------------------------------

    #[test]
    fn a_partial_last_chunk_keeps_every_project_in_order() {
        // The release-suite catch (D-121): the first chunked version
        // dropped the last partial chunk; 26 projects = one full
        // chunk of 25 + one project in the second. Every id must come
        // back, in `sort_order`, with the per-project verdicts the
        // fixture decides (even-indexed projects assigned, odd ones
        // not).
        let ids: Vec<String> = (0..26).map(|index| format!("p{index:02}")).collect();
        let projects: Vec<SnapshotProject> = (0..26).map(|index| project(&ids[index])).collect();
        let assignments: Vec<Assignment> = (0..26)
            .filter(|index| index % 2 == 0)
            .map(|index| assigned(&ids[index], "i1"))
            .collect();
        let hashes: Vec<(&str, Option<&str>)> = ids.iter().map(|id| (id.as_str(), None)).collect();
        let verdicts = verdicts_of(
            projects,
            vec![instruction("i1", "Shared.")],
            assignments,
            &hashes,
        );
        let expected: Vec<(String, bool)> = (0..26)
            .map(|index| verdict(&ids[index], index % 2 == 0))
            .collect();
        assert_eq!(verdicts, expected);
    }

    #[test]
    fn group_assignments_indexes_every_assignment_once() {
        // The grouping map: every assignment's index lands under its
        // project's key, in snapshot order — the chunk scoping and
        // DB-021's `has_assignments` both read it.
        let grouped = group_assignments(&snapshot(
            vec![project("p1"), project("p2")],
            vec![instruction("i1", "A."), instruction("i2", "B.")],
            vec![
                assigned("p1", "i1"),
                assigned("p2", "i2"),
                assigned("p1", "i2"),
            ],
        ));
        let mut p1: Vec<usize> = grouped.get("p1").cloned().unwrap_or_default();
        p1.sort_unstable();
        assert_eq!(p1, vec![0, 2]);
        assert_eq!(grouped.get("p2").map(|v| v.as_slice()), Some(&[1usize][..]));
        assert!(
            !grouped.contains_key("p3"),
            "an unassigned project has no key"
        );
    }

    // ---- the verdict computation (DB-021 through the real build) --------

    #[test]
    fn a_never_synced_project_with_assignments_and_a_build_is_dirty() {
        // DB-021's first clause: active, never synced (`NULL`), a
        // non-empty build — dirty. The empty-string spelling of
        // "never synced" agrees (D-068).
        for never_synced in [None, Some("")] {
            let verdicts = verdicts_of(
                vec![project("p1")],
                vec![instruction("i1", "Be brief.")],
                vec![assigned("p1", "i1")],
                &[("p1", never_synced)],
            );
            assert_eq!(
                verdicts,
                vec![verdict("p1", true)],
                "stored={never_synced:?}"
            );
        }
    }

    #[test]
    fn a_never_synced_project_without_assignments_is_not_dirty_even_with_reading_lines() {
        // DB-021's exception sentence is absolute: no assignments,
        // never synced — not counted as dirty, even though scheme A
        // with enabled reading lines builds a non-empty managed text
        // (ALG-030 joins the lines without a single assignment).
        let mut unsynced = project("p1");
        unsynced.settings.extra_local = ExtraFile {
            enabled: true,
            position: ExtraFilePosition::Start,
        };
        unsynced.settings.extra_project = ExtraFile {
            enabled: true,
            position: ExtraFilePosition::End,
        };
        let verdicts = verdicts_of(
            vec![unsynced],
            vec![instruction("i1", "Never assigned.")],
            Vec::new(),
            &[("p1", None)],
        );
        assert_eq!(verdicts, vec![verdict("p1", false)]);
    }

    #[test]
    fn a_stored_hash_matching_the_current_build_keeps_the_project_clean() {
        // The after-sync state (DB-024 stored the hash of the build
        // the run wrote): clean.
        let verdicts = verdicts_of(
            vec![project("p1")],
            vec![instruction("i1", "Be brief.")],
            vec![assigned("p1", "i1")],
            &[("p1", Some(&synced_hash_of_plain_body()))],
        );
        assert_eq!(verdicts, vec![verdict("p1", false)]);
    }

    #[test]
    fn a_stored_hash_of_a_different_build_is_dirty() {
        // The synced-clean project, then the edit: the stored hash no
        // longer fingerprints the build — dirty (the difference
        // clause; OUT-013's warning case).
        let verdicts = verdicts_of(
            vec![project("p1")],
            vec![instruction("i1", "Be very brief.")],
            vec![assigned("p1", "i1")],
            &[("p1", Some(&synced_hash_of_plain_body()))],
        );
        assert_eq!(verdicts, vec![verdict("p1", true)]);
    }

    #[test]
    fn enabling_an_extra_file_line_makes_a_synced_project_dirty() {
        // DB-023's family through the real inputs: a settings change
        // alone (no library edit) moves the build — the reading line
        // enters the managed text and the hash follows. The stored
        // value is the hash of the same project with both extras off.
        let mut synced = project("p1");
        synced.settings.extra_local = ExtraFile {
            enabled: true,
            position: ExtraFilePosition::Start,
        };
        let verdicts = verdicts_of(
            vec![synced],
            vec![instruction("i1", "Be brief.")],
            vec![assigned("p1", "i1")],
            &[("p1", Some(&synced_hash_of_plain_body()))],
        );
        assert_eq!(verdicts, vec![verdict("p1", true)]);
    }

    #[test]
    fn per_project_verdicts_follow_the_snapshot_order() {
        // `sort_order` in, `sort_order` out: the consumers' diff keeps
        // the column order.
        let verdicts = verdicts_of(
            vec![project("p1"), project("p2"), project("p3")],
            vec![instruction("i1", "Shared text.")],
            vec![assigned("p1", "i1"), assigned("p2", "i1")],
            &[("p1", None), ("p2", None), ("p3", None)],
        );
        assert_eq!(
            verdicts,
            vec![
                verdict("p1", true),
                verdict("p2", true),
                verdict("p3", false)
            ]
        );
    }

    #[test]
    fn scoped_builds_never_cross_projects_wires() {
        // The scoping's own proof: two projects pick different
        // variants of one shared instruction, and each verdict
        // follows its OWN build — p1's stored hash (spelled over the
        // `Main` body) keeps it clean exactly because its scoped
        // snapshot built that body, while p2, whose scoped build is
        // the `Compact` body the same instruction also carries, does
        // not match p1's stored value and is dirty. A scoping bug
        // that leaked p2's assignment into p1's mini-snapshot (or the
        // reverse) flips both cells.
        let shared = LibraryItem::Instruction(Instruction {
            id: ItemId::new("shared".to_owned()),
            title: "Shared".to_owned(),
            state: InstructionState::Active,
            variants: vec![
                Variant {
                    id: VariantId::new("v-main".to_owned()),
                    name: "Main".to_owned(),
                    content: "Main body.".to_owned(),
                },
                Variant {
                    id: VariantId::new("v-compact".to_owned()),
                    name: "Compact".to_owned(),
                    content: "Compact body.".to_owned(),
                },
            ],
        });
        let p1_stored = {
            // Spelled directly through the public core seam: the
            // DB-024 value a sync of p1's Main-variant build stores.
            let target = fixture_target("p1", "AGENTS.md");
            build_hash(Scheme::A, &target, off(), off(), "Main body.")
        };
        let verdicts = verdicts_of(
            vec![project("p1"), project("p2")],
            vec![shared],
            vec![
                Assignment {
                    project_id: ProjectId::new("p1".to_owned()),
                    instruction_id: ItemId::new("shared".to_owned()),
                    variant_id: VariantId::new("v-main".to_owned()),
                },
                Assignment {
                    project_id: ProjectId::new("p2".to_owned()),
                    instruction_id: ItemId::new("shared".to_owned()),
                    variant_id: VariantId::new("v-compact".to_owned()),
                },
            ],
            &[
                ("p1", Some(p1_stored.as_str())),
                ("p2", Some(p1_stored.as_str())),
            ],
        );
        assert_eq!(
            verdicts,
            vec![verdict("p1", false), verdict("p2", true)],
            "p1 clean over its own Main build; p2 dirty over the \
             Compact build p1's stored hash does not fingerprint"
        );
    }

    // ---- clean pins for every scheme and settings shape ---------------

    /// A synced-clean verdict through an independently spelled stored
    /// hash: `settings` for the hash inputs, `path` for the target,
    /// `body` for the managed text (extras off → the body IS the
    /// managed; extras on → the reading lines join it, spelled here
    /// through the real `extra_file_lines` + the §7.5 join, not
    /// through `managed_text`).
    fn clean_pin(settings: ProjectWriteSettings, path: &Path, body: &str) -> Vec<(String, bool)> {
        let managed = if settings.extra_local.enabled || settings.extra_project.enabled {
            let lines = extra_file_lines(
                settings.base_name,
                settings.extra_project,
                settings.extra_local,
            );
            let mut parts: Vec<String> = Vec::new();
            if !lines.start.is_empty() {
                parts.push(lines.start.join("\n"));
            }
            parts.push(body.to_owned());
            if !lines.end.is_empty() {
                parts.push(lines.end.join("\n"));
            }
            parts.join("\n\n")
        } else {
            body.to_owned()
        };
        let stored_value = build_hash(
            settings.scheme,
            path,
            settings.extra_local,
            settings.extra_project,
            &managed,
        );
        let mut synced = project("p1");
        synced.settings = settings;
        verdicts_of(
            vec![synced],
            vec![instruction("i1", body)],
            vec![assigned("p1", "i1")],
            &[("p1", Some(stored_value.as_str()))],
        )
    }

    #[test]
    fn scheme_b_c_and_enabled_extras_each_stay_clean_over_their_own_hash() {
        // The clean verdict must hold for every settings shape, each
        // with its stored hash spelled independently: a swapped
        // `extra_file_lines` argument order or a wrong target path
        // moves the hash and the verdict turns dirty (the D-121
        // round-1 coverage hole, kept at the mechanics' new home).
        let extras =
            |local_enabled: bool,
             local_position: ExtraFilePosition,
             project_enabled: bool,
             project_position: ExtraFilePosition| ProjectWriteSettings {
                scheme: Scheme::A,
                base_name: BaseName::AgentsMd,
                extra_local: ExtraFile {
                    enabled: local_enabled,
                    position: local_position,
                },
                extra_project: ExtraFile {
                    enabled: project_enabled,
                    position: project_position,
                },
            };
        // Scheme B: `<root>/<base_name>`, no reading lines.
        let mut b = extras(false, ExtraFilePosition::End, false, ExtraFilePosition::End);
        b.scheme = Scheme::B;
        b.base_name = BaseName::ClaudeMd;
        assert_eq!(
            clean_pin(b, &fixture_target("p1", "CLAUDE.md"), "Body."),
            vec![verdict("p1", false)]
        );
        // Scheme C: `<root>/<STEM>.project.md`, no reading lines.
        let mut c = extras(false, ExtraFilePosition::End, false, ExtraFilePosition::End);
        c.scheme = Scheme::C;
        assert_eq!(
            clean_pin(c, &fixture_target("p1", "AGENTS.project.md"), "Body."),
            vec![verdict("p1", false)]
        );
        // Scheme A with the `.project.md` reading line enabled, at
        // start (`.local.md` off): the extras' positions and the
        // project-then-local argument order are fingerprinted by the
        // stored value — a swapped argument order moves the hash.
        let a_full = extras(
            false,
            ExtraFilePosition::End,
            true,
            ExtraFilePosition::Start,
        );
        assert_eq!(
            clean_pin(a_full, &fixture_target("p1", "AGENTS.md"), "Body."),
            vec![verdict("p1", false)]
        );
    }

    // ---- the stored-hash map's contract ---------------------------------

    #[test]
    fn a_missing_stored_key_reads_as_never_synced() {
        // The map is keyed by row id; an absent key (a project the
        // caller's read did not cover) must read like `None`, never
        // panic or default to clean-with-a-hash.
        let verdicts = compute_dirty_verdicts(
            &snapshot(
                vec![project("p1")],
                vec![instruction("i1", "Be brief.")],
                vec![assigned("p1", "i1")],
            ),
            &HashMap::new(),
        );
        assert_eq!(verdicts, vec![verdict("p1", true)]);
    }
}
