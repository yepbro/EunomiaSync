//! The immutable build snapshot (spec §9.1, ALG-001).
//!
//! A sync builds every project from one immutable snapshot read in a
//! single read transaction (SYS-005, ALG-001): the active projects in
//! `sort_order` order, all library items in `position` order with
//! their active variants, and the assignments. The snapshot is the
//! only data the build sees — this crate touches neither the database
//! nor the file system, so the store layer assembles the value and
//! hands it over.
//!
//! Ordering is carried by collection order, not by repeated numbers:
//! DB-005 and DB-011 make library-item `position` a gapless `1..=N`
//! row, so the item vector preserves the 1-based position order
//! (`index + 1` is the position); the projects keep whatever order the
//! store's `ORDER BY sort_order` produced. No second copy of the same
//! fact is stored, because a copy could only drift.
//!
//! [`BuildSnapshot::new`] validates the whole graph once, at the
//! boundary, so the build code downstream can be a total function:
//! every assignment resolves to a known project, a known instruction
//! and one of that instruction's variants — no lookup can miss.

use std::collections::HashMap;
use std::path::PathBuf;

use thiserror::Error;

use crate::assignment::{Assignment, AssignmentTarget};
use crate::library::{ItemId, LibraryItem, VariantId};
use crate::project::{ProjectId, ProjectWriteSettings};

/// One active project as the build sees it (spec ALG-001).
///
/// Only build-relevant fields: identity (sync-run reporting and the
/// assignment join), the human name (sync summaries and the D-25
/// confirmation list), the repository root (target paths derive from
/// it: `<root>/<base_name>` or `<root>/<STEM>.project.md`), and the
/// write settings. UI-only attributes (label, color) and
/// housekeeping columns (timestamps, last-synced hash) are
/// deliberately absent — the snapshot is a build input, not a project
/// listing. Archived projects never appear in a snapshot, so there is
/// no state field to check.
///
/// Carrying a [`PathBuf`] is a value, not file-system access: this
/// crate never opens it.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use eunomia_core::project::{
///     BaseName, ExtraFile, ExtraFilePosition, ProjectId, ProjectWriteSettings, Scheme,
/// };
/// use eunomia_core::snapshot::SnapshotProject;
///
/// let project = SnapshotProject {
///     id: ProjectId::new("p1".to_owned()),
///     name: "demo".to_owned(),
///     root: Path::new("/repos/demo").to_owned(),
///     settings: ProjectWriteSettings {
///         scheme: Scheme::A,
///         base_name: BaseName::AgentsMd,
///         extra_local: ExtraFile { enabled: false, position: ExtraFilePosition::End },
///         extra_project: ExtraFile { enabled: false, position: ExtraFilePosition::End },
///     },
/// };
/// assert_eq!(project.name, "demo");
/// assert_eq!(project.settings.scheme, Scheme::A);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotProject {
    /// The project's identifier.
    pub id: ProjectId,
    /// The project's display name.
    pub name: String,
    /// The project's repository root, in canonical form (DB-030).
    pub root: PathBuf,
    /// The project's write settings.
    pub settings: ProjectWriteSettings,
}

