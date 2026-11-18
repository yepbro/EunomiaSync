//! The assignment link (spec §2 "Assignment").

use serde::{Deserialize, Serialize};

use crate::library::{ItemId, VariantId};
use crate::project::ProjectId;

/// A "project — instruction — selected variant" link (spec §2
/// "Assignment").
///
/// One instruction has at most one assignment in one project (spec
/// D-02): the schema §6.2 `project_instructions` primary key
/// `(project_id, instruction_id)` enforces it, and the build snapshot
/// re-checks it at assembly (see [`crate::snapshot::BuildSnapshot`]).
/// The referenced variant must be an active variant of the referenced
/// instruction (DB-012). Assignments to hidden instructions are valid
/// and are kept — the build simply excludes them (D-11, DB-013).
///
/// # Examples
///
/// ```
/// use eunomia_core::assignment::Assignment;
/// use eunomia_core::library::{ItemId, VariantId};
/// use eunomia_core::project::ProjectId;
///
/// let assignment = Assignment {
///     project_id: ProjectId::new("p1".to_owned()),
///     instruction_id: ItemId::new("i1".to_owned()),
///     variant_id: VariantId::new("v1".to_owned()),
/// };
/// assert_eq!(assignment.project_id.as_str(), "p1");
/// assert_eq!(assignment.instruction_id.as_str(), "i1");
/// assert_eq!(assignment.variant_id.as_str(), "v1");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Assignment {
    /// The assigned project.
    pub project_id: ProjectId,
    /// The assigned instruction.
    pub instruction_id: ItemId,
    /// The selected variant of that instruction.
    pub variant_id: VariantId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_serializes_with_the_column_field_names() {
        let assignment = Assignment {
            project_id: ProjectId::new("p1".to_owned()),
            instruction_id: ItemId::new("i1".to_owned()),
            variant_id: VariantId::new("v1".to_owned()),
        };
        let json = serde_json::to_string(&assignment).unwrap();
        assert_eq!(
            json,
            r#"{"project_id":"p1","instruction_id":"i1","variant_id":"v1"}"#
        );
        assert_eq!(
            serde_json::from_str::<Assignment>(&json).unwrap(),
            assignment
        );
    }
}
