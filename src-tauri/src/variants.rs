//! The Library-module variant commands (spec §7.10): the by-id read
//! `variant_get` (T-351, D-198) and the five write commands (T-329;
//! the batch absorbing T-087…T-091) `variant_create`,
//! `variant_update`, `variant_set_main`, `variant_delete`,
//! `variant_restore` — the first `src-tauri` consumers of the
//! `eunomia-store` library repository, the §6.9 library validators,
//! the §9.8 version recorder and the `variant.*` journal members, in
//! the `scan.rs` shape of a §7.10 command group living in its own
//! module (D-189).
//!
//! `variant_get` is the family's READ (D-198): one `Db::run`
//! transaction over the store's by-id reader, answering the stored
//! row — active or soft-deleted alike, `state` says which — plus the
//! derived `is_empty` flag (ALG-002's normalize, the same oracle
//! `library_get`'s inline entries carry). Nothing is written,
//! journaled or versioned, and the text is never truncated (DB-006:
//! the MAIN-006 tooltip's "first 600 characters" is the UI's own
//! clamp over the full returned text).
//!
//! Every WRITE command runs ONE `Db::run` transaction (D-037, §6.9's
//! "the whole operation runs in one transaction"): the store write,
//! the version row and the journal entry commit or roll back together —
//! pinned by a staged-trigger test. §6.9 field validation of the
//! caller's input runs BEFORE the database is even resolved (pure
//! CPU, no lock, the `scan_commit` gate order), so a refusal names
//! the field without touching anything. (The read is the exception
//! on both counts: `variant_get` writes, versions and journals
//! nothing, and its one refusal — an unknown id — is decided INSIDE
//! its transaction, where the by-id read runs.)
//!
//! **SYS-005 does not apply here** (§5.4's own sentence: "Library
//! edits during a sync are allowed; they do not go into the current
//! run and stay dirty") — unlike the project-*changing* commands,
//! these take no `reject_during_sync` gate, pinned by a test that
//! holds the flag up. The DB-022 dirty recalculation rides
//! `Db::run`'s commit hook with no extra wiring: a variant write
//! dirties exactly the projects whose build it changed.
//!
//! **Versions and the journal are keyed to the real change** (D-189
//! reading 5): `variant_update` journals `variant.changed` iff
//! ALG-070's hash gate inserted a version (§9.8's own definition of
//! "a real change", D-16 — a save without changes writes no version
//! AND no journal); create/delete/restore journal unconditionally
//! after a genuine transition; `variant_set_main` writes no version
//! (D-16: the main flag is not snapshot-visible), journals the
//! `variant.changed` "main choice" member, and REFUSES an empty
//! promotion (LIB-036, the defensive surface behind the editor's
//! disabled star): a variant whose ALG-002-normalized text is empty
//! — the same oracle `variant_get`'s `is_empty` flag carries — is
//! the typed `E_VALIDATION` refusal naming `id`, decided inside the
//! transaction before any write. Idempotent re-sends
//! (delete on already-deleted, restore on active, set_main on the
//! current main) are verified no-ops — nothing written, journaled or
//! versioned (the `projects_reorder` replayed-drag shape).
//!
//! The rule-8 posture (SEC-008's sibling at the error boundary):
//! `params` carry fields, reasons, counts and indexes — never the
//! variant's name or text; the two `LibraryError`-to-`AppError`
//! mappers below are the D-093 normalization this module owns
//! (typed outcomes → `E_VALIDATION` field refusals; corrupt cells →
//! the D-037 fallback with a class-only `LibraryCorrupt` diagnostic;
//! SQLite → the shared base classifier).
//!
//! Failures reach the log through the typed
//! `logging::variant_*_failed` edges (§10's every-error rule); a
//! successful write is the journal entry itself (the D-051 mirror
//! policy — info+user+ok is not mirrored to the file log).

use eunomia_core::library_validators::{
    LibraryFieldError, validate_variant_name, validate_variant_text,
};
use eunomia_core::version_snapshot::{variant_snapshot, variant_snapshot_hash};
use eunomia_store::action_log::{
    Action, Details, Entity, EntityType, Level, LogEntry, Outcome, Visibility, insert,
};
use eunomia_store::library::{
    ItemKind, LibraryError, NewVariant, Variant, VariantState, VariantUpdate, create_variant,
    delete_variant, get_item, get_variant, list_assignments_for_variant, list_variants,
    restore_variant, set_main_variant, update_variant,
};
use eunomia_store::library_versions::{Recorded, RevisionError, VariantOp, record_variant};
use eunomia_store::settings::{SettingKey, SettingValue};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};

use crate::commands::{activity_log_to_app_error, managed_db, rejected_field};
use crate::db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};

// ---------------------------------------------------------------------------
// The wire DTO
// ---------------------------------------------------------------------------

/// One `instruction_variants` row on the wire (§6.2 columns the UI
/// reaches): every variant command's answer — the row as it stands
/// after the operation, in the `ExistingProject` house shape (the
/// closed vocabularies in their stored spellings, the DB-002 UTC
/// timestamps as text).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct VariantRecord {
    /// Row id, UUIDv7 TEXT (DB-002).
    pub(crate) id: String,
    /// The owning instruction's id.
    pub(crate) instruction_id: String,
    /// The variant name (§6.9: 1–100 characters, unique within the
    /// instruction among ACTIVE rows, NOCASE).
    pub(crate) name: String,
    /// The variant text (DB-006, stored in the §6.9 folded form).
    pub(crate) content: String,
    /// The main-variant flag (DB-010: exactly one active main per
    /// instruction; metadata, not snapshot-visible — D-16).
    pub(crate) is_main: bool,
    /// `active` or `deleted` (§6.4's soft delete).
    pub(crate) state: String,
    /// Creation time, DB-002 UTC ISO-8601.
    pub(crate) created_at: String,
    /// Last snapshot-visible write of the row.
    pub(crate) updated_at: String,
    /// When the variant was soft-deleted; `None` while active.
    pub(crate) deleted_at: Option<String>,
}

