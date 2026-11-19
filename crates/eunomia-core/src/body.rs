//! Building the body of a project's target file (spec §9.2).
//!
//! The body is the Markdown every scheme writes the same way: section
//! headings that survive the ALG-020 inclusion rule and the normalized
//! texts (ALG-002) of the assigned variants, in library order. This
//! module is a direct transcription of the §9.2 pseudocode over the
//! immutable [`BuildSnapshot`](crate::snapshot::BuildSnapshot) — a
//! pure function with no I/O, like the rest of this crate.
//!
//! Rules the body lives by (spec §9.2, ALG-020…022):
//!
//! - **`included`** is the set of instructions that are active AND
//!   assigned to the project. A hidden instruction keeps its
//!   assignment in the snapshot but never reaches a file, and it does
//!   not keep a heading either (spec D-11, DB-013) — the state check
//!   is inside the pseudocode's `included`, not an afterthought.
//! - A heading goes in when its section — everything up to the next
//!   heading of the same or a higher level — holds at least one
//!   `included` instruction (ALG-020). Parents follow automatically:
//!   a nested section is inside the parent's range.
//! - An instruction with an empty variant is `included`: it keeps its
//!   heading but contributes no block (spec D-13).
//! - The instruction's own name never enters the file (ALG-022); only
//!   the variant text does, after
//!   [`normalize::variant_text`](crate::normalize::variant_text).
//! - Blocks are separated by exactly one empty line (two line breaks);
//!   the body carries no trailing line break of its own.
//!
//! Boundary: everything scheme-specific is a later pipeline step and
//! deliberately absent here — the extra-file reading lines, the
//! creation marker and the empty-line-plus-marker tail of ALG-030, and
//! scheme B's markers with the surrounding file (ALG-031) all consume
//! this body; the base name / `<STEM>.project.md` file skeleton is
//! likewise not the body's concern. The returned value is the same for
//! schemes A, B and C, so [`build_body`] reads no project settings
//! beyond the project's identity.
//!
//! # Examples
//!
//! The example of spec §9.2 verbatim: a library of `## Style` with
//! instructions A and B, `### Tests` with instruction C, `## Release`
//! with instruction D — a project with only C assigned gets `## Style`,
//! `### Tests` and C's text, and no `## Release`:
//!
//! ```
//! use eunomia_core::assignment::Assignment;
//! use eunomia_core::body::build_body;
//! use eunomia_core::library::{
//!     Heading, HeadingLevel, Instruction, InstructionState, ItemId, LibraryItem, Variant, VariantId,
//! };
//! use eunomia_core::project::ProjectId;
//! use eunomia_core::snapshot::BuildSnapshot;
//!
//! # use eunomia_core::project::{BaseName, ExtraFile, ExtraFilePosition, ProjectWriteSettings, Scheme};
//! # use eunomia_core::snapshot::SnapshotProject;
//! # use std::path::Path;
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
//! let heading =
//!     |id: &str, level: u8, title: &str| match HeadingLevel::try_from(level) {
//!         Ok(level) => LibraryItem::Heading(Heading {
//!             id: ItemId::new(id.to_owned()),
//!             level,
//!             title: title.to_owned(),
//!         }),
//!         Err(error) => panic!("invalid level: {error}"),
//!     };
//! let instruction = |id: &str, content: &str| {
//!     LibraryItem::Instruction(Instruction {
//!         id: ItemId::new(id.to_owned()),
//!         title: format!("Instruction {id}"),
//!         state: InstructionState::Active,
//!         variants: vec![Variant {
//!             id: VariantId::new(format!("v-{id}")),
//!             name: "Main".to_owned(),
//!             content: content.to_owned(),
//!         }],
//!     })
//! };
//!
//! let snapshot = BuildSnapshot::new(
//!     vec![project.clone()],
//!     vec![
//!         heading("h-style", 2, "Style"),
//!         instruction("a", "Answer in English."),
//!         instruction("b", "Be brief."),
//!         heading("h-tests", 3, "Tests"),
//!         instruction("c", "Run the test suite before every commit."),
//!         heading("h-release", 2, "Release"),
//!         instruction("d", "Tag releases."),
//!     ],
//!     vec![Assignment {
//!         project_id: project.id.clone(),
//!         instruction_id: ItemId::new("c".to_owned()),
//!         variant_id: VariantId::new("v-c".to_owned()),
//!     }],
//! )
//! .expect("the example library is a valid snapshot");
//!
//! let body = build_body(&snapshot, &project.id).expect("the project is in the snapshot");
//! assert_eq!(
//!     body,
//!     "## Style\n\n### Tests\n\nRun the test suite before every commit."
//! );
//! assert!(!body.contains("Release"));
//! ```

