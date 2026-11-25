//! The Assignments module's three commands (spec §7.10, T-331):
//! `assignment_set`, `assignments_bulk` and `assignment_set_custom`.
//!
//! Every write runs ONE `Db::run` transaction (D-037): the assignment
//! mutation and its single `assignment.changed` journal entry commit or
//! roll back together. These are library edits, so SYS-005 does not gate
//! them. No command opens or writes a project file (OUT-010).
//!
//! D-125's current custom-text model replaces the absorbed card's removed
//! `variant_fork_for_project`: custom text lives on the assignment itself,
//! never in a cloned variant. Bulk work is linear in the number of unique
//! requested row/column pairs plus the active-variant scan (when no variant
//! is supplied) and validated text length; each pair uses indexed store
//! reads/writes.

use std::collections::{HashMap, HashSet};

use eunomia_core::library_validators::{LibraryFieldError, validate_variant_text};
use eunomia_store::action_log::{
    Action, Details, Entity, EntityType, Level, LogEntry, Outcome, Visibility, insert,
};
use eunomia_store::library::{
    Assignment, AssignmentTarget, ItemKind, LibraryError, NewAssignment, VariantState, assign,
    get_assignment, get_item, get_variant, list_active_variants, list_variants, reassign,
    set_custom, unassign,
};
use eunomia_store::projects;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};

use crate::commands::{activity_log_to_app_error, managed_db, projects_store_to_app_error};
use crate::db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::logging;

/// The assignment's one content target on the wire. The tagged enum keeps
/// "variant and custom" and "neither" unrepresentable (DB-012).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum AssignmentTargetRecord {
    /// An active variant of the assignment's instruction.
    Variant(String),
    /// Per-project custom text stored on the assignment itself.
    Custom(String),
}

/// One assignment row as it stands after the command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct AssignmentRecord {
    /// The assigned project.
    pub(crate) project_id: String,
    /// The assigned instruction.
    pub(crate) instruction_id: String,
    /// The single content target (DB-012).
    pub(crate) target: AssignmentTargetRecord,
    /// Creation time, DB-002 UTC.
    pub(crate) created_at: String,
    /// Last real write of this row, DB-002 UTC.
    pub(crate) updated_at: String,
}

impl AssignmentRecord {
    fn from_store(assignment: Assignment) -> Self {
        let target = match assignment.target {
            AssignmentTarget::Variant(variant_id) => AssignmentTargetRecord::Variant(variant_id),
            AssignmentTarget::Custom(custom_text) => AssignmentTargetRecord::Custom(custom_text),
        };
        Self {
            project_id: assignment.project_id,
            instruction_id: assignment.instruction_id,
            target,
            created_at: assignment.created_at,
            updated_at: assignment.updated_at,
        }
    }
}

/// One project/instruction pair returned by a bulk operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct AssignmentPair {
    /// The project that already carried the instruction.
    pub(crate) project_id: String,
    /// The instruction already assigned to that project.
    pub(crate) instruction_id: String,
}

/// The measured result of one bulk operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct BulkAssignmentResult {
    /// How many assignment rows were inserted, changed or removed.
    pub(crate) changed: u32,
    /// Existing pairs left untouched, in request order (including custom
    /// cells, which bulk operations never rewrite).
    pub(crate) already_assigned: Vec<AssignmentPair>,
}

fn field_refusal(field: &str, reason: &str) -> AppError {
    let mut error = AppError::new(ErrorCode::Validation);
    error.params.insert("field".to_owned(), field.to_owned());
    error.params.insert("reason".to_owned(), reason.to_owned());
    error
}

fn library_fallback(kind: &str, message: &str) -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some(kind.to_owned()),
        os_message: Some(message.to_owned()),
    });
    error
}

