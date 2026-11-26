//! Action-log repository (spec §7.7 LOG-001…005, §6.2).
//!
//! The `action_log` table is the journal of everything the user and the
//! app did: projects, path exclusions (§6.4), settings, instructions,
//! variants, assignments, headings, sync runs, backups, errors
//! (LOG-001…004, plus the recorded beyond-requirement members — see
//! [`Action`]). Every entry carries the time, the level, the
//! visibility (LOG-005: `user` entries feed the "Activity" window,
//! `diagnostic` entries live only in the file log and diagnostics),
//! the action, the object it happened to, the result, and — for file
//! writes — the project and path.
//!
//! # Transactional semantics
//!
//! [`insert`] takes a `&Connection` and is a single statement, so it
//! joins whatever transaction the caller has open (the
//! [`settings::set`] shape): a successful operation writes its entry
//! **inside the operation's own transaction**, and if the operation
//! rolls back, the entry rolls back with it — the journal must not
//! claim an action that never happened. The mirror case is resolved
//! the other way around: an entry whose `result` is
//! [`Outcome::Error`] usually describes an operation whose transaction
//! *was rolled back*, so it cannot live in that transaction (it would
//! vanish with it). The producer follows the fail-then-log pattern:
//! let the operation's transaction roll back, then call [`insert`]
//! again in the **next** transaction (the executor's next run) with
//! the failed action and [`Outcome::Error`]. Both halves are pinned by
//! the integration tests: a rolled-back operation leaves no entry,
//! and the fail-then-log entry survives.
//!
//! # SEC-008: `details` cannot carry instruction text
//!
//! The `details` column is JSON, and SEC-008 (spec §13) holds for the
//! database journal exactly as for the file log: instruction text and
//! file content never enter it; paths, codes, IDs and counters do.
//! The mechanism is structural, in the D-032 facade spirit: [`Details`]
//! is a closed enum whose variants accept only numbers, paths and the
//! validated [`FailureCode`] newtype — there is no variant that takes
//! free text, so the variant content a failed operation held in scope
//! has nowhere to go. The unit tests pin that contract from both
//! sides: the canonical JSON forms, and the rejection of instruction-
//! shaped strings by [`FailureCode::new`].
//!
//! # Limits and cleanup (LOG-006, §6.4 "Log and runs")
//!
//! Retention is one housekeeping concern with one entry point,
//! [`cleanup`]: a journal entry is deleted when it falls outside the
//! newest `log.max_entries` entries **or** is strictly older than
//! `log.max_days` days (the disjunction of LOG-006 — either limit
//! alone removes), and `sync_runs` keeps the last
//! [`SYNC_RUNS_KEPT`] runs. The limits arrive as a [`LogLimits`]
//! parameter — the settings repository owns their validation
//! (D-039) — and `cleanup` joins the caller's transaction like
//! [`insert`] does, so the app-layer executor reads the settings and
//! prunes in one transaction.
//!
//! Reading comes in two seams with two different orderings, both
//! deterministic. [`recent`] is the journal-replay seam: the newest
//! `limit` entries by **insert order** (descending id) — what "the
//! last N things written" means. [`list`] is the "Activity" window's
//! query (T-027, §7.7 UI row): filtered by date range, project,
//! action, result and visibility, paginated, ordered by `ts DESC,
//! id DESC` — **time-primary** (the user-visible axis of the window),
//! with the id as the tiebreaker so equal-millisecond entries have a
//! stable order across pages. The two orderings agree on a
//! healthy clock and diverge only after a backward clock step (a
//! late insert carrying an earlier `ts`): `recent` answers "in what
//! order was this written", `list` answers "when does this say it
//! happened".

use std::fmt;
use std::path::PathBuf;

use jiff::{SignedDuration, Timestamp};
use rusqlite::Connection;

use crate::storage::StorageError;
use crate::values::now_utc;

/// The severity of an entry (the `action_log.level` CHECK of §6.2).
///
/// The natural pairing with the result is the producer's to choose,
/// with one convention this module documents: an entry whose result is
/// [`Outcome::Error`] is written at [`Level::Error`] — a failure the
/// journal records is a failure the journal shouts about (§10's "every
/// error is written to the log").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    /// A routine, successful operation.
    Info,
    /// An operation that was skipped or degraded, but not failed.
    Warn,
    /// A failed operation or a detected fault (DB-017).
    Error,
}

impl Level {
    /// Every member, in the CHECK's order.
    pub const ALL: [Level; 3] = [Level::Info, Level::Warn, Level::Error];

    /// The stored member string (§6.2 CHECK literal).
    pub const fn as_str(&self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }

    /// Parses a stored member string, rejecting anything outside the
    /// CHECK vocabulary (case-sensitive, like the schema).
    fn from_member(member: &str) -> Result<Self, ActionLogError> {
        match member {
            "info" => Ok(Level::Info),
            "warn" => Ok(Level::Warn),
            "error" => Ok(Level::Error),
            _ => Err(ActionLogError::Member {
                column: "level",
                found: member.to_owned(),
            }),
        }
    }
}

/// Who an entry is for (the `action_log.visibility` CHECK of §6.2;
/// LOG-005): `user` entries are shown in the "Activity" window,
/// `diagnostic` entries live only in the file log and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Visibility {
    /// Shown in the "Activity" window (LOG-005).
    User,
    /// Only the file log and diagnostics (LOG-005).
    Diagnostic,
}

impl Visibility {
    /// Every member, in the CHECK's order.
    pub const ALL: [Visibility; 2] = [Visibility::User, Visibility::Diagnostic];

    /// The stored member string (§6.2 CHECK literal).
    pub const fn as_str(&self) -> &'static str {
        match self {
            Visibility::User => "user",
            Visibility::Diagnostic => "diagnostic",
        }
    }

    /// Parses a stored member string, rejecting anything outside the
    /// CHECK vocabulary (case-sensitive, like the schema).
    fn from_member(member: &str) -> Result<Self, ActionLogError> {
        match member {
            "user" => Ok(Visibility::User),
            "diagnostic" => Ok(Visibility::Diagnostic),
            _ => Err(ActionLogError::Member {
                column: "visibility",
                found: member.to_owned(),
            }),
        }
    }
}

/// The outcome of the logged operation (the `action_log.result` CHECK
/// of §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// The operation succeeded.
    Ok,
    /// The operation failed (§10: the error's code travels in
    /// [`Details::Failure`]).
    Error,
    /// The operation deliberately did nothing (an unchanged target, a
    /// skipped project).
    Skipped,
}

impl Outcome {
    /// Every member, in the CHECK's order.
    pub const ALL: [Outcome; 3] = [Outcome::Ok, Outcome::Error, Outcome::Skipped];

    /// The stored member string (§6.2 CHECK literal).
    pub const fn as_str(&self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Error => "error",
            Outcome::Skipped => "skipped",
        }
    }

    /// Parses a stored member string, rejecting anything outside the
    /// CHECK vocabulary (case-sensitive, like the schema).
    ///
    /// Public for the same reason as [`Action::from_member`]: the
    /// `activity_list` result filter (§7.7 UI row) parses its wire
    /// value here, keeping the vocabulary's single owner.
    pub fn from_member(member: &str) -> Result<Self, ActionLogError> {
        match member {
            "ok" => Ok(Outcome::Ok),
            "error" => Ok(Outcome::Error),
            "skipped" => Ok(Outcome::Skipped),
            _ => Err(ActionLogError::Member {
                column: "result",
                found: member.to_owned(),
            }),
        }
    }
}

/// What kind of object an entry is about (the `action_log.entity_type`
/// column). The schema leaves the column free TEXT; this enum closes
/// it for every writer, so the journal's readers (T-027, diagnostics)
/// see one vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityType {
    /// A `projects` row.
    Project,
    /// A `project_exclusions` row (§6.4's path exclusion — the
    /// projects-domain sibling table, so the member sits next to
    /// [`EntityType::Project`]).
    Exclusion,
    /// A `library_items` row of kind `instruction`.
    Instruction,
    /// A `library_items` row of kind `heading`.
    Heading,
    /// An `instruction_variants` row.
    Variant,
    /// A `project_instructions` row (or the bulk set of them).
    Assignment,
    /// A `sync_runs` row.
    SyncRun,
    /// A `backup_records` row.
    Backup,
    /// A `settings` row (`id` is the dotted key).
    Setting,
}

impl EntityType {
    /// Every member.
    pub const ALL: [EntityType; 9] = [
        EntityType::Project,
        EntityType::Exclusion,
        EntityType::Instruction,
        EntityType::Heading,
        EntityType::Variant,
        EntityType::Assignment,
        EntityType::SyncRun,
        EntityType::Backup,
        EntityType::Setting,
    ];