impl VariantRecord {
    /// Shapes a stored variant row into its wire form.
    fn from_store(variant: Variant) -> Self {
        VariantRecord {
            id: variant.id,
            instruction_id: variant.instruction_id,
            name: variant.name,
            content: variant.content,
            is_main: variant.is_main,
            state: variant.state.as_str().to_owned(),
            created_at: variant.created_at,
            updated_at: variant.updated_at,
            deleted_at: variant.deleted_at,
        }
    }
}

/// The `variant_get` answer (§7.10, D-198): the stored row — the
/// same nine fields every variant command's [`VariantRecord`]
/// carries, mapped by the same one constructor — plus the derived
/// `is_empty` flag. The editor's body and the MAIN-006 tooltip read
/// this answer; neither field set is forked from the write
/// commands' shape, so the two readers of a variant (the inline
/// list, the by-id read) cannot disagree about the row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct VariantGetResult {
    /// Row id, UUIDv7 TEXT (DB-002) — the id the caller sent.
    pub(crate) id: String,
    /// The owning instruction's id.
    pub(crate) instruction_id: String,
    /// The variant name (§6.9, 1–100 characters, unique within the
    /// instruction among ACTIVE rows, NOCASE).
    pub(crate) name: String,
    /// The variant text (DB-006) — the FULL stored text, never
    /// truncated: the MAIN-006 tooltip's "first 600 characters" is
    /// the UI's own clamp over the returned string, and the editor's
    /// body needs the whole text anyway (D-198: one read serves both).
    pub(crate) content: String,
    /// The main-variant flag (DB-010: exactly one active main per
    /// instruction; metadata, not snapshot-visible — D-16).
    pub(crate) is_main: bool,
    /// `active` or `deleted` (§6.4's soft delete) — answered AS
    /// STORED: a soft-deleted variant is served with `deleted` state
    /// and its `deleted_at` set, so a stale screen re-reading a held
    /// id sees the truth instead of a refusal (D-198).
    pub(crate) state: String,
    /// Creation time, DB-002 UTC ISO-8601.
    pub(crate) created_at: String,
    /// Last snapshot-visible write of the row.
    pub(crate) updated_at: String,
    /// When the variant was soft-deleted; `None` while active.
    pub(crate) deleted_at: Option<String>,
    /// LIB-014/D-13: this variant is EMPTY in the "outputs no text"
    /// sense, decided by the build's own oracle
    /// (`normalize::variant_text`, ALG-002) — exactly the derivation
    /// `library_get`'s inline entries carry (D-187), so a
    /// whitespace-only stored body counts exactly as it counts in a
    /// built file. Derived, never stored (the schema has no such
    /// column).
    pub(crate) is_empty: bool,
}

impl VariantGetResult {
    /// Shapes a stored variant row into the by-id answer: the
    /// [`VariantRecord`] mapping (one source for the nine stored
    /// fields, destructured here rather than re-typed) plus the
    /// derived emptiness.
    fn from_store(variant: Variant) -> Self {
        // The derivation is the `library_get` inline flag's own call
        // (commands.rs, D-187): ALG-002's normalize decides, so the
        // two readers of a variant never disagree about "empty".
        let is_empty = eunomia_core::normalize::variant_text(&variant.content).is_empty();
        let VariantRecord {
            id,
            instruction_id,
            name,
            content,
            is_main,
            state,
            created_at,
            updated_at,
            deleted_at,
        } = VariantRecord::from_store(variant);
        VariantGetResult {
            id,
            instruction_id,
            name,
            content,
            is_main,
            state,
            created_at,
            updated_at,
            deleted_at,
            is_empty,
        }
    }
}

/// Where a new variant's text comes from (§7.3 LIB-017…022's "a copy
/// of the main one or empty"), serialized as the spec's own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) enum VariantSource {
    /// The main variant's CURRENT stored text, read inside the same
    /// transaction (D-189 reading 2 — no TOCTOU with a concurrent
    /// edit on the one connection).
    #[serde(rename = "copy_main")]
    CopyMain,
    /// The empty text — an empty variant is legal and assignable
    /// (LIB-014/015, D-13).
    #[serde(rename = "empty")]
    Empty,
}

// ---------------------------------------------------------------------------
// The error mappers (the D-093 normalization this module owns)
// ---------------------------------------------------------------------------

