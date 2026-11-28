//! The build data snapshot read (spec §9.1 ALG-001, §5.4 SYS-005;
//! task T-100).
//!
//! A sync builds every project from one immutable snapshot of the
//! library and project data, and SYS-005 pins the consistency rule:
//! the snapshot is read in **one read transaction**, so library edits
//! committed while a sync runs do not leak into it — they stay
//! "dirty" for the next run. This module is that read: it composes
//! the repositories' reads inside the caller's transaction and maps
//! the TEXT rows into the domain types of
//! [`eunomia_core::snapshot::BuildSnapshot`] — the value the build
//! (and the DB-020 hash that feeds the dirty flag) works from.
//!
//! # One transaction, joined — not owned
//!
//! Like every repository function (D-037/D-051), [`read`] takes
//! `&Connection` and joins **the caller's** transaction: the
//! app-layer executor wraps one operation in one `BEGIN DEFERRED`, so
//! the production composition has the shape
//! `Db::run(|tx| build_snapshot::read(tx))` (the command maps
//! [`BuildSnapshotError`] into its own error type at that boundary —
//! the store stays executor-agnostic), and the four SELECTs below
//! share one SQLite read transaction. WAL
//! mode (DB-001, applied by [`crate::storage::open_db`]) pins that
//! transaction's view of the database at its **first** read
//! statement: whatever commits afterwards — through this or any other
//! connection — is invisible until the transaction ends. That is the
//! whole SYS-005 guarantee, and the isolation test pins it
//! deterministically (a second connection commits an edit between two
//! full snapshot reads inside one still-open transaction; the second
//! read returns the first read's value verbatim).
//!
//! The corollary is a calling contract, documented here because the
//! type system cannot carry it: called **outside** a transaction,
//! each SELECT would run in its own implicit read transaction and the
//! four reads could disagree — a torn snapshot. The store's
//! integration suite calls [`read`] bare only where no write can
//! interleave (a single-connection test database); the sync command
//! composes it inside `Db::run`.
//!
//! # The mapping layer
//!
//! The repositories stay TEXT in/out with store-local vocabularies
//! (D-017/D-093/D-097); this module is the crate's single place that
//! speaks `eunomia-core`. It wires the dependency edge D-017 reserved
//! for the first store consumer of core types. The row→domain
//! mappings are total on the closed vocabularies (each store member
//! has exactly one core member; a unit test pins the literals in both
//! directions), and [`BuildSnapshot::new`] re-validates the assembled
//! graph at the boundary — an active instruction without active
//! variants (DB-010), an assignment that does not resolve, a
//! duplicate id — so the build downstream is a total function over
//! the value.
//!
//! What the snapshot holds, per ALG-001:
//!
//! * the **active** projects, in `sort_order` order (archived
//!   projects and their assignments are filtered out — DB-014);
//! * **all** library items in `position` order — hidden instructions
//!   and headings included (hidden instructions keep their
//!   assignments, D-11/DB-013; the build excludes them per ALG-020);
//! * the **active** variants of every instruction, each
//!   instruction's variants in creation order (soft-deleted variants
//!   never reach a build);
//! * the assignments of the active projects, ordered by project
//!   `sort_order`, then by the assigned instruction's `position` — each
//!   one of the three §6.2 target shapes (a variant, MAIN-007's replace
//!   text, or MAIN-034's extend pair; the extend base rides the same
//!   active-variant validation a variant target gets, and the build
//!   reads its CURRENT text at build time, ALG-023).
//!
//! Variant texts ride the snapshot **as stored**: the save-time
//! CRLF→LF fold already happened at the write boundary (D-076/D-097),
//! and the ALG-002 normalization is a build-pipeline step, not a
//! snapshot concern.
//!
//! The `project_build_preview` command (MAIN-033, §7.10 — T-358) is
//! the read-side consumer of exactly this value: the next sync's
//! managed content is `build_snapshot::read` +
//! `eunomia_core::body::build_body` + the §9.3 scheme assembly in one
//! read transaction, and the extend composition already flows through
//! both halves — no new build path was needed for it.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use rusqlite::Connection;

