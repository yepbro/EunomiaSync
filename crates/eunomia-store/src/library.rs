//! Library repository (spec §6.2 `library_items`,
//! `instruction_variants`, `project_instructions`; §6.3 invariants
//! DB-005/DB-010/DB-011/DB-012/DB-013, §6.4 deleting and archiving;
//! task T-081).
//!
//! One module serves the three library tables because they share one
//! feature boundary — the instruction library of §7.3 (items and
//! their variants) and the assignment matrix of §7.4 (one variant of
//! one instruction per project) — and one error shape: the conflicts
//! below are runtime outcomes between individually legal rows, typed
//! here instead of string-matched.
//!
//! # Positions: one dense sequence over the whole item table
//!
//! `library_items.position` follows DB-005/DB-011: an integer
//! starting from 1 with no gaps, headings and instructions in one
//! shared ordered list (§7.3: one structure for all projects). The
//! migration's CHECK enforces only `position >= 1` (D-036); the
//! no-gaps half is this module's job:
//!
//! * [`create_item`] appends `MAX(position) + 1` — a scalar subquery
//!   *inside* the single `INSERT` (the [`crate::projects::create`]
//!   shape), so even a bare call cannot interleave;
//! * [`move_item`] renumbers the affected range (DB-005's own
//!   wording) and rejects a target outside `1..=N` with
//!   [`LibraryError::PositionOutOfRange`];
//! * [`set_item_order`] persists an exact whole-table permutation in
//!   one caller-owned transaction, the ALG-061 branch-move endpoint;
//! * [`delete_heading`] physically removes the row and closes the gap
//!   by shifting the tail down (§6.4: the section's instructions stay
//!   in place — positionally they shift up; structurally they join
//!   the previous section, which is the caller's reading);
//! * hiding keeps the row's position (§6.4 says hide keeps the data;
//!   keeping the place is the least-destructive reading — the archive
//!   precedent of [`crate::projects::archive`]), so the sequence is
//!   table-wide, not active-only.
//!
//! # Variants: DB-010 made constructive
//!
//! DB-010 ("each active instruction has at least one active variant
//! and exactly one active main variant") is only half enforceable in
//! DDL: the partial index `ux_variant_main` caps the main count, but
//! nothing schema-side demands a main exists. This module keeps the
//! other half *constructive* — an API caller cannot talk it out of
//! the invariant:
//!
//! * the first active variant of an instruction must be its main
//!   variant ([`LibraryError::MainRequired`]); a second active main
//!   is [`LibraryError::MainVariantExists`] (the index stays the
//!   floor);
//! * [`delete_variant`] refuses the last active variant
//!   ([`LibraryError::LastVariant`], LIB-025) and
//!   demotes-then-promotes when the main variant is deleted, so a
//!   window with two active mains never opens;
//! * [`create_item`] on an instruction deliberately does **not**
//!   create a variant: a bare instruction row violates DB-010 until
//!   its main variant lands. The §7.3 create dialog (LIB-002) always
//!   produces the pair; the command task composes [`create_item`] +
//!   [`create_variant`] in its one transaction — the same
//!   producer-composition boundary as archive + exclusion (D-093).
//!
//! # Deleting a variant moves its assignments first (DB-012)
//!
//! DB-012: "before it is deleted, all its assignments are moved to
//! another variant" — [`delete_variant`] does exactly that, in the
//! caller's transaction: the assignments of the deleted variant are
//! re-pointed at the replacement, then the variant soft-deletes. No
//! assignment disappears silently (the LIB-023 acceptance criterion).
//! **One** replacement takes over all of the deleted variant's
//! assignments; LIB-023's per-project choices compose [`reassign`]
//! calls after the delete, in the same transaction. The dialog's
//! default ("the main one by default") is the repository's default
//! too: `replacement: None` targets the current active main variant.
//! Naming a replacement that is not an active sibling of the same
//! instruction is [`LibraryError::ReplacementInvalid`]; deleting the
//! main variant without naming a successor is
//! [`LibraryError::ReplacementRequired`] (LIB-023: deleting the main
//! variant requires choosing a new main one); an instruction whose
//! variants have no main at all (DB-010 already broken by
//! out-of-band data) is surfaced as [`LibraryError::NoActiveMain`],
//! never guessed at (DB-017).
//!
//! # Assignments
//!
//! One variant of one instruction per project (D-02, the composite
//! PRIMARY KEY): [`assign`] rejects a second assignment of the same
//! instruction to the same project with
//! [`LibraryError::AlreadyAssigned`]; changing the variant is
//! [`reassign`] (MAIN-005: other projects keep theirs); removing it
//! is [`unassign`] (LIB-020: without deleting the variant). The
//! variant must be an *active* variant of *this* instruction (DB-012;
//! [`LibraryError::VariantNotAssignable`]) — the composite FOREIGN
//! KEY is the schema's floor, the typed pre-check is this module's
//! contract. A hidden instruction keeps and gains assignments
//! (DB-013: it is excluded from the build, not from the matrix data);
//! headings have no cells (MAIN-001) and assigning one is
//! [`LibraryError::NotAnInstruction`].
//!
//! # What this module does not do
//!
//! * It does not re-validate the §6.9 *field* rules (title/name
//!   lengths, the DB-006 content limit, the DB-016 marker lines):
//!   rejection-style validation lives in `eunomia_core` at the
//!   command boundary; the schema CHECKs are the floor here and
//!   surface as [`LibraryError::Sql`]. The §6.9 clauses that are
//!   inherently store-owned are implemented here (D-076's split):
//!   the main-instruction-name uniqueness among *visible*
//!   instructions, case-insensitive (a cross-row query —
//!   [`LibraryError::DuplicateInstructionTitle`], checked by
//!   [`create_item`]/[`update_item`]/[`restore_item`]), and the
//!   save-time CRLF→LF fold of variant text (what is stored is the
//!   folded form, so the DB-006 CHECK measures it). The NOCASE twin
//!   of an active variant name
//!   ([`LibraryError::DuplicateVariantName`], the partial index
//!   `ux_variant_name`'s scope) and the DB-010/DB-012/LIB-025 rules
//!   the schema cannot express are typed here as well.
//! * It does not journal (D-051: the producer writes the `action_log`
//!   row at its `Db::run` boundary, the [`crate::settings::set`]
//!   precedent) and does not write revisions (§9.8 is the history
//!   task's; LIB-026/027's "version on a real change" composes over
//!   these functions later).
//! * It does not own cross-entity compositions: resolving and moving a
//!   heading's whole branch (LIB-032/ALG-060…062) is a command-layer
//!   composition over the core algorithms and [`set_item_order`]; bulk
//!   matrix operations (§7.4) and the LIB-023 *per-project* replacement
//!   choices compose [`assign`]/[`delete_variant`]+[`reassign`], each
//!   in one transaction.
//!
//! # `updated_at` follows the snapshot notion of a real change
//!
//! §9.8/D-16 exclude the main-variant flag and the library order from
//! snapshots; `updated_at` follows the same line (the
//! [`crate::projects::reorder`] precedent — its renumbering writes no
//! `updated_at` either): renaming an item, changing a heading level,
//! renaming or editing a variant, and every state transition
//! (hide/restore, delete/restore) restamp the row; a pure position
//! move, the renumbering it shifts neighbors by, and a main-flag
//! promotion restamp nothing.
//!
//! # Transactions, signatures, corrupt cells
//!
//! Every function joins the caller's transaction (D-037: the
//! app-layer executor supplies the one `BEGIN DEFERRED`; D-051's
//! executor-agnostic rule). Single-statement writers are atomic on
//! their own; the multi-statement ones ([`move_item`],
//! [`delete_heading`], [`set_main_variant`], [`delete_variant`]) are
//! atomic **only** inside the caller's transaction — their docs say
//! so, and the integration suite pins the rollbacks. All signatures
//! stay TEXT and store-local enums (D-017/D-093: no `StorageError`
//! nesting — the first command task maps `LibraryError` beside the
//! errors it already unwraps; the crate's one `eunomia-core` edge
//! lives in [`crate::build_snapshot`], wired by its first core
//! consumer, D-105 — this module keeps TEXT in/out). A stored
//! cell outside its closed vocabulary (`kind`, `state`, a variant
//! `state`, the heading-level matrix) fails the read with
//! [`LibraryError::Member`] — corruption or a newer app version
//! (D-039), never a value to guess at.

use std::collections::HashSet;
use std::fmt;

use rusqlite::Connection;
use rusqlite::OptionalExtension;

use crate::values::{new_id, now_utc};

// ---- Vocabularies -------------------------------------------------------------

/// What a library item is (§6.2 `library_items.kind`).
///
/// The two kinds are immutable: an item is born an instruction or a
/// heading and stays it (v1 has no kind-changing operation; LIB-033's
/// "Change level" edits a heading's level, not its kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// A library instruction with variants and assignments (§7.3).
    Instruction,
    /// A section heading of level 2–6 structuring the shared list
    /// (LIB-030…034). Headings have no variants, no assignments and
    /// cannot be hidden (§6.2 CHECK).
    Heading,
}

impl ItemKind {
    /// The stored member string (§6.2 CHECK vocabulary).
    pub const fn as_str(&self) -> &'static str {
        match self {
            ItemKind::Instruction => "instruction",
            ItemKind::Heading => "heading",
        }
    }

    /// Parses a stored member string, rejecting anything outside the
    /// CHECK vocabulary (case-sensitive, like the schema).
    fn from_member(member: &str) -> Result<Self, LibraryError> {
        match member {
            "instruction" => Ok(ItemKind::Instruction),
            "heading" => Ok(ItemKind::Heading),
            _ => Err(LibraryError::Member {
                column: "kind",
                found: member.to_owned(),
            }),
        }
    }
}

/// The lifecycle state of a library item (§6.2
/// `library_items.state`).
///
/// v1 has no physical instruction delete (§6.4): an instruction
/// leaves the visible library only by entering
/// [`ItemState::Hidden`], and returns only by restore — with its
/// variants and assignments (DB-013). Headings are always active
/// (the §6.2 CHECK `kind = 'instruction' OR state = 'active'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemState {
    /// The item shows in the library and takes part in builds.
    Active,
    /// Hidden (LIB-005…007, 012): data, variants and assignments
    /// stay, the item is not written into files (D-11).
    Hidden,
}

impl ItemState {
    /// The stored member string (§6.2 CHECK vocabulary).
    pub const fn as_str(&self) -> &'static str {
        match self {
            ItemState::Active => "active",
            ItemState::Hidden => "hidden",
        }
    }

    /// Parses a stored member string (see [`ItemKind::from_member`]).
    fn from_member(member: &str) -> Result<Self, LibraryError> {
        match member {
            "active" => Ok(ItemState::Active),
            "hidden" => Ok(ItemState::Hidden),
            _ => Err(LibraryError::Member {
                column: "state",
                found: member.to_owned(),
            }),
        }
    }
}

/// The lifecycle state of a variant (§6.2
/// `instruction_variants.state`).
///
/// Variant deletion is soft (§6.4): the row stays for the "Restore
/// variant" command (LIB-026/027) and frees its name for reuse (the
/// `ux_variant_name` partial index covers active rows only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VariantState {
    /// The variant can be assigned (DB-012).
    Active,
    /// Soft-deleted (LIB-023…025); its assignments were moved to a
    /// sibling before the delete.
    Deleted,
}