fn store_error(error: LibraryError) -> AppError {
    match error {
        LibraryError::Sql(source) => db::sqlite_error_to_app_error(source),
        LibraryError::NotFound { .. } => field_refusal("id", "no assignment row has this id"),
        LibraryError::NotAnInstruction { .. } => {
            field_refusal("instruction_id", "a heading has no assignment cell")
        }
        LibraryError::AlreadyAssigned { .. } => field_refusal(
            "instruction_id",
            "the project already has this instruction assigned",
        ),
        LibraryError::VariantNotAssignable { .. } => field_refusal(
            "variant_id",
            "the variant must be active and belong to the same instruction (DB-012)",
        ),
        LibraryError::NoActiveMain { .. } | LibraryError::MainRequired { .. } => field_refusal(
            "instruction_id",
            "the instruction has no active main variant (DB-010)",
        ),
        LibraryError::Member { column, .. } => library_fallback(
            "LibraryCorrupt",
            &format!("a stored {column} cell is outside its vocabulary"),
        ),
        _ => library_fallback(
            "LibraryError",
            "the library repository refused an outcome these assignment commands do not map",
        ),
    }
}

fn custom_text_error(error: LibraryFieldError) -> AppError {
    match error {
        LibraryFieldError::VariantTextTooLong { length } => {
            let mut refusal =
                field_refusal("custom_text", "custom text is at most 262144 characters");
            refusal
                .params
                .insert("length".to_owned(), length.to_string());
            refusal
        }
        LibraryFieldError::VariantTextNul { index } => {
            let mut refusal = field_refusal("custom_text", "custom text must not contain NUL");
            refusal.params.insert("index".to_owned(), index.to_string());
            refusal
        }
        LibraryFieldError::ReservedContent { line, marker } => {
            let mut refusal = AppError::new(ErrorCode::ContentReserved);
            refusal
                .params
                .insert("field".to_owned(), "custom_text".to_owned());
            refusal.params.insert("line".to_owned(), line.to_string());
            refusal.params.insert(
                "marker".to_owned(),
                match marker {
                    eunomia_core::markers::MarkerLine::Begin => eunomia_core::markers::BEGIN_MARKER,
                    eunomia_core::markers::MarkerLine::End => eunomia_core::markers::END_MARKER,
                }
                .to_owned(),
            );
            refusal
        }
        _ => library_fallback(
            "LibraryFieldError",
            "a different library field validator outcome reached a custom-text command",
        ),
    }
}

fn ensure_project(
    transaction: &rusqlite::Transaction<'_>,
    project_id: &str,
) -> Result<(), AppError> {
    projects::get(transaction, project_id)
        .map_err(projects_store_to_app_error)?
        .ok_or_else(|| field_refusal("project_id", "no project has this id"))?;
    Ok(())
}

fn ensure_instruction(
    transaction: &rusqlite::Transaction<'_>,
    instruction_id: &str,
) -> Result<(), AppError> {
    let item = get_item(transaction, instruction_id)
        .map_err(store_error)?
        .ok_or_else(|| field_refusal("instruction_id", "no instruction has this id"))?;
    if item.kind != ItemKind::Instruction {
        return Err(field_refusal(
            "instruction_id",
            "a heading has no assignment cell",
        ));
    }
    Ok(())
}

fn ensure_pair(
    transaction: &rusqlite::Transaction<'_>,
    project_id: &str,
    instruction_id: &str,
) -> Result<(), AppError> {
    ensure_project(transaction, project_id)?;
    ensure_instruction(transaction, instruction_id)
}

fn active_main(
    transaction: &rusqlite::Transaction<'_>,
    instruction_id: &str,
) -> Result<String, AppError> {
    list_variants(transaction, instruction_id)
        .map_err(store_error)?
        .into_iter()
        .find(|variant| variant.is_main && variant.state == VariantState::Active)
        .map(|variant| variant.id)
        .ok_or_else(|| {
            field_refusal(
                "instruction_id",
                "the instruction has no active main variant (DB-010)",
            )
        })
}

fn ensure_assignable(
    transaction: &rusqlite::Transaction<'_>,
    instruction_id: &str,
    variant_id: &str,
) -> Result<(), AppError> {
    let variant = get_variant(transaction, variant_id)
        .map_err(store_error)?
        .ok_or_else(|| {
            field_refusal(
                "variant_id",
                "the variant must be active and belong to the same instruction (DB-012)",
            )
        })?;
    if variant.instruction_id != instruction_id || variant.state != VariantState::Active {
        return Err(field_refusal(
            "variant_id",
            "the variant must be active and belong to the same instruction (DB-012)",
        ));
    }
    Ok(())
}