use eunomia_core::assignment::{Assignment as CoreAssignment, AssignmentTarget as CoreTarget};
use eunomia_core::library::Heading as CoreHeading;
use eunomia_core::library::HeadingLevel;
use eunomia_core::library::Instruction as CoreInstruction;
use eunomia_core::library::InstructionState as CoreInstructionState;
use eunomia_core::library::InvalidHeadingLevel;
use eunomia_core::library::ItemId;
use eunomia_core::library::LibraryItem as CoreLibraryItem;
use eunomia_core::library::Variant as CoreVariant;
use eunomia_core::library::VariantId;
use eunomia_core::project::BaseName as CoreBaseName;
use eunomia_core::project::ExtraFile as CoreExtraFile;
use eunomia_core::project::ExtraFilePosition;
use eunomia_core::project::ProjectId;
use eunomia_core::project::ProjectWriteSettings;
use eunomia_core::project::Scheme as CoreScheme;
use eunomia_core::snapshot::{BuildSnapshot, SnapshotError, SnapshotProject};

use crate::library::{self, ItemKind, ItemState, LibraryError};
use crate::projects::{self, ListScope, ProjectsError};
use crate::settings::{BaseName, ExtraFile, Position, Scheme};

/// Why a snapshot read failed.
///
/// Returned directly (the D-093/D-097 boundary: no
/// [`crate::storage::StorageError`] nesting, hand-rolled impls — no
/// `thiserror` enters this crate with the core edge, which is
/// path-only). The first two variants are the repositories' own read
/// failures — a corrupt vocabulary cell (D-039) or a SQL error;
/// [`BuildSnapshotError::Invalid`] is core's assembly validation
/// (DB-010, DB-012, D-02) firing on the assembled graph.
#[derive(Debug)]
pub enum BuildSnapshotError {
    /// Reading the projects repository failed.
    Projects(ProjectsError),
    /// Reading the library repository failed.
    Library(LibraryError),
    /// A heading row reached the mapping with a level outside `2..=6`
    /// — unreachable through [`crate::library`]'s decoder (it
    /// validates the same §6.2 matrix on read), so this names a
    /// decoder/map drift, not a data state.
    HeadingLevel(InvalidHeadingLevel),
    /// The assembled graph failed [`BuildSnapshot::new`]'s
    /// validation: an active instruction without active variants
    /// (DB-010), an assignment that does not resolve to a snapshot
    /// project/instruction or whose variant target is not active and local
    /// (DB-012; custom targets deliberately have no variant), a second
    /// assignment of one instruction for one project (D-02), or a
    /// duplicate id. Surfaced at sync start with a named cause
    /// instead of a mysterious write later in the pipeline.
    Invalid(SnapshotError),
}

impl fmt::Display for BuildSnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildSnapshotError::Projects(err) => write!(f, "reading projects failed: {err}"),
            BuildSnapshotError::Library(err) => write!(f, "reading the library failed: {err}"),
            BuildSnapshotError::HeadingLevel(err) => write!(f, "reading a heading failed: {err}"),
            BuildSnapshotError::Invalid(err) => {
                write!(f, "the assembled snapshot is inconsistent: {err}")
            }
        }
    }
}

impl std::error::Error for BuildSnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BuildSnapshotError::Projects(err) => Some(err),
            BuildSnapshotError::Library(err) => Some(err),
            BuildSnapshotError::HeadingLevel(err) => Some(err),
            BuildSnapshotError::Invalid(err) => Some(err),
        }
    }
}