impl VariantState {
    /// The stored member string (§6.2 CHECK vocabulary).
    pub const fn as_str(&self) -> &'static str {
        match self {
            VariantState::Active => "active",
            VariantState::Deleted => "deleted",
        }
    }

    /// Parses a stored member string (see [`ItemKind::from_member`]).
    fn from_member(member: &str) -> Result<Self, LibraryError> {
        match member {
            "active" => Ok(VariantState::Active),
            "deleted" => Ok(VariantState::Deleted),
            _ => Err(LibraryError::Member {
                column: "state",
                found: member.to_owned(),
            }),
        }
    }
}

/// Which rows [`list_items`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemScope {
    /// Every row, active and hidden — the "Show hidden" view (D-14)
    /// and diagnostics.
    All,
    /// Only `state = 'active'` rows: the visible library and the
    /// build. Headings are always active, so they are always here.
    Active,
    /// Only `state = 'hidden'` rows — the "Restore" surface
    /// (LIB-005…007).
    Hidden,
}

// ---- Row models -----------------------------------------------------------------

/// The columns of a `library_items` row, in the fixed order every
/// query of this module selects them in — one list, so the read
/// mapping and the SQL cannot drift apart.
const ITEM_COLUMNS: &str =
    "id, kind, position, heading_level, title, state, created_at, updated_at, hidden_at";

/// A stored library item row (§6.2 `library_items`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryItem {
    /// Row id, UUIDv7 TEXT (DB-002), generated by [`create_item`].
    pub id: String,
    /// Instruction or heading; immutable after creation.
    pub kind: ItemKind,
    /// The DB-005/DB-011 order position, gapless `1..=N` over the
    /// whole table; owned by [`create_item`], [`move_item`] and
    /// [`delete_heading`], never by [`update_item`].
    pub position: i64,
    /// `Some(level in 2..=6)` for a heading, `None` for an
    /// instruction (the §6.2 CHECK matrix; LIB-030).
    pub heading_level: Option<u8>,
    /// The display title (schema CHECK: 1–200 characters after
    /// trimming; §6.9).
    pub title: String,
    /// Active or hidden (§6.4).
    pub state: ItemState,
    /// Creation time, DB-002 UTC ([`crate::values::now_utc`]).
    pub created_at: String,
    /// Last snapshot-visible write of this row by this module (see
    /// the module docs for what restamps and what does not).
    pub updated_at: String,
    /// When the item was hidden (§6.4); `None` while active. The
    /// invariant `hidden_at IS NOT NULL ⟺ state = 'hidden'` is
    /// maintained by [`hide_item`]/[`restore_item`].
    pub hidden_at: Option<String>,
}

/// The raw cells of one `library_items` row, as rusqlite read them —
/// the decode step is fallible on the vocabulary columns, and the
/// `query_map` closure must stay infallible (the [`crate::projects`]
/// `ProjectRow` pattern).
struct ItemRow {
    id: String,
    kind: String,
    position: i64,
    heading_level: Option<i64>,
    title: String,
    state: String,
    created_at: String,
    updated_at: String,
    hidden_at: Option<String>,
}

impl ItemRow {
    /// Reads one row in the fixed [`ITEM_COLUMNS`] order.
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(ItemRow {
            id: row.get(0)?,
            kind: row.get(1)?,
            position: row.get(2)?,
            heading_level: row.get(3)?,
            title: row.get(4)?,
            state: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            hidden_at: row.get(8)?,
        })
    }

    /// Decodes the vocabulary columns into the typed model.
    ///
    /// # Errors
    ///
    /// Returns [`LibraryError::Member`] for a cell outside its closed
    /// vocabulary — the kind/state members or the heading-level
    /// matrix (corruption or a newer app version, D-039; the CHECKs
    /// make such cells unreachable through this module).
    fn into_item(self) -> Result<LibraryItem, LibraryError> {
        let kind = ItemKind::from_member(&self.kind)?;
        let state = ItemState::from_member(&self.state)?;
        let heading_level = decode_heading_level(kind, self.heading_level)?;
        Ok(LibraryItem {
            id: self.id,
            kind,
            position: self.position,
            heading_level,
            title: self.title,
            state,
            created_at: self.created_at,
            updated_at: self.updated_at,
            hidden_at: self.hidden_at,
        })
    }
}

/// Decodes `heading_level` against the item's kind: an instruction
/// carries `NULL`, a heading a level in `2..=6` — the §6.2 CHECK
/// matrix, mirrored on the read so a corrupt cell fails loudly
/// instead of decoding into a lie.
fn decode_heading_level(kind: ItemKind, cell: Option<i64>) -> Result<Option<u8>, LibraryError> {
    match (kind, cell) {
        (ItemKind::Instruction, None) => Ok(None),
        (ItemKind::Instruction, Some(level)) => Err(LibraryError::Member {
            column: "heading_level",
            found: level.to_string(),
        }),
        (ItemKind::Heading, Some(level)) if (2..=6).contains(&level) => {
            Ok(Some(u8::try_from(level).expect("a level in 2..=6 fits u8")))
        }
        (ItemKind::Heading, cell) => Err(LibraryError::Member {
            column: "heading_level",
            found: cell.map_or_else(|| String::from("NULL"), |level| level.to_string()),
        }),
    }
}

/// The input of [`create_item`], as a kind-carrying enum: the
/// kind/level matrix of §6.2 is unrepresentable wrong at the type
/// level — a heading cannot be born without its level, an instruction
/// cannot be born with one. Only the level's `2..=6` range stays a
/// runtime concern (the CHECK floor surfaces it as
/// [`LibraryError::Sql`]; the §6.9 validators own the friendly
/// message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewLibraryItem<'a> {
    /// A new instruction; its main variant is a separate
    /// [`create_variant`] call — see the module docs for the DB-010
    /// composition.
    Instruction {
        /// The display title (§6.9: 1–200 characters after trimming).
        title: &'a str,
    },
    /// A new section heading (LIB-030).
    Heading {
        /// The display title (§6.9).
        title: &'a str,
        /// The heading level; `2..=6` (schema CHECK).
        level: u8,
    },
}

/// The input of [`create_variant`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVariant<'a> {
    /// The instruction this variant belongs to; must exist and be an
    /// instruction ([`LibraryError::NotFound`] /
    /// [`LibraryError::NotAnInstruction`] otherwise).
    pub instruction_id: &'a str,
    /// The variant name (schema CHECK: 1–100 characters after
    /// trimming; unique among the instruction's active variants
    /// NOCASE — §6.9, the `ux_variant_name` scope).
    pub name: &'a str,
    /// The variant text (DB-006: at most 262144 characters, schema
    /// CHECK). May be empty: an empty variant is assignable
    /// (LIB-014/015). §6.9's save-time CRLF→LF fold applies before
    /// the store (D-076): the stored form is the folded form.
    pub content: &'a str,
    /// Whether this is the instruction's main variant. `true` is
    /// rejected when an active main already exists, `false` when none
    /// does (DB-010, kept constructive — see the module docs).
    pub is_main: bool,
}

/// A stored variant row (§6.2 `instruction_variants`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// Row id, UUIDv7 TEXT (DB-002), generated by [`create_variant`].
    pub id: String,
    /// The owning instruction's id.
    pub instruction_id: String,
    /// The variant name (§6.9).
    pub name: String,
    /// The variant text (DB-006).
    pub content: String,
    /// The main-variant flag (DB-010; not part of snapshots, D-16 —
    /// flipping it does not restamp `updated_at`).
    pub is_main: bool,
    /// Active or soft-deleted (§6.4).
    pub state: VariantState,
    /// Creation time, DB-002 UTC.
    pub created_at: String,
    /// Last snapshot-visible write of this row by this module.
    pub updated_at: String,
    /// When the variant was soft-deleted; `None` while active. The
    /// invariant `deleted_at IS NOT NULL ⟺ state = 'deleted'` is
    /// maintained by [`delete_variant`]/[`restore_variant`].
    pub deleted_at: Option<String>,
}

/// The raw cells of one `instruction_variants` row (see [`ItemRow`]).
struct VariantRow {
    id: String,
    instruction_id: String,
    name: String,
    content: String,
    is_main: bool,
    state: String,
    created_at: String,
    updated_at: String,
    deleted_at: Option<String>,
}

/// The columns of a `instruction_variants` row, in the fixed read
/// order.
const VARIANT_COLUMNS: &str =
    "id, instruction_id, name, content, is_main, state, created_at, updated_at, deleted_at";

impl VariantRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(VariantRow {
            id: row.get(0)?,
            instruction_id: row.get(1)?,
            name: row.get(2)?,
            content: row.get(3)?,
            is_main: row.get(4)?,
            state: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            deleted_at: row.get(8)?,
        })
    }

    fn into_variant(self) -> Result<Variant, LibraryError> {
        Ok(Variant {
            id: self.id,
            instruction_id: self.instruction_id,
            name: self.name,
            content: self.content,
            is_main: self.is_main,
            state: VariantState::from_member(&self.state)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
        })
    }
}

/// The input of [`assign`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAssignment<'a> {
    /// The project receiving the assignment; must exist
    /// ([`LibraryError::NotFound`] otherwise).
    pub project_id: &'a str,
    /// The instruction being assigned; must exist and be an
    /// instruction ([`LibraryError::NotAnInstruction`] for a heading).
    pub instruction_id: &'a str,
    /// The assigned variant; must be an active variant of this
    /// instruction (DB-012).
    pub variant_id: &'a str,
}

/// A stored assignment row (§6.2 `project_instructions`, §7.4): one
/// variant of one instruction per project (D-02).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// The assigned project.
    pub project_id: String,
    /// The assigned instruction.
    pub instruction_id: String,
    /// The assigned variant (DB-012: an active variant of
    /// `instruction_id`).
    pub variant_id: String,
    /// Creation time, DB-002 UTC.
    pub created_at: String,
    /// Last write of this row (a [`reassign`] or a
    /// [`delete_variant`] moving the assignment to a replacement).
    pub updated_at: String,
}

// ---- Errors ----------------------------------------------------------------------

