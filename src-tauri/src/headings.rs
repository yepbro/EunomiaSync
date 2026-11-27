//! The four Library-structure commands (spec §7.10, T-330; the batch
//! absorbing T-092…T-095): `heading_create`, `heading_update`,
//! `heading_delete` and `items_move`.
//!
//! Each command runs ONE `Db::run` transaction (D-037): the library
//! write, any level/order changes and the `action_log` entry commit or
//! roll back together. Headings have no revisions (§6.2); the journal
//! is their only history. Library edits remain allowed during a sync
//! (§5.4), so these commands take no SYS-005 gate.
//!
//! Hierarchy policy stays in `eunomia-core`: ALG-060 resolves branches,
//! ALG-061 moves ranges, and ALG-062 validates point-wise, manual ±1 and
//! automatic drop alignment. This module translates semantic wire
//! targets into those pure operations and persists the resulting order
//! through the store repository. It never re-implements a branch walk.
//!
//! Rule 8: title text is validated at the boundary but never enters an
//! error or log payload. `params` carry fields, reasons, levels,
//! deltas, counts and indexes only; stored titles stay in the library.

use std::collections::HashMap;

use eunomia_core::heading_range::{HeadingBranchError, heading_branch};
use eunomia_core::levels::{
    LevelError, change_heading_level, shift_branch_level, shift_branch_to_level,
};
use eunomia_core::library::{HeadingLevel, InstructionState, LibraryItem as CoreLibraryItem};
use eunomia_core::library_validators::{LibraryFieldError, validate_heading_title};
use eunomia_core::reorder::{ReorderError, move_branch, move_range};
use eunomia_store::action_log::{
    Action, Details, Entity, EntityType, Level, LogEntry, Outcome, Visibility, insert,
};
use eunomia_store::library::{
    ItemKind, ItemScope, ItemState, ItemUpdate, LibraryError, LibraryItem, NewLibraryItem,
    create_item, delete_heading, get_item, list_items, move_item, set_item_order, update_item,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};

use crate::commands::{activity_log_to_app_error, managed_db};
use crate::db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::logging;

// ---------------------------------------------------------------------------
// Wire DTOs
// ---------------------------------------------------------------------------

/// One heading row on the wire: the created or updated heading as it
/// stands after the command's transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct HeadingRecord {
    /// The heading's UUIDv7 TEXT id (DB-002).
    pub(crate) id: String,
    /// The stored, trimmed title (§6.9).
    pub(crate) title: String,
    /// The heading level, 2–6.
    pub(crate) level: u8,
    /// The table-wide dense position (DB-005/DB-011).
    pub(crate) position: i32,
    /// Creation time, DB-002 UTC ISO-8601.
    pub(crate) created_at: String,
    /// Last title/level write, DB-002 UTC ISO-8601.
    pub(crate) updated_at: String,
}