use std::collections::HashMap;

use crate::library::{ItemId, LibraryItem, Variant, VariantId};
use crate::normalize;
use crate::project::ProjectId;
use crate::snapshot::BuildSnapshot;

/// The separator between two blocks (spec §9.2: `BLANK_LINE` = two
/// line breaks = exactly one empty line, ALG-022).
const BLANK_LINE: &str = "\n\n";

/// Builds one project's body from the snapshot (spec §9.2).
///
/// The function walks the snapshot's library items in order — the
/// library's `position` order, which the snapshot preserves (DB-005,
/// LIB-035, DoD 5) — and collects the blocks the §9.2 pseudocode
/// collects: headings whose section holds at least one included
/// instruction (ALG-020) and the normalized texts of the included
/// instructions' assigned variants. The blocks are joined with one
/// empty line between neighbours; the result carries no trailing line
/// break (the file skeleton of ALG-030 adds everything after the
/// body).
///
/// The body is scheme-independent (A, B and C write the same body;
/// see the module docs), so the project's write settings are not read.
/// The call is linear in the snapshot's item and assignment counts; a
/// full sync building many projects calls it per project, and batching
/// that reuse belongs to the sync pipeline, not to this function.
///
/// An empty result is a legitimate outcome — a project whose every
/// section is empty or hidden builds an empty body, which the sync
/// pipeline runs the spec D-25 confirmation for. It is returned as
/// [`Some`](`Option::Some`) of an empty [`String`], distinct from
/// [`None`](`Option::None`), which means the project is not among the
/// snapshot's projects at all — a caller that mixed up identifiers.
///
/// # Panics
///
/// Panics if an assignment's variant is missing from its instruction's
/// variants — impossible for a snapshot built through
/// [`BuildSnapshot::new`](crate::snapshot::BuildSnapshot::new), which
/// rejects the shape (DB-012) at assembly; the panic marks a broken
/// invariant, not a data condition.
///
/// # Examples
///
/// ```
/// # use eunomia_core::assignment::Assignment;
/// # use eunomia_core::body::build_body;
/// # use eunomia_core::library::{
/// #     Heading, HeadingLevel, Instruction, InstructionState, ItemId, LibraryItem, Variant,
/// #     VariantId,
/// # };
/// # use eunomia_core::project::ProjectId;
/// # use eunomia_core::snapshot::BuildSnapshot;
/// # use std::path::Path;
/// # use eunomia_core::project::{BaseName, ExtraFile, ExtraFilePosition, ProjectWriteSettings, Scheme};
/// # use eunomia_core::snapshot::SnapshotProject;
/// # let project = SnapshotProject {
/// #     id: ProjectId::new("p1".to_owned()),
/// #     name: "demo".to_owned(),
/// #     root: Path::new("/repos/demo").to_owned(),
/// #     settings: ProjectWriteSettings {
/// #         scheme: Scheme::A,
/// #         base_name: BaseName::AgentsMd,
/// #         extra_local: ExtraFile { enabled: false, position: ExtraFilePosition::End },
/// #         extra_project: ExtraFile { enabled: false, position: ExtraFilePosition::End },
/// #     },
/// # };
/// # let instruction = |id: &str, content: &str| {
/// #     LibraryItem::Instruction(Instruction {
/// #         id: ItemId::new(id.to_owned()),
/// #         title: format!("Instruction {id}"),
/// #         state: InstructionState::Active,
/// #         variants: vec![Variant {
/// #             id: VariantId::new(format!("v-{id}")),
/// #             name: "Main".to_owned(),
/// #             content: content.to_owned(),
/// #         }],
/// #     })
/// # };
/// let snapshot = BuildSnapshot::new(
///     vec![project.clone()],
///     vec![instruction("a", "First."), instruction("b", "Second.")],
///     vec![Assignment {
///         project_id: project.id.clone(),
///         instruction_id: ItemId::new("b".to_owned()),
///         variant_id: VariantId::new("v-b".to_owned()),
///     }],
/// )
/// .expect("valid");
///
/// // Only B is assigned: no headings exist, so the body is B's text…
/// let body = build_body(&snapshot, &project.id).expect("in the snapshot");
/// assert_eq!(body, "Second.");
///
/// // …and an unknown project has no body at all.
/// assert!(build_body(&snapshot, &ProjectId::new("nope".to_owned())).is_none());
/// ```
#[must_use]
pub fn build_body(snapshot: &BuildSnapshot, project: &ProjectId) -> Option<String> {
    if !snapshot.projects().iter().any(|known| known.id == *project) {
        return None;
    }

    let selected = selected_variants(snapshot, project);
    let items = snapshot.items();
    let mut blocks: Vec<String> = Vec::new();

    for (index, item) in items.iter().enumerate() {
        match item {
            LibraryItem::Heading(heading) => {
                // §9.2: end = first j > k with a heading of level <= L,
                // else len(items); the heading goes in when any item of
                // items[k+1..end) is included.
                let level = heading.level.as_u8();
                let end = section_end(items, index, level);
                let section_includes = items[index + 1..end]
                    .iter()
                    .any(|section_item| selected.contains_key(section_item.item_id()));
                if section_includes {
                    blocks.push(heading_block(level, &heading.title));
                }
            }
            LibraryItem::Instruction(instruction) => {
                if let Some(variant) = selected.get(&instruction.id) {
                    let text = normalize::variant_text(&variant.content);
                    if !text.is_empty() {
                        blocks.push(text);
                    }
                }
            }
        }
    }

    Some(blocks.join(BLANK_LINE))
}