/// Maps a library field failure onto the §7.10 wire: the §6.9 rule
/// failures are `E_VALIDATION` naming the field with the measured
/// facts (`length`, `index`, `character`, `line`) riding `params`;
/// the reserved marker line (DB-016) is `E_CONTENT_RESERVED` — §10's
/// own split, the one `eunomia-core`'s module docs prescribe for this
/// boundary. Never the offending text itself (D-189 reading 7): the
/// UI already holds the draft it sent.
fn library_field_to_app_error(error: LibraryFieldError) -> AppError {
    match error {
        LibraryFieldError::VariantNameEmpty => field_refusal(
            "variant_name",
            "a variant name needs 1-100 characters after trimming",
        ),
        LibraryFieldError::VariantNameTooLong { length } => with_number(
            field_refusal("variant_name", "a variant name is at most 100 characters"),
            "length",
            length,
        ),
        LibraryFieldError::VariantNameLineBreak { index, character } => with_param(
            with_number(
                field_refusal("variant_name", "a variant name is one line"),
                "index",
                index,
            ),
            "character",
            break_name(character),
        ),
        LibraryFieldError::VariantNameNul { index } => with_number(
            field_refusal("variant_name", "a variant name must not contain NUL"),
            "index",
            index,
        ),
        LibraryFieldError::VariantTextTooLong { length } => with_number(
            field_refusal("variant_text", "variant text is at most 262144 characters"),
            "length",
            length,
        ),
        LibraryFieldError::VariantTextNul { index } => with_number(
            field_refusal("variant_text", "variant text must not contain NUL"),
            "index",
            index,
        ),
        LibraryFieldError::ReservedContent { line, marker } => {
            let mut error = AppError::new(ErrorCode::ContentReserved);
            // The field rides the refusal too (§7.3's aria-invalid
            // rule: "reserved markers from DB-016 mark the field");
            // `line` and `marker` are the measured facts.
            error
                .params
                .insert("field".to_owned(), "variant_text".to_owned());
            error.params.insert("line".to_owned(), line.to_string());
            error.params.insert(
                "marker".to_owned(),
                match marker {
                    eunomia_core::markers::MarkerLine::Begin => eunomia_core::markers::BEGIN_MARKER,
                    eunomia_core::markers::MarkerLine::End => eunomia_core::markers::END_MARKER,
                }
                .to_owned(),
            );
            error
        }
        // The instruction/heading title members are not reachable
        // from these commands (no command here writes either field);
        // the class-only fallback keeps the mapping total without
        // mislabeling a field.
        _ => library_fallback(
            "LibraryFieldError",
            "a library field validator rejected a field these commands do not map",
        ),
    }
}

/// The stable name of a line-break character for `params` — a short
/// code, never a control byte in a diagnostic surface.
fn break_name(character: char) -> &'static str {
    match character {
        '\n' => "LF",
        '\r' => "CR",
        _ => "line break",
    }
}

/// Maps a store failure of the library repository onto the §7.10
/// wire, by what the failure is: the typed LIB/DB-010/DB-012
/// outcomes are `E_VALIDATION` field refusals (the honest §10 fit of
/// the frozen registry — no not-found code exists, and the D-037
/// fallback would misname a stale screen as a database problem, the
/// `exclusion_remove` rule); a corrupt vocabulary cell takes the
/// D-037 fallback with a class-only `LibraryCorrupt` diagnostic —
/// never the stored value (the T-027 round-2 rule); SQLite failures
/// go through the shared base classifier. Members no command of this
/// module can produce (the item/assignment/position family) share
/// the class-only fallback so the match stays total.
fn library_store_to_app_error(error: LibraryError) -> AppError {
    match error {
        LibraryError::Sql(source) => db::sqlite_error_to_app_error(source),
        // By-id writers: an id that matches no row is the caller's
        // stale screen, not a database fault.
        LibraryError::NotFound { id } => rejected_field("id", &id, "no variant with this id"),
        LibraryError::NotAnInstruction { id } => {
            rejected_field("instruction_id", &id, "a heading has no variants")
        }
        // The name is deliberately NOT carried (rule 8): the UI holds
        // the draft; the log gets the field and the rule.
        LibraryError::DuplicateVariantName { .. } => field_refusal(
            "variant_name",
            "an active variant of this instruction already carries this name \
             (case-insensitive)",
        ),
        LibraryError::MainRequired { .. } | LibraryError::NoActiveMain { .. } => field_refusal(
            "instruction_id",
            "the instruction has no active main variant (DB-010)",
        ),
        LibraryError::MainVariantExists { .. } => field_refusal(
            "instruction_id",
            "the instruction already has an active main variant",
        ),
        LibraryError::LastVariant { .. } => field_refusal(
            "id",
            "the last active variant of an instruction cannot be deleted",
        ),
        LibraryError::ReplacementRequired { .. } => field_refusal(
            "replacement",
            "deleting the main variant requires naming its successor",
        ),
        LibraryError::ReplacementInvalid { replacement, .. } => rejected_field(
            "replacement",
            &replacement,
            "the replacement is not an active variant of the same instruction",
        ),
        LibraryError::VariantNotActive { id } => {
            rejected_field("id", &id, "the variant is soft-deleted; restore it first")
        }
        // Corrupt cells and the members no command here can produce.
        LibraryError::Member { column, .. } => library_fallback(
            "LibraryCorrupt",
            &format!("a stored {column} cell is outside its vocabulary"),
        ),
        _ => library_fallback(
            "LibraryError",
            "the library repository refused an outcome these commands do not map",
        ),
    }
}

/// The class-only D-037 fallback shared by the corrupt-cell and
/// unreachable arms: the `projects_fallback` shape with this
/// module's own kind label.
fn library_fallback(kind: &str, message: &str) -> AppError {
    let mut app_error = AppError::new(ErrorCode::DbLocked);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some(kind.to_owned()),
        os_message: Some(message.to_owned()),
    });
    app_error
}