impl HeadingRecord {
    /// Shapes a store heading row into its wire form.
    fn from_store(item: LibraryItem) -> Self {
        let level = item
            .heading_level
            .expect("a store heading row always carries its decoded level");
        HeadingRecord {
            id: item.id,
            title: item.title,
            level,
            position: i32::try_from(item.position)
                .expect("the library row count fits the wire's i32 position"),
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}

/// The one mutually exclusive operation `heading_update` performs.
/// Adjacent tagging keeps primitive values (`String`, `i32`) in a
/// `value` field while `mode` makes the variants impossible to mix.
/// Numeric wire values are widened deliberately so malformed integers
/// reach the command's `E_VALIDATION` boundary instead of failing serde.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "mode", content = "value", rename_all = "snake_case")]
pub(crate) enum HeadingUpdate {
    /// Rename this heading (LIB-030/031).
    Rename(String),
    /// Change only this heading's level (LIB-032). The wire uses i32
    /// so every JSON integer reaches the command's `E_VALIDATION`
    /// boundary; core still receives its closed `u8` level.
    ChangeLevel(i32),
    /// Shift every heading in this heading's ALG-060 branch by ±1
    /// (ALG-062). The wire uses i32 for the same boundary reason;
    /// core still receives its closed `i8` delta.
    ShiftBranch(i32),
}

/// A semantic drop destination for `items_move`. The frontend names
/// the row/heading it dropped onto; Rust resolves the current insertion
/// gap inside the transaction, so a stale numeric position cannot move
/// the wrong item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum MoveTarget {
    /// The original-list gap immediately after this row.
    AfterItem {
        /// The row the item was dropped on.
        id: String,
    },
    /// The end of this heading's section. A moved heading branch also
    /// aligns its root to the target heading's level + 1 (ALG-062).
    IntoHeading {
        /// The heading the item/branch was dropped on.
        id: String,
    },
    /// The first root position. A moved heading branch aligns to level
    /// 2 (LIB-032/ALG-062).
    Root,
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// An `E_VALIDATION` refusal with no user-content echo.
fn field_refusal(field: &str, reason: &str) -> AppError {
    let mut error = AppError::new(ErrorCode::Validation);
    error.params.insert("field".to_owned(), field.to_owned());
    error.params.insert("reason".to_owned(), reason.to_owned());
    error
}

/// Adds one measured numeric fact to a refusal.
fn with_number(mut error: AppError, key: &str, value: usize) -> AppError {
    error.params.insert(key.to_owned(), value.to_string());
    error
}

/// Adds one short code (never title text) to a refusal.
fn with_code(mut error: AppError, key: &str, value: &str) -> AppError {
    error.params.insert(key.to_owned(), value.to_owned());
    error
}

/// Adds one signed wire integer to a refusal without narrowing it first.
fn with_i32(mut error: AppError, key: &str, value: i32) -> AppError {
    error.params.insert(key.to_owned(), value.to_string());
    error
}

/// Maps the four heading-title validator outcomes. The title itself is
/// deliberately absent: the dialog already holds the draft.
fn heading_title_error(error: LibraryFieldError) -> AppError {
    match error {
        LibraryFieldError::HeadingTitleEmpty => field_refusal(
            "name",
            "a heading name needs 1-200 characters after trimming",
        ),
        LibraryFieldError::HeadingTitleTooLong { length } => with_number(
            field_refusal("name", "a heading name is at most 200 characters"),
            "length",
            length,
        ),
        LibraryFieldError::HeadingTitleLineBreak { index, character } => {
            let character = match character {
                '\n' => "LF",
                '\r' => "CR",
                _ => "line break",
            };
            with_code(
                with_number(
                    field_refusal("name", "a heading name is one line"),
                    "index",
                    index,
                ),
                "character",
                character,
            )
        }
        LibraryFieldError::HeadingTitleNul { index } => with_number(
            field_refusal("name", "a heading name must not contain NUL"),
            "index",
            index,
        ),
        _ => library_fallback(
            "LibraryFieldError",
            "a different library field validator outcome reached a heading command",
        ),
    }
}

/// Maps a core level failure to `E_VALIDATION`. `field` names the
/// caller-facing operation: `level`, `branch_shift`, or the automatic
/// move alignment's `heading_level`.
fn level_error(error: LevelError, field: &str) -> AppError {
    match error {
        LevelError::LevelOutOfRange { index, value } => {
            let mut error = with_number(
                field_refusal(field, "heading levels must stay within 2 to 6"),
                "level",
                usize::from(value),
            );
            error.params.insert("index".to_owned(), index.to_string());
            error
        }
        LevelError::InvalidDelta { delta } => {
            let mut error = field_refusal(
                "branch_shift",
                "a manual branch level shift must be exactly -1 or +1",
            );
            error.params.insert("delta".to_owned(), delta.to_string());
            error
        }
        LevelError::BranchLevelOutOfRange {
            index,
            delta,
            offender,
            offender_level,
        } => {
            let mut error = field_refusal(
                field,
                "the whole branch was refused because a heading would leave 2 to 6",
            );
            for (key, value) in [
                ("index", index.to_string()),
                ("delta", delta.to_string()),
                ("offender", offender.to_string()),
                ("offender_level", offender_level.to_string()),
            ] {
                error.params.insert(key.to_owned(), value);
            }
            error
        }
        LevelError::IndexOutOfBounds { index, len } => {
            let mut error = with_number(
                field_refusal("id", "the heading no longer exists in the library"),
                "index",
                index,
            );
            error.params.insert("len".to_owned(), len.to_string());
            error
        }
        LevelError::NotAHeading { index } => with_number(
            field_refusal("id", "the row is not a heading"),
            "index",
            index,
        ),
        LevelError::Branch(_) => {
            field_refusal("id", "the requested heading branch could not be resolved")
        }
    }
}

/// Maps an ALG-060 branch-resolution failure through the same wire
/// envelope as the ALG-061 move guard.
fn branch_error(error: HeadingBranchError) -> AppError {
    reorder_error(ReorderError::Branch(error))
}

/// Maps the ALG-061 guard to the semantic `target` field.
fn reorder_error(error: ReorderError) -> AppError {
    match error {
        ReorderError::TargetInsideMovedRange { range, target } => {
            let mut error = field_refusal("target", "a branch cannot move inside its own range");
            error
                .params
                .insert("range_start".to_owned(), range.start.to_string());
            error
                .params
                .insert("range_end".to_owned(), range.end.to_string());
            error.params.insert("target".to_owned(), target.to_string());
            error
        }
        ReorderError::InvalidRange { start, end, len } => {
            let mut error =
                field_refusal("target", "the moved range is not a non-empty library range");
            error.params.insert("start".to_owned(), start.to_string());
            error.params.insert("end".to_owned(), end.to_string());
            error.params.insert("len".to_owned(), len.to_string());
            error
        }
        ReorderError::TargetOutOfBounds { target, len } => {
            let mut error = field_refusal("target", "the move target is outside the library");
            error.params.insert("target".to_owned(), target.to_string());
            error.params.insert("len".to_owned(), len.to_string());
            error
        }
        ReorderError::Branch(_) => {
            field_refusal("id", "the requested heading branch could not be resolved")
        }
    }
}

/// Maps repository failures. Caller/stale-id outcomes are validation;
/// corrupt cells and impossible order permutations take the D-037
/// class-only fallback; SQLite uses the shared base classifier.
fn library_error(error: LibraryError) -> AppError {
    match error {
        LibraryError::Sql(source) => db::sqlite_error_to_app_error(source),
        LibraryError::NotFound { .. } => field_refusal("id", "no library row has this id"),
        LibraryError::NotAHeading { .. } => field_refusal("id", "the row is not a heading"),
        LibraryError::PositionOutOfRange { requested, max } => {
            let mut error = field_refusal("target", "the target position is outside the library");
            error
                .params
                .insert("requested".to_owned(), requested.to_string());
            error.params.insert("max".to_owned(), max.to_string());
            error
        }
        LibraryError::Member { column, .. } => library_fallback(
            "LibraryCorrupt",
            &format!("a stored {column} cell is outside its vocabulary"),
        ),
        LibraryError::InvalidItemOrder { .. }
        | LibraryError::DuplicateOrderId { .. }
        | LibraryError::UnknownOrderId { .. } => library_fallback(
            "LibraryOrder",
            "the core order did not form an exact library permutation",
        ),
        _ => library_fallback(
            "LibraryError",
            "a different library outcome reached a structure command",
        ),
    }
}

/// The D-037 fallback with a class-only diagnostic.
fn library_fallback(kind: &str, message: &str) -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some(kind.to_owned()),
        os_message: Some(message.to_owned()),
    });
    error
}