/// Resolves the project's build set: the `included` set of §9.2 plus
/// each included instruction's assigned variant.
///
/// This is the per-project resolution D-042 keeps out of the snapshot
/// state: the assignments scoped to one project, joined with the
/// active instructions (a hidden instruction is never selected —
/// D-11/DB-013), each mapped to its assigned variant. D-02 and the
/// snapshot's assembly guarantee the join is unique per instruction
/// and that every variant resolves (DB-012).
fn selected_variants<'a>(
    snapshot: &'a BuildSnapshot,
    project: &ProjectId,
) -> HashMap<&'a ItemId, &'a Variant> {
    let mut assigned_variant: HashMap<&ItemId, &VariantId> = HashMap::new();
    for assignment in snapshot.assignments() {
        if assignment.project_id == *project {
            assigned_variant.insert(&assignment.instruction_id, &assignment.variant_id);
        }
    }

    let mut selected = HashMap::with_capacity(assigned_variant.len());
    for item in snapshot.items() {
        let LibraryItem::Instruction(instruction) = item else {
            continue;
        };
        if !instruction.is_active() {
            continue;
        }
        let Some(&variant_id) = assigned_variant.get(&instruction.id) else {
            continue;
        };
        // BuildSnapshot::new rejects a variant outside its instruction
        // (SnapshotError::VariantMismatch, DB-012), so this cannot miss
        // for a snapshot built through the constructor.
        let variant = instruction
            .variants
            .iter()
            .find(|variant| variant.id == *variant_id)
            .expect(
                "snapshot validation guarantees the assigned variant belongs to its instruction",
            );
        selected.insert(&instruction.id, variant);
    }
    selected
}