/// Maps a version-recording failure: SQLite through the shared base
/// classifier, the recorder's entity probes (a caller bug over rows
/// this module just wrote, or out-of-band damage) the class-only
/// fallback — the member name is the whole diagnostic, never the
/// entity's id.
fn revision_store_to_app_error(error: RevisionError) -> AppError {
    match error {
        RevisionError::Sql(source) => db::sqlite_error_to_app_error(source),
        RevisionError::NotFound { .. } => revision_fallback("NotFound"),
        RevisionError::NotAnInstruction { .. } => revision_fallback("NotAnInstruction"),
        RevisionError::Member { .. } => revision_fallback("Member"),
    }
}

/// The [`revision_store_to_app_error`] fallback arm over the member
/// name.
fn revision_fallback(member: &str) -> AppError {
    library_fallback(
        "RevisionError",
        &format!("the version recorder refused the entity ({member})"),
    )
}

/// An `E_VALIDATION` refusal naming the field — the
/// [`rejected_field`] shape without a `value` param: these refusals
/// carry only the field, the rule and the measured numbers (D-189
/// reading 7 — the project commands' `value` precedent is not
/// followed for name and text, and the id-shaped refusals that DO
/// carry one build themselves through [`rejected_field`]).
fn field_refusal(field: &str, reason: &str) -> AppError {
    let mut error = AppError::new(ErrorCode::Validation);
    error.params.insert("field".to_owned(), field.to_owned());
    error.params.insert("reason".to_owned(), reason.to_owned());
    error
}

/// Adds a string param to a refusal (rule 8: fields, reasons, counts
/// and indexes — short codes — are the blessed diagnostic facts).
fn with_param(mut error: AppError, key: &str, value: &str) -> AppError {
    error.params.insert(key.to_owned(), value.to_owned());
    error
}

/// Adds a measured number to a refusal's `params`.
fn with_number(error: AppError, key: &str, value: usize) -> AppError {
    with_param(error, key, &value.to_string())
}

// ---------------------------------------------------------------------------
// The shared internals
// ---------------------------------------------------------------------------

/// Reads the §6.5 `history.max_revisions` limit through the store's
/// single read path (D-039) inside the caller's transaction — so the
/// prune is atomic with the write it follows. A missing row answers
/// the §6.5 default; a corrupt cell answers the settings corrupt
/// fallback (never `E_VALIDATION` — corruption is not the caller's
/// input, the T-027 round-2 rule).
fn max_revisions(transaction: &rusqlite::Transaction<'_>) -> Result<i64, AppError> {
    match eunomia_store::settings::get(transaction, SettingKey::HistoryMaxRevisions) {
        Ok(SettingValue::Int(value)) => Ok(value),
        // Unreachable: `get` re-parses every stored cell through the
        // key's §6.5 contract (the `WriteDefaults` rule), typed
        // honestly instead of `unwrap`ped.
        Ok(_) => Err(crate::commands::settings_cell_to_app_error(
            eunomia_store::storage::StorageError::Settings {
                key: SettingKey::HistoryMaxRevisions.as_str().to_owned(),
                source: eunomia_store::settings::SettingsError::Type {
                    expected: "int",
                    found: "another variant",
                },
            },
        )),
        Err(error) => Err(crate::commands::settings_cell_to_app_error(error)),
    }
}

/// Records one version of the variant (§9.8, inside the caller's
/// transaction) with the snapshot pair of the row as it stands — the
/// ALG-070 gate decides whether a row is written.
fn record_version(
    transaction: &rusqlite::Transaction<'_>,
    variant: &Variant,
    op: VariantOp,
) -> Result<Recorded, AppError> {
    let snapshot = variant_snapshot(&variant.name, &variant.content);
    let hash = variant_snapshot_hash(&variant.name, &variant.content);
    record_variant(
        transaction,
        &variant.id,
        op,
        &snapshot,
        &hash,
        max_revisions(transaction)?,
    )
    .map_err(revision_store_to_app_error)
}

/// Writes the operation's journal entry inside its own transaction
/// (D-051) — the `variant.*` member at info/user/ok naming the
/// variant's entity columns, with the optional typed payload — and
/// records it into the transaction's mirror buffer (T-324/D-228:
/// the executor flushes only after the commit).
fn journal_variant(
    transaction: &rusqlite::Transaction<'_>,
    mirror: &mut crate::logging::MirrorBuffer,
    action: Action,
    variant: &Variant,
    details: Option<Details>,
) -> Result<(), AppError> {
    let mut entry = LogEntry::new(action, Level::Info, Visibility::User, Outcome::Ok);
    entry.entity = Some(Entity {
        kind: EntityType::Variant,
        id: variant.id.as_str(),
    });
    entry.details = details;
    insert(transaction, &entry).map_err(activity_log_to_app_error)?;
    mirror.record(&entry);
    Ok(())
}

// ---------------------------------------------------------------------------
// `variant_get` (T-351, §7.10 Library module, §7.3 LIB-002/014,
// MAIN-006, DB-006; the contract D-198 fixed)
// ---------------------------------------------------------------------------