fn journal(
    transaction: &rusqlite::Transaction<'_>,
    project_id: Option<&str>,
    entity_id: Option<&str>,
    changed: u64,
) -> Result<(), AppError> {
    let mut entry = LogEntry::new(
        Action::AssignmentChanged,
        Level::Info,
        Visibility::User,
        Outcome::Ok,
    );
    entry.entity = entity_id.map(|id| Entity {
        kind: EntityType::Assignment,
        id,
    });
    entry.project_id = project_id;
    entry.details = Some(Details::Bulk { changed });
    insert(transaction, &entry).map_err(activity_log_to_app_error)?;
    logging::action_logged(&entry);
    Ok(())
}

fn unique_ids(ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::with_capacity(ids.len());
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// Enables/disables one project's assignment and optionally selects its
/// variant. On an unassigned cell, `variant_id = null` selects the active
/// main; on an already-enabled custom cell it is a no-op, while an explicit
/// variant id converts the cell. Disabling removes the row. A same-target
/// re-send writes/journals nothing.
#[tauri::command]
#[specta::specta]
pub(crate) async fn assignment_set<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
    instruction_id: String,
    enabled: bool,
    variant_id: Option<String>,
) -> Result<Option<AssignmentRecord>, AppError> {
    assignment_set_impl(app, project_id, instruction_id, enabled, variant_id)
        .await
        .inspect_err(logging::assignment_set_failed)
}

async fn assignment_set_impl<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
    instruction_id: String,
    enabled: bool,
    variant_id: Option<String>,
) -> Result<Option<AssignmentRecord>, AppError> {
    if !enabled && variant_id.is_some() {
        return Err(field_refusal(
            "variant_id",
            "a variant is only meaningful when enabling an assignment",
        ));
    }
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            ensure_pair(transaction, &project_id, &instruction_id)?;
            let current =
                get_assignment(transaction, &project_id, &instruction_id).map_err(store_error)?;
            if !enabled {
                if current.is_none() {
                    return Ok(None);
                }
                unassign(transaction, &project_id, &instruction_id).map_err(store_error)?;
                let entity_id = format!("{project_id}/{instruction_id}");
                journal(transaction, Some(&project_id), Some(&entity_id), 1)?;
                return Ok(None);
            }

            if let Some(current) = &current
                && matches!(current.target, AssignmentTarget::Custom(_))
                && variant_id.is_none()
            {
                return Ok(Some(AssignmentRecord::from_store(current.clone())));
            }

            let target = match variant_id.as_deref() {
                Some(variant_id) => {
                    ensure_assignable(transaction, &instruction_id, variant_id)?;
                    variant_id.to_owned()
                }
                None => active_main(transaction, &instruction_id)?,
            };
            if let Some(current) = &current
                && current.target == AssignmentTarget::Variant(target.clone())
            {
                return Ok(Some(AssignmentRecord::from_store(current.clone())));
            }
            let assignment = if current.is_some() {
                reassign(transaction, &project_id, &instruction_id, &target)
            } else {
                assign(
                    transaction,
                    &NewAssignment {
                        project_id: &project_id,
                        instruction_id: &instruction_id,
                        variant_id: &target,
                    },
                )
            }
            .map_err(store_error)?;
            let entity_id = format!("{project_id}/{instruction_id}");
            journal(transaction, Some(&project_id), Some(&entity_id), 1)?;
            Ok(Some(AssignmentRecord::from_store(assignment)))
        })
        .await
}

/// Sets per-project custom text on one assignment. `Some("")` is a legal
/// custom target; `None` replaces an existing custom target with the active
/// main and is a no-op for an absent or variant-backed cell.
#[tauri::command]
#[specta::specta]
pub(crate) async fn assignment_set_custom<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
    instruction_id: String,
    content: Option<String>,
) -> Result<Option<AssignmentRecord>, AppError> {
    assignment_set_custom_impl(app, project_id, instruction_id, content)
        .await
        .inspect_err(logging::assignment_set_custom_failed)
}

async fn assignment_set_custom_impl<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
    instruction_id: String,
    content: Option<String>,
) -> Result<Option<AssignmentRecord>, AppError> {
    let content = content
        .map(|raw| {
            validate_variant_text(&raw)
                .map(str::to_owned)
                .map_err(custom_text_error)
        })
        .transpose()?;
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            ensure_pair(transaction, &project_id, &instruction_id)?;
            let before =
                get_assignment(transaction, &project_id, &instruction_id).map_err(store_error)?;
            let after = set_custom(
                transaction,
                &project_id,
                &instruction_id,
                content.as_deref(),
            )
            .map_err(store_error)?;
            if before != after {
                let entity_id = format!("{project_id}/{instruction_id}");
                journal(transaction, Some(&project_id), Some(&entity_id), 1)?;
            }
            Ok(after.map(AssignmentRecord::from_store))
        })
        .await
}