/// A violation found while assembling a [`BuildSnapshot`].
///
/// The schema's constraints (§6.2) and the store's invariants already
/// guarantee most of these facts for a healthy database; the checks
/// exist because the snapshot is assembled by hand-written queries,
/// and a wrong filter or join there must surface here — at sync start,
/// with a named cause — instead of as a mysterious write later in the
/// pipeline.
///
/// # Examples
///
/// ```
/// use eunomia_core::library::{ItemId, VariantId};
/// use eunomia_core::snapshot::SnapshotError;
///
/// let error = SnapshotError::VariantMismatch {
///     item_id: ItemId::new("i1".to_owned()),
///     variant_id: VariantId::new("v2".to_owned()),
/// };
/// assert_eq!(
///     error.to_string(),
///     "variant v2 is not an active variant of instruction i1"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SnapshotError {
    /// An assignment references a project missing from the snapshot
    /// (the assembly must scope assignments to the snapshot's active
    /// projects).
    #[error("assignment references project {project_id} missing from the snapshot")]
    UnknownProject {
        /// The missing project.
        project_id: ProjectId,
    },
    /// An assignment references a library item missing from the
    /// snapshot.
    #[error("assignment references library item {item_id} missing from the snapshot")]
    UnknownInstruction {
        /// The missing item.
        item_id: ItemId,
    },
    /// An assignment targets a heading; only instructions can be
    /// assigned (§2).
    #[error("assignment targets heading {item_id}; headings cannot be assigned")]
    AssignmentToHeading {
        /// The heading item.
        item_id: ItemId,
    },
    /// The referenced variant — a variant target or an extend cell's base —
    /// is not an active variant of the referenced instruction (DB-012).
    #[error("variant {variant_id} is not an active variant of instruction {item_id}")]
    VariantMismatch {
        /// The assigned instruction.
        item_id: ItemId,
        /// The variant that does not belong to it.
        variant_id: VariantId,
    },
    /// A project already has an assignment for the instruction
    /// (D-02: one instruction, at most one content target per project).
    #[error("project {project_id} already has an assignment for instruction {item_id}")]
    DuplicateAssignment {
        /// The project with the duplicate.
        project_id: ProjectId,
        /// The instruction assigned twice.
        item_id: ItemId,
    },
    /// Two snapshot projects share one identifier.
    #[error("duplicate project id {project_id}")]
    DuplicateProjectId {
        /// The duplicated identifier.
        project_id: ProjectId,
    },
    /// Two library items share one identifier.
    #[error("duplicate library item id {item_id}")]
    DuplicateItemId {
        /// The duplicated identifier.
        item_id: ItemId,
    },
    /// An active instruction carries no active variants (DB-010).
    #[error("active instruction {item_id} has no active variants")]
    NoActiveVariants {
        /// The instruction without variants.
        item_id: ItemId,
    },
}

/// The immutable data snapshot a sync builds from (spec §9.1, ALG-001).
///
/// The value holds exactly what ALG-001 names, in domain form:
///
/// * the **active** projects, in `sort_order` order (archived projects
///   and their assignments are filtered out at assembly — DB-014);
/// * **all** library items in `position` order, hidden instructions
///   and headings included (hidden instructions keep their
///   assignments, D-11; the build excludes them per ALG-020);
/// * the **active** variants of every instruction (soft-deleted
///   variants never reach a build);
/// * the assignments of the snapshot's projects.
///
/// Variant texts are carried as stored; normalization (ALG-002) is a
/// build-pipeline step applied downstream, not a snapshot concern.
///
/// # Examples
///
/// A minimal library tree — one heading, one instruction with one
/// variant, one assigned project:
///
/// ```
/// use std::path::Path;
///
/// use eunomia_core::assignment::{Assignment, AssignmentTarget};
/// use eunomia_core::library::{
///     Heading, HeadingLevel, Instruction, InstructionState, ItemId, LibraryItem, Variant,
///     VariantId,
/// };
/// use eunomia_core::project::{
///     BaseName, ExtraFile, ExtraFilePosition, ProjectId, ProjectWriteSettings, Scheme,
/// };
/// use eunomia_core::snapshot::{BuildSnapshot, SnapshotProject};
///
/// let project = SnapshotProject {
///     id: ProjectId::new("p1".to_owned()),
///     name: "demo".to_owned(),
///     root: Path::new("/repos/demo").to_owned(),
///     settings: ProjectWriteSettings {
///         scheme: Scheme::A,
///         base_name: BaseName::AgentsMd,
///         extra_local: ExtraFile { enabled: false, position: ExtraFilePosition::End },
///         extra_project: ExtraFile { enabled: false, position: ExtraFilePosition::End },
///     },
/// };
/// let style = LibraryItem::Heading(Heading {
///     id: ItemId::new("h1".to_owned()),
///     level: HeadingLevel::H2,
///     title: "Style".to_owned(),
/// });
/// let concise = LibraryItem::Instruction(Instruction {
///     id: ItemId::new("i1".to_owned()),
///     title: "Be concise".to_owned(),
///     state: InstructionState::Active,
///     variants: vec![Variant {
///         id: VariantId::new("v1".to_owned()),
///         name: "Main".to_owned(),
///         content: "Be brief.".to_owned(),
///     }],
/// });
/// let assignment = Assignment {
///     project_id: project.id.clone(),
///     instruction_id: ItemId::new("i1".to_owned()),
///     target: AssignmentTarget::Variant(VariantId::new("v1".to_owned())),
/// };
///
/// let snapshot = BuildSnapshot::new(
///     vec![project],
///     vec![style, concise],
///     vec![assignment],
/// )
/// .expect("the minimal tree is valid");
///
/// assert_eq!(snapshot.projects().len(), 1);
/// assert_eq!(snapshot.items().len(), 2);
/// assert_eq!(snapshot.assignments().len(), 1);
/// // The items keep the library order they were handed over in:
/// assert!(matches!(snapshot.items()[0], LibraryItem::Heading(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildSnapshot {
    projects: Vec<SnapshotProject>,
    items: Vec<LibraryItem>,
    assignments: Vec<Assignment>,
}