/// Reads one variant by its row `id` (§7.10 `variant_get`, the
/// contract D-198 fixed, spec-only until this command): answers the
/// STORED row — every §6.2 column the write commands'
/// [`VariantRecord`] carries — plus the derived `is_empty` flag
/// (LIB-014/D-13 through ALG-002's normalize, the same oracle
/// `library_get`'s inline entries carry).
///
/// The read serves the instruction editor's body (§7.3) and the
/// MAIN-006 tooltip's text half: it takes the identity
/// `library_get`'s inline entries and the other variant commands
/// carry, and answers whatever the row says — an ACTIVE or
/// SOFT-DELETED variant alike, `state` (and `deleted_at`) telling
/// which, so a stale screen re-reading an id that was soft-deleted
/// in another window sees the truth instead of a refusal. An
/// unknown id is the typed `E_VALIDATION` refusal naming `id` (the
/// `exclusion_remove` unknown-id shape — §10 has no not-found code,
/// and the D-037 fallback would misname a stale screen as a
/// database problem).
///
/// The command never truncates: `content` is the full stored text
/// (DB-006); the tooltip's "first 600 characters, 400 ms delay" is
/// the UI's own clamp over the returned string.
///
/// A pure read (the `sync_status`/`sync_last_result` shape, D-138):
/// one `Db::run` transaction over the store's by-id reader — no
/// SYS-005 gate (§5.4 scopes it to the project-*changing*
/// commands), no journal (D-051: reads journal nothing), no version,
/// no mirror. Recovery mode (no database) answers the honest
/// [`managed_db`] fallback.
///
/// Failures reach the log with their §10 code
/// ([`logging::variant_get_failed`]); a successful read is silent
/// (nothing happened that the journal would record).
#[tauri::command]
#[specta::specta]
pub(crate) async fn variant_get<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<VariantGetResult, AppError> {
    variant_get_impl(app, id)
        .await
        .inspect_err(crate::logging::variant_get_failed)
}

/// The body of [`variant_get`], split the family way so the one
/// logging edge covers every failure path: the missing database and
/// the by-id read.
///
/// Complexity over untrusted input: the id is one indexed lookup;
/// the answer copies the row the store read (linear in the row's own
/// text, DB-006's bound).
async fn variant_get_impl<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<VariantGetResult, AppError> {
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            // The store's by-id reader serves active and soft-deleted
            // rows alike (its own "Restore variant" surface's read):
            // the answer carries the state, the caller reads it.
            let variant = get_variant(transaction, &id)
                .map_err(library_store_to_app_error)?
                // The typed refusal of the `exclusion_remove`
                // unknown-id shape (D-198): caller input that names
                // no row — nothing was written.
                .ok_or_else(|| rejected_field("id", &id, "no variant with this id"))?;
            Ok(VariantGetResult::from_store(variant))
        })
        .await
}

// ---------------------------------------------------------------------------
// `variant_create` (T-087, §7.10 Library module, §7.3 LIB-017…022,
// §6.9, DB-010, ALG-070/071, LOG-001…004)
// ---------------------------------------------------------------------------

/// Adds one variant to an instruction (§7.3 LIB-017…022's "Add
/// variant"): a copy of the main one or an empty one, with the
/// CALLER's name.
///
/// The name is §6.9-validated before anything runs (1–100 characters
/// after trimming, one line, no NUL) and the store enforces the
/// NOCASE uniqueness among the instruction's ACTIVE variants
/// (§6.9, the `ux_variant_name` scope — a soft-deleted twin does not
/// block). The "first free name `Variant N`" prefill is the dialog's
/// own affordance (a UI task): the command takes the name the caller
/// chose and answers the typed refusal when a retry converges on a
/// taken one.
///
/// [`VariantSource::CopyMain`] reads the main variant's CURRENT
/// stored text inside the SAME transaction — the copy cannot drift
/// from a concurrent edit (the one connection serializes the read
/// and the write, D-037). An instruction with no active main (DB-010
/// already broken out-of-band) is surfaced, never guessed at (the
/// DB-017 posture). A created variant is never main: the first
/// active variant of an instruction is its main by construction,
/// and `variant_set_main` owns every later flag move.
///
/// One transaction: the insert, the `create` version (ALG-071: the
/// entity's first, `rev_no = 1`) and the `variant.created` journal
/// entry (LOG-001…004, the variant's entity columns) commit or roll
/// back together. Recovery mode (no database) answers the honest
/// [`managed_db`] fallback.
///
/// Failures reach the log with their §10 code
/// ([`logging::variant_create_failed`]); a successful create is the
/// journal entry itself (the D-051 mirror policy).
#[tauri::command]
#[specta::specta]
pub(crate) async fn variant_create<R: Runtime>(
    app: AppHandle<R>,
    instruction_id: String,
    name: String,
    source: VariantSource,
) -> Result<VariantRecord, AppError> {
    variant_create_impl(app, instruction_id, name, source)
        .await
        .inspect_err(crate::logging::variant_create_failed)
}