// ---------------------------------------------------------------------------
// Shared repository/core translation
// ---------------------------------------------------------------------------

/// The by-id heading read shared by update and delete.
fn heading_of(transaction: &rusqlite::Transaction<'_>, id: &str) -> Result<LibraryItem, AppError> {
    let item = get_item(transaction, id)
        .map_err(library_error)?
        .ok_or_else(|| field_refusal("id", "no heading has this id"))?;
    if item.kind != ItemKind::Heading {
        return Err(field_refusal(
            "id",
            "the row is an instruction, not a heading",
        ));
    }
    Ok(item)
}

/// Finds a row by id and names the caller-facing field on a stale id.
fn index_of(rows: &[LibraryItem], id: &str, field: &str) -> Result<usize, AppError> {
    rows.iter()
        .position(|row| row.id == id)
        .ok_or_else(|| field_refusal(field, "no library row has this id"))
}

/// Converts the store's ordered rows into core's pure hierarchy model.
/// Variants are deliberately empty: ALG-060/061/062 read only kind,
/// heading level, id, title and instruction state. The store decoder
/// has already enforced the heading-level matrix.
fn core_items(rows: &[LibraryItem]) -> Vec<CoreLibraryItem> {
    rows.iter()
        .map(|row| match row.kind {
            ItemKind::Heading => CoreLibraryItem::Heading(eunomia_core::library::Heading {
                id: eunomia_core::library::ItemId::new(row.id.clone()),
                level: HeadingLevel::try_from(
                    row.heading_level
                        .expect("a store heading row always carries its decoded level"),
                )
                .expect("the store decoder guarantees a heading level in 2 to 6"),
                title: row.title.clone(),
            }),
            ItemKind::Instruction => {
                CoreLibraryItem::Instruction(eunomia_core::library::Instruction {
                    id: eunomia_core::library::ItemId::new(row.id.clone()),
                    title: row.title.clone(),
                    state: match row.state {
                        ItemState::Active => InstructionState::Active,
                        ItemState::Hidden => InstructionState::Hidden,
                    },
                    variants: Vec::new(),
                })
            }
        })
        .collect()
}