impl BuildSnapshot {
    /// Assembles and validates a snapshot from the three collections
    /// of ALG-001.
    ///
    /// # Errors
    ///
    /// Returns the first violation found, in collection order:
    ///
    /// * duplicate project or library-item identifiers
    ///   ([`SnapshotError::DuplicateProjectId`],
    ///   [`SnapshotError::DuplicateItemId`]);
    /// * an active instruction without active variants
    ///   ([`SnapshotError::NoActiveVariants`], DB-010 — hidden
    ///   instructions are not held to it, mirroring the invariant's
    ///   wording);
    /// * per assignment, in order: an unknown project
    ///   ([`SnapshotError::UnknownProject`]), an unknown item
    ///   ([`SnapshotError::UnknownInstruction`]), a heading where an
    ///   instruction is required
    ///   ([`SnapshotError::AssignmentToHeading`]), a second assignment
    ///   of the same instruction for one project
    ///   ([`SnapshotError::DuplicateAssignment`], D-02), or a variant —
    ///   a variant target or an extend cell's base (MAIN-034) — that is
    ///   not one of the instruction's active variants
    ///   ([`SnapshotError::VariantMismatch`], DB-012).
    ///
    /// Assignments to hidden instructions are valid (D-11, DB-013).
    /// An empty snapshot — no projects, no items — is valid (§9.4:
    /// a run with no active projects succeeds).
    pub fn new(
        projects: Vec<SnapshotProject>,
        items: Vec<LibraryItem>,
        assignments: Vec<Assignment>,
    ) -> Result<Self, SnapshotError> {
        use std::collections::HashSet;
        use std::collections::hash_map::Entry;

        let mut project_ids = HashSet::with_capacity(projects.len());
        for project in &projects {
            if !project_ids.insert(project.id.clone()) {
                return Err(SnapshotError::DuplicateProjectId {
                    project_id: project.id.clone(),
                });
            }
        }

        let mut items_by_id = HashMap::with_capacity(items.len());
        for item in &items {
            match items_by_id.entry(item.item_id().clone()) {
                Entry::Vacant(slot) => {
                    slot.insert(item);
                }
                Entry::Occupied(occupied) => {
                    return Err(SnapshotError::DuplicateItemId {
                        item_id: occupied.key().clone(),
                    });
                }
            }
        }

        for item in &items {
            if let LibraryItem::Instruction(instruction) = item
                && instruction.is_active()
                && instruction.variants.is_empty()
            {
                return Err(SnapshotError::NoActiveVariants {
                    item_id: instruction.id.clone(),
                });
            }
        }

        let mut assigned_pairs = HashSet::with_capacity(assignments.len());
        for assignment in &assignments {
            if !project_ids.contains(&assignment.project_id) {
                return Err(SnapshotError::UnknownProject {
                    project_id: assignment.project_id.clone(),
                });
            }
            let Some(item) = items_by_id.get(&assignment.instruction_id) else {
                return Err(SnapshotError::UnknownInstruction {
                    item_id: assignment.instruction_id.clone(),
                });
            };
            let LibraryItem::Instruction(instruction) = item else {
                return Err(SnapshotError::AssignmentToHeading {
                    item_id: assignment.instruction_id.clone(),
                });
            };
            if !assigned_pairs.insert((
                assignment.project_id.clone(),
                assignment.instruction_id.clone(),
            )) {
                return Err(SnapshotError::DuplicateAssignment {
                    project_id: assignment.project_id.clone(),
                    item_id: assignment.instruction_id.clone(),
                });
            }
            if let Some(variant_id) = match &assignment.target {
                // DB-012: a variant target — alone (MAIN-001) or as an
                // extend cell's base (MAIN-034) — must be one of the
                // instruction's active variants.
                AssignmentTarget::Variant(variant_id) => Some(variant_id),
                AssignmentTarget::Extend { base, .. } => Some(base),
                AssignmentTarget::Custom(_) => None,
            } && !instruction.variants.iter().any(|v| v.id == *variant_id)
            {
                return Err(SnapshotError::VariantMismatch {
                    item_id: assignment.instruction_id.clone(),
                    variant_id: variant_id.clone(),
                });
            }
        }

        Ok(Self {
            projects,
            items,
            assignments,
        })
    }