    /// The stored member string.
    pub const fn as_str(&self) -> &'static str {
        match self {
            EntityType::Project => "project",
            EntityType::Exclusion => "exclusion",
            EntityType::Instruction => "instruction",
            EntityType::Heading => "heading",
            EntityType::Variant => "variant",
            EntityType::Assignment => "assignment",
            EntityType::SyncRun => "sync_run",
            EntityType::Backup => "backup",
            EntityType::Setting => "setting",
        }
    }

    /// Parses a stored member string, rejecting unknown values.
    fn from_member(member: &str) -> Result<Self, ActionLogError> {
        match member {
            "project" => Ok(EntityType::Project),
            "exclusion" => Ok(EntityType::Exclusion),
            "instruction" => Ok(EntityType::Instruction),
            "heading" => Ok(EntityType::Heading),
            "variant" => Ok(EntityType::Variant),
            "assignment" => Ok(EntityType::Assignment),
            "sync_run" => Ok(EntityType::SyncRun),
            "backup" => Ok(EntityType::Backup),
            "setting" => Ok(EntityType::Setting),
            _ => Err(ActionLogError::Member {
                column: "entity_type",
                found: member.to_owned(),
            }),
        }
    }
}

/// The closed action vocabulary of LOG-001…004 — what the journal
/// records. The variant list below is the requirement's own list,
/// member by member (see the module docs and DECISIONS.md for the
/// mapping table), followed by the beyond-requirement members the
/// app's own decisions added (each marked in place); future features
/// extend it consciously, like every other closed vocabulary in this
/// crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // LOG-001…004: "adding, changing, archiving, and restoring
    // projects" — manual add and scan commit, update/relink/reorder,
    // archive, restore.
    /// A project was added (manual or scan commit).
    ProjectAdded,
    /// A project's row changed (update, relink, reorder).
    ProjectChanged,
    /// A project was archived.
    ProjectArchived,
    /// A project was restored from the archived state.
    ProjectRestored,
    /// LOG-001…004: "changing settings".
    /// A setting was changed (`id` is the dotted key).
    SettingsChanged,
    // LOG-001…004: "creating, editing, hiding, and restoring
    // instructions".
    /// An instruction was created.
    InstructionCreated,
    /// An instruction was edited.
    InstructionChanged,
    /// An instruction was hidden.
    InstructionHidden,
    /// An instruction was restored from hidden.
    InstructionRestored,
    // LOG-001…004: "creating, editing, hiding, and restoring
    // variants" — for variants the "hiding" half is the soft delete
    // of §6.4 (there is no hidden state for variants).
    /// A variant was created.
    VariantCreated,
    /// A variant was edited (content, name, main choice).
    VariantChanged,
    /// A variant was soft-deleted (§6.4).
    VariantDeleted,
    /// A variant was restored from soft delete.
    VariantRestored,
    /// LOG-001…004: "changing assignments (one entry with the number
    /// of changes for a bulk operation)" — the number travels in
    /// [`Details::Bulk`].
    /// Assignments changed (single or bulk; the count is in details).
    AssignmentChanged,
    // LOG-001…004: "heading operations" — §6.4: headings are deleted
    // physically; renaming and moving is recorded only here.
    /// A heading was created.
    HeadingCreated,
    /// A heading was renamed, re-levelled (including a branch shift),
    /// or moved.
    HeadingChanged,
    /// A heading was deleted (physical delete, §6.4).
    HeadingDeleted,
    // LOG-001…004: "sync runs and their results per project" — the
    // run's own lifecycle entries (start, finish, abort — T-124/D-223
    // added the start and the abort beside D-051's finish) plus one
    // entry per project.
    /// A sync run started (§9.4 phase 0's record; the run-level
    /// bracket's opening half).
    SyncRunStarted,
    /// A sync run finished (the run-level summary).
    SyncRunFinished,
    /// A sync run aborted before finishing — the §10-coded stops
    /// (the backup failure's cancellation, S10's "cancel entry"; a
    /// pipeline error's terminal row), never the user-shaped stops
    /// (§10 names no code for those).
    SyncRunAborted,
    /// One project's outcome inside a sync run.
    SyncProjectResult,
    /// LOG-001…004: "backup creation" (BKP target-file copies; the
    /// path travels in [`Details::File`]).
    /// A backup of a project's target file was created.
    BackupCreated,
    /// LOG-001…004: "errors" — an error with no successful-action
    /// entry of its own (a failed operation's fail-then-log entry uses
    /// the failed action with [`Outcome::Error`]; this variant is for
    /// faults, e.g. a DB-017 invariant violation).
    /// An error or fault was reported.
    ErrorReported,
    // Beyond LOG-001…004: journal members the app's own decisions
    // added to the requirement's list, as consciously as the members
    // above. §6.4's one physical-delete entity outside the library —
    // the path exclusion — gets its own verb (the D-106 tracked
    // follow-up, landed by T-245): before it, `exclusion_remove`
    // journaled `project.changed` with no entity, a row attributable
    // to no object.
    /// A path exclusion was removed (§6.4 "Path exclusion", the
    /// physical delete; the row id travels in the entity columns as
    /// [`EntityType::Exclusion`], the path in [`Details::File`]).
    ExclusionRemoved,
}

impl Action {
    /// Every member, in the order the module docs group them.
    pub const ALL: [Action; 24] = [
        Action::ProjectAdded,
        Action::ProjectChanged,
        Action::ProjectArchived,
        Action::ProjectRestored,
        Action::SettingsChanged,
        Action::InstructionCreated,
        Action::InstructionChanged,
        Action::InstructionHidden,
        Action::InstructionRestored,
        Action::VariantCreated,
        Action::VariantChanged,
        Action::VariantDeleted,
        Action::VariantRestored,
        Action::AssignmentChanged,
        Action::HeadingCreated,
        Action::HeadingChanged,
        Action::HeadingDeleted,
        Action::SyncRunStarted,
        Action::SyncRunFinished,
        Action::SyncRunAborted,
        Action::SyncProjectResult,
        Action::BackupCreated,
        Action::ErrorReported,
        Action::ExclusionRemoved,
    ];

    /// The stored action string — `object.verb`, lowercase, the shape
    /// the schema tests already pin (`project.added`).
    pub const fn as_str(&self) -> &'static str {
        match self {
            Action::ProjectAdded => "project.added",
            Action::ProjectChanged => "project.changed",
            Action::ProjectArchived => "project.archived",
            Action::ProjectRestored => "project.restored",
            Action::SettingsChanged => "settings.changed",
            Action::InstructionCreated => "instruction.created",
            Action::InstructionChanged => "instruction.changed",
            Action::InstructionHidden => "instruction.hidden",
            Action::InstructionRestored => "instruction.restored",
            Action::VariantCreated => "variant.created",
            Action::VariantChanged => "variant.changed",
            Action::VariantDeleted => "variant.deleted",
            Action::VariantRestored => "variant.restored",
            Action::AssignmentChanged => "assignment.changed",
            Action::HeadingCreated => "heading.created",
            Action::HeadingChanged => "heading.changed",
            Action::HeadingDeleted => "heading.deleted",
            Action::SyncRunStarted => "sync.run_started",
            Action::SyncRunFinished => "sync.run_finished",
            Action::SyncRunAborted => "sync.run_aborted",
            Action::SyncProjectResult => "sync.project_result",
            Action::BackupCreated => "backup.created",
            Action::ErrorReported => "error.reported",
            Action::ExclusionRemoved => "exclusion.removed",
        }
    }

    /// Parses a stored action string, rejecting anything outside the
    /// closed vocabulary. A stored row can only carry a member (every
    /// writer goes through [`insert`]), so an unknown action means the
    /// row was written by a newer app version or tampered with — the
    /// read surfaces it instead of guessing (the same
    /// corrupt-cells-error policy as [`settings`], D-039).
    ///
    /// Public: the future IPC layer (§7.10 `activity_list`) parses its
    /// wire strings through this same parser, so the vocabulary has one
    /// owner — a filter the store would reject is a filter the command
    /// layer rejects identically, and a new action member needs no
    /// second match to keep in sync.
    pub fn from_member(member: &str) -> Result<Self, ActionLogError> {
        match member {
            "project.added" => Ok(Action::ProjectAdded),
            "project.changed" => Ok(Action::ProjectChanged),
            "project.archived" => Ok(Action::ProjectArchived),
            "project.restored" => Ok(Action::ProjectRestored),
            "settings.changed" => Ok(Action::SettingsChanged),
            "instruction.created" => Ok(Action::InstructionCreated),
            "instruction.changed" => Ok(Action::InstructionChanged),
            "instruction.hidden" => Ok(Action::InstructionHidden),
            "instruction.restored" => Ok(Action::InstructionRestored),
            "variant.created" => Ok(Action::VariantCreated),
            "variant.changed" => Ok(Action::VariantChanged),
            "variant.deleted" => Ok(Action::VariantDeleted),
            "variant.restored" => Ok(Action::VariantRestored),
            "assignment.changed" => Ok(Action::AssignmentChanged),
            "heading.created" => Ok(Action::HeadingCreated),
            "heading.changed" => Ok(Action::HeadingChanged),
            "heading.deleted" => Ok(Action::HeadingDeleted),
            "sync.run_started" => Ok(Action::SyncRunStarted),
            "sync.run_finished" => Ok(Action::SyncRunFinished),
            "sync.run_aborted" => Ok(Action::SyncRunAborted),
            "sync.project_result" => Ok(Action::SyncProjectResult),
            "backup.created" => Ok(Action::BackupCreated),
            "error.reported" => Ok(Action::ErrorReported),
            "exclusion.removed" => Ok(Action::ExclusionRemoved),
            _ => Err(ActionLogError::Member {
                column: "action",
                found: member.to_owned(),
            }),
        }
    }
}