/// The body of [`variant_create`], split so the one logging edge
/// covers every failure path (the `activity_list` shape).
///
/// Complexity over untrusted input: the name validation is one
/// linear pass over at most 100 characters; the copy mode reads the
/// instruction's variant list once (bounded by the screen's own
/// size); everything else is the store's own single-row insert.
async fn variant_create_impl<R: Runtime>(
    app: AppHandle<R>,
    instruction_id: String,
    name: String,
    source: VariantSource,
) -> Result<VariantRecord, AppError> {
    // §6.9 first, before the database is even resolved: a pure
    // validation with no lock and no I/O (the `scan_commit` gate
    // order).
    let validated_name = validate_variant_name(&name).map_err(library_field_to_app_error)?;
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            // The instruction must exist and be an instruction — the
            // pre-read the copy mode needs anyway (the store's own
            // check is the same rule one layer down; the pre-read
            // shapes the `instruction_id` field refusal).
            let item = get_item(transaction, &instruction_id)
                .map_err(library_store_to_app_error)?
                .ok_or_else(|| {
                    rejected_field(
                        "instruction_id",
                        &instruction_id,
                        "no instruction with this id",
                    )
                })?;
            if item.kind != ItemKind::Instruction {
                return Err(rejected_field(
                    "instruction_id",
                    &instruction_id,
                    "a heading has no variants",
                ));
            }
            // LIB-017…022's two modes. The copy is the main's stored
            // text as it stands inside this transaction.
            let content = match source {
                VariantSource::CopyMain => {
                    let variants = list_variants(transaction, &instruction_id)
                        .map_err(library_store_to_app_error)?;
                    variants
                        .into_iter()
                        .find(|variant| variant.is_main && variant.state == VariantState::Active)
                        .ok_or_else(|| {
                            field_refusal(
                                "instruction_id",
                                "the instruction has no active main variant to copy (DB-010)",
                            )
                        })?
                        .content
                }
                VariantSource::Empty => String::new(),
            };
            // The insert: the store folds the content (a no-op over
            // the already-folded copy), enforces the name CHECKs and
            // the NOCASE twin, and demands an active main (DB-010's
            // constructive half — this command never creates one).
            let variant = create_variant(
                transaction,
                &NewVariant {
                    instruction_id: &instruction_id,
                    name: &validated_name,
                    content: &content,
                    is_main: false,
                },
            )
            .map_err(library_store_to_app_error)?;
            // §9.8: the create is the entity's first version — the
            // hash gate always inserts here (no prior version).
            record_version(transaction, &variant, VariantOp::Create)?;
            journal_variant(transaction, mirror, Action::VariantCreated, &variant, None)?;
            Ok(VariantRecord::from_store(variant))
        })
        .await
}

// ---------------------------------------------------------------------------
// `variant_update` (T-088, §7.10 Library module, §7.3 LIB-009…016,
// §6.9, DB-006, DB-016, ALG-070)
// ---------------------------------------------------------------------------

/// Saves a variant's name and text (§7.3 LIB-009…016): each field is
/// optional — `None` (a JSON `null`) leaves that field unchanged,
/// the store's own update shape.
///
/// The rules, in §6.9's order: the name is 1–100 characters after
/// trimming, one line, NUL-free; the text is at most 262,144
/// characters, NUL-free, and free of the DB-016 reserved marker
/// lines. §10's own rows split the refusals: the field rules answer
/// `E_VALIDATION` naming the field (`variant_name`/`variant_text`)
/// with the measured `length`/`index` riding `params`; a reserved
/// marker line answers `E_CONTENT_RESERVED` with the built form's
/// 1-based `line` and the D-19 `marker` literal. The CRLF→LF fold is
/// the store's own save-time clause (D-076): the stored form — and
/// this answer's `content` — is the folded form.
///
/// A version is created only on a real change (ALG-070: the
/// snapshot hash against the entity's last version; D-16) — a save
/// without changes writes no version, and the journal stays silent
/// with it (the D-051 no-false-claim rule). A genuine change is ONE
/// version and ONE `variant.changed` entry, whatever the field
/// combination. A soft-deleted variant cannot be edited (restore it
/// first); the NOCASE twin of an active sibling refuses the rename.
///
/// One transaction: the row write, the version row and the journal
/// entry. Recovery mode answers the honest [`managed_db`] fallback.
///
/// Failures reach the log with their §10 code
/// ([`logging::variant_update_failed`]); a successful save is the
/// journal entry itself (the D-051 mirror policy) — a no-op save
/// writes neither.
#[tauri::command]
#[specta::specta]
pub(crate) async fn variant_update<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    name: Option<String>,
    content: Option<String>,
) -> Result<VariantRecord, AppError> {
    variant_update_impl(app, id, name, content)
        .await
        .inspect_err(crate::logging::variant_update_failed)
}

/// The body of [`variant_update`], split the `activity_list` way.
///
/// Complexity over untrusted input: each present field gets one
/// linear validator pass (the text's three passes — length, NUL,
/// markers — are the core validator's own §6.9 order); the store's
/// write and the snapshot are single-row.
async fn variant_update_impl<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    name: Option<String>,
    content: Option<String>,
) -> Result<VariantRecord, AppError> {
    // §6.9 first, field by field, before the database is even
    // resolved. The validator returns the input unchanged (D-076's
    // validation/normalization split): the trim of the name is the
    // one transformation this boundary owns; the CRLF fold stays the
    // store's.
    let validated_name: Option<String> = match name {
        Some(raw) => Some(validate_variant_name(&raw).map_err(library_field_to_app_error)?),
        None => None,
    };
    let validated_content: Option<String> = match content {
        Some(raw) => Some(
            validate_variant_text(&raw)
                .map(str::to_owned)
                .map_err(library_field_to_app_error)?,
        ),
        None => None,
    };
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            let update = VariantUpdate {
                name: validated_name.as_deref(),
                content: validated_content.as_deref(),
            };
            // The fieldless call is the store's own verified no-op
            // (nothing is written, `updated_at` is not churned).
            let variant =
                update_variant(transaction, &id, &update).map_err(library_store_to_app_error)?;
            // ALG-070: the version — and with it the journal entry —
            // exists only for a real change of the snapshot (D-16).
            match record_version(transaction, &variant, VariantOp::Update)? {
                Recorded::Inserted { .. } => {
                    journal_variant(transaction, mirror, Action::VariantChanged, &variant, None)?;
                }
                // A save whose folded outcome equals the stored
                // values: no version, no journal (the D-051
                // no-false-claim rule).
                Recorded::Unchanged => {}
            }
            Ok(VariantRecord::from_store(variant))
        })
        .await
}