/// Why a library operation failed.
///
/// Returned **directly** by every function of this module — not
/// nested in [`crate::storage::StorageError`] (the D-093 boundary:
/// the app layer matches `StorageError` exhaustively, so the
/// repository's typed error stays beside it until a task allowed to
/// edit `src-tauri` normalizes the shapes).
///
/// The typed variants are the runtime outcomes between individually
/// legal rows and the service-layer halves of §6.3 the schema cannot
/// express (DB-010/DB-011/DB-012, LIB-023/025). Everything else a
/// caller can hit — §6.9 values that bypassed the `eunomia_core`
/// validators (the schema CHECK floor, DB-006 included), a closed
/// connection, a tampered schema — is [`LibraryError::Sql`].
#[derive(Debug)]
pub enum LibraryError {
    /// No row with this id. Produced by the by-id writers whose
    /// successful outcome is "the row now has this shape". For the
    /// pair-keyed assignment writers the id is the
    /// `"<project_id>/<instruction_id>"` pair.
    NotFound {
        /// The id that matched nothing.
        id: String,
    },
    /// The `library_items` row exists but is a heading; the operation
    /// is defined on instructions only ([`create_variant`],
    /// [`assign`]). Headings have no variants and no cells (MAIN-001).
    NotAnInstruction {
        /// The heading's id.
        id: String,
    },
    /// The `library_items` row exists but is an instruction;
    /// [`delete_heading`] is the one physical delete of §6.4, and it
    /// is headings-only.
    NotAHeading {
        /// The instruction's id.
        id: String,
    },
    /// An active (visible) instruction already carries the title
    /// NOCASE-equal to the new one — §6.9's main-instruction-name
    /// clause ("unique among visible instructions,
    /// case-insensitive"). The clause is a cross-row query, so it
    /// lives here and not in the `eunomia_core` field validators
    /// (D-076); a *hidden* twin does not block (it is not visible),
    /// and the conflict surfaces when the twin is restored
    /// ([`restore_item`]). Heading titles are exempt (the §6.9 row is
    /// the instruction name's).
    DuplicateInstructionTitle {
        /// The title that collides.
        title: String,
    },
    /// An active variant of this instruction already carries the name
    /// NOCASE-equal to the new one — the partial index
    /// `ux_variant_name`'s scope (§6.9). The name is freed by
    /// soft-deleting the twin and taken again by
    /// [`restore_variant`].
    DuplicateVariantName {
        /// The instruction the name was taken on.
        instruction_id: String,
        /// The name that collides.
        name: String,
    },
    /// [`create_variant`] with `is_main = true` while the
    /// instruction already has an active main variant (DB-010: exactly
    /// one). The partial index `ux_variant_main` is the floor.
    MainVariantExists {
        /// The instruction that already has a main variant.
        instruction_id: String,
    },
    /// [`create_variant`] with `is_main = false` while the
    /// instruction has no active main variant: the first active
    /// variant must be the main one (DB-010, the constructive half
    /// the schema cannot enforce).
    MainRequired {
        /// The instruction left without a main variant.
        instruction_id: String,
    },
    /// The last active variant of an instruction cannot be deleted
    /// (LIB-025; DB-010's "at least one active variant"). The
    /// instruction's variants are untouched.
    LastVariant {
        /// The instruction that would be left without variants.
        instruction_id: String,
    },
    /// [`delete_variant`] on the instruction's main variant without a
    /// named replacement: LIB-023 requires choosing a new main one.
    ReplacementRequired {
        /// The instruction whose main variant is being deleted.
        instruction_id: String,
    },
    /// The named replacement is not an active variant of the same
    /// instruction (unknown id, another instruction's variant, a
    /// soft-deleted sibling, or the variant being deleted itself).
    ReplacementInvalid {
        /// The instruction of the variant being deleted.
        instruction_id: String,
        /// The replacement id that failed the check.
        replacement: String,
    },
    /// [`delete_variant`] needed the instruction's active main variant
    /// as the default replacement and found none — DB-010 is already
    /// broken by out-of-band data (the flag is reachable by a plain
    /// `UPDATE … SET is_main = 0`, which violates no CHECK). DB-017's
    /// posture: surfaced, not repaired and not panicked on.
    NoActiveMain {
        /// The instruction left without a main variant.
        instruction_id: String,
    },
    /// [`move_item`] received a target position outside `1..=N` (N =
    /// the table's row count; DB-005). The table is untouched.
    PositionOutOfRange {
        /// The rejected target.
        requested: i64,
        /// The table's row count — the highest legal position.
        max: i64,
    },
    /// A whole-table order replacement did not name exactly the current
    /// number of rows.
    InvalidItemOrder {
        /// The table's row count.
        expected: usize,
        /// The number of ids supplied.
        actual: usize,
    },
    /// A whole-table order replacement named one row more than once.
    DuplicateOrderId {
        /// The repeated row id.
        id: String,
    },
    /// A whole-table order replacement named an id outside the table.
    UnknownOrderId {
        /// The unknown row id.
        id: String,
    },
    /// The project already has an assignment for this instruction
    /// (D-02, the composite PRIMARY KEY). Changing the variant is
    /// [`reassign`], removing it [`unassign`].
    AlreadyAssigned {
        /// The project that already has the instruction assigned.
        project_id: String,
        /// The instruction that is already assigned.
        instruction_id: String,
    },
    /// The variant is not assignable to this instruction (DB-012):
    /// unknown id, another instruction's variant, or a
    /// soft-deleted variant.
    VariantNotAssignable {
        /// The instruction being assigned.
        instruction_id: String,
        /// The variant that failed the check.
        variant_id: String,
    },
    /// The variant row exists but is soft-deleted; the operation needs
    /// an active variant ([`update_variant`], [`set_main_variant`]).
    VariantNotActive {
        /// The soft-deleted variant's id.
        id: String,
    },
    /// A stored cell holds a value outside its closed vocabulary
    /// (`kind`, `state`, a variant `state`, the heading-level matrix).
    /// Every writer goes through this module, so this means
    /// corruption or a row written by a newer app version; the read
    /// surfaces it instead of guessing (the D-039 corrupt-cells
    /// policy).
    Member {
        /// The column the value came from.
        column: &'static str,
        /// The offending value.
        found: String,
    },
    /// The statement or query itself failed — a schema CHECK rejection
    /// (a §6.9 value that bypassed the validators; see the module
    /// docs for the validation split), a closed connection, a tampered
    /// schema. Everything SQLite reports that is not one of the typed
    /// outcomes above.
    Sql(rusqlite::Error),
}

impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryError::NotFound { id } => write!(f, "no row with id {id:?}"),
            LibraryError::NotAnInstruction { id } => {
                write!(f, "library item {id:?} is not an instruction")
            }
            LibraryError::NotAHeading { id } => write!(f, "library item {id:?} is not a heading"),
            LibraryError::DuplicateInstructionTitle { title } => {
                write!(f, "a visible instruction titled {title:?} already exists")
            }
            LibraryError::DuplicateVariantName {
                instruction_id,
                name,
            } => {
                write!(
                    f,
                    "an active variant named {name:?} already exists on instruction \
                     {instruction_id:?}"
                )
            }
            LibraryError::MainVariantExists { instruction_id } => write!(
                f,
                "instruction {instruction_id:?} already has an active main variant"
            ),
            LibraryError::MainRequired { instruction_id } => write!(
                f,
                "the first active variant of instruction {instruction_id:?} must be \
                 its main variant"
            ),
            LibraryError::LastVariant { instruction_id } => write!(
                f,
                "the last active variant of instruction {instruction_id:?} cannot be \
                 deleted"
            ),
            LibraryError::ReplacementRequired { instruction_id } => write!(
                f,
                "deleting the main variant of instruction {instruction_id:?} requires \
                 a named replacement"
            ),
            LibraryError::ReplacementInvalid {
                instruction_id,
                replacement,
            } => write!(
                f,
                "replacement {replacement:?} is not an active variant of instruction \
                 {instruction_id:?}"
            ),
            LibraryError::NoActiveMain { instruction_id } => write!(
                f,
                "instruction {instruction_id:?} has no active main variant (DB-010 is \
                 already broken; refusing to guess a default replacement)"
            ),
            LibraryError::PositionOutOfRange { requested, max } => {
                write!(f, "position {requested} is outside 1..={max}")
            }
            LibraryError::InvalidItemOrder { expected, actual } => {
                write!(f, "a whole-table order needs {expected} ids, got {actual}")
            }
            LibraryError::DuplicateOrderId { id } => {
                write!(f, "whole-table order repeats id {id:?}")
            }
            LibraryError::UnknownOrderId { id } => {
                write!(f, "whole-table order names unknown id {id:?}")
            }
            LibraryError::AlreadyAssigned {
                project_id,
                instruction_id,
            } => write!(
                f,
                "project {project_id:?} already has an assignment for instruction \
                 {instruction_id:?}"
            ),
            LibraryError::VariantNotAssignable {
                instruction_id,
                variant_id,
            } => write!(
                f,
                "variant {variant_id:?} is not an active variant of instruction \
                 {instruction_id:?}"
            ),
            LibraryError::VariantNotActive { id } => {
                write!(f, "variant {id:?} is not active")
            }
            LibraryError::Member { column, found } => {
                write!(f, "{found:?} is not a valid {column} value")
            }
            LibraryError::Sql(err) => write!(f, "database error: {err}"),
        }
    }
}

impl std::error::Error for LibraryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LibraryError::Sql(err) => Some(err),
            _ => None,
        }
    }
}

/// Lets `?` lift a rusqlite failure into the typed error.
impl From<rusqlite::Error> for LibraryError {
    fn from(err: rusqlite::Error) -> Self {
        LibraryError::Sql(err)
    }
}

// ---- Shared read helpers ----------------------------------------------------------