/// A §10 registry error code (`E_SYNC_RUNNING`, `E_VALIDATION`, …) —
/// the code half of [`Details::Failure`].
///
/// A validating newtype, not a free string: [`FailureCode::new`]
/// accepts only the registry's shape (`E_` followed by uppercase
/// ASCII letters, digits and underscores) and rejects everything else
/// — the SEC-008 boundary, because the strings that must never enter
/// `details` (instruction text, file content, prose) all fail this
/// shape. The member list itself lives in the app layer's §7.10
/// registry (`errors.rs`); the store checks the form, not the
/// membership, so the two crates stay decoupled (D-017) and a new
/// code needs no store change.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FailureCode(String);

impl FailureCode {
    /// Validates `code` against the §10 registry shape and returns the
    /// typed code.
    ///
    /// # Errors
    ///
    /// Returns [`ActionLogError::FailureCode`] when `code` is not
    /// `E_` followed by one or more groups of uppercase ASCII letters
    /// and digits joined by single underscores.
    pub fn new(code: &str) -> Result<Self, ActionLogError> {
        if is_registry_shape(code) {
            Ok(FailureCode(code.to_owned()))
        } else {
            Err(ActionLogError::FailureCode {
                found: code.to_owned(),
            })
        }
    }

    /// The code string, for storage.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether `code` has the §10 registry shape: `E_` plus uppercase
/// ASCII alphanumeric groups joined by single underscores, no leading
/// or trailing underscore, at least one character after `E_`
/// (`E_A`-style single-letter tails are fine in principle but every
/// real code is longer; the shape stays permissive there on purpose —
/// the registry's membership is the app layer's fact).
fn is_registry_shape(code: &str) -> bool {
    let Some(tail) = code.strip_prefix("E_") else {
        return false;
    };
    if tail.is_empty() {
        return false;
    }
    let mut previous_underscore = false;
    for (index, ch) in tail.char_indices() {
        match ch {
            'A'..='Z' | '0'..='9' => previous_underscore = false,
            '_' => {
                // No double underscores, none leading (after `E_`),
                // none trailing.
                if previous_underscore || index == 0 || index + ch.len_utf8() == tail.len() {
                    return false;
                }
                previous_underscore = true;
            }
            // Anything else — spaces, lowercase, punctuation, the
            // bytes of instruction text — is not a registry code.
            _ => return false,
        }
    }
    !previous_underscore
}

impl fmt::Display for FailureCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The typed payload of an entry's `details` column (SEC-008 — see
/// the module docs). A closed enum: every variant's fields are
/// numbers, paths or a validated [`FailureCode`]; there is no variant
/// that accepts free text, so instruction content and file bodies
/// have no route into the column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Details {
    /// How many rows a bulk operation changed (LOG-001…004: "one
    /// entry with the number of changes for a bulk operation").
    Bulk {
        /// The number of changed assignments.
        changed: u64,
    },
    /// The path an entry is about (LOG-001…004: "for file writes, the
    /// project and path"; SEC-008: paths may be logged).
    File {
        /// The file's path.
        path: PathBuf,
    },
    /// Why an operation failed: its §10 error code, optionally the
    /// path the failure is about (§10: "every error is written to
    /// the log with its code … path"), and optionally the raw OS
    /// error code when an OS call produced the failure (§10's
    /// fourth field, T-124/D-223 — a plain number, SEC-008's
    /// numeric facts; `None` for failures no syscall backs, and
    /// omitted from the JSON then, so rows written before the field
    /// existed parse unchanged).
    Failure {
        /// The §10 registry code of the failure.
        code: FailureCode,
        /// The path involved in the failure, when one is.
        path: Option<PathBuf>,
        /// The OS error code behind the failure (`errno` /
        /// `GetLastError`), when an OS call produced it.
        os_code: Option<i32>,
    },
}

impl Details {
    /// The canonical JSON form stored in the `details` column:
    /// `{"changed":3}`, `{"path":"…"}`,
    /// `{"code":"E_…","path":"…","os_code":5}` (the path and the
    /// OS code of [`Details::Failure`] are omitted when absent;
    /// member order in these examples is presentation — serde_json's
    /// default map sorts the keys, and nothing consumes the order:
    /// the read side parses the cell as a raw `Value`).
    ///
    /// Paths are stored in their `Display` form, which is lossy for a
    /// non-UTF-8 path (each unrepresentable byte becomes U+FFFD) —
    /// the same convention the file log's path fields use, and the
    /// honest boundary of a TEXT column; the identifiers a path needs
    /// for correlation (`project_id`, `entity_id`) travel in their own
    /// columns.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Details::Bulk { changed } => serde_json::json!({ "changed": changed }),
            Details::File { path } => serde_json::json!({
                "path": path.display().to_string(),
            }),
            Details::Failure {
                code,
                path,
                os_code,
            } => {
                // Each absent member omitted (a legacy row's parser
                // reads absence, not a null, so old and new spellings
                // stay one shape); the map is serde_json's default
                // (key-sorted), the same as every other details cell.
                let mut json = serde_json::Map::new();
                json.insert("code".to_owned(), serde_json::json!(code.as_str()));
                if let Some(path) = path {
                    json.insert(
                        "path".to_owned(),
                        serde_json::json!(path.display().to_string()),
                    );
                }
                if let Some(os_code) = os_code {
                    json.insert("os_code".to_owned(), serde_json::json!(os_code));
                }
                serde_json::Value::Object(json)
            }
        }
    }
}

/// The object an entry is about: the `entity_type` + `entity_id`
/// pair of §6.2, typed. `id` is the entity's row identifier — a
/// UUIDv7 for most entities, the dotted key for a settings entry.
/// These fields are identifiers, not prose: SEC-008's structural
/// guarantee covers [`Details`] (the payload), while `id` and the
/// other `*_id` fields are `&str` the producer vouches for — a
/// producer that stuffs text into an id column bugs its own call
/// site, and a typed id wrapper would not stop it (any string parses
/// as "the id"); the future IPC layer's validation is where producer
/// inputs become typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entity<'a> {
    /// What kind of object `id` names.
    pub kind: EntityType,
    /// The object's identifier.
    pub id: &'a str,
}

/// One journal entry to write — everything except the time and the
/// autoincrement id, which [`insert`] owns.
///
/// The usual entry for a successful user action is
/// `level: Level::Info, visibility: Visibility::User, result:
/// Outcome::Ok`; failures are `Level::Error` with [`Details::Failure`]
/// carrying the §10 code; technical entries (a DB-017 invariant
/// finding, a daily housekeeping fact) are `Visibility::Diagnostic`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry<'a> {
    /// The severity.
    pub level: Level,
    /// Who the entry is for (LOG-005).
    pub visibility: Visibility,
    /// What happened.
    pub action: Action,
    /// The object it happened to, when the action is about one.
    pub entity: Option<Entity<'a>>,
    /// The project involved ("for file writes, the project and path",
    /// LOG-001…004; also the per-project sync results).
    pub project_id: Option<&'a str>,
    /// The sync run involved (sync entries).
    pub sync_run_id: Option<&'a str>,
    /// The operation's outcome.
    pub result: Outcome,
    /// The typed payload (SEC-008), when the entry needs one.
    pub details: Option<Details>,
}

impl<'a> LogEntry<'a> {
    /// Builds the minimal entry for `action` at `level`/`visibility`
    /// with `result` — the fields every producer sets first; the
    /// object, project, run and details are added on the returned
    /// value by struct update or mutation.
    #[must_use = "an unused entry writes nothing; pass it to `insert`"]
    pub fn new(action: Action, level: Level, visibility: Visibility, result: Outcome) -> Self {
        LogEntry {
            level,
            visibility,
            action,
            entity: None,
            project_id: None,
            sync_run_id: None,
            result,
            details: None,
        }
    }
}