/// Persists a core permutation and any heading-level changes in the
/// caller's transaction. Order writes are O(N); level updates touch
/// only headings whose value actually changed.
fn apply_core_items(
    transaction: &rusqlite::Transaction<'_>,
    before: &[LibraryItem],
    after: &[CoreLibraryItem],
) -> Result<(), AppError> {
    let same_order = before.len() == after.len()
        && before
            .iter()
            .zip(after)
            .all(|(before, after)| before.id == after.item_id().as_str());
    if !same_order {
        let ordered_ids: Vec<&str> = after.iter().map(|item| item.item_id().as_str()).collect();
        set_item_order(transaction, &ordered_ids).map_err(library_error)?;
    }

    let levels: HashMap<&str, u8> = after
        .iter()
        .filter_map(|item| match item {
            CoreLibraryItem::Heading(heading) => Some((heading.id.as_str(), heading.level.as_u8())),
            CoreLibraryItem::Instruction(_) => None,
        })
        .collect();
    for row in before {
        if row.kind != ItemKind::Heading {
            continue;
        }
        let old = row
            .heading_level
            .expect("a store heading row always carries its decoded level");
        let new = levels.get(row.id.as_str()).copied().unwrap_or(old);
        if new != old {
            update_item(
                transaction,
                &row.id,
                &ItemUpdate {
                    title: None,
                    heading_level: Some(new),
                },
            )
            .map_err(library_error)?;
        }
    }
    Ok(())
}

/// Writes one successful structure action inside the caller's
/// transaction and records it into the transaction's mirror buffer
/// (T-324/D-228: the executor applies the D-051 mirror policy at the
/// post-commit flush).
fn journal(
    transaction: &rusqlite::Transaction<'_>,
    mirror: &mut crate::logging::MirrorBuffer,
    action: Action,
    entity_type: EntityType,
    entity_id: &str,
    details: Option<Details>,
) -> Result<(), AppError> {
    let mut entry = LogEntry::new(action, Level::Info, Visibility::User, Outcome::Ok);
    entry.entity = Some(Entity {
        kind: entity_type,
        id: entity_id,
    });
    entry.details = details;
    insert(transaction, &entry).map_err(activity_log_to_app_error)?;
    mirror.record(&entry);
    Ok(())
}

// ---------------------------------------------------------------------------
// heading_create (T-092)
// ---------------------------------------------------------------------------

/// Creates a heading of level 2–6 (§7.3 LIB-030/031, §6.9). `parent =
/// None` appends a root at the table end; `Some(heading_id)` appends a
/// child at the end of that heading's ALG-060 branch.
///
/// Name validation is one linear pass and runs before the database is
/// resolved. The insert, optional placement move and `heading.created`
/// journal entry share one transaction. No revision row is written.
#[tauri::command]
#[specta::specta]
pub(crate) async fn heading_create<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    level: i32,
    parent: Option<String>,
) -> Result<HeadingRecord, AppError> {
    heading_create_impl(app, name, level, parent)
        .await
        .inspect_err(logging::heading_create_failed)
}