/// Runs a single-row item query and decodes the row, mapping a
/// corrupt vocabulary cell to the typed error (D-039).
fn query_optional_item(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Option<LibraryItem>, LibraryError> {
    let mut statement = conn.prepare(sql)?;
    statement
        .query_row(params, ItemRow::from_row)
        .optional()?
        .map(ItemRow::into_item)
        .transpose()
}

/// Runs a single-row variant query and decodes the row (see
/// [`query_optional_item`]).
fn query_optional_variant(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Option<Variant>, LibraryError> {
    let mut statement = conn.prepare(sql)?;
    statement
        .query_row(params, VariantRow::from_row)
        .optional()?
        .map(VariantRow::into_variant)
        .transpose()
}

/// Runs a single-row assignment query. The row has no vocabulary
/// columns, so the decode is infallible.
fn query_optional_assignment(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Option<Assignment>, LibraryError> {
    let mut statement = conn.prepare(sql)?;
    statement
        .query_row(params, |row| {
            Ok(Assignment {
                project_id: row.get(0)?,
                instruction_id: row.get(1)?,
                variant_id: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })
        .optional()
        .map_err(LibraryError::Sql)
}

/// Reads the row for a conditional writer whose `UPDATE` matched
/// nothing: the existing row (idempotent re-run) or
/// [`LibraryError::NotFound`] (no such id) — the
/// [`crate::projects`] `existing_row` pattern.
fn existing_item(conn: &Connection, id: &str) -> Result<LibraryItem, LibraryError> {
    get_item(conn, id)?.ok_or_else(|| LibraryError::NotFound { id: id.to_owned() })
}

// ---- Items: create and read -------------------------------------------------------

/// Inserts a library item and returns the stored row.
///
/// The row id is a fresh UUIDv7 TEXT and the timestamps are stamped
/// with the DB-002 UTC format; `position` is `MAX(position) + 1` over
/// the whole table (1 for the first row) — computed by a scalar
/// subquery *inside* the `INSERT`, so the append is one statement and
/// cannot interleave even outside a transaction (the
/// [`crate::projects::create`] shape). `state` starts `active`,
/// `hidden_at` starts `NULL`.
///
/// A new instruction is born without variants, which leaves DB-010
/// unmet until its main variant lands: the §7.3 create dialog always
/// produces the pair, and the command task composes this call with
/// [`create_variant`] in one transaction (see the module docs).
/// An instruction title NOCASE-equal to a *visible* instruction's is
/// rejected ([`LibraryError::DuplicateInstructionTitle`], §6.9's
/// cross-row clause — this layer's job per D-076); heading titles are
/// exempt from the clause.
///
/// # Errors
///
/// Returns [`LibraryError::DuplicateInstructionTitle`] for a visible
/// instruction-name twin, and [`LibraryError::Sql`] for a schema CHECK
/// rejection — a §6.9 value that bypassed the validators, or a heading
/// level outside `2..=6` (the store's floor; see the module docs for
/// the validation split).
pub fn create_item(
    conn: &Connection,
    new: &NewLibraryItem<'_>,
) -> Result<LibraryItem, LibraryError> {
    if let NewLibraryItem::Instruction { title } = new
        && active_title_taken(conn, title, None)?
    {
        return Err(LibraryError::DuplicateInstructionTitle {
            title: (*title).to_owned(),
        });
    }
    let id = new_id();
    let (kind, heading_level, title) = match new {
        NewLibraryItem::Instruction { title } => ("instruction", None, *title),
        NewLibraryItem::Heading { title, level } => ("heading", Some(i64::from(*level)), *title),
    };
    conn.execute(
        "INSERT INTO library_items (
             id, kind, position, heading_level, title, state,
             created_at, updated_at, hidden_at
         ) VALUES (
             ?1, ?2, (SELECT COALESCE(MAX(position), 0) + 1 FROM library_items),
             ?3, ?4, 'active', ?5, ?5, NULL
         )",
        rusqlite::params![id, kind, heading_level, title, now_utc()],
    )?;
    // Unreachable while the INSERT above succeeded (one connection,
    // D-037 — no concurrent delete exists); a reasoned panic (the
    // house precedent for broken invariants) instead of a misleading
    // NotFound.
    Ok(get_item(conn, &id)?.expect("the row just inserted must be readable"))
}

/// Reads one library item by id; `None` when no row matches.
///
/// # Errors
///
/// Returns [`LibraryError::Member`] when a stored vocabulary cell is
/// corrupt (D-039), and [`LibraryError::Sql`] when the query fails.
pub fn get_item(conn: &Connection, id: &str) -> Result<Option<LibraryItem>, LibraryError> {
    query_optional_item(
        conn,
        &format!("SELECT {ITEM_COLUMNS} FROM library_items WHERE id = ?1"),
        [id],
    )
}

/// Lists library items in `position` order (the DB-011 shared
/// sequence), filtered by [`ItemScope`]. The active list is a dense
/// `1..=N`-ordered subset only while nothing is hidden: hiding keeps
/// the row's position (see the module docs), so read an active list
/// as an ordered subset, never as `1..=K`.
///
/// # Errors
///
/// Returns [`LibraryError::Member`] when any returned row has a
/// corrupt vocabulary cell (D-039), and [`LibraryError::Sql`] when
/// the query fails.
pub fn list_items(conn: &Connection, scope: ItemScope) -> Result<Vec<LibraryItem>, LibraryError> {
    let sql = match scope {
        ItemScope::All => {
            format!("SELECT {ITEM_COLUMNS} FROM library_items ORDER BY position")
        }
        ItemScope::Active => format!(
            "SELECT {ITEM_COLUMNS} FROM library_items \
             WHERE state = 'active' ORDER BY position"
        ),
        ItemScope::Hidden => format!(
            "SELECT {ITEM_COLUMNS} FROM library_items \
             WHERE state = 'hidden' ORDER BY position"
        ),
    };
    let mut statement = conn.prepare(&sql)?;
    let rows = statement
        .query_map([], ItemRow::from_row)?
        .map(|raw| {
            raw.map_err(LibraryError::Sql)
                .and_then(|row| row.into_item())
        })
        .collect::<Result<Vec<_>, LibraryError>>()?;
    Ok(rows)
}

// ---- Items: update, hide/restore, move, delete -------------------------------------

/// The mutable fields of a library item, as an update instruction:
/// `None` leaves the field unchanged.
///
/// Deliberately absent: `kind` (immutable — see [`ItemKind`]),
/// `position` ([`move_item`] owns it), `state`
/// ([`hide_item`]/[`restore_item`] own it), `created_at`, and any way
/// to clear a heading's level (a heading without a level is not a
/// representable row, §6.2 CHECK).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ItemUpdate<'a> {
    /// The new title, if it changes.
    pub title: Option<&'a str>,
    /// The heading's new level, if it changes (LIB-033 "Change
    /// level"; `2..=6`). Writing a level onto an instruction row hits
    /// the §6.2 CHECK and surfaces as [`LibraryError::Sql`].
    pub heading_level: Option<u8>,
}

/// Adds one SET assignment and its bound value to the dynamic UPDATE
/// of [`update_item`] / [`update_variant`] (the [`crate::projects`]
/// `push_field` shape).
fn push_field(
    assignments: &mut Vec<&'static str>,
    values: &mut Vec<Box<dyn rusqlite::ToSql>>,
    assignment: &'static str,
    value: Box<dyn rusqlite::ToSql>,
) {
    assignments.push(assignment);
    values.push(value);
}

/// Builds and runs the dynamic `UPDATE … WHERE id = ?` shared by
/// [`update_item`] and [`update_variant`]; returns the changed-row
/// count.
fn update_by_id(
    conn: &Connection,
    table: &'static str,
    assignments: &[&'static str],
    values: &mut Vec<Box<dyn rusqlite::ToSql>>,
    id: &str,
) -> Result<usize, LibraryError> {
    let sql = format!(
        "UPDATE {table} SET {} WHERE id = ?",
        assignments
            .iter()
            .map(|assignment| format!("{assignment} = ?"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    // The last placeholder is the WHERE clause's id.
    values.push(Box::new(id.to_owned()));
    let param_refs: Vec<&dyn rusqlite::ToSql> = values.iter().map(|value| value.as_ref()).collect();
    conn.execute(sql.as_str(), param_refs.as_slice())
        .map_err(LibraryError::Sql)
}

/// Applies an update to the library item with this id and returns the
/// stored row.
///
/// Exactly the fields present are written; `updated_at` is restamped
/// whenever at least one field is written, and an update with no
/// field present writes nothing and returns the row unchanged (the
/// [`crate::projects::update`] convention). A `heading_level` change
/// is a real snapshot change for a heading (the level shapes the
/// built file, ALG-020), so it restamps like a rename. A new
/// instruction title NOCASE-equal to another *visible* instruction's
/// is [`LibraryError::DuplicateInstructionTitle`] (§6.9, D-076): a
/// rename of a hidden instruction may legally twin it with a visible
/// one — the conflict surfaces when the item is restored
/// ([`restore_item`]).
///
/// # Errors
///
/// Returns [`LibraryError::NotFound`] when no row has this id,
/// [`LibraryError::DuplicateInstructionTitle`] for a taken visible
/// title, and [`LibraryError::Sql`] when the statement fails —
/// including the §6.2 CHECK floor for a level written onto an
/// instruction or a level outside `2..=6` (see the module docs for
/// the validation split).
pub fn update_item(
    conn: &Connection,
    id: &str,
    update: &ItemUpdate<'_>,
) -> Result<LibraryItem, LibraryError> {
    if let Some(title) = update.title
        && active_title_taken(conn, title, Some(id))?
    {
        return Err(LibraryError::DuplicateInstructionTitle {
            title: title.to_owned(),
        });
    }
    let mut assignments: Vec<&'static str> = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(title) = update.title {
        push_field(
            &mut assignments,
            &mut values,
            "title",
            Box::new(title.to_owned()),
        );
    }
    if let Some(level) = update.heading_level {
        push_field(
            &mut assignments,
            &mut values,
            "heading_level",
            Box::new(i64::from(level)),
        );
    }

    if assignments.is_empty() {
        // Nothing to write: a no-op must not churn `updated_at`.
        return existing_item(conn, id);
    }
    push_field(
        &mut assignments,
        &mut values,
        "updated_at",
        Box::new(now_utc()),
    );
    let changed = update_by_id(conn, "library_items", &assignments, &mut values, id)?;
    if changed == 0 {
        return Err(LibraryError::NotFound { id: id.to_owned() });
    }
    // Unreachable: the UPDATE above matched the row (one connection,
    // D-037 — no concurrent delete exists).
    Ok(get_item(conn, id)?.expect("the row just updated must be readable"))
}

/// Hides the instruction with this id (§6.4): `state` becomes
/// `hidden`, `hidden_at` is stamped with the DB-002 UTC time,
/// `updated_at` follows.
///
/// The hidden instruction keeps its variants, its assignments (DB-013
/// — it stays in the matrix data, just out of the build, D-11) and
/// its position in the table-wide sequence. §6.4's "Restore" is
/// [`restore_item`].
///
/// Idempotent: an already-hidden instruction is returned unchanged
/// (no restamp, no `hidden_at` drift — the
/// [`crate::projects::archive`] rule). Hiding a heading is not
/// representable (the §6.2 CHECK keeps headings active) and surfaces
/// as [`LibraryError::Sql`].
///
/// # Errors
///
/// Returns [`LibraryError::NotFound`] when no row has this id, and
/// [`LibraryError::Sql`] when the statement fails (the heading CHECK
/// included).
pub fn hide_item(conn: &Connection, id: &str) -> Result<LibraryItem, LibraryError> {
    let changed = conn.execute(
        "UPDATE library_items
         SET state = 'hidden', hidden_at = ?1, updated_at = ?1
         WHERE id = ?2 AND state = 'active'",
        rusqlite::params![now_utc(), id],
    )?;
    if changed == 0 {
        // Either the id matches nothing, or the row is already
        // hidden; the read tells the two apart.
        return existing_item(conn, id);
    }
    // Unreachable: the UPDATE above matched an active row (one
    // connection, D-037 — no concurrent delete exists).
    Ok(get_item(conn, id)?.expect("the row just hidden must be readable"))
}

/// Restores the hidden instruction with this id (LIB-005…007):
/// `state` returns to `active`, `hidden_at` is cleared (the invariant
/// `hidden_at IS NOT NULL ⟺ state = 'hidden'`), `updated_at` is
/// restamped. The instruction returns with its variants, its
/// assignments and its old position.
///
/// A title NOCASE-equal to a visible instruction's may have appeared
/// while the item was hidden (a hidden twin did not block the rename
/// or the create): restoring into it is
/// [`LibraryError::DuplicateInstructionTitle`] (§6.9, D-076) — rename
/// one of the two first.
///
/// Idempotent: an already-active item is returned unchanged — a
/// heading always is (it can never be hidden), so restoring one is
/// the documented no-op.
///
/// # Errors
///
/// Returns [`LibraryError::NotFound`] when no row has this id,
/// [`LibraryError::DuplicateInstructionTitle`] for a taken visible
/// title, and [`LibraryError::Sql`] when the statement fails.
pub fn restore_item(conn: &Connection, id: &str) -> Result<LibraryItem, LibraryError> {
    let item = existing_item(conn, id)?;
    if item.state != ItemState::Hidden {
        // Idempotent: an already-active item — a heading always is —
        // is returned unchanged.
        return Ok(item);
    }
    if active_title_taken(conn, &item.title, Some(id))? {
        return Err(LibraryError::DuplicateInstructionTitle { title: item.title });
    }
    let changed = conn.execute(
        "UPDATE library_items
         SET state = 'active', hidden_at = NULL, updated_at = ?1
         WHERE id = ?2 AND state = 'hidden'",
        rusqlite::params![now_utc(), id],
    )?;
    if changed == 0 {
        return existing_item(conn, id);
    }
    // Unreachable: the UPDATE above matched a hidden row (one
    // connection, D-037 — no concurrent delete exists).
    Ok(get_item(conn, id)?.expect("the row just restored must be readable"))
}

/// Moves the library item with this id to `to_position`, renumbering
/// the affected range so the sequence stays dense `1..=N` (DB-005:
/// "reordering renumbers the affected range in one transaction";
/// DB-011).
///
/// `to_position` must be inside `1..=N` with N the table's row count
/// (under the gapless invariant, `COUNT(*) == MAX(position)`); a
/// target outside is [`LibraryError::PositionOutOfRange`] and the
/// table is left untouched. A move to the item's current position
/// writes nothing. Position is table-wide metadata, so this neither
/// restamps `updated_at` nor cares about the item's state — a hidden
/// row moves like any other (see the module docs).
///
/// Atomicity: this is a multi-statement operation and joins the
/// caller's transaction like every function here — but unlike the
/// single-statement writers it is **only** atomic inside one. The
/// app-layer executor (D-037) supplies the transaction; the
/// integration suite pins the rollback through an explicit
/// transaction. Moving a heading's whole branch (LIB-032) is the
/// caller's composition of moves.
///
/// # Errors
///
/// Returns [`LibraryError::NotFound`] when no row has this id,
/// [`LibraryError::PositionOutOfRange`] for a target outside
/// `1..=N`, and [`LibraryError::Sql`] when a statement fails.
pub fn move_item(
    conn: &Connection,
    id: &str,
    to_position: i64,
) -> Result<LibraryItem, LibraryError> {
    let item = existing_item(conn, id)?;
    // The row count is the highest legal position under the gapless
    // invariant (DB-011) — and the safer bound if a corrupt gap ever
    // widened MAX past COUNT.
    let max: i64 = conn.query_row("SELECT COUNT(*) FROM library_items", [], |row| row.get(0))?;
    if !(1..=max).contains(&to_position) {
        return Err(LibraryError::PositionOutOfRange {
            requested: to_position,
            max,
        });
    }
    if item.position != to_position {
        // The affected range shifts by one; the moved row lands on
        // the target. The `position >= 1` CHECK holds throughout:
        // shifting up starts from `to_position >= 1`, shifting down
        // never touches row 1's position unless it is the moved row.
        if to_position < item.position {
            conn.execute(
                "UPDATE library_items
                 SET position = position + 1
                 WHERE position >= ?1 AND position < ?2",
                rusqlite::params![to_position, item.position],
            )?;
        } else {
            conn.execute(
                "UPDATE library_items
                 SET position = position - 1
                 WHERE position > ?1 AND position <= ?2",
                rusqlite::params![item.position, to_position],
            )?;
        }
        // Position-only: no `updated_at` (the reorder precedent, see
        // the module docs).
        conn.execute(
            "UPDATE library_items SET position = ?1 WHERE id = ?2",
            rusqlite::params![to_position, id],
        )?;
    }
    Ok(get_item(conn, id)?.expect("the row just moved must be readable"))
}

/// Replaces the complete table-wide item order with `ordered_ids` and
/// writes the DB-005/DB-011 positions `1..=N` in one caller-owned
/// transaction.
///
/// The input must be an exact permutation of the current `library_items`
/// ids. Length, duplicate and unknown-id refusals happen before the
/// first write. The schema has no UNIQUE index on `position`
/// (`docs/spec.md:258-269`), so each row may be updated independently;
/// intermediate duplicate positions inside the transaction are invisible
/// to other connections and the final sequence is dense. Position-only
/// writes never restamp `updated_at` (D-16).
///
/// Complexity: one ordered table read, one hash-set membership pass and
/// one prepared update per row — O(N) expected time and O(N) memory.
///
/// Atomicity: like [`move_item`], this multi-statement writer is atomic
/// only inside the caller's transaction (D-037).
///
/// # Errors
///
/// [`LibraryError::InvalidItemOrder`] for a length mismatch,
/// [`LibraryError::DuplicateOrderId`] for a repeated id,
/// [`LibraryError::UnknownOrderId`] for an id outside the table, and
/// [`LibraryError::Sql`] when a read or update fails.
pub fn set_item_order(conn: &Connection, ordered_ids: &[&str]) -> Result<(), LibraryError> {
    let existing = list_items(conn, ItemScope::All)?;
    if ordered_ids.len() != existing.len() {
        return Err(LibraryError::InvalidItemOrder {
            expected: existing.len(),
            actual: ordered_ids.len(),
        });
    }
    let existing_ids: HashSet<&str> = existing.iter().map(|item| item.id.as_str()).collect();
    let mut remaining = existing_ids.clone();
    for &id in ordered_ids {
        if !remaining.remove(id) {
            return Err(if existing_ids.contains(id) {
                LibraryError::DuplicateOrderId { id: id.to_owned() }
            } else {
                LibraryError::UnknownOrderId { id: id.to_owned() }
            });
        }
    }
    // Equal lengths plus every supplied id removed from the set imply an
    // empty remainder; keep the invariant explicit without a panic.
    if !remaining.is_empty() {
        return Err(LibraryError::InvalidItemOrder {
            expected: existing.len(),
            actual: ordered_ids.len(),
        });
    }

    let mut statement = conn.prepare("UPDATE library_items SET position = ?1 WHERE id = ?2")?;
    for (index, &id) in ordered_ids.iter().enumerate() {
        let position = i64::try_from(index + 1)
            .expect("a table row count read from SQLite fits a positive i64 position");
        statement.execute(rusqlite::params![position, id])?;
    }
    Ok(())
}

/// Physically deletes the heading with this id (§6.4: headings are
/// the library's one physical delete) and closes the gap by shifting
/// the tail down — the section's instructions stay in the list and
/// structurally become part of the previous section (LIB-030's
/// acceptance criterion).
///
/// Instructions have no physical delete in v1 (§6.4 — hide is their
/// lifecycle), so the kind is checked first:
/// [`LibraryError::NotAHeading`]. Nothing references a heading (no
/// variants, no assignments), so no RESTRICT can fire.
///
/// Atomicity: multi-statement (delete + renumber); atomic inside the
/// caller's transaction (D-037), pinned by the integration suite.
///
/// # Errors
///
/// Returns [`LibraryError::NotFound`] when no row has this id,
/// [`LibraryError::NotAHeading`] for an instruction, and
/// [`LibraryError::Sql`] when a statement fails.
pub fn delete_heading(conn: &Connection, id: &str) -> Result<(), LibraryError> {
    let item = existing_item(conn, id)?;
    if item.kind != ItemKind::Heading {
        return Err(LibraryError::NotAHeading { id: id.to_owned() });
    }
    conn.execute(
        "DELETE FROM library_items WHERE id = ?1 AND kind = 'heading'",
        [id],
    )?;
    // The renumbering is position-only: no `updated_at` (the module
    // docs' rule).
    conn.execute(
        "UPDATE library_items SET position = position - 1 WHERE position > ?1",
        [item.position],
    )?;
    Ok(())
}

// ---- Variants: helpers -------------------------------------------------------------

/// Whether the instruction has an active main variant (DB-010's
/// "exactly one" half under the constructive rules).
fn active_main_exists(conn: &Connection, instruction_id: &str) -> Result<bool, LibraryError> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM instruction_variants
             WHERE instruction_id = ?1 AND is_main = 1 AND state = 'active'
         )",
        [instruction_id],
        |row| row.get(0),
    )?;
    Ok(exists)
}

/// The id of the instruction's active main variant, if one exists.
fn active_main_id(conn: &Connection, instruction_id: &str) -> Result<Option<String>, LibraryError> {
    let id: Option<String> = conn
        .query_row(
            "SELECT id FROM instruction_variants
             WHERE instruction_id = ?1 AND is_main = 1 AND state = 'active'",
            [instruction_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(id)
}

/// Whether an active variant of the instruction already carries this
/// name NOCASE-equal — the partial index `ux_variant_name`'s scope.
/// `exclude` (a variant id) is exempt: it is the renaming variant
/// itself, keeping its own name must stay legal.
fn active_name_taken(
    conn: &Connection,
    instruction_id: &str,
    name: &str,
    exclude: Option<&str>,
) -> Result<bool, LibraryError> {
    // `IS NOT` is SQLite's null-safe inequality: bound NULL (no
    // exclusion) compares unequal to every real id, so the clause is
    // a no-op exactly when it should be.
    let taken: bool = conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM instruction_variants
             WHERE instruction_id = ?1 AND state = 'active'
               AND name = ?2 COLLATE NOCASE
               AND id IS NOT ?3
         )",
        rusqlite::params![instruction_id, name, exclude],
        |row| row.get(0),
    )?;
    Ok(taken)
}

/// How many active variants the instruction has besides `exclude`.
fn count_other_active_variants(
    conn: &Connection,
    instruction_id: &str,
    exclude: &str,
) -> Result<i64, LibraryError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM instruction_variants
         WHERE instruction_id = ?1 AND state = 'active' AND id IS NOT ?2",
        rusqlite::params![instruction_id, exclude],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Whether a *visible* (active) instruction already carries this
/// title NOCASE-equal — §6.9's main-instruction-name clause ("unique
/// among visible instructions, case-insensitive"), this layer's job
/// because it is a cross-row query (D-076). Hidden twins do not
/// block (not visible); the conflict surfaces on
/// [`restore_item`]. `exclude` (an item id) is exempt: it is the
/// renaming item itself. Heading titles are out of scope (the §6.9
/// row names the instruction name), so the probe matches
/// `kind = 'instruction'` rows only.
fn active_title_taken(
    conn: &Connection,
    title: &str,
    exclude: Option<&str>,
) -> Result<bool, LibraryError> {
    // `IS NOT` is SQLite's null-safe inequality: bound NULL (no
    // exclusion) compares unequal to every real id, so the clause is
    // a no-op exactly when it should be.
    let taken: bool = conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM library_items
             WHERE kind = 'instruction' AND state = 'active'
               AND title = ?1 COLLATE NOCASE
               AND id IS NOT ?2
         )",
        rusqlite::params![title, exclude],
        |row| row.get(0),
    )?;
    Ok(taken)
}

/// The save-time CRLF→LF fold of variant text (§6.9's variant-text
/// row: "CRLF is changed to LF on save"). This layer's own clause per
/// D-076's validation/normalization split: the REJECTION rules (NUL,
/// reserved markers, the length bound) are the `eunomia_core`
/// validators' — measured by the command task on the *folded* form —
/// and the build-time lone-CR/BOM/edge pass is ALG-002's. CRLF only:
/// a lone `\r` is deliberately left untouched here.
fn fold_crlf(content: &str) -> String {
    content.replace("\r\n", "\n")
}

// ---- Variants: create and read -----------------------------------------------------

/// Inserts a variant and returns the stored row.
///
/// The row id is a fresh UUIDv7 TEXT, the timestamps follow DB-002,
/// `state` starts `active`, `deleted_at` starts `NULL`. The DB-010
/// main rules are enforced constructively: `is_main = true` is
/// rejected with [`LibraryError::MainVariantExists`] when an active
/// main already exists, `is_main = false` with
/// [`LibraryError::MainRequired`] when none does (the first active
/// variant must be the main one). The NOCASE twin of an active
/// sibling's name is [`LibraryError::DuplicateVariantName`] (the
/// `ux_variant_name` scope — soft-deleting the twin frees the name).
///
/// The instruction row must exist and be an instruction: the foreign
/// key alone would accept a heading (it checks existence, not kind),
/// so the repository types the mismatch
/// ([`LibraryError::NotAnInstruction`], MAIN-001's "headings have no
/// cells"). Variants of a *hidden* instruction are legal — hide
/// keeps the variants (§6.4) and the rules above are state-blind.
///
/// # Errors
///
/// [`LibraryError::NotFound`] for an unknown instruction id,
/// [`LibraryError::NotAnInstruction`], [`LibraryError::MainRequired`],
/// [`LibraryError::MainVariantExists`],
/// [`LibraryError::DuplicateVariantName`], and
/// [`LibraryError::Sql`] for the CHECK floor (§6.9 name lengths,
/// DB-006 content limit) — see the module docs for the validation
/// split.
pub fn create_variant(conn: &Connection, new: &NewVariant<'_>) -> Result<Variant, LibraryError> {
    let item = get_item(conn, new.instruction_id)?.ok_or_else(|| LibraryError::NotFound {
        id: new.instruction_id.to_owned(),
    })?;
    if item.kind != ItemKind::Instruction {
        return Err(LibraryError::NotAnInstruction {
            id: new.instruction_id.to_owned(),
        });
    }
    if new.is_main {
        if active_main_exists(conn, new.instruction_id)? {
            return Err(LibraryError::MainVariantExists {
                instruction_id: new.instruction_id.to_owned(),
            });
        }
    } else if !active_main_exists(conn, new.instruction_id)? {
        return Err(LibraryError::MainRequired {
            instruction_id: new.instruction_id.to_owned(),
        });
    }
    if active_name_taken(conn, new.instruction_id, new.name, None)? {
        return Err(LibraryError::DuplicateVariantName {
            instruction_id: new.instruction_id.to_owned(),
            name: new.name.to_owned(),
        });
    }
    // §6.9's save-time fold (D-076): the stored form is the folded
    // form, so the schema CHECK measures what is actually stored —
    // a raw text one CRLF over the DB-006 limit is legal once folded.
    let content = fold_crlf(new.content);
    let id = new_id();
    conn.execute(
        "INSERT INTO instruction_variants (
             id, instruction_id, name, content, is_main, state,
             created_at, updated_at, deleted_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6, NULL)",
        rusqlite::params![
            id,
            new.instruction_id,
            new.name,
            content,
            i64::from(new.is_main),
            now_utc()
        ],
    )?;
    // Unreachable while the INSERT above succeeded (one connection,
    // D-037); a reasoned panic instead of a misleading NotFound.
    Ok(get_variant(conn, &id)?.expect("the row just inserted must be readable"))
}