/// The exclusive end of the heading's section: the index of the first
/// heading after `heading_index` with a level less than or equal to
/// `level` (the schema's 2–6 integer,
/// [`HeadingLevel::as_u8`](crate::library::HeadingLevel::as_u8)), or
/// the item count (§9.2, ALG-020).
///
/// Levels only run 2–6, so every item lies in at most five sections
/// and the whole walk stays linear in the item count.
fn section_end(items: &[LibraryItem], heading_index: usize, level: u8) -> usize {
    items
        .iter()
        .enumerate()
        .skip(heading_index + 1)
        .find_map(|(end, item)| match item {
            LibraryItem::Heading(heading) if heading.level.as_u8() <= level => Some(end),
            _ => None,
        })
        .unwrap_or(items.len())
}

/// The heading's block: `repeat('#', L) + ' ' + title` (§9.2).
fn heading_block(level: u8, title: &str) -> String {
    let hashes = "#".repeat(usize::from(level));
    format!("{hashes} {title}")
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;
    use std::path::Path;

    use super::*;
    use crate::assignment::Assignment;
    use crate::library::{
        Heading, HeadingLevel, Instruction, InstructionState, LibraryItem, Variant,
    };
    use crate::project::{BaseName, ExtraFile, ExtraFilePosition, ProjectWriteSettings, Scheme};
    use crate::snapshot::SnapshotProject;

    // ---- fixtures ------------------------------------------------------

    /// A canonical scheme-A project value (not a §6.5 default — those
    /// belong to the store's settings repository, D-039). The body is
    /// scheme-independent; one test below varies the settings on
    /// purpose to pin that, all others keep this default.
    fn project(id: &str) -> SnapshotProject {
        SnapshotProject {
            id: ProjectId::new(id.to_owned()),
            name: format!("project {id}"),
            root: Path::new("/repos").join(id),
            settings: ProjectWriteSettings {
                scheme: Scheme::A,
                base_name: BaseName::AgentsMd,
                extra_local: ExtraFile {
                    enabled: false,
                    position: ExtraFilePosition::End,
                },
                extra_project: ExtraFile {
                    enabled: false,
                    position: ExtraFilePosition::End,
                },
            },
        }
    }

    fn heading(id: &str, level: u8, title: &str) -> LibraryItem {
        let level = HeadingLevel::try_from(level)
            .unwrap_or_else(|error| panic!("invalid level {level}: {error}"));
        LibraryItem::Heading(Heading {
            id: ItemId::new(id.to_owned()),
            level,
            title: title.to_owned(),
        })
    }

    /// An active instruction with one variant named `v-<id>`.
    fn instruction(id: &str, content: &str) -> LibraryItem {
        LibraryItem::Instruction(Instruction {
            id: ItemId::new(id.to_owned()),
            title: format!("Instruction title {id}"),
            state: InstructionState::Active,
            variants: vec![Variant {
                id: VariantId::new(format!("v-{id}")),
                name: format!("Variant name {id}"),
                content: content.to_owned(),
            }],
        })
    }

    fn instruction_with_state(id: &str, content: &str, state: InstructionState) -> LibraryItem {
        let LibraryItem::Instruction(mut instruction) = instruction(id, content) else {
            unreachable!("instruction() always builds the instruction variant");
        };
        instruction.state = state;
        LibraryItem::Instruction(instruction)
    }

    /// An active instruction whose variants are given explicitly, for
    /// tests where two projects pick different variants of one
    /// instruction.
    fn instruction_with_variants(id: &str, contents: &[&str]) -> LibraryItem {
        LibraryItem::Instruction(Instruction {
            id: ItemId::new(id.to_owned()),
            title: format!("Instruction title {id}"),
            state: InstructionState::Active,
            variants: contents
                .iter()
                .enumerate()
                .map(|(number, content)| Variant {
                    id: VariantId::new(format!("v-{id}-{}", number + 1)),
                    name: format!("Variant name {id} #{}", number + 1),
                    content: (*content).to_owned(),
                })
                .collect(),
        })
    }

    fn assignment(project_id: &str, instruction_id: &str, variant_id: &str) -> Assignment {
        Assignment {
            project_id: ProjectId::new(project_id.to_owned()),
            instruction_id: ItemId::new(instruction_id.to_owned()),
            variant_id: VariantId::new(variant_id.to_owned()),
        }
    }

    /// The default assignment: project p1, the instruction's only
    /// variant `v-<id>`.
    fn assigned(instruction_id: &str) -> Assignment {
        assignment("p1", instruction_id, &format!("v-{instruction_id}"))
    }

    fn snapshot(
        projects: Vec<SnapshotProject>,
        items: Vec<LibraryItem>,
        assignments: Vec<Assignment>,
    ) -> BuildSnapshot {
        BuildSnapshot::new(projects, items, assignments)
            .expect("the test fixture is a valid snapshot")
    }

    fn body_of(snapshot: &BuildSnapshot, project_id: &str) -> String {
        let id = ProjectId::new(project_id.to_owned());
        build_body(snapshot, &id)
            .unwrap_or_else(|| panic!("project {project_id} is in the snapshot"))
    }

    // ---- the §9.2 oracle ------------------------------------------------

    #[test]
    fn spec_9_2_example_only_c_assigned_keeps_style_and_tests_drops_release() {
        // Library: ## Style → A, B; ### Tests → C; ## Release → D.
        // Only C is assigned: the body is ## Style, ### Tests, C's
        // text — and no ## Release. The expected string is hardcoded
        // from the spec's example, independent of the implementation.
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                heading("h-style", 2, "Style"),
                instruction("a", "Answer in English."),
                instruction("b", "Be brief."),
                heading("h-tests", 3, "Tests"),
                instruction("c", "Run the test suite before every commit."),
                heading("h-release", 2, "Release"),
                instruction("d", "Tag releases."),
            ],
            vec![assigned("c")],
        );

        assert_eq!(
            body_of(&snapshot, "p1"),
            "## Style\n\n### Tests\n\nRun the test suite before every commit."
        );
    }

    // ---- ALG-020: heading inclusion by section --------------------------

    #[test]
    fn a_parent_heading_is_kept_by_a_nested_section_and_an_empty_sibling_is_dropped() {
        // `## Empty` closes at `## Full` (same level): nothing assigned
        // lives in its range, so it is dropped. `## Full` survives on
        // its own instruction, and a parent survives on a nested
        // section's instruction alone (the §9.2 example pins that half;
        // this pins the dropping half and the sibling boundary).
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                heading("h-empty", 2, "Empty"),
                instruction("u", "Unassigned."),
                heading("h-full", 2, "Full"),
                instruction("f", "Assigned text."),
            ],
            vec![assigned("f")],
        );

        assert_eq!(body_of(&snapshot, "p1"), "## Full\n\nAssigned text.");
    }

    #[test]
    fn a_section_closed_by_a_higher_level_heading_stops_there() {
        // ### Nested closes at the next heading with level <= 3, which
        // is #### Deeper? No: 4 > 3, so #### Deeper is INSIDE ###
        // Nested's range, and both are inside ## Top. The last H2
        // ## Other closes everything above it. With only the deepest
        // instruction assigned, three headings survive and the final
        // section does not.
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                heading("h-top", 2, "Top"),
                heading("h-nested", 3, "Nested"),
                heading("h-deeper", 4, "Deeper"),
                instruction("deep", "Deep text."),
                heading("h-other", 2, "Other"),
                instruction("other", "Other text."),
            ],
            vec![assigned("deep")],
        );

        assert_eq!(
            body_of(&snapshot, "p1"),
            "## Top\n\n### Nested\n\n#### Deeper\n\nDeep text."
        );
    }

    #[test]
    fn a_skipped_heading_level_still_nests_inside_its_parent() {
        // ## then #### skips a level (allowed, MAIN-015): #### Deep is
        // inside ## Parent's range because its level 4 > 2, so the
        // parent is kept by the deep instruction alone.
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                heading("h-parent", 2, "Parent"),
                heading("h-deep", 4, "Deep"),
                instruction("deep", "Deep text."),
            ],
            vec![assigned("deep")],
        );

        assert_eq!(
            body_of(&snapshot, "p1"),
            "## Parent\n\n#### Deep\n\nDeep text."
        );
    }

    // ---- ALG-021: preamble and ownership --------------------------------

    #[test]
    fn instructions_before_the_first_heading_open_the_body_without_a_heading() {
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                instruction("intro", "Intro text."),
                heading("h-later", 2, "Later"),
                instruction("later", "Later text."),
            ],
            vec![assigned("intro"), assigned("later")],
        );

        assert_eq!(
            body_of(&snapshot, "p1"),
            "Intro text.\n\n## Later\n\nLater text."
        );
    }

    // ---- DB-013 / D-11: hidden instructions ------------------------------

    #[test]
    fn a_hidden_instruction_neither_builds_a_block_nor_keeps_a_heading() {
        // The hidden instruction IS assigned (the snapshot accepts the
        // assignment, D-11), but its section heading is dropped: the
        // §9.2 `included` set requires the active state, so nothing in
        // `## Buried`'s range is included. The active sibling section
        // builds normally.
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                heading("h-buried", 2, "Buried"),
                instruction_with_state("x", "Hidden text.", InstructionState::Hidden),
                heading("h-live", 2, "Live"),
                instruction("y", "Live text."),
            ],
            vec![assigned("x"), assigned("y")],
        );

        assert_eq!(body_of(&snapshot, "p1"), "## Live\n\nLive text.");
    }

    #[test]
    fn a_project_of_only_hidden_assignments_builds_an_empty_body() {
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                heading("h", 2, "Hidden work"),
                instruction_with_state("x", "Hidden text.", InstructionState::Hidden),
            ],
            vec![assigned("x")],
        );

        assert_eq!(body_of(&snapshot, "p1"), "");
    }

    // ---- D-13: the empty variant -----------------------------------------

    #[test]
    fn an_empty_variant_keeps_the_heading_but_writes_no_block() {
        // D-13: an empty variant "outputs no text. Counts as assigned
        // for the purpose of including headings."
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                heading("h-kept", 2, "Kept"),
                instruction("empty", ""),
                heading("h-next", 2, "Next"),
                instruction("full", "Full text."),
            ],
            vec![assigned("empty"), assigned("full")],
        );

        // No empty line pair for the empty variant: its block simply
        // does not exist.
        assert_eq!(body_of(&snapshot, "p1"), "## Kept\n\n## Next\n\nFull text.");
    }

    #[test]
    fn a_variant_that_normalizes_to_empty_also_keeps_the_heading() {
        // Edge-whitespace-only content normalizes to the empty string
        // (ALG-002); D-13's rule reads the normalized text.
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                heading("h-kept", 2, "Kept"),
                instruction("blank", "\r\n\u{feff}\n  \n\n"),
            ],
            vec![assigned("blank")],
        );

        assert_eq!(body_of(&snapshot, "p1"), "## Kept");
    }

    // ---- ALG-022: names and separators ------------------------------------

    #[test]
    fn instruction_and_variant_names_never_enter_the_body() {
        let snapshot = snapshot(
            vec![project("p1")],
            vec![instruction("i1", "Only the text.")],
            vec![assigned("i1")],
        );

        let body = body_of(&snapshot, "p1");
        assert_eq!(body, "Only the text.");
        assert!(!body.contains("Instruction title i1"));
        assert!(!body.contains("Variant name i1"));
    }

    #[test]
    fn blocks_are_separated_by_exactly_one_empty_line_and_nothing_trails() {
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                instruction("one", "One."),
                instruction("two", "Two."),
                instruction("three", "Three."),
            ],
            vec![assigned("one"), assigned("two"), assigned("three")],
        );

        let body = body_of(&snapshot, "p1");
        assert_eq!(body, "One.\n\nTwo.\n\nThree.");
        assert!(!body.ends_with('\n'));
        // Exactly one empty line: no "\n\n\n" anywhere.
        assert!(!body.contains("\n\n\n"));
    }

    // ---- OUT-001 / LIB-035 / DoD 5: order ---------------------------------

    #[test]
    fn the_library_order_wins_over_the_assignment_list_order() {
        // The assignments arrive in reverse; the body still follows the
        // library's position order (the snapshot's item order).
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                instruction("first", "First text."),
                instruction("second", "Second text."),
                instruction("third", "Third text."),
            ],
            vec![assigned("third"), assigned("first"), assigned("second")],
        );

        assert_eq!(
            body_of(&snapshot, "p1"),
            "First text.\n\nSecond text.\n\nThird text."
        );
    }

    // ---- ALG-002 / D-044: normalization ------------------------------------

    #[test]
    fn variant_text_is_normalized_before_it_enters_the_body() {
        // Raw stored text: a BOM, Windows line breaks, leading empty
        // lines and trailing whitespace — none of it reaches the file.
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                instruction("raw", "\u{feff}\r\n\r\nLine one.\r\nLine two.\r\n\n\r\n   "),
                heading("h", 2, "After"),
                instruction("clean", "Clean."),
            ],
            vec![assigned("raw"), assigned("clean")],
        );

        assert_eq!(
            body_of(&snapshot, "p1"),
            "Line one.\nLine two.\n\n## After\n\nClean."
        );
    }

    // ---- boundaries --------------------------------------------------------

    #[test]
    fn a_project_without_assignments_builds_the_empty_body() {
        // Some(""): a legitimate empty build (spec D-25's confirmation
        // case), not the absence of a project.
        let snapshot = snapshot(
            vec![project("p1")],
            vec![heading("h", 2, "Nothing here"), instruction("i", "Text.")],
            Vec::new(),
        );

        assert_eq!(body_of(&snapshot, "p1"), "");
    }

    #[test]
    fn an_unknown_project_builds_nothing() {
        let snapshot = snapshot(
            vec![project("p1")],
            vec![instruction("i1", "Text.")],
            vec![assigned("i1")],
        );

        let missing = ProjectId::new("p-missing".to_owned());
        assert_eq!(build_body(&snapshot, &missing), None);
    }

    #[test]
    fn an_empty_snapshot_builds_nothing_for_any_project() {
        let snapshot = snapshot(Vec::new(), Vec::new(), Vec::new());
        let any = ProjectId::new("p1".to_owned());
        assert_eq!(build_body(&snapshot, &any), None);
    }

    // ---- per-project scoping ------------------------------------------------

    #[test]
    fn two_projects_each_build_their_own_selected_variants() {
        // One instruction, two variants; each project picks another
        // one, and one project also gets an instruction the other does
        // not.
        let shared = instruction_with_variants("shared", &["P1 flavor.", "P2 flavor."]);
        let only_p2 = instruction("only-p2", "Only P2 text.");
        let snapshot = snapshot(
            vec![project("p1"), project("p2")],
            vec![shared, only_p2],
            vec![
                assignment("p1", "shared", "v-shared-1"),
                assignment("p2", "shared", "v-shared-2"),
                assignment("p2", "only-p2", "v-only-p2"),
            ],
        );

        assert_eq!(body_of(&snapshot, "p1"), "P1 flavor.");
        assert_eq!(body_of(&snapshot, "p2"), "P2 flavor.\n\nOnly P2 text.");
    }

    // ---- the combined scenario ------------------------------------------------

    #[test]
    fn the_body_is_the_same_for_every_scheme_and_setting() {
        // The §9.2 body ignores the write settings entirely (the module
        // boundary): three projects differing in every settings field —
        // scheme A/B/C, base name, both extra files enabled at both
        // positions — build byte-identical bodies from one library.
        // A future refactor that wires settings into the body fails
        // here even if every other test still passes.
        let settings = |scheme: Scheme,
                        base_name: BaseName,
                        local: ExtraFilePosition,
                        project: ExtraFilePosition| {
            ProjectWriteSettings {
                scheme,
                base_name,
                extra_local: ExtraFile {
                    enabled: true,
                    position: local,
                },
                extra_project: ExtraFile {
                    enabled: true,
                    position: project,
                },
            }
        };
        let mut pa = project("pa");
        pa.settings = settings(
            Scheme::A,
            BaseName::AgentsMd,
            ExtraFilePosition::Start,
            ExtraFilePosition::Start,
        );
        let mut pb = project("pb");
        pb.settings = settings(
            Scheme::B,
            BaseName::AgentsMd,
            ExtraFilePosition::End,
            ExtraFilePosition::End,
        );
        let mut pc = project("pc");
        pc.settings = settings(
            Scheme::C,
            BaseName::ClaudeMd,
            ExtraFilePosition::Start,
            ExtraFilePosition::End,
        );

        let snapshot = snapshot(
            vec![pa, pb, pc],
            vec![heading("h", 2, "Shared"), instruction("i1", "Same text.")],
            vec![
                assignment("pa", "i1", "v-i1"),
                assignment("pb", "i1", "v-i1"),
                assignment("pc", "i1", "v-i1"),
            ],
        );

        let expected = "## Shared\n\nSame text.";
        assert_eq!(body_of(&snapshot, "pa"), expected);
        assert_eq!(body_of(&snapshot, "pb"), expected);
        assert_eq!(body_of(&snapshot, "pc"), expected);
    }

    #[test]
    fn a_combined_scenario_builds_every_rule_into_one_body() {
        // Library:
        //   (preamble) P     assigned
        //   ## S1        A   assigned, raw text needing normalization
        //   ## S2        H   hidden but assigned — keeps no heading
        //   ### S2a      E   assigned, empty variant — keeps S2 and S2a
        //   ## S3        U   unassigned — S3 dropped entirely
        //   (after S3)  Z   unassigned
        let snapshot = snapshot(
            vec![project("p1")],
            vec![
                instruction("p", "Preamble text."),
                heading("h-s1", 2, "S1"),
                instruction("a", "\r\nS1 text.\r\n\r\n"),
                heading("h-s2", 2, "S2"),
                instruction_with_state("h", "Hidden.", InstructionState::Hidden),
                heading("h-s2a", 3, "S2a"),
                instruction("e", ""),
                heading("h-s3", 2, "S3"),
                instruction("u", "Unassigned."),
                instruction("z", "Also unassigned."),
            ],
            vec![assigned("p"), assigned("a"), assigned("h"), assigned("e")],
        );

        // S2 is kept by E: an empty variant counts as assigned for
        // headings (D-13), and E sits inside S2's range through the
        // nested S2a. S3 and both unassigned instructions are gone, H
        // left no trace, A's text arrived normalized.
        assert_eq!(
            body_of(&snapshot, "p1"),
            "Preamble text.\n\n## S1\n\nS1 text.\n\n## S2\n\n### S2a"
        );
    }
}