    /// The active projects, in `sort_order` order (ALG-001).
    #[must_use]
    pub fn projects(&self) -> &[SnapshotProject] {
        &self.projects
    }

    /// All library items, in `position` order — hidden instructions
    /// and headings included (ALG-001).
    #[must_use]
    pub fn items(&self) -> &[LibraryItem] {
        &self.items
    }

    /// The assignments of the snapshot's projects.
    #[must_use]
    pub fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::library::{Heading, HeadingLevel, Instruction, InstructionState, Variant};

    /// A minimal single-project fixture: one heading with one assigned
    /// instruction below it.
    struct Fixture {
        project: SnapshotProject,
        style: LibraryItem,
        concise: LibraryItem,
        assignment: Assignment,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                project: SnapshotProject {
                    id: ProjectId::new("p1".to_owned()),
                    name: "demo".to_owned(),
                    root: Path::new("/repos/demo").to_owned(),
                    settings: write_settings(),
                },
                style: LibraryItem::Heading(Heading {
                    id: ItemId::new("h1".to_owned()),
                    level: HeadingLevel::H2,
                    title: "Style".to_owned(),
                }),
                concise: LibraryItem::Instruction(Instruction {
                    id: ItemId::new("i1".to_owned()),
                    title: "Be concise".to_owned(),
                    state: InstructionState::Active,
                    variants: vec![Variant {
                        id: VariantId::new("v1".to_owned()),
                        name: "Main".to_owned(),
                        content: "Be brief.".to_owned(),
                    }],
                }),
                assignment: Assignment {
                    project_id: ProjectId::new("p1".to_owned()),
                    instruction_id: ItemId::new("i1".to_owned()),
                    target: AssignmentTarget::Variant(VariantId::new("v1".to_owned())),
                },
            }
        }
    }

    /// A canonical scheme-A test value (not a §6.5 default — those
    /// belong to the store's settings repository, D-039).
    fn write_settings() -> ProjectWriteSettings {
        use crate::project::{BaseName, ExtraFile, ExtraFilePosition, Scheme};
        ProjectWriteSettings {
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
        }
    }

    #[test]
    fn new_accepts_the_minimal_tree_and_preserves_order() {
        let fixture = Fixture::new();
        let second_project = SnapshotProject {
            id: ProjectId::new("p2".to_owned()),
            name: "second".to_owned(),
            root: Path::new("/repos/second").to_owned(),
            settings: write_settings(),
        };
        let second_assignment = Assignment {
            project_id: second_project.id.clone(),
            instruction_id: ItemId::new("i1".to_owned()),
            target: AssignmentTarget::Variant(VariantId::new("v1".to_owned())),
        };

        let snapshot = BuildSnapshot::new(
            vec![fixture.project.clone(), second_project],
            vec![fixture.style.clone(), fixture.concise.clone()],
            vec![fixture.assignment.clone(), second_assignment],
        )
        .expect("the minimal tree is valid");

        assert_eq!(snapshot.projects().len(), 2);
        assert_eq!(snapshot.projects()[0].name, "demo");
        assert_eq!(snapshot.items().len(), 2);
        assert!(matches!(snapshot.items()[0], LibraryItem::Heading(_)));
        assert!(matches!(snapshot.items()[1], LibraryItem::Instruction(_)));
        assert_eq!(snapshot.assignments().len(), 2);
    }

    #[test]
    fn new_accepts_an_empty_snapshot() {
        let snapshot = BuildSnapshot::new(Vec::new(), Vec::new(), Vec::new())
            .expect("an empty snapshot is valid (§9.4)");
        assert!(snapshot.projects().is_empty());
        assert!(snapshot.items().is_empty());
        assert!(snapshot.assignments().is_empty());
    }

    #[test]
    fn new_rejects_an_assignment_to_an_unknown_project() {
        let fixture = Fixture::new();
        let mut assignment = fixture.assignment.clone();
        assignment.project_id = ProjectId::new("p-missing".to_owned());
        let error = BuildSnapshot::new(
            vec![fixture.project],
            vec![fixture.style, fixture.concise],
            vec![assignment.clone()],
        )
        .unwrap_err();
        assert_eq!(
            error,
            SnapshotError::UnknownProject {
                project_id: ProjectId::new("p-missing".to_owned())
            }
        );
    }

    #[test]
    fn new_rejects_an_assignment_to_an_unknown_instruction() {
        let fixture = Fixture::new();
        let mut assignment = fixture.assignment.clone();
        assignment.instruction_id = ItemId::new("i-missing".to_owned());
        let error = BuildSnapshot::new(
            vec![fixture.project],
            vec![fixture.style, fixture.concise],
            vec![assignment],
        )
        .unwrap_err();
        assert_eq!(
            error,
            SnapshotError::UnknownInstruction {
                item_id: ItemId::new("i-missing".to_owned())
            }
        );
    }

    #[test]
    fn new_rejects_an_assignment_to_a_heading() {
        let fixture = Fixture::new();
        let mut assignment = fixture.assignment.clone();
        assignment.instruction_id = ItemId::new("h1".to_owned());
        let error = BuildSnapshot::new(
            vec![fixture.project],
            vec![fixture.style, fixture.concise],
            vec![assignment],
        )
        .unwrap_err();
        assert_eq!(
            error,
            SnapshotError::AssignmentToHeading {
                item_id: ItemId::new("h1".to_owned())
            }
        );
    }

    #[test]
    fn new_rejects_a_variant_of_another_instruction() {
        let fixture = Fixture::new();
        let other = LibraryItem::Instruction(Instruction {
            id: ItemId::new("i2".to_owned()),
            title: "Run tests".to_owned(),
            state: InstructionState::Active,
            variants: vec![Variant {
                id: VariantId::new("v2".to_owned()),
                name: "Main".to_owned(),
                content: "cargo test".to_owned(),
            }],
        });
        let mut assignment = fixture.assignment.clone();
        assignment.target = AssignmentTarget::Variant(VariantId::new("v2".to_owned()));
        let error = BuildSnapshot::new(
            vec![fixture.project],
            vec![fixture.style, fixture.concise, other],
            vec![assignment],
        )
        .unwrap_err();
        assert_eq!(
            error,
            SnapshotError::VariantMismatch {
                item_id: ItemId::new("i1".to_owned()),
                variant_id: VariantId::new("v2".to_owned()),
            }
        );
    }

    #[test]
    fn new_rejects_an_extend_base_that_is_not_the_instructions_variant() {
        // MAIN-034's DB-012 arm: the extend cell's base is held to the
        // same active-same-instruction rule as a variant target.
        let fixture = Fixture::new();
        let other = LibraryItem::Instruction(Instruction {
            id: ItemId::new("i2".to_owned()),
            title: "Run tests".to_owned(),
            state: InstructionState::Active,
            variants: vec![Variant {
                id: VariantId::new("v2".to_owned()),
                name: "Main".to_owned(),
                content: "cargo test".to_owned(),
            }],
        });
        let mut assignment = fixture.assignment.clone();
        assignment.target = AssignmentTarget::Extend {
            base: VariantId::new("v2".to_owned()),
            supplement: "supplement".to_owned(),
        };
        let error = BuildSnapshot::new(
            vec![fixture.project],
            vec![fixture.style, fixture.concise, other],
            vec![assignment],
        )
        .unwrap_err();
        assert_eq!(
            error,
            SnapshotError::VariantMismatch {
                item_id: ItemId::new("i1".to_owned()),
                variant_id: VariantId::new("v2".to_owned()),
            }
        );
    }

    #[test]
    fn new_accepts_an_extend_target_over_the_instructions_own_variant() {
        // The legal complement: the base resolves inside the same
        // instruction, the snapshot accepts the pair.
        let fixture = Fixture::new();
        let mut assignment = fixture.assignment.clone();
        assignment.target = AssignmentTarget::Extend {
            base: VariantId::new("v1".to_owned()),
            supplement: "supplement".to_owned(),
        };
        let snapshot = BuildSnapshot::new(
            vec![fixture.project],
            vec![fixture.style, fixture.concise],
            vec![assignment],
        )
        .expect("the extend pair over the instruction's own variant is valid");
        assert_eq!(snapshot.assignments().len(), 1);
    }

    #[test]
    fn new_rejects_two_assignments_of_one_instruction_in_one_project() {
        // D-02: one instruction has at most one assignment per project,
        // even when a second variant exists to pick.
        let fixture = Fixture::new();
        let concise_two_variants = LibraryItem::Instruction(Instruction {
            id: ItemId::new("i1".to_owned()),
            title: "Be concise".to_owned(),
            state: InstructionState::Active,
            variants: vec![
                Variant {
                    id: VariantId::new("v1".to_owned()),
                    name: "Main".to_owned(),
                    content: "Be brief.".to_owned(),
                },
                Variant {
                    id: VariantId::new("v2".to_owned()),
                    name: "Verbose".to_owned(),
                    content: "Be brief, but explain why.".to_owned(),
                },
            ],
        });
        let mut duplicate = fixture.assignment.clone();
        duplicate.target = AssignmentTarget::Variant(VariantId::new("v2".to_owned()));
        let error = BuildSnapshot::new(
            vec![fixture.project],
            vec![fixture.style, concise_two_variants],
            vec![fixture.assignment, duplicate],
        )
        .unwrap_err();
        assert_eq!(
            error,
            SnapshotError::DuplicateAssignment {
                project_id: ProjectId::new("p1".to_owned()),
                item_id: ItemId::new("i1".to_owned()),
            }
        );
    }

    #[test]
    fn new_allows_the_same_instruction_in_two_projects_and_two_variants_in_two_projects() {
        // The complement of the duplicate test: one instruction, two
        // variants, two projects — each project a different variant.
        let fixture = Fixture::new();
        let second_project = SnapshotProject {
            id: ProjectId::new("p2".to_owned()),
            name: "second".to_owned(),
            root: Path::new("/repos/second").to_owned(),
            settings: write_settings(),
        };
        let concise_two_variants = LibraryItem::Instruction(Instruction {
            id: ItemId::new("i1".to_owned()),
            title: "Be concise".to_owned(),
            state: InstructionState::Active,
            variants: vec![
                Variant {
                    id: VariantId::new("v1".to_owned()),
                    name: "Main".to_owned(),
                    content: "Be brief.".to_owned(),
                },
                Variant {
                    id: VariantId::new("v2".to_owned()),
                    name: "Verbose".to_owned(),
                    content: "Be brief, but explain why.".to_owned(),
                },
            ],
        });
        let second_assignment = Assignment {
            project_id: second_project.id.clone(),
            instruction_id: ItemId::new("i1".to_owned()),
            target: AssignmentTarget::Variant(VariantId::new("v2".to_owned())),
        };
        let snapshot = BuildSnapshot::new(
            vec![fixture.project, second_project],
            vec![fixture.style, concise_two_variants],
            vec![fixture.assignment, second_assignment],
        )
        .expect("different projects may pick different variants (D-02)");
        assert_eq!(snapshot.assignments().len(), 2);
    }

    #[test]
    fn new_rejects_duplicate_project_ids() {
        let fixture = Fixture::new();
        let twin = SnapshotProject {
            id: ProjectId::new("p1".to_owned()),
            name: "twin".to_owned(),
            root: Path::new("/repos/twin").to_owned(),
            settings: write_settings(),
        };
        let error = BuildSnapshot::new(vec![fixture.project, twin], vec![], vec![]).unwrap_err();
        assert_eq!(
            error,
            SnapshotError::DuplicateProjectId {
                project_id: ProjectId::new("p1".to_owned())
            }
        );
    }

    #[test]
    fn new_rejects_duplicate_item_ids() {
        let fixture = Fixture::new();
        let twin = LibraryItem::Heading(Heading {
            id: ItemId::new("h1".to_owned()),
            level: HeadingLevel::H3,
            title: "Twin".to_owned(),
        });
        let error = BuildSnapshot::new(vec![fixture.project], vec![fixture.style, twin], vec![])
            .unwrap_err();
        assert_eq!(
            error,
            SnapshotError::DuplicateItemId {
                item_id: ItemId::new("h1".to_owned())
            }
        );
    }

    #[test]
    fn new_rejects_an_active_instruction_without_variants() {
        let fixture = Fixture::new();
        let bare = LibraryItem::Instruction(Instruction {
            id: ItemId::new("i2".to_owned()),
            title: "No variants".to_owned(),
            state: InstructionState::Active,
            variants: Vec::new(),
        });
        let error = BuildSnapshot::new(
            vec![fixture.project],
            vec![fixture.style, fixture.concise, bare],
            vec![fixture.assignment],
        )
        .unwrap_err();
        assert_eq!(
            error,
            SnapshotError::NoActiveVariants {
                item_id: ItemId::new("i2".to_owned())
            }
        );
    }

    #[test]
    fn new_allows_a_hidden_instruction_without_variants() {
        // DB-010 requires variants for active instructions only; hidden
        // ones are held to the spec's letter, not more.
        let fixture = Fixture::new();
        let hidden = LibraryItem::Instruction(Instruction {
            id: ItemId::new("i2".to_owned()),
            title: "Hidden".to_owned(),
            state: InstructionState::Hidden,
            variants: Vec::new(),
        });
        let snapshot = BuildSnapshot::new(
            vec![fixture.project],
            vec![fixture.style, fixture.concise, hidden],
            vec![fixture.assignment],
        );
        assert!(snapshot.is_ok());
    }

    #[test]
    fn new_allows_an_assignment_to_a_hidden_instruction() {
        // D-11 / DB-013: hidden instructions keep their assignments.
        let fixture = Fixture::new();
        let hidden_concise = match fixture.concise.clone() {
            LibraryItem::Instruction(mut instruction) => {
                instruction.state = InstructionState::Hidden;
                LibraryItem::Instruction(instruction)
            }
            other => other,
        };
        let snapshot = BuildSnapshot::new(
            vec![fixture.project],
            vec![fixture.style, hidden_concise],
            vec![fixture.assignment],
        );
        assert!(snapshot.is_ok());
    }
}