async fn heading_create_impl<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    level: i32,
    parent: Option<String>,
) -> Result<HeadingRecord, AppError> {
    let name = validate_heading_title(&name).map_err(heading_title_error)?;
    let level = u8::try_from(level).map_err(|_| {
        with_i32(
            field_refusal("level", "heading levels must stay within 2 to 6"),
            "level",
            level,
        )
    })?;
    HeadingLevel::try_from(level).map_err(|_| {
        with_i32(
            field_refusal("level", "heading levels must stay within 2 to 6"),
            "level",
            i32::from(level),
        )
    })?;
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            let rows = list_items(transaction, ItemScope::All).map_err(library_error)?;
            let target = match parent.as_deref() {
                None => rows.len() as i64 + 1,
                Some(parent_id) => {
                    let index = index_of(&rows, parent_id, "parent")?;
                    if rows[index].kind != ItemKind::Heading {
                        return Err(field_refusal("parent", "a parent must be a heading row"));
                    }
                    let core = core_items(&rows);
                    let branch = heading_branch(&core, index).map_err(branch_error)?;
                    branch.range.end as i64 + 1
                }
            };
            let created = create_item(
                transaction,
                &NewLibraryItem::Heading {
                    title: &name,
                    level,
                },
            )
            .map_err(library_error)?;
            let placed = if created.position != target {
                move_item(transaction, &created.id, target).map_err(library_error)?
            } else {
                created
            };
            journal(
                transaction,
                mirror,
                Action::HeadingCreated,
                EntityType::Heading,
                &placed.id,
                None,
            )?;
            Ok(HeadingRecord::from_store(placed))
        })
        .await
}

// ---------------------------------------------------------------------------
// heading_update (T-092/T-094)
// ---------------------------------------------------------------------------

/// Applies exactly one heading operation: rename, point-wise level
/// change, or whole-branch ±1 shift. The core owns the level rules; a
/// same-value operation writes and journals nothing. Every real change
/// journals `heading.changed` in the same transaction and writes no
/// revision.
#[tauri::command]
#[specta::specta]
pub(crate) async fn heading_update<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    update: HeadingUpdate,
) -> Result<HeadingRecord, AppError> {
    heading_update_impl(app, id, update)
        .await
        .inspect_err(logging::heading_update_failed)
}