/// Reads one variant by id — active or soft-deleted (the "Restore
/// variant" surface, LIB-026/027); `None` when no row matches.
///
/// # Errors
///
/// Returns [`LibraryError::Member`] when the stored `state` cell is
/// corrupt (D-039), and [`LibraryError::Sql`] when the query fails.
pub fn get_variant(conn: &Connection, id: &str) -> Result<Option<Variant>, LibraryError> {
    query_optional_variant(
        conn,
        &format!("SELECT {VARIANT_COLUMNS} FROM instruction_variants WHERE id = ?1"),
        [id],
    )
}

/// Lists every variant of the instruction — active and soft-deleted
/// alike — in creation order (`created_at, id`: the DB-002
/// fixed-width UTC shape orders chronologically, the UUIDv7 id breaks
/// same-millisecond ties in insertion order, D-031). The caller
/// filters by [`Variant::state`]: the editor's list is the active
/// subset, the restore picker needs the deleted ones.
///
/// # Errors
///
/// Returns [`LibraryError::Member`] when any returned row has a
/// corrupt `state` cell (D-039), and [`LibraryError::Sql`] when the
/// query fails.
pub fn list_variants(
    conn: &Connection,
    instruction_id: &str,
) -> Result<Vec<Variant>, LibraryError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {VARIANT_COLUMNS} FROM instruction_variants \
         WHERE instruction_id = ?1 ORDER BY created_at, id"
    ))?;
    let rows = statement
        .query_map([instruction_id], VariantRow::from_row)?
        .map(|raw| {
            raw.map_err(LibraryError::Sql)
                .and_then(|row| row.into_variant())
        })
        .collect::<Result<Vec<_>, LibraryError>>()?;
    Ok(rows)
}