/// Applies a row/column product in one transaction. A variant is legal
/// only when one unique instruction row is selected; null selects each
/// instruction's active main. Existing pairs (including custom cells) are
/// left untouched and returned in `already_assigned`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn assignments_bulk<R: Runtime>(
    app: AppHandle<R>,
    instruction_ids: Vec<String>,
    project_ids: Vec<String>,
    enabled: bool,
    variant_id: Option<String>,
) -> Result<BulkAssignmentResult, AppError> {
    assignments_bulk_impl(app, instruction_ids, project_ids, enabled, variant_id)
        .await
        .inspect_err(logging::assignments_bulk_failed)
}

async fn assignments_bulk_impl<R: Runtime>(
    app: AppHandle<R>,
    instruction_ids: Vec<String>,
    project_ids: Vec<String>,
    enabled: bool,
    variant_id: Option<String>,
) -> Result<BulkAssignmentResult, AppError> {
    let instruction_ids = unique_ids(instruction_ids);
    let project_ids = unique_ids(project_ids);
    if let Some(_) = &variant_id
        && (!enabled || instruction_ids.len() != 1)
    {
        return Err(field_refusal(
            "variant_id",
            "a variant is only meaningful when enabling exactly one instruction row",
        ));
    }
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            // Validate the complete request before the first write. The
            // transaction is still the atomicity floor if a later store
            // statement or the journal refuses.
            for project_id in &project_ids {
                ensure_project(transaction, project_id)?;
            }
            for instruction_id in &instruction_ids {
                ensure_instruction(transaction, instruction_id)?;
            }
            let mut mains: HashMap<String, String> = HashMap::new();
            if enabled {
                if let Some(variant_id) = variant_id.as_deref() {
                    let instruction_id = instruction_ids[0].as_str();
                    ensure_assignable(transaction, instruction_id, variant_id)?;
                    mains.insert(instruction_id.to_owned(), variant_id.to_owned());
                } else {
                    for variant in list_active_variants(transaction).map_err(store_error)? {
                        if variant.is_main {
                            mains.insert(variant.instruction_id, variant.id);
                        }
                    }
                    for instruction_id in &instruction_ids {
                        if !mains.contains_key(instruction_id.as_str()) {
                            return Err(field_refusal(
                                "instruction_id",
                                "the instruction has no active main variant (DB-010)",
                            ));
                        }
                    }
                }
            }

            let mut changed = 0usize;
            let mut already_assigned = Vec::new();
            for instruction_id in &instruction_ids {
                for project_id in &project_ids {
                    let current = get_assignment(transaction, project_id, instruction_id)
                        .map_err(store_error)?;
                    if enabled {
                        if current.is_some() {
                            already_assigned.push(AssignmentPair {
                                project_id: (*project_id).to_owned(),
                                instruction_id: (*instruction_id).to_owned(),
                            });
                            continue;
                        }
                        let target = mains
                            .get(instruction_id.as_str())
                            .expect("the enabled variant map was validated for every row");
                        assign(
                            transaction,
                            &NewAssignment {
                                project_id,
                                instruction_id,
                                variant_id: target,
                            },
                        )
                        .map_err(store_error)?;
                        changed += 1;
                    } else if let Some(current) = current {
                        if matches!(current.target, AssignmentTarget::Custom(_)) {
                            already_assigned.push(AssignmentPair {
                                project_id: (*project_id).to_owned(),
                                instruction_id: (*instruction_id).to_owned(),
                            });
                            continue;
                        }
                        unassign(transaction, project_id, instruction_id).map_err(store_error)?;
                        changed += 1;
                    }
                }
            }
            if changed > 0 {
                journal(
                    transaction,
                    None,
                    None,
                    u64::try_from(changed).expect("an assignment batch cannot exceed u64 rows"),
                )?;
            }
            Ok(BulkAssignmentResult {
                changed: u32::try_from(changed).expect("an in-memory assignment batch fits u32"),
                already_assigned,
            })
        })
        .await
}