/// Reads the immutable build snapshot (spec §9.1 ALG-001) in the
/// caller's transaction: the active projects in `sort_order` order,
/// all library items in `position` order with their active variants,
/// and the assignments of the active projects — returned as core's
/// validated [`BuildSnapshot`].
///
/// Joins the caller's transaction like every repository function
/// (D-037/D-051): call this inside one transaction for the SYS-005
/// guarantee — see the module docs for the WAL mechanics and the
/// calling contract. Inside the transaction this function only reads;
/// it writes nothing, journals nothing (D-051's producer rule), and
/// normalizes nothing (ALG-002 belongs to the build pipeline).
///
/// # Errors
///
/// Returns [`BuildSnapshotError::Projects`] or
/// [`BuildSnapshotError::Library`] when a repository read fails (a
/// corrupt vocabulary cell surfaces as its `Member` error, D-039),
/// and [`BuildSnapshotError::Invalid`] when the assembled graph fails
/// core's validation — e.g. an active instruction whose active
/// variants are missing (DB-010): a bare `create_item` committed
/// without its main variant is exactly that state, so it is surfaced
/// here rather than guessed around.
pub fn read(conn: &Connection) -> Result<BuildSnapshot, BuildSnapshotError> {
    let projects = projects::list(conn, ListScope::Active).map_err(BuildSnapshotError::Projects)?;
    let items =
        library::list_items(conn, library::ItemScope::All).map_err(BuildSnapshotError::Library)?;
    let variants = library::list_active_variants(conn).map_err(BuildSnapshotError::Library)?;
    let assignments =
        library::list_assignments_for_active_projects(conn).map_err(BuildSnapshotError::Library)?;

    // Variants arrive grouped per instruction (list_active_variants
    // orders by instruction_id first); regrouping into the map keeps
    // the per-instruction creation order and lets the item walk below
    // take each instruction's vector by value.
    let mut variants_by_instruction: HashMap<String, Vec<CoreVariant>> =
        HashMap::with_capacity(items.len());
    for variant in variants {
        variants_by_instruction
            .entry(variant.instruction_id)
            .or_default()
            .push(CoreVariant {
                id: VariantId::new(variant.id),
                name: variant.name,
                content: variant.content,
            });
    }

    let snapshot_projects = projects
        .into_iter()
        .map(|project| snapshot_project(&project))
        .collect::<Vec<_>>();
    let snapshot_items = items
        .into_iter()
        .map(|item| snapshot_item(item, &mut variants_by_instruction))
        .collect::<Result<Vec<_>, BuildSnapshotError>>()?;
    let snapshot_assignments = assignments
        .into_iter()
        .map(|assignment| CoreAssignment {
            project_id: ProjectId::new(assignment.project_id),
            instruction_id: ItemId::new(assignment.instruction_id),
            target: match assignment.target {
                library::AssignmentTarget::Variant(variant_id) => {
                    CoreTarget::Variant(VariantId::new(variant_id))
                }
                library::AssignmentTarget::Custom(custom_text) => CoreTarget::Custom(custom_text),
                library::AssignmentTarget::Extend { base, supplement } => CoreTarget::Extend {
                    base: VariantId::new(base),
                    supplement,
                },
            },
        })
        .collect::<Vec<_>>();

    BuildSnapshot::new(snapshot_projects, snapshot_items, snapshot_assignments)
        .map_err(BuildSnapshotError::Invalid)
}

/// Maps one stored project row into the snapshot's project view: the
/// build-relevant fields only (identity, name, repository root, write
/// settings), with the vocabulary translated member-by-member.
fn snapshot_project(project: &projects::Project) -> SnapshotProject {
    SnapshotProject {
        id: ProjectId::new(project.id.clone()),
        name: project.name.clone(),
        // The canonical path, verbatim as stored (DB-030: the stored
        // path is the canonicalize output; a PathBuf is a value here,
        // core never opens it).
        root: PathBuf::from(&project.path),
        settings: ProjectWriteSettings {
            scheme: snapshot_scheme(project.scheme),
            base_name: snapshot_base_name(project.base_name),
            extra_local: snapshot_extra_file(project.extra_local),
            extra_project: snapshot_extra_file(project.extra_project),
        },
    }
}

/// Maps one stored library item into the snapshot's item view,
/// attaching the instruction's active variants (taken from the map —
/// the item walk is the only consumer).
fn snapshot_item(
    item: library::LibraryItem,
    variants_by_instruction: &mut HashMap<String, Vec<CoreVariant>>,
) -> Result<CoreLibraryItem, BuildSnapshotError> {
    match item.kind {
        ItemKind::Heading => {
            // Unreachable through the library decoder: a heading row
            // always decodes with its level (the §6.2 CHECK matrix is
            // mirrored on the read). A panic here names a decoder/map
            // drift, not a data state; the range half stays a typed
            // error because `HeadingLevel::try_from` is fallible.
            let level = item.heading_level.expect("a heading carries its level");
            Ok(CoreLibraryItem::Heading(CoreHeading {
                id: ItemId::new(item.id),
                level: HeadingLevel::try_from(level).map_err(BuildSnapshotError::HeadingLevel)?,
                title: item.title,
            }))
        }
        ItemKind::Instruction => {
            // Take the instruction's variants before its id moves into
            // the core value.
            let variants = variants_by_instruction.remove(&item.id).unwrap_or_default();
            Ok(CoreLibraryItem::Instruction(CoreInstruction {
                id: ItemId::new(item.id),
                title: item.title,
                state: snapshot_state(item.state),
                variants,
            }))
        }
    }
}