// ---------------------------------------------------------------------------
// `variant_set_main` (T-089, §7.10 Library module, §7.3
// LIB-017…022, DB-010, D-16)
// ---------------------------------------------------------------------------

/// Makes the variant its instruction's main one (§7.3 LIB-021 "Make
/// main"), demoting the previous main — after the change the
/// instruction has exactly one active main variant (DB-010, THE
/// card criterion).
///
/// The flag change is metadata, not a snapshot change (D-16): no
/// version is written — `set_main` owns neither the name nor the
/// text of the §9.8 snapshot. The journal entry is the `variant.
/// changed` action's own "main choice" member. Promoting the current
/// main (the UI's disabled button, defensively re-sent) is the
/// verified no-op: nothing is written, journaled or versioned — an
/// already-main variant KEEPS its flag while empty (LIB-036's badge
/// rule). Promoting a variant whose normalized text is empty is
/// refused instead (LIB-036, the defensive surface behind the
/// editor's disabled star): the D-13/ALG-002 `is_empty` oracle —
/// the same derivation [`VariantGetResult`]'s flag carries — with
/// the typed `E_VALIDATION` refusal naming `id`, decided inside the
/// transaction before any write. A soft-deleted variant cannot take
/// the flag; an unknown id is the typed by-id refusal.
///
/// One transaction: the demote/promote pair and the journal entry
/// (the store's two-statement swap stays atomic inside it — the
/// `ux_variant_main` partial index would reject any other order).
/// Recovery mode answers the honest [`managed_db`] fallback.
///
/// Failures reach the log with their §10 code
/// ([`logging::variant_set_main_failed`]); a successful promotion is
/// the journal entry itself (the D-051 mirror policy).
#[tauri::command]
#[specta::specta]
pub(crate) async fn variant_set_main<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<VariantRecord, AppError> {
    variant_set_main_impl(app, id)
        .await
        .inspect_err(crate::logging::variant_set_main_failed)
}

/// The body of [`variant_set_main`], split the `activity_list` way.
async fn variant_set_main_impl<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<VariantRecord, AppError> {
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            // The pre-read: the by-id refusal and the verified no-op
            // (promoting the current main is a net nothing — the
            // store would demote and re-promote it).
            let variant = get_variant(transaction, &id)
                .map_err(library_store_to_app_error)?
                .ok_or_else(|| rejected_field("id", &id, "no variant with this id"))?;
            if variant.is_main && variant.state == VariantState::Active {
                return Ok(VariantRecord::from_store(variant));
            }
            // LIB-036: an empty variant cannot be made the main one —
            // the D-13/ALG-002 emptiness oracle, the exact derivation
            // `variant_get`'s `is_empty` flag and `library_get`'s
            // inline entries carry (D-232/D-187 — no fork). Active
            // rows alone: a soft-deleted operand keeps the store's
            // own `VariantNotActive` refusal below. The guard sits
            // after the verified no-op (an already-main variant keeps
            // its flag while empty — LIB-036's badge rule) and before
            // the write: a refused promotion writes no row, no
            // version and no journal entry (the D-231 validate-first
            // seam; the mirror buffer never records).
            if variant.state == VariantState::Active
                && eunomia_core::normalize::variant_text(&variant.content).is_empty()
            {
                return Err(rejected_field(
                    "id",
                    &id,
                    "an empty variant cannot be made the main variant",
                ));
            }
            // The flag swap: DB-010's exactly-one holds through the
            // demote-then-promote order the store owns.
            let promoted =
                set_main_variant(transaction, &id).map_err(library_store_to_app_error)?;
            // D-16: no version for a main-flag change — the journal's
            // "main choice" member is the whole record.
            journal_variant(transaction, mirror, Action::VariantChanged, &promoted, None)?;
            Ok(VariantRecord::from_store(promoted))
        })
        .await
}

// ---------------------------------------------------------------------------
// `variant_delete` (T-090, §7.10 Library module, §7.3 LIB-023…025,
// §6.4, DB-012, D-12/D-125, ALG-071)
// ---------------------------------------------------------------------------

/// Soft-deletes a variant (§6.4) after moving its assignments to one
/// replacement — LIB-023's flow, one replacement for ALL of the
/// variant's assignments.
///
/// `replacement: None` (LIB-023's default, D-125's C3) moves every
/// assignment to the instruction's main variant; `Some(id)` names
/// the successor instead — an ACTIVE variant of the SAME
/// instruction. Deleting the MAIN variant requires the named
/// successor (the typed refusal otherwise — the card's "when
/// deleting the main a new main is mandatory"); the successor then
/// takes over the main flag with the assignments (DB-010 stays at
/// exactly one). D-125's C2 ruling — "the main variant cannot be
/// deleted at all" — is the UI-level rule (the disabled Delete
/// button): the command keeps the store's full contract, the layering
/// the spec's own LIB-023/024/025 row records for the defensive
/// surfaces behind that button.
///
/// The last active variant of an instruction cannot be deleted
/// (LIB-025, DB-010's "at least one") — the absolute refusal,
/// whatever the replacement names. Deleting an already-deleted
/// variant is the verified no-op (a stale screen's double click):
/// the row comes back unchanged, nothing journaled or versioned.
///
/// One transaction (D-12's "everything in one transaction", §6.9's
/// "an entity is never saved partly"): the assignment moves, the
/// soft delete, the `delete` version (ALG-071) and the
/// `variant.deleted` journal entry — whose `Details::Bulk` carries
/// the moved-assignment count, the number the LIB-023 confirmation
/// names. A failure between the writes rolls back everything (a
/// staged-trigger test pins it). Recovery mode answers the honest
/// [`managed_db`] fallback.
///
/// Failures reach the log with their §10 code
/// ([`logging::variant_delete_failed`]); a successful delete is the
/// journal entry itself (the D-051 mirror policy).
#[tauri::command]
#[specta::specta]
pub(crate) async fn variant_delete<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    replacement: Option<String>,
) -> Result<VariantRecord, AppError> {
    variant_delete_impl(app, id, replacement)
        .await
        .inspect_err(crate::logging::variant_delete_failed)
}