/// One stored journal row, as [`recent`] and [`list`] return it: every
/// column, typed back. `details` comes back as parsed JSON (the read
/// path validates that the cell is JSON; interpreting the fields is
/// the reading task's business, T-027).
#[derive(Debug, Clone, PartialEq)]
pub struct LogRecord {
    /// The autoincrement id (§6.2).
    pub id: i64,
    /// The entry's time, DB-002 UTC ISO-8601.
    pub ts: String,
    /// The severity.
    pub level: Level,
    /// Who the entry is for (LOG-005).
    pub visibility: Visibility,
    /// What happened.
    pub action: Action,
    /// The object it happened to, when the entry names one.
    pub entity: Option<(EntityType, String)>,
    /// The project involved, when the entry names one.
    pub project_id: Option<String>,
    /// The sync run involved, when the entry names one.
    pub sync_run_id: Option<String>,
    /// The operation's outcome.
    pub result: Outcome,
    /// The parsed `details` JSON, when the entry has details.
    pub details: Option<serde_json::Value>,
}

/// Why an action-log read or construction failed.
#[derive(Debug)]
pub enum ActionLogError {
    /// A stored cell holds a value outside its closed vocabulary
    /// (level, visibility, result, action, entity_type). Every writer
    /// goes through [`insert`], so this means corruption or a row
    /// written by a newer app version; the read surfaces it instead
    /// of guessing (the D-039 corrupt-cells policy).
    Member {
        /// The column the value came from.
        column: &'static str,
        /// The offending value.
        found: String,
    },
    /// A `details` cell does not hold valid JSON.
    Json(serde_json::Error),
    /// A [`FailureCode`] was built from a string outside the §10
    /// registry shape (SEC-008: the shape check is what keeps
    /// instruction-shaped text out of the code field).
    FailureCode {
        /// The rejected string.
        found: String,
    },
    /// An [`ActivityQuery`] date **bound** is not the exact DB-002 TEXT
    /// shape. String range comparison over `ts` is only equivalent to
    /// chronological comparison when both bounds have the fixed
    /// `YYYY-MM-DDTHH:MM:SS.sssZ` shape ([`values::is_db002_shape`]);
    /// anything else would silently mis-order, so it is rejected
    /// instead of guessed at. Distinct from [`ActionLogError::TsCell`]
    /// (the read-side twin): this variant is the **caller's input**
    /// and maps to `E_VALIDATION` at the command boundary; the cell
    /// variant is stored data and takes the corrupt-cells policy.
    Timestamp {
        /// Which bound was invalid (`"from_ts"`/`"to_ts"`).
        field: &'static str,
        /// The rejected value.
        found: String,
    },
    /// A **stored** `ts` cell is not the exact DB-002 shape — the
    /// read-side half of the shape contract (the D-039 corrupt-cells
    /// policy applied to the column the ordering and range semantics
    /// stand on): a machine-written row always carries
    /// [`values::now_utc`]'s shape, so a shapeless cell means
    /// corruption or version skew, and reading it must surface that
    /// instead of sorting it textually.
    TsCell {
        /// The offending stored value.
        found: String,
    },
    /// An [`ActivityQuery`] date range is inverted: `from_ts` is
    /// strictly later than `to_ts`. An inverted range matches nothing
    /// by accident of the comparison operators; rejecting it keeps
    /// "empty result" meaning "no entries in the range" (the D-039
    /// explicit-error-over-silent-default policy, applied to a query).
    Range {
        /// The rejected bounds, in order.
        from: String,
        /// The rejected bounds, in order.
        to: String,
    },
}

impl fmt::Display for ActionLogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionLogError::Member { column, found } => {
                write!(f, "{found:?} is not a valid {column} value")
            }
            ActionLogError::Json(err) => write!(f, "invalid details JSON: {err}"),
            ActionLogError::FailureCode { found } => {
                write!(
                    f,
                    "{found:?} is not a §10 registry code (E_UPPERCASE_DIGITS)"
                )
            }
            ActionLogError::Timestamp { field, found } => {
                write!(
                    f,
                    "{found:?} is not a DB-002 timestamp (YYYY-MM-DDTHH:MM:SS.sssZ) \
                     for {field}"
                )
            }
            ActionLogError::TsCell { found } => {
                write!(
                    f,
                    "a stored ts cell {found:?} is not a DB-002 timestamp \
                     (YYYY-MM-DDTHH:MM:SS.sssZ)"
                )
            }
            ActionLogError::Range { from, to } => {
                write!(f, "inverted date range: from {from} is later than to {to}")
            }
        }
    }
}

impl std::error::Error for ActionLogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ActionLogError::Json(err) => Some(err),
            _ => None,
        }
    }
}

/// Writes one entry and returns its autoincrement id.
///
/// The time is stamped here with the DB-002 UTC format ([`now_utc`]),
/// so every entry shares one timestamp convention regardless of its
/// producer. The statement joins the caller's transaction (the
/// [`settings::set`] shape): inside `Db::run`'s operation the entry
/// commits — or rolls back — **with** the operation; the fail-then-log
/// half of the journal's transactional semantics is the same function
/// called in the next transaction after the failed one rolled back
/// (see the module docs).
///
/// # Errors
///
/// Returns [`StorageError::Sql`] when the statement fails (a CHECK
/// rejection cannot happen through this API — the enums only produce
/// members — but a tampered schema or a closed connection can).
pub fn insert(conn: &Connection, entry: &LogEntry<'_>) -> Result<i64, StorageError> {
    let details = match &entry.details {
        Some(details) => {
            let json = details.to_json();
            // A serde_json::Value always serializes (the settings
            // repository's own expectation for the same shape).
            Some(serde_json::to_string(&json).expect("a serde_json::Value always serializes"))
        }
        None => None,
    };
    let entity_type = entry.entity.map(|entity| entity.kind.as_str());
    let entity_id = entry.entity.map(|entity| entity.id);
    conn.query_row(
        "INSERT INTO action_log
             (ts, level, visibility, action, entity_type, entity_id,
              project_id, sync_run_id, result, details)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         RETURNING id",
        rusqlite::params![
            now_utc(),
            entry.level.as_str(),
            entry.visibility.as_str(),
            entry.action.as_str(),
            entity_type,
            entity_id,
            entry.project_id,
            entry.sync_run_id,
            entry.result.as_str(),
            details,
        ],
        |row| row.get(0),
    )
    .map_err(StorageError::from)
}