/// Lists every **active** variant of **every** instruction — the
/// ALG-001 snapshot's variant read, one statement instead of a
/// [`list_variants`] call per instruction (the snapshot is hot: every
/// sync and every DB-022 hash recompute reads it).
///
/// The order is `instruction_id, created_at, id`: grouped per
/// instruction, and within one instruction the same creation order
/// [`list_variants`] produces (`created_at, id` — the DB-002
/// fixed-width UTC shape orders chronologically, the UUIDv7 id breaks
/// same-millisecond ties in insertion order, D-031). Soft-deleted
/// variants never reach a build (§9.1: "active variants") and stay
/// out; hidden instructions' active variants are included — ALG-001
/// reads "all library items … active variants" with no visibility
/// filter, and assignments may still point at a hidden instruction
/// (DB-013).
///
/// # Errors
///
/// Returns [`LibraryError::Sql`] when the query fails. A corrupt
/// `state` cell cannot surface here: the `WHERE` filter selects only
/// the `'active'` literal, so [`LibraryError::Member`] is unreachable
/// in this read and the corrupt row simply drops out (the D-105
/// nuance — the same one the active project scope has for
/// `projects.state`).
pub fn list_active_variants(conn: &Connection) -> Result<Vec<Variant>, LibraryError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {VARIANT_COLUMNS} FROM instruction_variants \
         WHERE state = 'active' ORDER BY instruction_id, created_at, id"
    ))?;
    let rows = statement
        .query_map([], VariantRow::from_row)?
        .map(|raw| {
            raw.map_err(LibraryError::Sql)
                .and_then(|row| row.into_variant())
        })
        .collect::<Result<Vec<_>, LibraryError>>()?;
    Ok(rows)
}

// ---- Variants: update, main flag, delete, restore ---------------------------------

/// The mutable fields of a variant (LIB-009…016), as an update
/// instruction: `None` leaves the field unchanged.
///
/// Deliberately absent: `instruction_id` (a variant never changes
/// its instruction), `is_main` ([`set_main_variant`] owns the flag),
/// `state` ([`delete_variant`]/[`restore_variant`] own it),
/// `created_at`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VariantUpdate<'a> {
    /// The new name, if it changes (NOCASE-unique among the
    /// instruction's active variants, §6.9).
    pub name: Option<&'a str>,
    /// The new text, if it changes (DB-006: at most 262144
    /// characters, schema CHECK; §6.9's save-time CRLF→LF fold
    /// applies before the store, D-076 — the stored form is the
    /// folded form; DB-016's marker lines are the `eunomia_core`
    /// validators' rejection, not this floor's).
    pub content: Option<&'a str>,
}

/// Applies an update to the variant with this id and returns the
/// stored row.
///
/// Exactly the fields present are written; `updated_at` is restamped
/// whenever at least one field is written (name and text are the
/// variant snapshot, §9.8), and a fieldless update writes nothing and
/// returns the row unchanged. A rename onto the NOCASE twin of an
/// active sibling is [`LibraryError::DuplicateVariantName`]; keeping
/// the variant's own name is exempt.
///
/// # Errors
///
/// Returns [`LibraryError::NotFound`] when no row has this id,
/// [`LibraryError::VariantNotActive`] for a soft-deleted variant (no
/// editing the dead; [`restore_variant`] first),
/// [`LibraryError::DuplicateVariantName`] for a taken name, and
/// [`LibraryError::Sql`] for the CHECK floor (§6.9 name lengths,
/// DB-006 content limit).
pub fn update_variant(
    conn: &Connection,
    id: &str,
    update: &VariantUpdate<'_>,
) -> Result<Variant, LibraryError> {
    let current =
        get_variant(conn, id)?.ok_or_else(|| LibraryError::NotFound { id: id.to_owned() })?;
    if current.state != VariantState::Active {
        return Err(LibraryError::VariantNotActive { id: id.to_owned() });
    }
    if let Some(name) = update.name
        && active_name_taken(conn, &current.instruction_id, name, Some(id))?
    {
        return Err(LibraryError::DuplicateVariantName {
            instruction_id: current.instruction_id,
            name: name.to_owned(),
        });
    }

    let mut assignments: Vec<&'static str> = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(name) = update.name {
        push_field(
            &mut assignments,
            &mut values,
            "name",
            Box::new(name.to_owned()),
        );
    }
    if let Some(content) = update.content {
        // §6.9's save-time fold (D-076) — see [`create_variant`].
        push_field(
            &mut assignments,
            &mut values,
            "content",
            Box::new(fold_crlf(content)),
        );
    }
    if assignments.is_empty() {
        // Nothing to write: a no-op must not churn `updated_at`.
        return Ok(current);
    }
    push_field(
        &mut assignments,
        &mut values,
        "updated_at",
        Box::new(now_utc()),
    );
    let changed = update_by_id(conn, "instruction_variants", &assignments, &mut values, id)?;
    if changed == 0 {
        // Unreachable: the row was read moments ago (one connection,
        // D-037); the branch keeps the statement contract explicit.
        return Err(LibraryError::NotFound { id: id.to_owned() });
    }
    Ok(get_variant(conn, id)?.expect("the row just updated must be readable"))
}

/// Makes the variant with this id its instruction's main variant
/// (LIB-021 "Make main"), demoting the previous main.
///
/// The flag change is metadata, not a snapshot change (D-16), so no
/// `updated_at` is written — see the module docs. Idempotent in
/// effect: promoting the current main demotes and re-promotes it,
/// a net no-op.
///
/// Atomicity: two statements — demote the instruction's active mains,
/// then promote the target. The order matters: `ux_variant_main`
/// rejects two active mains, so promoting first could collide with
/// the not-yet-demoted old main, while demote-then-promote passes
/// through zero mains, which the partial index allows. The pair is
/// atomic inside the caller's transaction (D-037); the integration
/// suite pins the rollback.
///
/// # Errors
///
/// Returns [`LibraryError::NotFound`] when no row has this id,
/// [`LibraryError::VariantNotActive`] for a soft-deleted variant, and
/// [`LibraryError::Sql`] when a statement fails.
pub fn set_main_variant(conn: &Connection, id: &str) -> Result<Variant, LibraryError> {
    let variant =
        get_variant(conn, id)?.ok_or_else(|| LibraryError::NotFound { id: id.to_owned() })?;
    if variant.state != VariantState::Active {
        return Err(LibraryError::VariantNotActive { id: id.to_owned() });
    }
    conn.execute(
        "UPDATE instruction_variants
         SET is_main = 0
         WHERE instruction_id = ?1 AND is_main = 1 AND state = 'active'",
        [variant.instruction_id.as_str()],
    )?;
    conn.execute(
        "UPDATE instruction_variants SET is_main = 1 WHERE id = ?1",
        [id],
    )?;
    // Unreachable: the UPDATE above matched the row (one connection,
    // D-037 — no concurrent delete exists).
    Ok(get_variant(conn, id)?.expect("the row just promoted must be readable"))
}

