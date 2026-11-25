//! The assignment link (spec §2 "Assignment", MAIN-007).

use serde::{Deserialize, Serialize};

use crate::library::{ItemId, VariantId};
use crate::project::ProjectId;

/// The one content target of an assignment (DB-012): an active variant of
/// the instruction, or per-project custom text instead of a variant.
///
/// The enum makes the database's exactly-one-target rule representable in
/// Rust: an assignment cannot carry both targets or neither.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum AssignmentTarget {
    /// The selected active variant of the assignment's instruction.
    Variant(VariantId),
    /// Text stored on this project's assignment itself (MAIN-007).
    Custom(String),
}

/// A "project — instruction — selected variant or custom text" link.
///
/// One instruction has at most one assignment in one project (spec D-02):
/// the schema §6.2 `project_instructions` primary key enforces it, and the
/// build snapshot re-checks it at assembly. A variant target must name an
/// active variant of the referenced instruction (DB-012). Assignments to
/// hidden instructions are valid and kept — the build excludes them
/// (D-11, DB-013).
///
/// # Examples
///
/// ```
/// use eunomia_core::assignment::{Assignment, AssignmentTarget};
/// use eunomia_core::library::{ItemId, VariantId};
/// use eunomia_core::project::ProjectId;
///
/// let assignment = Assignment {
///     project_id: ProjectId::new("p1".to_owned()),
///     instruction_id: ItemId::new("i1".to_owned()),
///     target: AssignmentTarget::Variant(VariantId::new("v1".to_owned())),
/// };
/// assert_eq!(assignment.project_id.as_str(), "p1");
/// assert_eq!(assignment.instruction_id.as_str(), "i1");
/// assert!(matches!(assignment.target, AssignmentTarget::Variant(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Assignment {
    /// The assigned project.
    pub project_id: ProjectId,
    /// The assigned instruction.
    pub instruction_id: ItemId,
    /// The assignment's single content target.
    pub target: AssignmentTarget,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_targets_serialize_without_an_illegal_both_or_neither_shape() {
        let variant = Assignment {
            project_id: ProjectId::new("p1".to_owned()),
            instruction_id: ItemId::new("i1".to_owned()),
            target: AssignmentTarget::Variant(VariantId::new("v1".to_owned())),
        };
        let variant_json = serde_json::to_string(&variant).unwrap();
        assert_eq!(
            variant_json,
            r#"{"project_id":"p1","instruction_id":"i1","target":{"kind":"variant","value":"v1"}}"#
        );
        assert_eq!(
            serde_json::from_str::<Assignment>(&variant_json).unwrap(),
            variant
        );

        let custom = Assignment {
            project_id: ProjectId::new("p1".to_owned()),
            instruction_id: ItemId::new("i1".to_owned()),
            target: AssignmentTarget::Custom("project text".to_owned()),
        };
        let custom_json = serde_json::to_string(&custom).unwrap();
        assert_eq!(
            custom_json,
            r#"{"project_id":"p1","instruction_id":"i1","target":{"kind":"custom","value":"project text"}}"#
        );
        assert_eq!(
            serde_json::from_str::<Assignment>(&custom_json).unwrap(),
            custom
        );
        assert!(serde_json::from_str::<Assignment>(
            r#"{"project_id":"p1","instruction_id":"i1","target":{"kind":"unknown","value":"x"}}"#
        )
        .is_err());
    }
}