async fn heading_update_impl<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    update: HeadingUpdate,
) -> Result<HeadingRecord, AppError> {
    // Rename validation belongs before database resolution; level
    // validation stays inside core so its operand-before-value error
    // precedence is preserved.
    let rename = match &update {
        HeadingUpdate::Rename(name) => {
            Some(validate_heading_title(name).map_err(heading_title_error)?)
        }
        HeadingUpdate::ChangeLevel(_) | HeadingUpdate::ShiftBranch(_) => None,
    };
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            let heading = heading_of(transaction, &id)?;
            match update {
                HeadingUpdate::Rename(_) => {
                    let name = rename.as_ref().expect("the rename arm validated its name");
                    if heading.title == name.as_str() {
                        return Ok(HeadingRecord::from_store(heading));
                    }
                    let updated = update_item(
                        transaction,
                        &id,
                        &ItemUpdate {
                            title: Some(name.as_str()),
                            heading_level: None,
                        },
                    )
                    .map_err(library_error)?;
                    journal(
                        transaction,
                        mirror,
                        Action::HeadingChanged,
                        EntityType::Heading,
                        &updated.id,
                        None,
                    )?;
                    Ok(HeadingRecord::from_store(updated))
                }
                HeadingUpdate::ChangeLevel(level) => {
                    let rows = list_items(transaction, ItemScope::All).map_err(library_error)?;
                    let index = index_of(&rows, &id, "id")?;
                    let level = u8::try_from(level).map_err(|_| {
                        with_i32(
                            with_number(
                                field_refusal("level", "heading levels must stay within 2 to 6"),
                                "index",
                                index,
                            ),
                            "level",
                            level,
                        )
                    })?;
                    let core = core_items(&rows);
                    let changed = change_heading_level(&core, index, level)
                        .map_err(|error| level_error(error, "level"))?;
                    let new_level = match &changed[index] {
                        CoreLibraryItem::Heading(heading) => heading.level.as_u8(),
                        CoreLibraryItem::Instruction(_) => {
                            return Err(field_refusal("id", "the row is not a heading"));
                        }
                    };
                    if heading.heading_level == Some(new_level) {
                        return Ok(HeadingRecord::from_store(heading));
                    }
                    let updated = update_item(
                        transaction,
                        &id,
                        &ItemUpdate {
                            title: None,
                            heading_level: Some(new_level),
                        },
                    )
                    .map_err(library_error)?;
                    journal(
                        transaction,
                        mirror,
                        Action::HeadingChanged,
                        EntityType::Heading,
                        &updated.id,
                        None,
                    )?;
                    Ok(HeadingRecord::from_store(updated))
                }
                HeadingUpdate::ShiftBranch(delta) => {
                    let delta = i8::try_from(delta).map_err(|_| {
                        with_i32(
                            field_refusal(
                                "branch_shift",
                                "a manual branch level shift must be exactly -1 or +1",
                            ),
                            "delta",
                            delta,
                        )
                    })?;
                    let rows = list_items(transaction, ItemScope::All).map_err(library_error)?;
                    let index = index_of(&rows, &id, "id")?;
                    let core = core_items(&rows);
                    let shifted = shift_branch_level(&core, index, delta)
                        .map_err(|error| level_error(error, "branch_shift"))?;
                    let changed = core
                        .iter()
                        .zip(&shifted)
                        .filter(|(before, after)| before != after)
                        .count();
                    apply_core_items(transaction, &rows, &shifted)?;
                    let updated = heading_of(transaction, &id)?;
                    journal(
                        transaction,
                        mirror,
                        Action::HeadingChanged,
                        EntityType::Heading,
                        &updated.id,
                        Some(Details::Bulk {
                            changed: u64::try_from(changed)
                                .expect("a library branch cannot exceed u64 rows"),
                        }),
                    )?;
                    Ok(HeadingRecord::from_store(updated))
                }
            }
        })
        .await
}

// ---------------------------------------------------------------------------
// heading_delete (T-093)
// ---------------------------------------------------------------------------

/// Physically deletes one heading (§6.4 LIB-033). The store closes the
/// position gap; every instruction and nested heading stays in its
/// previous relative order and therefore belongs to the preceding
/// section — which is exactly why LIB-037 (§6.4) refuses a level-2
/// heading whose section still holds ANY item (instructions, nested
/// headings, hidden rows included): a root section has no previous
/// section to move them into. The refusal is the typed `E_VALIDATION`
/// naming `id` with the blocking item count, checked before any write;
/// an empty root and every nested heading delete as before. The
/// delete and `heading.deleted` journal entry share one transaction;
/// no revision is written.
#[tauri::command]
#[specta::specta]
pub(crate) async fn heading_delete<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<(), AppError> {
    heading_delete_impl(app, id)
        .await
        .inspect_err(logging::heading_delete_failed)
}

async fn heading_delete_impl<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), AppError> {
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            let heading = heading_of(transaction, &id)?;
            // LIB-037: only a ROOT section can be refused, so the
            // ordered read and the ALG-060 walk run for level-2
            // headings alone. The widened scope keeps hidden
            // instructions in the picture (D-11: they are items of
            // the structure), and the branch's section covers
            // instructions and nested headings alike — the module
            // never re-implements the walk.
            if heading.heading_level == Some(2) {
                let rows = list_items(transaction, ItemScope::All).map_err(library_error)?;
                let index = index_of(&rows, &id, "id")?;
                let section = heading_branch(&core_items(&rows), index)
                    .map_err(branch_error)?
                    .section();
                if !section.is_empty() {
                    return Err(with_number(
                        field_refusal(
                            "id",
                            "a top-level section that still holds items cannot be deleted",
                        ),
                        "items",
                        section.len(),
                    ));
                }
            }
            delete_heading(transaction, &id).map_err(library_error)?;
            journal(
                transaction,
                mirror,
                Action::HeadingDeleted,
                EntityType::Heading,
                &id,
                None,
            )?;
            Ok(())
        })
        .await
}