/// The body of [`variant_delete`], split the `activity_list` way.
///
/// Complexity over untrusted input: the ids are opaque strings read
/// once; the assignment census is one indexed read bounded by the
/// assignments the variant actually holds.
async fn variant_delete_impl<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    replacement: Option<String>,
) -> Result<VariantRecord, AppError> {
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            // The pre-read: the by-id refusal and the verified no-op
            // (the store's delete is idempotent; the journal must not
            // claim a second transition that did not happen).
            let variant = get_variant(transaction, &id)
                .map_err(library_store_to_app_error)?
                .ok_or_else(|| rejected_field("id", &id, "no variant with this id"))?;
            if variant.state == VariantState::Deleted {
                return Ok(VariantRecord::from_store(variant));
            }
            // The count the confirmation and the journal's Bulk
            // payload name — read BEFORE the move, inside the same
            // transaction.
            let moved = list_assignments_for_variant(transaction, &id)
                .map_err(library_store_to_app_error)?
                .len();
            // The store's own order: assignments move first (DB-012:
            // nothing disappears silently), then the soft delete,
            // then the successor's promotion.
            let deleted = delete_variant(transaction, &id, replacement.as_deref())
                .map_err(library_store_to_app_error)?;
            // §9.8: the delete is a version (the snapshot-blind op —
            // a genuine transition always records).
            record_version(transaction, &deleted, VariantOp::Delete)?;
            journal_variant(
                transaction,
                mirror,
                Action::VariantDeleted,
                &deleted,
                Some(Details::Bulk {
                    changed: u64::try_from(moved)
                        .expect("an assignment table wider than u64 rows is impossible"),
                }),
            )?;
            Ok(VariantRecord::from_store(deleted))
        })
        .await
}

// ---------------------------------------------------------------------------
// `variant_restore` (T-091, §7.10 Library module, §7.3 LIB-026/027,
// ALG-071, D-009)
// ---------------------------------------------------------------------------

/// Brings a soft-deleted variant back (§7.3 LIB-026/027, the command
/// name fixed by D-009 and §7.10): `state` returns to `active`,
/// `deleted_at` clears, a `restore` version is recorded and a
/// `variant.restored` journal entry written.
///
/// The variant returns WITHOUT its old main flag (the delete cleared
/// it; DB-010 keeps exactly one active main — `variant_set_main` is
/// the way back) and without its old assignments (they moved to a
/// sibling by the delete; assigning is the assignment commands'
/// business). The name may have been REUSED while the variant was
/// deleted (the NOCASE index covers active rows only): a collision
/// is the `E_VALIDATION` field refusal naming `variant_name`, and
/// nothing changes — THE card criterion. Restoring an active variant
/// is the verified no-op; an unknown id is the typed by-id refusal.
///
/// One transaction: the restore, the `restore` version and the
/// journal entry. Recovery mode answers the honest [`managed_db`]
/// fallback.
///
/// Failures reach the log with their §10 code
/// ([`logging::variant_restore_failed`]); a successful restore is
/// the journal entry itself (the D-051 mirror policy).
#[tauri::command]
#[specta::specta]
pub(crate) async fn variant_restore<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<VariantRecord, AppError> {
    variant_restore_impl(app, id)
        .await
        .inspect_err(crate::logging::variant_restore_failed)
}

/// The body of [`variant_restore`], split the `activity_list` way.
async fn variant_restore_impl<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<VariantRecord, AppError> {
    let database = managed_db(&app)?;
    database
        .run_with_mirror(move |transaction, mirror| {
            // The pre-read: the by-id refusal and the verified no-op
            // (the store's restore is idempotent over an active row).
            let variant = get_variant(transaction, &id)
                .map_err(library_store_to_app_error)?
                .ok_or_else(|| rejected_field("id", &id, "no variant with this id"))?;
            if variant.state == VariantState::Active {
                return Ok(VariantRecord::from_store(variant));
            }
            // The name-collision check is the store's own (the NOCASE
            // twin of an ACTIVE sibling — THE card criterion's
            // `E_VALIDATION`).
            let restored = restore_variant(transaction, &id).map_err(library_store_to_app_error)?;
            // §9.8: the restore is a version (the snapshot-blind op —
            // it alternates with delete, so a genuine transition
            // always records).
            record_version(transaction, &restored, VariantOp::Restore)?;
            journal_variant(
                transaction,
                mirror,
                Action::VariantRestored,
                &restored,
                None,
            )?;
            Ok(VariantRecord::from_store(restored))
        })
        .await
}