/// Soft-deletes the variant with this id (§6.4) after moving its
/// assignments to a replacement, and returns the stored row.
///
/// The LIB-023 flow: **one** replacement takes over *all* of the
/// deleted variant's assignments — and its main flag, if the deleted
/// variant was main. A per-project replacement choice (the dialog
/// allows a different variant for each affected project) composes
/// [`reassign`] calls after this delete, inside the same caller
/// transaction; the composition is pinned by a test.
///
/// * `replacement: Some(id)` names the sibling that takes over. It
///   must be an *active* variant of the *same* instruction
///   ([`LibraryError::ReplacementInvalid`]
///   otherwise; the deleted variant itself does not qualify).
/// * `replacement: None` defaults to the instruction's active main
///   variant (LIB-023's "the main one by default") — unless the
///   deleted variant *is* the main, which requires a named successor
///   ([`LibraryError::ReplacementRequired`]). With no assignments and
///   a non-main variant, `None` moves nothing. An instruction whose
///   active variants have no main (DB-010 already broken by
///   out-of-band data — the flag column violates no CHECK when
///   cleared by a plain `UPDATE`) is surfaced, not guessed at
///   ([`LibraryError::NoActiveMain`], DB-017).
/// * The last active variant of an instruction cannot be deleted
///   ([`LibraryError::LastVariant`], LIB-025; DB-010's "at least
///   one"). The check runs first: it is the absolute refusal, whatever
///   the replacement names.
///
/// The statements run in a fixed order — assignments first (DB-012:
/// moved *before* the delete; no assignment disappears silently),
/// then the soft delete (clearing `is_main`: a deleted row must not
/// count as the main, and demoting first keeps `ux_variant_main`
/// satisfiable mid-sequence), then the promotion. The soft delete
/// restamps `updated_at` (a state change); the promotion does not
/// (metadata, D-16).
///
/// Idempotent: deleting an already-deleted variant returns the row
/// unchanged — its assignments were moved by the first delete, so
/// there is nothing left to do.
///
/// Atomicity: multi-statement; atomic inside the caller's transaction
/// (D-037), pinned by the integration suite.
///
/// # Errors
///
/// Returns [`LibraryError::NotFound`] when no row has this id, and
/// the typed LIB-023/025 outcomes above, and
/// [`LibraryError::Sql`] when a statement fails.
pub fn delete_variant(
    conn: &Connection,
    id: &str,
    replacement: Option<&str>,
) -> Result<Variant, LibraryError> {
    let variant =
        get_variant(conn, id)?.ok_or_else(|| LibraryError::NotFound { id: id.to_owned() })?;
    if variant.state == VariantState::Deleted {
        return Ok(variant);
    }
    // LIB-025 first: the absolute refusal, whatever the replacement
    // names.
    if count_other_active_variants(conn, &variant.instruction_id, id)? == 0 {
        return Err(LibraryError::LastVariant {
            instruction_id: variant.instruction_id,
        });
    }
    let target: String = match replacement {
        Some(replacement_id) => {
            let candidate = get_variant(conn, replacement_id)?.ok_or_else(|| {
                LibraryError::ReplacementInvalid {
                    instruction_id: variant.instruction_id.clone(),
                    replacement: replacement_id.to_owned(),
                }
            })?;
            if candidate.id == variant.id
                || candidate.instruction_id != variant.instruction_id
                || candidate.state != VariantState::Active
            {
                return Err(LibraryError::ReplacementInvalid {
                    instruction_id: variant.instruction_id,
                    replacement: replacement_id.to_owned(),
                });
            }
            candidate.id
        }
        None => {
            if variant.is_main {
                return Err(LibraryError::ReplacementRequired {
                    instruction_id: variant.instruction_id,
                });
            }
            // LIB-023's default. Unreachable as `None` through this
            // module's own writers (DB-010 is kept constructive), but
            // the flag column is reachable by a plain out-of-band
            // `UPDATE … SET is_main = 0` that violates no CHECK — so
            // the broken invariant is surfaced (DB-017: no automatic
            // repair), never guessed at.
            match active_main_id(conn, &variant.instruction_id)? {
                Some(main_id) => main_id,
                None => {
                    return Err(LibraryError::NoActiveMain {
                        instruction_id: variant.instruction_id,
                    });
                }
            }
        }
    };
    let now = now_utc();
    // DB-012: the assignments move first; a moved assignment is a
    // real change of its row, so `updated_at` follows.
    conn.execute(
        "UPDATE project_instructions
         SET variant_id = ?1, updated_at = ?2
         WHERE variant_id = ?3",
        rusqlite::params![target, now, id],
    )?;
    conn.execute(
        "UPDATE instruction_variants
         SET state = 'deleted', is_main = 0, deleted_at = ?1, updated_at = ?1
         WHERE id = ?2",
        rusqlite::params![now, id],
    )?;
    if variant.is_main {
        conn.execute(
            "UPDATE instruction_variants SET is_main = 1 WHERE id = ?1",
            [target.as_str()],
        )?;
    }
    // Unreachable: the UPDATE above matched the row (one connection,
    // D-037 — no concurrent delete exists).
    Ok(get_variant(conn, id)?.expect("the row just deleted must be readable"))
}

/// Restores the soft-deleted variant with this id (LIB-026/027
/// "Restore variant"): `state` returns to `active`, `deleted_at` is
/// cleared (the invariant `deleted_at IS NOT NULL ⟺ state =
/// 'deleted'`), `updated_at` is restamped.
///
/// The variant returns *without* its old main flag (the delete
/// cleared it; DB-010 keeps exactly one active main) and without its
/// old assignments (they were moved to a sibling by the delete —
/// re-assigning is the caller's [`reassign`]). The name may have been
/// reused while the variant was deleted (the NOCASE index covers
/// active rows only): a collision is
/// [`LibraryError::DuplicateVariantName`].
///
/// Idempotent: an already-active variant is returned unchanged.
///
/// # Errors
///
/// Returns [`LibraryError::NotFound`] when no row has this id,
/// [`LibraryError::DuplicateVariantName`] for a reused name, and
/// [`LibraryError::Sql`] when the statement fails.
pub fn restore_variant(conn: &Connection, id: &str) -> Result<Variant, LibraryError> {
    let variant =
        get_variant(conn, id)?.ok_or_else(|| LibraryError::NotFound { id: id.to_owned() })?;
    if variant.state == VariantState::Active {
        return Ok(variant);
    }
    if active_name_taken(conn, &variant.instruction_id, &variant.name, Some(id))? {
        return Err(LibraryError::DuplicateVariantName {
            instruction_id: variant.instruction_id,
            name: variant.name,
        });
    }
    conn.execute(
        "UPDATE instruction_variants
         SET state = 'active', deleted_at = NULL, updated_at = ?1
         WHERE id = ?2",
        rusqlite::params![now_utc(), id],
    )?;
    // Unreachable: the UPDATE above matched the row (one connection,
    // D-037 — no concurrent delete exists).
    Ok(get_variant(conn, id)?.expect("the row just restored must be readable"))
}

// ---- Assignments -------------------------------------------------------------------

/// Verifies DB-012's assignability: the variant must be an *active*
/// variant of *this* instruction. The composite FOREIGN KEY is the
/// schema's floor (it blocks the cross-instruction pair at INSERT
/// time); this pre-check types the outcome and also covers the
/// soft-deleted variant, which no constraint covers.
fn ensure_assignable_variant(
    conn: &Connection,
    instruction_id: &str,
    variant_id: &str,
) -> Result<(), LibraryError> {
    let variant =
        get_variant(conn, variant_id)?.ok_or_else(|| LibraryError::VariantNotAssignable {
            instruction_id: instruction_id.to_owned(),
            variant_id: variant_id.to_owned(),
        })?;
    if variant.instruction_id != instruction_id || variant.state != VariantState::Active {
        return Err(LibraryError::VariantNotAssignable {
            instruction_id: instruction_id.to_owned(),
            variant_id: variant_id.to_owned(),
        });
    }
    Ok(())
}

/// Inserts an assignment and returns the stored row: the project gets
/// this variant of this instruction (D-02).
///
/// The pre-checks type the three runtime outcomes: an unknown project
/// or instruction id ([`LibraryError::NotFound`]), a heading as the
/// instruction ([`LibraryError::NotAnInstruction`] — MAIN-001's
/// "headings have no cells"), a second assignment of the same
/// instruction to the same project
/// ([`LibraryError::AlreadyAssigned`], the composite PRIMARY KEY;
/// changing the variant is [`reassign`]), and a variant that is not
/// an active variant of this instruction
/// ([`LibraryError::VariantNotAssignable`], DB-012). A *hidden*
/// instruction takes and keeps assignments (DB-013) — it is excluded
/// from the build, not from the matrix data.
///
/// # Errors
///
/// The typed outcomes above, and [`LibraryError::Sql`] when a
/// statement fails.
pub fn assign(conn: &Connection, new: &NewAssignment<'_>) -> Result<Assignment, LibraryError> {
    // A raw EXISTS rather than `crate::projects::get`: the projects
    // error type is its own, and this module's contract only needs
    // the existence fact (the FOREIGN KEY stays the floor for
    // anything racing a delete — impossible under the one connection,
    // D-037).
    let project_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
        [new.project_id],
        |row| row.get(0),
    )?;
    if !project_exists {
        return Err(LibraryError::NotFound {
            id: new.project_id.to_owned(),
        });
    }
    let item = get_item(conn, new.instruction_id)?.ok_or_else(|| LibraryError::NotFound {
        id: new.instruction_id.to_owned(),
    })?;
    if item.kind != ItemKind::Instruction {
        return Err(LibraryError::NotAnInstruction {
            id: new.instruction_id.to_owned(),
        });
    }
    if get_assignment(conn, new.project_id, new.instruction_id)?.is_some() {
        return Err(LibraryError::AlreadyAssigned {
            project_id: new.project_id.to_owned(),
            instruction_id: new.instruction_id.to_owned(),
        });
    }
    ensure_assignable_variant(conn, new.instruction_id, new.variant_id)?;
    conn.execute(
        "INSERT INTO project_instructions (
             project_id, instruction_id, variant_id, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?4)",
        rusqlite::params![
            new.project_id,
            new.instruction_id,
            new.variant_id,
            now_utc()
        ],
    )?;
    // Unreachable while the INSERT above succeeded (one connection,
    // D-037); a reasoned panic instead of a misleading NotFound.
    Ok(get_assignment(conn, new.project_id, new.instruction_id)?
        .expect("the row just inserted must be readable"))
}

/// Reads one assignment by its natural key (`project_id`,
/// `instruction_id`); `None` when the project has no assignment for
/// this instruction.
///
/// # Errors
///
/// Returns [`LibraryError::Sql`] when the query fails.
pub fn get_assignment(
    conn: &Connection,
    project_id: &str,
    instruction_id: &str,
) -> Result<Option<Assignment>, LibraryError> {
    query_optional_assignment(
        conn,
        "SELECT project_id, instruction_id, variant_id, created_at, updated_at
         FROM project_instructions WHERE project_id = ?1 AND instruction_id = ?2",
        rusqlite::params![project_id, instruction_id],
    )
}