/// The store's scheme vocabulary → core's (the same three §6.2
/// `projects.scheme` CHECK literals).
fn snapshot_scheme(scheme: Scheme) -> CoreScheme {
    match scheme {
        Scheme::A => CoreScheme::A,
        Scheme::B => CoreScheme::B,
        Scheme::C => CoreScheme::C,
    }
}

/// The store's base-name vocabulary → core's (`AGENTS.md` /
/// `CLAUDE.md`).
fn snapshot_base_name(base_name: BaseName) -> CoreBaseName {
    match base_name {
        BaseName::AgentsMd => CoreBaseName::AgentsMd,
        BaseName::ClaudeMd => CoreBaseName::ClaudeMd,
    }
}

/// The store's position vocabulary → core's (`start` / `end`).
fn snapshot_position(position: Position) -> ExtraFilePosition {
    match position {
        Position::Start => ExtraFilePosition::Start,
        Position::End => ExtraFilePosition::End,
    }
}

/// One extra-file option: the enabled flag passes through, the
/// position vocabulary translates.
fn snapshot_extra_file(extra: ExtraFile) -> CoreExtraFile {
    CoreExtraFile {
        enabled: extra.enabled,
        position: snapshot_position(extra.position),
    }
}

/// The store's item-state vocabulary → core's (`active` / `hidden`;
/// headings never carry a state on either side).
fn snapshot_state(state: ItemState) -> CoreInstructionState {
    match state {
        ItemState::Active => CoreInstructionState::Active,
        ItemState::Hidden => CoreInstructionState::Hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vocabulary mappings are total and literal-preserving: each
    /// store member maps onto the core member carrying the same
    /// schema/settings literal. A swapped mapping (say `A` → `C`)
    /// fails this test even though both sides compile — the literals
    /// are the oracle, not the variant names.
    #[test]
    fn vocabulary_mappings_preserve_the_schema_literals() {
        for store in [Scheme::A, Scheme::B, Scheme::C] {
            assert_eq!(snapshot_scheme(store).as_str(), store.as_str());
        }
        for store in [BaseName::AgentsMd, BaseName::ClaudeMd] {
            assert_eq!(snapshot_base_name(store).as_str(), store.as_str());
        }
        for store in [Position::Start, Position::End] {
            assert_eq!(snapshot_position(store).as_str(), store.as_str());
        }
        for store in [ItemState::Active, ItemState::Hidden] {
            assert_eq!(snapshot_state(store).as_str(), store.as_str());
        }
        for extra in [
            ExtraFile {
                enabled: true,
                position: Position::Start,
            },
            ExtraFile {
                enabled: false,
                position: Position::End,
            },
        ] {
            let mapped = snapshot_extra_file(extra);
            assert_eq!(mapped.enabled, extra.enabled);
            assert_eq!(mapped.position.as_str(), extra.position.as_str());
        }
    }

    /// The error Display shapes callers log at sync start.
    #[test]
    fn error_display_shapes() {
        use eunomia_core::library::{ItemId, VariantId};
        let invalid = BuildSnapshotError::Invalid(SnapshotError::VariantMismatch {
            item_id: ItemId::new("i1".to_owned()),
            variant_id: VariantId::new("v2".to_owned()),
        });
        assert_eq!(
            invalid.to_string(),
            "the assembled snapshot is inconsistent: \
             variant v2 is not an active variant of instruction i1"
        );
        let heading = BuildSnapshotError::HeadingLevel(InvalidHeadingLevel { value: 7 });
        assert_eq!(
            heading.to_string(),
            "reading a heading failed: invalid heading level 7: headings are levels 2 to 6"
        );
    }
}