// ---------------------------------------------------------------------------
// items_move (T-095)
// ---------------------------------------------------------------------------

/// Moves one instruction or a whole heading branch in one transaction
/// (ALG-060/061). The target names a semantic drop outcome; the core
/// computes the original-list insertion gap and rejects a branch's own
/// interior. Dropping a branch on a heading/header also aligns every
/// heading level through ALG-062 before any write.
///
/// Complexity over database input: the ordered read, core conversion,
/// branch resolution, move, optional alignment and order persistence are
/// each O(N); level updates touch only changed headings. No text scan is
/// nested inside another. A real move journals one `heading.changed` or
/// `instruction.changed` entry with the moved-row count; an identity
/// move writes and journals nothing.
#[tauri::command]
#[specta::specta]
pub(crate) async fn items_move<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    target: MoveTarget,
) -> Result<(), AppError> {
    items_move_impl(app, id, target)
        .await
        .inspect_err(logging::items_move_failed)
}

async fn items_move_impl<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    target: MoveTarget,
) -> Result<(), AppError> {
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            let rows = list_items(transaction, ItemScope::All).map_err(library_error)?;
            let source_index = index_of(&rows, &id, "id")?;
            let source_is_heading = rows[source_index].kind == ItemKind::Heading;
            let core = core_items(&rows);
            // ALG-060 resolves the dragged range ONCE, before any level
            // alignment or insertion can change the surrounding geometry.
            let source_range = if source_is_heading {
                heading_branch(&core, source_index)
                    .map_err(branch_error)?
                    .range
            } else {
                source_index..source_index + 1
            };

            let (target_index, aligned_root_level) = match &target {
                MoveTarget::AfterItem { id: target_id } => {
                    let index = index_of(&rows, target_id, "target")?;
                    (index + 1, None)
                }
                MoveTarget::IntoHeading { id: target_id } => {
                    let index = index_of(&rows, target_id, "target")?;
                    if rows[index].kind != ItemKind::Heading {
                        return Err(field_refusal(
                            "target",
                            "a drop target must be a heading row",
                        ));
                    }
                    if source_is_heading && source_range.contains(&index) {
                        return Err(field_refusal(
                            "target",
                            "a branch cannot be dropped inside itself or its descendants",
                        ));
                    }
                    let target_branch = heading_branch(&core, index).map_err(branch_error)?;
                    let level = rows[index]
                        .heading_level
                        .expect("a store heading row always carries its decoded level");
                    (
                        target_branch.range.end,
                        source_is_heading.then_some(level + 1),
                    )
                }
                MoveTarget::Root => (0, source_is_heading.then_some(2)),
            };

            let moved = if let Some(level) = aligned_root_level {
                // Align exactly the dragged pre-move range. Re-resolving
                // the branch after insertion could absorb an adjacent
                // section when the old root level closed on it but the
                // new, deeper root level does not.
                let aligned = shift_branch_to_level(&core, source_index, level)
                    .map_err(|error| level_error(error, "heading_level"))?;
                move_range(&aligned, source_range.clone(), target_index).map_err(reorder_error)?
            } else if source_is_heading {
                move_branch(&core, source_index, target_index).map_err(reorder_error)?
            } else {
                move_range(&core, source_range.clone(), target_index).map_err(reorder_error)?
            };

            if moved == core {
                return Ok(());
            }
            let moved_count = source_range.len();
            apply_core_items(transaction, &rows, &moved)?;
            let (action, entity_type) = if source_is_heading {
                (Action::HeadingChanged, EntityType::Heading)
            } else {
                (Action::InstructionChanged, EntityType::Instruction)
            };
            journal(
                transaction,
                mirror,
                action,
                entity_type,
                &id,
                Some(Details::Bulk {
                    changed: u64::try_from(moved_count)
                        .expect("a moved library range cannot exceed u64 rows"),
                }),
            )?;
            Ok(())
        })
        .await
}