/// Lists every assignment of the project in **library order** — the
/// order of the assigned instructions in the shared `position`
/// sequence (OUT-001: "the order in the file equals the order in the
/// library"). Assignments of hidden instructions are included
/// (DB-013: they stay matrix data); the build filters them (D-11) —
/// that filter is the caller's.
///
/// # Errors
///
/// Returns [`LibraryError::Sql`] when the query fails.
pub fn list_assignments_for_project(
    conn: &Connection,
    project_id: &str,
) -> Result<Vec<Assignment>, LibraryError> {
    let mut statement = conn.prepare(
        "SELECT a.project_id, a.instruction_id, a.variant_id, a.created_at, a.updated_at
         FROM project_instructions a
         JOIN library_items i ON i.id = a.instruction_id
         WHERE a.project_id = ?1
         ORDER BY i.position",
    )?;
    let rows = statement
        .query_map([project_id], |row| {
            Ok(Assignment {
                project_id: row.get(0)?,
                instruction_id: row.get(1)?,
                variant_id: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?
        .map(|raw| raw.map_err(LibraryError::Sql))
        .collect::<Result<Vec<_>, LibraryError>>()?;
    Ok(rows)
}

/// Lists the assignments of **every active project** — the ALG-001
/// snapshot's assignment read, scoped to the projects the snapshot
/// carries (DB-014: an archived project keeps its assignment rows but
/// takes part in no sync; core's snapshot validation rejects an
/// assignment of a project it does not hold, so the scoping is this
/// read's, not the caller's memory).
///
/// The order is the snapshot's own reading order: projects by
/// `sort_order`, and within one project the assigned instructions in
/// library `position` order (OUT-001, the same order
/// [`list_assignments_for_project`] produces for a single project).
/// Assignments of hidden instructions are included (DB-013) — the
/// build filters them (D-11), the snapshot carries them.
///
/// # Errors
///
/// Returns [`LibraryError::Sql`] when the query fails.
pub fn list_assignments_for_active_projects(
    conn: &Connection,
) -> Result<Vec<Assignment>, LibraryError> {
    let mut statement = conn.prepare(
        "SELECT a.project_id, a.instruction_id, a.variant_id, a.created_at, a.updated_at
         FROM project_instructions a
         JOIN projects p ON p.id = a.project_id
         JOIN library_items i ON i.id = a.instruction_id
         WHERE p.state = 'active'
         ORDER BY p.sort_order, i.position",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(Assignment {
                project_id: row.get(0)?,
                instruction_id: row.get(1)?,
                variant_id: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?
        .map(|raw| raw.map_err(LibraryError::Sql))
        .collect::<Result<Vec<_>, LibraryError>>()?;
    Ok(rows)
}

/// Lists every assignment of the variant, in creation order
/// (`created_at, project_id` ties break stably) — the LIB-023 delete
/// dialog's "which projects are affected" read.
///
/// # Errors
///
/// Returns [`LibraryError::Sql`] when the query fails.
pub fn list_assignments_for_variant(
    conn: &Connection,
    variant_id: &str,
) -> Result<Vec<Assignment>, LibraryError> {
    let mut statement = conn.prepare(
        "SELECT project_id, instruction_id, variant_id, created_at, updated_at
         FROM project_instructions WHERE variant_id = ?1
         ORDER BY created_at, project_id",
    )?;
    let rows = statement
        .query_map([variant_id], |row| {
            Ok(Assignment {
                project_id: row.get(0)?,
                instruction_id: row.get(1)?,
                variant_id: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?
        .map(|raw| raw.map_err(LibraryError::Sql))
        .collect::<Result<Vec<_>, LibraryError>>()?;
    Ok(rows)
}

/// Changes the assigned variant of one assignment (MAIN-005: changing
/// the variant in one project does not change other projects) and
/// returns the stored row.
///
/// The assignment must exist first ([`LibraryError::NotFound`] with
/// the `"<project_id>/<instruction_id>"` pair key when it does not —
/// checked before the variant, so an absent assignment is reported as
/// absent whatever the variant names), and the new variant must be an
/// active variant of the same instruction
/// ([`LibraryError::VariantNotAssignable`], DB-012 — a soft-deleted
/// variant cannot be assigned). Re-assigning the variant the row
/// already holds is a verified no-op (nothing is written, no
/// restamp); a real change restamps `updated_at`: the row's variant
/// is its content.
///
/// # Errors
///
/// Returns [`LibraryError::NotFound`] when the project has no
/// assignment for this instruction,
/// [`LibraryError::VariantNotAssignable`] for a variant that is not an
/// active sibling, and [`LibraryError::Sql`] when the statement
/// fails.
pub fn reassign(
    conn: &Connection,
    project_id: &str,
    instruction_id: &str,
    variant_id: &str,
) -> Result<Assignment, LibraryError> {
    let current = get_assignment(conn, project_id, instruction_id)?.ok_or_else(|| {
        LibraryError::NotFound {
            id: format!("{project_id}/{instruction_id}"),
        }
    })?;
    if current.variant_id == variant_id {
        // Already on this variant: a no-op must not churn `updated_at`.
        return Ok(current);
    }
    ensure_assignable_variant(conn, instruction_id, variant_id)?;
    let changed = conn.execute(
        "UPDATE project_instructions
         SET variant_id = ?1, updated_at = ?2
         WHERE project_id = ?3 AND instruction_id = ?4",
        rusqlite::params![variant_id, now_utc(), project_id, instruction_id],
    )?;
    if changed == 0 {
        return Err(LibraryError::NotFound {
            id: format!("{project_id}/{instruction_id}"),
        });
    }
    // Unreachable: the UPDATE above matched the row (one connection,
    // D-037 — no concurrent delete exists).
    Ok(get_assignment(conn, project_id, instruction_id)?
        .expect("the row just updated must be readable"))
}

/// Removes the assignment (LIB-020: an assignment can be removed
/// without deleting the variant; MAIN-001's toggle-off).
///
/// # Errors
///
/// Returns [`LibraryError::NotFound`] (id
/// `"<project_id>/<instruction_id>"`) when the project has no
/// assignment for this instruction, and [`LibraryError::Sql`] when
/// the statement fails.
pub fn unassign(
    conn: &Connection,
    project_id: &str,
    instruction_id: &str,
) -> Result<(), LibraryError> {
    let changed = conn.execute(
        "DELETE FROM project_instructions
         WHERE project_id = ?1 AND instruction_id = ?2",
        rusqlite::params![project_id, instruction_id],
    )?;
    if changed == 0 {
        return Err(LibraryError::NotFound {
            id: format!("{project_id}/{instruction_id}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The re-serialization of every enum this module stores must
    /// produce exactly the member the schema CHECK accepts — a
    /// drifted member would make a written row unreadable, so the
    /// round trip is pinned here (the `projects` module's precedent).
    #[test]
    fn vocabulary_members_round_trip_through_the_schema_literals() {
        for (kind, literal) in [
            (ItemKind::Instruction, "instruction"),
            (ItemKind::Heading, "heading"),
        ] {
            assert_eq!(kind.as_str(), literal);
            assert_eq!(ItemKind::from_member(literal).unwrap(), kind);
            assert!(matches!(
                ItemKind::from_member(&format!("{literal}x")),
                Err(LibraryError::Member { column: "kind", .. })
            ));
        }
        for (state, literal) in [(ItemState::Active, "active"), (ItemState::Hidden, "hidden")] {
            assert_eq!(state.as_str(), literal);
            assert_eq!(ItemState::from_member(literal).unwrap(), state);
            assert!(matches!(
                ItemState::from_member(&format!("{literal}x")),
                Err(LibraryError::Member {
                    column: "state",
                    ..
                })
            ));
        }
        for (state, literal) in [
            (VariantState::Active, "active"),
            (VariantState::Deleted, "deleted"),
        ] {
            assert_eq!(state.as_str(), literal);
            assert_eq!(VariantState::from_member(literal).unwrap(), state);
            assert!(matches!(
                VariantState::from_member(&format!("{literal}x")),
                Err(LibraryError::Member {
                    column: "state",
                    ..
                })
            ));
        }
    }

    /// The heading-level matrix mirrors the §6.2 CHECK: instructions
    /// carry NULL, headings a level in `2..=6` — the boundaries and
    /// both illegal shapes are rejected as `Member`.
    #[test]
    fn heading_level_decodes_the_check_matrix() {
        assert_eq!(
            decode_heading_level(ItemKind::Instruction, None).unwrap(),
            None
        );
        assert_eq!(
            decode_heading_level(ItemKind::Heading, Some(2)).unwrap(),
            Some(2)
        );
        assert_eq!(
            decode_heading_level(ItemKind::Heading, Some(6)).unwrap(),
            Some(6)
        );
        for cell in [Some(0), Some(1), Some(7), Some(99)] {
            assert!(matches!(
                decode_heading_level(ItemKind::Heading, cell),
                Err(LibraryError::Member {
                    column: "heading_level",
                    ..
                })
            ));
        }
        assert!(matches!(
            decode_heading_level(ItemKind::Heading, None),
            Err(LibraryError::Member {
                column: "heading_level",
                ref found
            }) if found == "NULL"
        ));
        assert!(matches!(
            decode_heading_level(ItemKind::Instruction, Some(3)),
            Err(LibraryError::Member {
                column: "heading_level",
                ..
            })
        ));
    }

    /// `Display` stays lowercase and sentence-shaped without trailing
    /// punctuation (the store's error-message convention).
    #[test]
    fn error_display_is_lowercase_and_specific() {
        assert_eq!(
            LibraryError::NotFound {
                id: String::from("x")
            }
            .to_string(),
            "no row with id \"x\""
        );
        assert_eq!(
            LibraryError::NotAnInstruction {
                id: String::from("h")
            }
            .to_string(),
            "library item \"h\" is not an instruction"
        );
        assert_eq!(
            LibraryError::NotAHeading {
                id: String::from("i")
            }
            .to_string(),
            "library item \"i\" is not a heading"
        );
        assert_eq!(
            LibraryError::DuplicateInstructionTitle {
                title: String::from("Be concise")
            }
            .to_string(),
            "a visible instruction titled \"Be concise\" already exists"
        );
        assert_eq!(
            LibraryError::DuplicateVariantName {
                instruction_id: String::from("i"),
                name: String::from("Main"),
            }
            .to_string(),
            "an active variant named \"Main\" already exists on instruction \"i\""
        );
        assert_eq!(
            LibraryError::MainVariantExists {
                instruction_id: String::from("i")
            }
            .to_string(),
            "instruction \"i\" already has an active main variant"
        );
        assert_eq!(
            LibraryError::MainRequired {
                instruction_id: String::from("i")
            }
            .to_string(),
            "the first active variant of instruction \"i\" must be its main variant"
        );
        assert_eq!(
            LibraryError::LastVariant {
                instruction_id: String::from("i")
            }
            .to_string(),
            "the last active variant of instruction \"i\" cannot be deleted"
        );
        assert_eq!(
            LibraryError::ReplacementRequired {
                instruction_id: String::from("i")
            }
            .to_string(),
            "deleting the main variant of instruction \"i\" requires a named replacement"
        );
        assert_eq!(
            LibraryError::ReplacementInvalid {
                instruction_id: String::from("i"),
                replacement: String::from("r"),
            }
            .to_string(),
            "replacement \"r\" is not an active variant of instruction \"i\""
        );
        assert_eq!(
            LibraryError::NoActiveMain {
                instruction_id: String::from("i")
            }
            .to_string(),
            "instruction \"i\" has no active main variant (DB-010 is already broken; \
             refusing to guess a default replacement)"
        );
        assert_eq!(
            LibraryError::PositionOutOfRange {
                requested: 4,
                max: 3,
            }
            .to_string(),
            "position 4 is outside 1..=3"
        );
        assert_eq!(
            LibraryError::AlreadyAssigned {
                project_id: String::from("p"),
                instruction_id: String::from("i"),
            }
            .to_string(),
            "project \"p\" already has an assignment for instruction \"i\""
        );
        assert_eq!(
            LibraryError::VariantNotAssignable {
                instruction_id: String::from("i"),
                variant_id: String::from("v"),
            }
            .to_string(),
            "variant \"v\" is not an active variant of instruction \"i\""
        );
        assert_eq!(
            LibraryError::VariantNotActive {
                id: String::from("v")
            }
            .to_string(),
            "variant \"v\" is not active"
        );
        assert_eq!(
            LibraryError::Member {
                column: "kind",
                found: String::from("bogus"),
            }
            .to_string(),
            "\"bogus\" is not a valid kind value"
        );
    }
}