/// Reads the newest `limit` entries, newest first (descending id).
///
/// The minimal read seam: the "Activity" window's filters and
/// pagination (by date, project, type, result) are T-027's; this
/// query proves the round-trip and gives that task its row shape.
///
/// Every vocabulary cell is validated on the way out: an unknown
/// action, level, visibility, result or entity_type, a `ts` cell
/// outside the DB-002 shape, or a `details` cell that is not valid
/// JSON, fails the read with [`StorageError::ActionLog`] (the D-039
/// corrupt-cells policy — the journal is machine-written, so a bad
/// cell means corruption or version skew). The `details` cell's
/// *contents* are validated for well-formed JSON only: `recent`
/// returns the parsed value as-is, and interpreting the payload (and
/// rejecting unknown shapes inside it) is the reading task's
/// business, T-027.
///
/// # Errors
///
/// Returns [`StorageError::ActionLog`] when a row fails validation
/// and [`StorageError::Sql`] when the query fails.
pub fn recent(conn: &Connection, limit: u32) -> Result<Vec<LogRecord>, StorageError> {
    let mut statement = conn.prepare(
        "SELECT id, ts, level, visibility, action, entity_type, entity_id,
                project_id, sync_run_id, result, details
         FROM action_log
         ORDER BY id DESC
         LIMIT ?1",
    )?;
    let records = statement
        .query_map([i64::from(limit)], |row| {
            Ok(LogRow {
                id: row.get(0)?,
                ts: row.get(1)?,
                level: row.get(2)?,
                visibility: row.get(3)?,
                action: row.get(4)?,
                entity_type: row.get(5)?,
                entity_id: row.get(6)?,
                project_id: row.get(7)?,
                sync_run_id: row.get(8)?,
                result: row.get(9)?,
                details: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    records.into_iter().map(LogRow::try_from_row).collect()
}

/// One "Activity" query (spec §7.7 UI row, T-027): which journal
/// entries to return, in what page. Every optional field is a filter
/// that must match (they combine with AND); `limit`/`offset` select
/// the page after filtering.
///
/// The store is the query executor, not the policy holder: it applies
/// exactly the filters it is handed, including [`Visibility`]. The
/// "user entries only unless diagnostics were explicitly requested"
/// default (LOG-005) is the IPC boundary's decision — there it is
/// untrusted input that chooses, here it is a typed parameter a
/// Rust-side caller sets deliberately.
///
/// Date bounds are compared as **text** against the `ts` column, and
/// that is only equivalent to chronological comparison because the
/// bounds must have the exact DB-002 shape
/// ([`values::is_db002_shape`] — fixed width, fixed offsets, always
/// UTC `Z`); anything else fails [`list`] with
/// [`ActionLogError::Timestamp`] instead of silently mis-ordering.
/// Both bounds are **inclusive** — with millisecond-resolution
/// timestamps, a whole day is `…T00:00:00.000Z` to `…T23:59:59.999Z`,
/// and an entry stamped exactly at a bound is inside the range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityQuery {
    /// The inclusive lower bound on `ts` (DB-002 shape), when set.
    pub from_ts: Option<String>,
    /// The inclusive upper bound on `ts` (DB-002 shape), when set.
    pub to_ts: Option<String>,
    /// The exact `project_id` to match, when set.
    pub project_id: Option<String>,
    /// The actions to match (the §7.7 "type" filter): the entry's
    /// action must be one of these. Empty means no action filter —
    /// every action matches.
    pub actions: Vec<Action>,
    /// The `result` to match, when set.
    pub result: Option<Outcome>,
    /// The `visibility` to match: `Some(v)` keeps only entries of that
    /// visibility, `None` keeps both (LOG-005 names the two audiences;
    /// which one a caller serves is the caller's fact).
    pub visibility: Option<Visibility>,
    /// The page size: how many entries to return at most. The IPC
    /// layer bounds this (a read-only window, not a dump); the store
    /// itself only requires that a `u32` page of rows fits in memory.
    pub limit: u32,
    /// How many matching entries to skip before the page begins —
    /// `offset 0` is the first (newest) page.
    pub offset: u32,
}

/// One page of the "Activity" query's result (T-027): the entries and
/// the total number of entries the **filters** match (independent of
/// `limit`/`offset`) — the count a paginating UI needs to render
/// "page N of M". Both queries run in the caller's transaction (the
/// app-layer executor of DECISIONS.md D-037 wraps them in one), so the
/// page and its total describe one consistent state.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityPage {
    /// The matching entries of the requested page, newest first
    /// (`ts DESC, id DESC`).
    pub records: Vec<LogRecord>,
    /// How many entries the filters match in total.
    pub total: u64,
}

/// Reads one filtered, paginated page of the journal — the "Activity"
/// window's query (spec §7.7 UI row; LOG-005).
///
/// Ordering is `ts DESC, id DESC`: time is the window's user-visible
/// axis, and the id tiebreaker makes the order of entries sharing one
/// millisecond deterministic — the same `ActivityQuery` always returns
/// the same pages ([`recent`] keeps its insert-order semantics for
/// journal replay; see the module docs). The `ix_log_ts` index serves
/// the range scans.
///
/// The statement text is assembled from fixed literals only — filter
/// values never enter the SQL string, they travel as bound parameters
/// (the `action IN (…)` placeholder list is generated `?`s), so no
/// filter value can change the query's shape.
///
/// Every vocabulary cell of every returned row is validated on the way
/// out, exactly like [`recent`] (the D-039 corrupt-cells policy):
/// an unknown action, level, visibility, result or entity_type, a
/// `ts` cell outside the DB-002 shape, or a `details` cell that is
/// not valid JSON, fails the whole read with
/// [`StorageError::ActionLog`].
///
/// # Errors
///
/// Returns [`StorageError::ActionLog`] when a date bound is not the
/// DB-002 shape ([`ActionLogError::Timestamp`]) or the range is
/// inverted ([`ActionLogError::Range`]), when a returned row fails
/// vocabulary, `ts`-shape ([`ActionLogError::TsCell`]) or JSON
/// validation, and [`StorageError::Sql`] when either statement fails.
pub fn list(conn: &Connection, query: &ActivityQuery) -> Result<ActivityPage, StorageError> {
    validate_query_bounds(query)?;
    // Filter clauses: fixed SQL fragments joined with AND, with every
    // value bound as a parameter. `conditions` and `params` grow in
    // lockstep — the Nth `?` of the assembled WHERE binds the Nth
    // parameter.
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(from) = &query.from_ts {
        conditions.push("ts >= ?".to_owned());
        params.push(rusqlite::types::Value::Text(from.clone()));
    }
    if let Some(to) = &query.to_ts {
        conditions.push("ts <= ?".to_owned());
        params.push(rusqlite::types::Value::Text(to.clone()));
    }
    if let Some(project_id) = &query.project_id {
        conditions.push("project_id = ?".to_owned());
        params.push(rusqlite::types::Value::Text(project_id.clone()));
    }
    if !query.actions.is_empty() {
        // Generated placeholders only — the action strings themselves
        // are bound parameters, never SQL text.
        let placeholders = vec!["?"; query.actions.len()].join(", ");
        conditions.push(format!("action IN ({placeholders})"));
        params.extend(
            query
                .actions
                .iter()
                .map(|action| rusqlite::types::Value::Text(action.as_str().to_owned())),
        );
    }
    if let Some(result) = query.result {
        conditions.push("result = ?".to_owned());
        params.push(rusqlite::types::Value::Text(result.as_str().to_owned()));
    }
    if let Some(visibility) = query.visibility {
        conditions.push("visibility = ?".to_owned());
        params.push(rusqlite::types::Value::Text(visibility.as_str().to_owned()));
    }
    let where_sql = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM action_log {where_sql}"),
        rusqlite::params_from_iter(params.iter().cloned()),
        |row| row.get(0),
    )?;

    let mut statement = conn.prepare(&format!(
        "SELECT id, ts, level, visibility, action, entity_type, entity_id,
                project_id, sync_run_id, result, details
         FROM action_log {where_sql}
         ORDER BY ts DESC, id DESC
         LIMIT ? OFFSET ?"
    ))?;
    let page_params = params.iter().cloned().chain([
        rusqlite::types::Value::Integer(i64::from(query.limit)),
        rusqlite::types::Value::Integer(i64::from(query.offset)),
    ]);
    let records = statement
        .query_map(rusqlite::params_from_iter(page_params), |row| {
            Ok(LogRow {
                id: row.get(0)?,
                ts: row.get(1)?,
                level: row.get(2)?,
                visibility: row.get(3)?,
                action: row.get(4)?,
                entity_type: row.get(5)?,
                entity_id: row.get(6)?,
                project_id: row.get(7)?,
                sync_run_id: row.get(8)?,
                result: row.get(9)?,
                details: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(LogRow::try_from_row)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ActivityPage {
        records,
        total: u64::try_from(total).unwrap_or(0),
    })
}

/// Validates the date bounds of an [`ActivityQuery`]: each bound must
/// have the exact DB-002 shape (the precondition under which text
/// comparison equals chronological comparison), and the range must not
/// be inverted.
fn validate_query_bounds(query: &ActivityQuery) -> Result<(), StorageError> {
    for (field, bound) in [("from_ts", &query.from_ts), ("to_ts", &query.to_ts)] {
        if let Some(found) = bound
            && !crate::values::is_db002_shape(found)
        {
            return Err(invalid(ActionLogError::Timestamp {
                field,
                found: found.clone(),
            }));
        }
    }
    if let (Some(from), Some(to)) = (&query.from_ts, &query.to_ts)
        && from > to
    {
        return Err(invalid(ActionLogError::Range {
            from: from.clone(),
            to: to.clone(),
        }));
    }
    Ok(())
}

// ---- LOG-006 retention (§6.4 "Log and runs", §6.5 limits) --------------------

/// How many `sync_runs` rows are kept (§6.4's "the last 500 runs").
///
/// A fixed number, not a setting: §6.5 has no key for it, and LOG-006
/// draws only the *log* limits from settings. The constant lives here,
/// next to the query that enforces it, so the §6.4 row and its
/// implementation cannot drift apart silently (a change is a spec
/// decision, made consciously in this file).
pub const SYNC_RUNS_KEPT: i64 = 500;

/// The LOG-006 log limits as [`cleanup`] consumes them: the §6.5
/// `log.max_entries` and `log.max_days` values.
///
/// Plain `i64` fields on purpose: the ranges (1000–100000, 7–3650)
/// have exactly one owner — the settings repository, which validates
/// them on every read and write ([`crate::settings`], D-039) — and
/// re-validating here would duplicate that canon. The precondition is
/// therefore part of the contract: a caller passes §6.5-validated
/// values (the app-layer seam reads both keys through
/// [`crate::settings::get`] in the same transaction as the prune).
/// Out-of-range values are the caller's bug and behave as SQLite
/// decides, which is **not** uniform: a *negative* `max_entries`
/// keeps everything on the count criterion (SQLite treats a negative
/// `LIMIT` as unlimited), while `max_entries = 0` empties the
/// keep-set and deletes the whole journal — a plausible "unlimited"
/// sentinel is exactly the value that wipes the table. A negative
/// `max_days` pushes the age cutoff into the future and deletes up
/// to everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogLimits {
    /// How many of the newest journal entries are kept
    /// (`log.max_entries`, §6.5: 1000–100000, default 10 000).
    pub max_entries: i64,
    /// How many days of journal history are kept (`log.max_days`,
    /// §6.5: 7–3650, default 180).
    pub max_days: i64,
}

/// What one [`cleanup`] run removed — the payload of the housekeeping
/// log line and the return value the app layer's degrade decision
/// hangs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CleanupCounts {
    /// How many `action_log` rows were deleted.
    pub log_entries: u64,
    /// How many `sync_runs` rows were deleted (their
    /// `sync_run_projects` rows went with them through the schema's
    /// `ON DELETE CASCADE`, D-036).
    pub sync_runs: u64,
}

/// Enforces the LOG-006 retention (§6.4 "Log and runs"): deletes
/// journal entries beyond the limits and sync runs beyond the last
/// [`SYNC_RUNS_KEPT`], and returns what it removed.
///
/// # The disjunction of the two log limits
///
/// A journal entry is deleted when **either** LOG-006 limit alone
/// rejects it: it is outside the newest `max_entries` entries
/// (ordered `ts DESC, id DESC` — the same deterministic ordering as
/// [`list`], so "the newest N" means the same thing here and in the
/// Activity window), **or** its `ts` is strictly older than the
/// cutoff `now − max_days` days ("10,000 entries **or** 180 days").
/// Equivalently, an entry survives only by being inside the newest N
/// *and* not older than the cutoff — so tightening either setting
/// prunes immediately. The age boundary is strict on purpose and
/// pinned by a test: an entry stamped *exactly* `max_days` days
/// before `now` is at the limit, not past it, and survives; one
/// millisecond older goes. The cutoff is compared as DB-002-shaped
/// text (`ts < ?`), which equals chronological comparison for that
/// exact shape ([`crate::values::is_db002_shape`] — the same
/// precondition the range filters of [`list`] stand on).
///
/// # Sync runs
///
/// `sync_runs` keeps the newest [`SYNC_RUNS_KEPT`] rows ordered
/// `started_at DESC, id DESC` — the run twin of the journal's
/// ordering, with the UUIDv7 id (D-031) breaking `started_at` ties.
/// Older runs are deleted outright (§6.4 has no "pruned" status for
/// runs); their `sync_run_projects` rows follow through the schema's
/// `ON DELETE CASCADE` (D-036 — requires `PRAGMA foreign_keys=ON`,
/// which the production open applies on every connection, D-030),
/// and `backup_records.sync_run_id` becomes NULL by the same
/// schema's `ON DELETE SET NULL` — a file backup outliving its run
/// keeps its row (BKP records are not run-scoped history). Journal
/// entries naming a deleted run keep their `sync_run_id` string: the
/// schema has no foreign key there (§6.2), and the journal is a
/// historical record — an id that names a pruned run still names
/// *what happened*.
///
/// # Transaction, idempotency, boundaries
///
/// Both statements join the caller's transaction (the [`insert`]
/// shape): a caller wrapping the settings read and this prune in one
/// transaction (the `Db::run` executor, D-037) gets one consistent
/// operation that rolls back as a set — pinned by a test that
/// rollbacks instead of committing. The operation is idempotent: a
/// second run over the pruned state removes nothing (also pinned).
/// Honest boundary: a stored `ts` cell outside the DB-002 shape (a
/// corrupt or hand-edited cell — every writer goes through
/// [`insert`]) is not validated here; the text comparisons stay
/// total-but-arbitrary for such a cell (it may or may not be pruned),
/// and surfacing the corruption stays the read path's job
/// ([`ActionLogError::TsCell`], the D-039 corrupt-cells policy).
///
/// # Errors
///
/// Returns [`StorageError::Sql`] when either statement fails.
///
/// # Panics
///
/// Panics only when the cutoff arithmetic overflows a `jiff`
/// `Timestamp` — impossible for §6.5-validated `max_days` values
/// (7–3650; see [`LogLimits`]).
pub fn cleanup(
    conn: &Connection,
    limits: &LogLimits,
    now: Timestamp,
) -> Result<CleanupCounts, StorageError> {
    let cutoff = age_cutoff(now, limits.max_days);
    let log_entries = conn.execute(
        "DELETE FROM action_log
          WHERE id NOT IN (
              SELECT id FROM action_log ORDER BY ts DESC, id DESC LIMIT ?1
          )
             OR ts < ?2",
        rusqlite::params![limits.max_entries, cutoff],
    )?;
    let sync_runs = conn.execute(
        "DELETE FROM sync_runs
          WHERE id NOT IN (
              SELECT id FROM sync_runs
               ORDER BY started_at DESC, id DESC LIMIT ?1
          )",
        rusqlite::params![SYNC_RUNS_KEPT],
    )?;
    Ok(CleanupCounts {
        log_entries: u64::try_from(log_entries).unwrap_or(0),
        sync_runs: u64::try_from(sync_runs).unwrap_or(0),
    })
}

/// The `ts` value an entry must not be strictly older than:
/// `now − max_days` days, rendered in the DB-002 shape so the SQL
/// text comparison `ts < cutoff` equals the chronological one.
///
/// Days are exactly 24 h here (a `SignedDuration`, not a calendar
/// `Span`): `ts` cells are UTC instants, and LOG-006's "days" bounds
/// the age of an instant — no timezone exists that could make a day
/// anything but 86 400 s for this comparison.
///
/// The cutoff is rendered at the DB-002 millisecond resolution —
/// `%.3f` truncates (jiff's own documented behavior, the same
/// convention [`crate::values::now_utc`] stamps cells with), so both
/// sides of the comparison live at the system's own precision and
/// the boundary is exact there; a `now` with sub-millisecond digits
/// simply rounds the cutoff down to the last whole millisecond.
fn age_cutoff(now: Timestamp, max_days: i64) -> String {
    let days = SignedDuration::from_secs(max_days.saturating_mul(86_400));
    now.checked_sub(days)
        .expect("§6.5 bounds max_days to 7–3650; the cutoff cannot leave the Timestamp range")
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// The raw strings of one row, as rusqlite read them — the decode
/// step ([`LogRecord::try_from_row`]) is fallible on the vocabulary
/// columns, and rusqlite's `query_map` closure must stay infallible.
struct LogRow {
    id: i64,
    ts: String,
    level: String,
    visibility: String,
    action: String,
    entity_type: Option<String>,
    entity_id: Option<String>,
    project_id: Option<String>,
    sync_run_id: Option<String>,
    result: String,
    details: Option<String>,
}

impl LogRow {
    /// Decodes one raw row into a [`LogRecord`], validating every
    /// vocabulary cell and the `details` JSON.
    ///
    /// `ts` is validated against the DB-002 shape
    /// ([`values::is_db002_shape`]) like every other column: `ts` is
    /// the column the read's ordering and range semantics stand on
    /// (text comparison equals chronological comparison only for this
    /// exact shape), so a shapeless stored value is a corrupt cell the
    /// read must surface — not silently sort textually (the D-039
    /// policy, extended to `ts` by the T-027 round-1 review).
    fn try_from_row(self) -> Result<LogRecord, StorageError> {
        if !crate::values::is_db002_shape(&self.ts) {
            return Err(invalid(ActionLogError::TsCell { found: self.ts }));
        }
        let entity = match (self.entity_type, self.entity_id) {
            (Some(kind), Some(id)) => Some((EntityType::from_member(&kind).map_err(invalid)?, id)),
            // The pair is written together or not at all; a row with
            // exactly one half is corruption, and reading it as "no
            // entity" would silently drop the half that exists.
            (None, None) => None,
            (Some(kind), None) => {
                return Err(invalid(ActionLogError::Member {
                    column: "entity_id",
                    found: format!("{kind:?} entry without entity_id"),
                }));
            }
            (None, Some(id)) => {
                return Err(invalid(ActionLogError::Member {
                    column: "entity_type",
                    found: format!("entity_id {id:?} without entity_type"),
                }));
            }
        };
        let details = match self.details {
            Some(text) => {
                Some(
                    serde_json::from_str(&text).map_err(|source| StorageError::ActionLog {
                        source: ActionLogError::Json(source),
                    })?,
                )
            }
            None => None,
        };
        Ok(LogRecord {
            id: self.id,
            ts: self.ts,
            level: Level::from_member(&self.level).map_err(invalid)?,
            visibility: Visibility::from_member(&self.visibility).map_err(invalid)?,
            action: Action::from_member(&self.action).map_err(invalid)?,
            entity,
            project_id: self.project_id,
            sync_run_id: self.sync_run_id,
            result: Outcome::from_member(&self.result).map_err(invalid)?,
            details,
        })
    }
}

/// Wraps an [`ActionLogError`] into its [`StorageError`] variant.
fn invalid(source: ActionLogError) -> StorageError {
    StorageError::ActionLog { source }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stored vocabulary literals, hardcoded from the spec — an
    /// oracle independent of the production enums (a mutated literal
    /// or a narrowed vocabulary fails these tests). `level`,
    /// `visibility` and `result` are §6.2 CHECK literals; `action`
    /// and `entity_type` live over free-TEXT columns the schema
    /// leaves open (§6.2), closed instead by the app-level D-051
    /// vocabulary (extended by D-119) — pinned here the same way so
    /// the two kinds of literal share one oracle.
    #[test]
    fn enums_round_trip_the_schema_check_literals() {
        let levels = ["info", "warn", "error"];
        assert_eq!(Level::ALL.len(), levels.len());
        for (member, literal) in Level::ALL.iter().zip(levels) {
            assert_eq!(member.as_str(), literal);
        }

        let visibilities = ["user", "diagnostic"];
        assert_eq!(Visibility::ALL.len(), visibilities.len());
        for (member, literal) in Visibility::ALL.iter().zip(visibilities) {
            assert_eq!(member.as_str(), literal);
        }

        let outcomes = ["ok", "error", "skipped"];
        assert_eq!(Outcome::ALL.len(), outcomes.len());
        for (member, literal) in Outcome::ALL.iter().zip(outcomes) {
            assert_eq!(member.as_str(), literal);
        }

        let entity_types = [
            "project",
            "exclusion",
            "instruction",
            "heading",
            "variant",
            "assignment",
            "sync_run",
            "backup",
            "setting",
        ];
        assert_eq!(EntityType::ALL.len(), entity_types.len());
        for (member, literal) in EntityType::ALL.iter().zip(entity_types) {
            assert_eq!(member.as_str(), literal);
        }
    }

    /// LOG-001…004's own list, member by member, hardcoded from the
    /// spec — every category of the requirement is covered by at
    /// least one `Action` member, and every member maps to exactly
    /// one category. This is the mapping table DECISIONS.md records,
    /// pinned as an oracle: dropping or renaming a member out of its
    /// category fails this test. Two categories deliberately map to
    /// no member of their own: a *failed* operation is journaled as
    /// its own action with `result = error` (the fail-then-log
    /// semantics), and a DB-017 invariant violation is an
    /// [`Action::ErrorReported`] — recorded here so the mapping stays
    /// total and conscious. The vocabulary also carries members
    /// **beyond** the requirement's list, each added by a recorded
    /// decision (the D-106 follow-up member, T-245); they are listed
    /// after the requirement's categories so the enum's documented
    /// order stays the flattened category order.
    #[test]
    fn the_action_vocabulary_covers_log_001_to_004_member_by_member() {
        let categories: [(&str, &[Action]); 13] = [
            ("adding projects", &[Action::ProjectAdded]),
            ("changing projects", &[Action::ProjectChanged]),
            ("archiving projects", &[Action::ProjectArchived]),
            ("restoring projects", &[Action::ProjectRestored]),
            ("changing settings", &[Action::SettingsChanged]),
            ("creating instructions", &[Action::InstructionCreated]),
            ("editing instructions", &[Action::InstructionChanged]),
            ("hiding instructions", &[Action::InstructionHidden]),
            ("restoring instructions", &[Action::InstructionRestored]),
            ("creating variants", &[Action::VariantCreated]),
            ("editing variants", &[Action::VariantChanged]),
            (
                "hiding variants (their hide is the §6.4 soft delete)",
                &[Action::VariantDeleted],
            ),
            ("restoring variants", &[Action::VariantRestored]),
        ];
        let mut covered: Vec<Action> = Vec::new();
        for (_, members) in categories {
            covered.extend_from_slice(members);
        }
        let remaining_categories: [(&str, &[Action]); 6] = [
            (
                "changing assignments (bulk: one entry with the count)",
                &[Action::AssignmentChanged],
            ),
            (
                "heading operations",
                &[
                    Action::HeadingCreated,
                    Action::HeadingChanged,
                    Action::HeadingDeleted,
                ],
            ),
            (
                "sync runs (their lifecycle: started, finished, aborted)",
                &[
                    Action::SyncRunStarted,
                    Action::SyncRunFinished,
                    Action::SyncRunAborted,
                ],
            ),
            ("sync results per project", &[Action::SyncProjectResult]),
            ("backup creation", &[Action::BackupCreated]),
            (
                "errors (incl. DB-017 invariant violations)",
                &[Action::ErrorReported],
            ),
        ];
        for (_, members) in remaining_categories {
            covered.extend_from_slice(members);
        }
        // Beyond the requirement's list: members added by recorded
        // decisions, still exactly one category each.
        let beyond_requirement: [(&str, &[Action]); 1] = [(
            "removing path exclusions (§6.4; the D-106 follow-up, T-245)",
            &[Action::ExclusionRemoved],
        )];
        for (_, members) in beyond_requirement {
            covered.extend_from_slice(members);
        }
        assert_eq!(
            covered.len(),
            Action::ALL.len(),
            "every Action member must map to exactly one category"
        );
        for member in Action::ALL {
            let occurrences = covered.iter().filter(|&&a| a == member).count();
            assert_eq!(
                occurrences,
                1,
                "{} must appear in exactly one category",
                member.as_str()
            );
        }
        // Placement, not just coverage: the flattened category order
        // is the enum's own documented order, so a member moved
        // between categories fails here too (a swap of two
        // single-member categories passes the counts above and only
        // fails this equality).
        assert_eq!(
            covered,
            Action::ALL.to_vec(),
            "the category order must be the enum's order"
        );
    }

    /// The action strings are the `object.verb` shape the schema
    /// tests already use (`project.added`), all distinct.
    #[test]
    fn action_strings_are_dotted_and_unique() {
        let mut strings: Vec<&str> = Action::ALL.iter().map(|a| a.as_str()).collect();
        strings.sort_unstable();
        let len = strings.len();
        strings.dedup();
        assert_eq!(strings.len(), len, "action strings must be unique");
        for string in &strings {
            assert!(
                string.split('.').count() == 2,
                "{string} must be object.verb"
            );
            assert!(
                string.bytes().all(|b| b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || b == b'.'
                    || b == b'_'),
                "{string} must be lowercase"
            );
        }
    }

    /// The T-245 members round-trip their stored strings in both
    /// directions, and the vocabulary grew by exactly one member
    /// each: near-miss spellings — the other exclusion verb, a
    /// plural entity, a lone object — are still rejected, so the
    /// extension did not open the closed vocabularies.
    #[test]
    fn the_exclusion_members_round_trip_and_reject_near_misses() {
        // as_str → from_member → as_str is the identity for both.
        assert_eq!(
            Action::from_member(Action::ExclusionRemoved.as_str())
                .expect("the stored spelling must parse"),
            Action::ExclusionRemoved
        );
        assert_eq!(
            Action::ExclusionRemoved.as_str(),
            "exclusion.removed",
            "the D-106 follow-up's own spelling"
        );
        assert_eq!(
            EntityType::from_member(EntityType::Exclusion.as_str())
                .expect("the stored spelling must parse"),
            EntityType::Exclusion
        );
        assert_eq!(EntityType::Exclusion.as_str(), "exclusion");
        // Near misses stay outside the vocabulary.
        for rejected in ["exclusion.added", "exclusion.remove", "exclusions.removed"] {
            match Action::from_member(rejected) {
                Err(ActionLogError::Member { column, found }) => {
                    assert_eq!(column, "action");
                    assert_eq!(found, rejected);
                }
                other => panic!("{rejected:?} must be rejected, got {other:?}"),
            }
        }
        for rejected in ["exclusions", "exclusion_row", "project_exclusion"] {
            match EntityType::from_member(rejected) {
                Err(ActionLogError::Member { column, found }) => {
                    assert_eq!(column, "entity_type");
                    assert_eq!(found, rejected);
                }
                other => panic!("{rejected:?} must be rejected, got {other:?}"),
            }
        }
    }

    /// The sync-run lifecycle members (T-124/D-223): both new
    /// spellings round-trip, and near misses — the verb-only forms,
    /// the cancelled synonym, a merged word — stay outside the
    /// vocabulary, so the extension did not open it.
    #[test]
    fn the_sync_run_members_round_trip_and_reject_near_misses() {
        // as_str → from_member → as_str is the identity for both new
        // members; the spellings come from §9.4's own status words
        // ("the run gets the `aborted` status") and the card's own
        // "Запуск" over LOG-001…004's "sync runs".
        for (member, spelling) in [
            (Action::SyncRunStarted, "sync.run_started"),
            (Action::SyncRunAborted, "sync.run_aborted"),
        ] {
            assert_eq!(
                Action::from_member(member.as_str()).expect("the stored spelling must parse"),
                member
            );
            assert_eq!(member.as_str(), spelling);
        }
        // Near misses stay outside the vocabulary.
        for rejected in [
            "sync.run_start",
            "sync.run_cancelled",
            "sync.runaborted",
            "sync.run_started_by_user",
        ] {
            match Action::from_member(rejected) {
                Err(ActionLogError::Member { column, found }) => {
                    assert_eq!(column, "action");
                    assert_eq!(found, rejected);
                }
                other => panic!("{rejected:?} must be rejected, got {other:?}"),
            }
        }
    }

    /// The §10 registry shape: every code of the spec's §10 table
    /// passes; everything instruction-shaped fails (SEC-008's boundary
    /// for the code field).
    #[test]
    fn failure_codes_accept_registry_shape_and_reject_text() {
        // The full §10 error table (22 codes), hardcoded from the
        // spec — an oracle independent of the app layer's registry
        // (which carries only the MVP subset, D-034).
        let registry = [
            "E_SYNC_RUNNING",
            "E_NOT_A_REPO",
            "E_PROJECT_UNAVAILABLE",
            "E_NO_WRITE_ACCESS",
            "E_TARGET_IS_DIR",
            "E_TARGET_BROKEN_LINK",
            "E_TARGET_OUTSIDE_ROOT",
            "E_MARKERS_INVALID",
            "E_NOT_UTF8",
            "E_BACKUP_FAILED",
            "E_DISK_FULL",
            "E_FILE_LOCKED",
            "E_WRITE_FAILED",
            "E_VERIFY_FAILED",
            "E_CONTENT_RESERVED",
            "E_VALIDATION",
            "E_DB_LOCKED",
            "E_DB_CORRUPT",
            "E_DB_TOO_NEW",
            "E_RESTORE_SOURCE_MISSING",
            "E_IMPORT_INVALID",
            "E_UPDATE_FAILED",
        ];
        for code in registry {
            assert_eq!(
                FailureCode::new(code).expect(code).as_str(),
                code,
                "{code} has the registry shape"
            );
        }
        // Instruction text, file content and prose: none of these may
        // pass as a code. The markers are the same shape the SEC-008
        // facade tests use.
        let instruction_text = "\
            # Agent Guidelines\n\
            SECRET-INSTRUCTION-MARKER: always answer in the user's language.\n\
            Follow every step of SECRET-RECIPE-4f2a before touching the build.\n\
            The deployment token is sk-live-9d1e2f3a4b5c.\n";
        for rejected in [
            instruction_text,
            "SECRET-RECIPE-4f2a",
            "sk-live-9d1e2f3a4b5c",
            "always answer in the user's language",
            "",
            "E_",
            "E_UPPER lower",
            "E_lower",
            "E_DB__LOCKED",
            "E_DB_",
            "_DB_LOCKED",
            "E_DB_LOCKED!",
            "e_db_locked",
            "E DB LOCKED",
            "E-DB-LOCKED",
        ] {
            match FailureCode::new(rejected) {
                Err(ActionLogError::FailureCode { found }) => assert_eq!(found, rejected),
                other => panic!("{rejected:?} must be rejected, got {other:?}"),
            }
        }
    }

    /// The canonical JSON forms of the details variants — the shapes
    /// the column contract pins (and all SEC-008-safe: numbers,
    /// paths, codes).
    #[test]
    fn details_json_forms_are_the_pinned_shapes() {
        assert_eq!(
            Details::Bulk { changed: 3 }.to_json(),
            serde_json::json!({ "changed": 3 })
        );
        assert_eq!(
            Details::File {
                path: PathBuf::from("/repo/AGENTS.md")
            }
            .to_json(),
            serde_json::json!({ "path": "/repo/AGENTS.md" })
        );
        assert_eq!(
            Details::Failure {
                code: FailureCode::new("E_SYNC_RUNNING").unwrap(),
                path: None,
                os_code: None,
            }
            .to_json(),
            serde_json::json!({ "code": "E_SYNC_RUNNING" })
        );
        assert_eq!(
            Details::Failure {
                code: FailureCode::new("E_NO_WRITE_ACCESS").unwrap(),
                path: Some(PathBuf::from("/repo/AGENTS.md")),
                os_code: None,
            }
            .to_json(),
            serde_json::json!({ "code": "E_NO_WRITE_ACCESS", "path": "/repo/AGENTS.md" })
        );
        // §10's fourth field (T-124/D-223): the OS code rides when an
        // OS call produced the error, as a plain number (SEC-008's
        // numeric facts), and is omitted when it did not — the
        // legacy no-`os_code` rows parse unchanged on the read side.
        assert_eq!(
            Details::Failure {
                code: FailureCode::new("E_FILE_LOCKED").unwrap(),
                path: Some(PathBuf::from("/repo/AGENTS.md")),
                os_code: Some(32),
            }
            .to_json(),
            serde_json::json!({
                "code": "E_FILE_LOCKED",
                "path": "/repo/AGENTS.md",
                "os_code": 32
            })
        );
        assert_eq!(
            Details::Failure {
                code: FailureCode::new("E_BACKUP_FAILED").unwrap(),
                path: None,
                os_code: Some(13),
            }
            .to_json(),
            serde_json::json!({ "code": "E_BACKUP_FAILED", "os_code": 13 })
        );
    }

    /// SEC-008, structurally: an entry built next to a realistic
    /// instruction text carries no marker of that text in its details
    /// JSON — the text has no route into the typed payload. This is
    /// the unit half; the integration suite drives the same contract
    /// through the database round-trip, and the facade test drives it
    /// through the file log.
    #[test]
    fn a_details_payload_built_beside_instruction_text_carries_none_of_it() {
        let instruction_text = "\
            SECRET-INSTRUCTION-MARKER: always answer in the user's language.\n\
            Follow every step of SECRET-RECIPE-4f2a before touching the build.\n";
        let entry = LogEntry {
            level: Level::Error,
            visibility: Visibility::Diagnostic,
            action: Action::VariantChanged,
            entity: Some(Entity {
                kind: EntityType::Variant,
                id: "0193a2aa-6c9e-7c8e-9c6d-1a2b3c4d5e6f",
            }),
            project_id: None,
            sync_run_id: None,
            result: Outcome::Error,
            details: Some(Details::Failure {
                code: FailureCode::new("E_CONTENT_RESERVED").unwrap(),
                path: Some(PathBuf::from("/repo/AGENTS.md")),
                os_code: None,
            }),
        };
        let json = entry.details.expect("the entry has details").to_json();
        let text = json.to_string();
        for marker in [
            "SECRET-INSTRUCTION-MARKER",
            "SECRET-RECIPE-4f2a",
            "always answer",
        ] {
            assert!(
                !text.contains(marker),
                "SEC-008 violation: {marker} leaked into details {text}"
            );
        }
        assert!(text.contains("E_CONTENT_RESERVED"), "the code is carried");
        assert!(text.contains("/repo/AGENTS.md"), "the path is carried");
        let _ = instruction_text;
    }

    /// `LogEntry::new` fills exactly the four always-set fields.
    #[test]
    fn log_entry_new_sets_the_core_fields_only() {
        let entry = LogEntry::new(
            Action::SettingsChanged,
            Level::Info,
            Visibility::User,
            Outcome::Ok,
        );
        assert_eq!(entry.action, Action::SettingsChanged);
        assert_eq!(entry.level, Level::Info);
        assert_eq!(entry.visibility, Visibility::User);
        assert_eq!(entry.result, Outcome::Ok);
        assert!(entry.entity.is_none());
        assert!(entry.project_id.is_none());
        assert!(entry.sync_run_id.is_none());
        assert!(entry.details.is_none());
    }

    /// The age cutoff is the DB-002 rendering of
    /// `now − max_days × 24 h` — the exact string the SQL comparison
    /// `ts < ?` binds, pinned with hardcoded oracle literals (a wrong
    /// format directive or a wrong multiplier fails this test without
    /// any database involved). The month rollover case is deliberate:
    /// pure seconds arithmetic, no calendar component to get wrong.
    #[test]
    fn age_cutoff_renders_now_minus_days_in_db002_shape() {
        let now =
            Timestamp::from_millisecond(1_795_046_400_000).expect("a fixed instant must parse");
        // 2026-11-19T00:00:00.000Z minus 180 days = 2026-05-23T00:00:00.000Z
        // (May has 31, June 30, July 31, August 31, September 30,
        // October 31, November 19 — 180 days back crosses six month
        // boundaries, so the arithmetic and the calendar agree only if
        // the multiplication is exactly 86 400 s per day).
        assert_eq!(age_cutoff(now, 180), "2026-05-23T00:00:00.000Z");
        // The §6.5 bounds: the smallest and the largest legal day
        // counts, both rendered without panicking and in shape.
        assert!(crate::values::is_db002_shape(&age_cutoff(now, 7)));
        assert!(crate::values::is_db002_shape(&age_cutoff(now, 3650)));
        // One day keeps the millisecond precision of `now`.
        let with_millis =
            Timestamp::from_millisecond(1_795_046_400_123).expect("a fixed instant must parse");
        assert_eq!(age_cutoff(with_millis, 1), "2026-11-18T00:00:00.123Z");
    }
}
