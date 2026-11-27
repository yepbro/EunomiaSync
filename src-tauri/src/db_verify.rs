//! The DB invariant checks and the `db_verify` command (spec §6.3
//! DB-010…017, §7.10's System row, §10's non-corrupt half; T-131).
//!
//! §6.3's sentence splits the startup database work in two: the
//! `PRAGMA quick_check` corruption gate lives in [`crate::db`] (the
//! open-time `E_DB_CORRUPT` path, a §10 fatal), and "an invariant
//! check" — this module — runs beside it as a **consistency** sweep:
//! DB-017's own words ("written to the log at the error level and
//! shown in diagnostics … no automatic data repair") make a violation
//! a recorded fact, never an abort. The check therefore lives at the
//! session start ([`crate::start_database_session`], after the ERR-001
//! daily copy — the copy is the day's as-found snapshot, the journal
//! entries it writes come after it) and on demand through the command.
//!
//! # What is checked, and what cannot be
//!
//! Each check states its oracle — the §6.3 sentence — and the SQL
//! verifies exactly that sentence's data half. Two rows of the
//! requirement are deliberately NOT data checks:
//!
//! - **DB-013** ("a hidden instruction can have assignments, but is
//!   not included in the build") has no data invariant to verify: the
//!   first half is a permission (an assignment to a hidden
//!   instruction is legal rows, nothing to require), and the second
//!   half is build behavior, owned and pinned by `eunomia_core`'s
//!   body builder (the `is_active` selection and its
//!   "a hidden instruction neither builds a block nor keeps a
//!   heading" test). `db_verify` answers nothing for DB-013.
//! - **DB-014** ("an archived project keeps its assignments, does not
//!   take part in syncs, and is not counted as dirty") has one
//!   data-checkable half: the archived row's internal coherence —
//!   `state = 'archived'` ⟺ `archived_at IS NOT NULL`, the invariant
//!   the store's own archive/restore pair maintains. "Keeps its
//!   assignments" is the absence of a delete path (the FK is
//!   `ON DELETE RESTRICT`, archiving touches only the project row);
//!   "does not take part in syncs" and "not counted as dirty" are
//!   behavior (the §9.1 snapshot reads active projects only, and the
//!   dirty recalculation lists `ListScope::Active`), each pinned by
//!   its own suite.
//!
//! The honest boundaries of the six checks that do run (each pinned
//! by a plant-and-find test in `db_verify_tests.rs`, plus a
//! neighboring-legal-state test it must NOT report):
//!
//! | Invariant | The data check | Cannot catch |
//! | --------- | -------------- | ------------ |
//! | DB-010 | per active instruction: at least one active variant, exactly one active main (zero variant rows included — the LEFT JOIN keeps them) | a hidden instruction's variant health (out of the sentence's scope); variant *content* correctness |
//! | DB-011 | the positions of ALL library rows (one ordered list, headings included) form 1..=N: row k in position order sits at position k | nothing positional beyond that (level structure, section ranges — `gaps.rs`'s own domain) |
//! | DB-012 | the assignment's target verified as DATA against the sentence's every half: a `variant_id` no row has (missing), a variant of another instruction (foreign — the composite FK's half), or a soft-deleted variant (deleted) | the exactly-one-target half (the v3 CHECK's floor, D-192/D-203) and the extend shape (MAIN-034 is a contract until migration 0004 — the shipped v3 CHECK rejects the row) |
//! | DB-014 | state ⟺ archived_at coherence per project row — a RELATED store invariant (the archive/restore pair's own), not a clause of DB-014 itself: the row is the only data-checkable fact beside the three behavior clauses | the behavior halves below (a reader must not take this row as full DB-014 coverage) |
//! | DB-015 | no `path_key` is shared by two project rows, either state | `project_exclusions` (a different table's uniqueness), the restore-offer flow (UI behavior) |
//! | DB-016 | no stored variant text or per-project custom text contains a line equal to a D-19 marker — the SAME oracle the save path rejects with `E_CONTENT_RESERVED` (ALG-002 normalize + line classification), swept over what is already stored | a marker that is not a whole line (legal text), and NUL/length violations (different rules, DB-006's floor is a CHECK) |
//!
//! # DB-017: the journal entry, the command answer, no repair
//!
//! A found violation is written **once per violation** to the action
//! log: `error.reported` (the `Action` member whose documented
//! purpose is "faults, e.g. a DB-017 invariant violation") at
//! `Level::Error` / `Visibility::Diagnostic` (LOG-005: the entry
//! lives in the file log — where the error-level mirror line names
//! it — and in diagnostics) with `Outcome::Error`. The entity
//! columns carry the offending row: the kind from the entity
//! vocabulary, the id as **"DB-0NN/&lt;row ids&gt;"** — the invariant
//! code prefixed to the row's own identifier (the compound
//! "{project}/{instruction}" convention the assignment journal uses,
//! extended by one segment) so the durable record and its file-log
//! mirror name the RULE and the ROW; every segment is an identifier
//! (SEC-008: codes and ids, never stored text). The §7.10 answer
//! carries the same facts typed — `invariant`, the row's kind and id
//! (unprefixed: the field `invariant` already names the rule), the
//! project when the violation is project-scoped, and a machine
//! detail (counts, positions, the marker literal and its line —
//! numbers and fixed literals only).
//!
//! The entries ride the check's own one transaction (D-051): a check
//! that fails mid-way rolls its earlier entries back with everything
//! else — an incomplete check claims nothing — and a clean database
//! writes NOTHING (the `mark_interrupted_runs` zero-case pattern: a
//! healthy startup is silent). Each entry's file-log mirror line is
//! recorded into the transaction's mirror buffer and flushed only
//! after the commit (T-324/D-228: a rollback or a busy retry leaves
//! no line for a row that never existed). A repeat run re-finds the
//! same violations and re-journals them: the journal is the history
//! of checks, not a state store, and DB-017 demands no dedup
//! machinery. Nothing else writes: there is no repair, no flag, no
//! row edit — the remedy is the user's (restore a copy, fix the data
//! by hand, recreate).
//!
//! # The command
//!
//! `db_verify` is a §7.10 System-row read (the diagnostics section's
//! on-demand half of §6.3): one `Db::run_with_mirror` transaction
//! runs every check and journals what it finds, and the answer is the
//! typed violation list. No SYS-005 gate: the sync's only `projects`
//! write is `record_last_synced` (`last_synced_at`/`last_synced_hash`
//! — columns none of these checks read), the library and assignment
//! tables are written only by the library repository (the
//! `sync_status`/`sync_last_result` reading), a check during a run
//! reads one consistent snapshot, and the journal inserts serialize
//! on the one connection like every writer's. Without a database the
//! command refuses with the honest recovery-mode envelope
//! (`managed_db`'s `E_DB_LOCKED` + `NoDatabase`), never a fake
//! report.
//!
//! # Why the app layer
//!
//! The checks are read-only SQL over the live connection plus the
//! journal writes — the shape `recovery.rs` (raw statements over the
//! connection) and every command's closure (journal inserts inside
//! `Db::run`) already use; the store crate is the schema owner and
//! stays untouched (D-017/D-051's scope line, the brief's own
//! preference). Placement in a module of its own, not `commands.rs`,
//! is D-189's reading 1 — the minimal conflict surface while
//! parallel tasks edit `commands.rs` from their own cards.

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};

use crate::commands::managed_db;
use crate::db::sqlite_error_to_app_error;
use crate::errors::AppError;
use eunomia_store::action_log::{
    self, Action, Entity, EntityType, Level, LogEntry, Outcome, Visibility,
};

// ---------------------------------------------------------------------------
// The wire answer (§7.10)
// ---------------------------------------------------------------------------

/// One violated invariant instance — the diagnostics half of DB-017:
/// what the `db_verify` answer carries (the journal entry is the
/// durable half). Every field is an identifier, a count, a position
/// or a fixed D-19 literal (SEC-008: stored text never crosses).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct DbVerifyViolation {
    /// The invariant's spec id — `"DB-010"` … `"DB-016"` (DB-013 has
    /// no data check; see the module docs).
    pub(crate) invariant: String,
    /// The offending row's kind, in the action-log entity vocabulary
    /// (`"instruction"`, `"variant"`, `"assignment"`, `"project"`).
    pub(crate) entity_type: String,
    /// The offending row's identifier — the row's own id, or the
    /// compound `"{project_id}/{instruction_id}"` for assignment rows
    /// (the journal's convention for the id-less table).
    pub(crate) entity_id: String,
    /// The project the violation is scoped to, when it is (the
    /// assignment, archived-state and path-key rows).
    pub(crate) project_id: Option<String>,
    /// The machine fact pinning the violation beyond the row itself,
    /// when one exists.
    pub(crate) detail: Option<DbViolationDetail>,
}

/// The typed "what exactly is wrong" of a violation — the same facts
/// the check's SQL computed, narrowed to numbers, ids and the fixed
/// marker literals (specta forbids 64-bit integers on the TS wire —
/// counts and positions saturate at `u32::MAX`, the D-049 decision).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum DbViolationDetail {
    /// DB-010: the instruction's variant counts — the pair that broke
    /// "at least one active variant and exactly one active main"
    /// (zero actives, zero mains, or more than one main).
    VariantCounts {
        /// The instruction's active variants.
        active: u32,
        /// The active variants flagged main among them.
        active_main: u32,
    },
    /// DB-011: the row sits at `position` where the 1..=N row puts
    /// `expected` — a duplicate, a hole or an overshoot, one number
    /// each.
    Position {
        /// The position the row actually holds.
        position: u32,
        /// The position the 1..=N row assigns this row (its 1-based
        /// place in position order).
        expected: u32,
    },
    /// DB-012: the assignment still points at this soft-deleted
    /// variant (LIB-023's move never ran over it).
    DeletedVariant {
        /// The soft-deleted variant's row id.
        variant_id: String,
    },
    /// DB-012: the assignment's `variant_id` names no variant row at
    /// all — a target only foreign data can leave behind (the
    /// composite FK holds every writer this app has).
    MissingVariant {
        /// The variant id the assignment names (no row has it).
        variant_id: String,
    },
    /// DB-012: the variant the assignment points at belongs to
    /// ANOTHER instruction — the same-instruction half of the
    /// sentence, verified as data (the composite FK's half).
    ForeignVariant {
        /// The variant id the assignment points at.
        variant_id: String,
        /// The instruction the variant actually belongs to.
        instruction_id: String,
    },
    /// DB-014: which half of the state/`archived_at` pair contradicts
    /// the other — `true`: an active row carrying a stamped
    /// `archived_at`; `false`: an archived row with none.
    ArchivedAtMismatch {
        /// Whether the contradicting row is the active one.
        on_active: bool,
    },
    /// DB-015: the canonical path key more than one project row
    /// shares (a path form, SEC-008's allowed class).
    SharedPathKey {
        /// The duplicated `path_key`.
        path_key: String,
    },
    /// DB-016: the reserved marker line the stored text contains —
    /// which D-19 literal, and its 1-based line in the built form.
    MarkerLine {
        /// The 1-based line the marker sits on.
        line: u32,
        /// The fixed marker literal (`<!-- EUNOMIASYNC:BEGIN -->` or
        /// `<!-- EUNOMIASYNC:END -->` — a D-19 constant, not stored
        /// text).
        marker: String,
    },
}

// ---------------------------------------------------------------------------
// The typed core both outputs share
// ---------------------------------------------------------------------------

/// One violation as the checks produce it — the single source of the
/// §7.10 row and the journal entry (DB-017's two halves).
#[derive(Debug, Clone, PartialEq)]
struct Violation {
    /// The invariant's spec id (`"DB-010"` … `"DB-016"`).
    invariant: &'static str,
    /// The offending row's kind — the journal's entity vocabulary.
    kind: EntityType,
    /// The offending row's identifier: its own id, or the compound
    /// `"{project_id}/{instruction_id}"` for assignment rows.
    row_id: String,
    /// The project the violation is scoped to, when it is.
    project_id: Option<String>,
    /// The machine fact pinning the violation, when one exists.
    detail: Option<DbViolationDetail>,
}

impl Violation {
    /// The §7.10 row: the typed core with the kind's stored literal
    /// (the house wire convention for closed vocabularies — the
    /// `VariantRecord`/`BackupCopyRecord` shape) and the identifier
    /// fields under the DTO's names.
    fn into_wire(self) -> DbVerifyViolation {
        DbVerifyViolation {
            invariant: self.invariant.to_owned(),
            entity_type: self.kind.as_str().to_owned(),
            entity_id: self.row_id,
            project_id: self.project_id,
            detail: self.detail,
        }
    }

    /// The journal entity id (DB-017): the invariant code prefixed to
    /// the row identifier — `"DB-012/<project>/<instruction>"` — the
    /// compound-id convention the assignment journal established,
    /// extended by its first segment. The durable record and its
    /// error-level file-log mirror then name the RULE and the ROW;
    /// every segment is an identifier (SEC-008).
    fn journal_entity_id(&self) -> String {
        format!("{}/{}", self.invariant, self.row_id)
    }
}

/// Narrows an SQLite count/position to the wire's `u32` (D-049: the
/// TS side has no 64-bit integers; a count beyond `u32::MAX` is
/// already absurd data, and saturating keeps the row on the wire).
fn narrow_u32(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

// ---------------------------------------------------------------------------
// The checks (§6.3, one function per invariant, id order)
// ---------------------------------------------------------------------------

/// DB-010: "Each active instruction has at least one active variant
/// and exactly one active main variant." One aggregate per active
/// instruction — the LEFT JOIN keeps an instruction with no variants
/// at all (count zero), the two conditional sums carry the sentence's
/// own pair — and the verdict is the sentence: fewer than one active,
/// or a main count that is not exactly one.
fn db_010(transaction: &Transaction<'_>) -> Result<Vec<Violation>, rusqlite::Error> {
    let mut statement = transaction.prepare(
        "SELECT i.id, \
                COALESCE(SUM(CASE WHEN v.state = 'active' THEN 1 ELSE 0 END), 0), \
                COALESCE(SUM(CASE WHEN v.state = 'active' AND v.is_main = 1 THEN 1 ELSE 0 END), 0) \
           FROM library_items AS i \
           LEFT JOIN instruction_variants AS v ON v.instruction_id = i.id \
          WHERE i.kind = 'instruction' AND i.state = 'active' \
          GROUP BY i.id \
          ORDER BY i.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut violations = Vec::new();
    for row in rows {
        let (id, active, active_main) = row?;
        if active < 1 || active_main != 1 {
            violations.push(Violation {
                invariant: "DB-010",
                kind: EntityType::Instruction,
                row_id: id,
                project_id: None,
                detail: Some(DbViolationDetail::VariantCounts {
                    active: narrow_u32(active),
                    active_main: narrow_u32(active_main),
                }),
            });
        }
    }
    Ok(violations)
}

/// DB-011: "Library item positions go in a row from 1 to N" (DB-005's
/// ordered list — ALL library rows, headings included). The whole
/// table in position order, then the sentence's arithmetic in Rust:
/// the row at 1-based place k must sit at position k — any difference
/// is a duplicate, a hole or an overshoot, one row and one pair of
/// numbers each. Linear in the table (a bounded, user-sized list).
fn db_011(transaction: &Transaction<'_>) -> Result<Vec<Violation>, rusqlite::Error> {
    let mut statement = transaction
        .prepare("SELECT id, kind, position FROM library_items ORDER BY position, id")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut violations = Vec::new();
    for (index, row) in rows.enumerate() {
        let (id, kind, position) = row?;
        // The 1-based place in position order (the deterministic
        // position-then-id sort above), what "row 1..=N" assigns.
        let expected = narrow_u32(index as i64 + 1);
        let position_u32 = narrow_u32(position);
        if position_u32 != expected {
            violations.push(Violation {
                invariant: "DB-011",
                kind: if kind == "heading" {
                    EntityType::Heading
                } else {
                    EntityType::Instruction
                },
                row_id: id,
                project_id: None,
                detail: Some(DbViolationDetail::Position {
                    position: position_u32,
                    expected,
                }),
            });
        }
    }
    Ok(violations)
}

/// DB-012: "An assignment points to an active variant of the same
/// instruction" — verified as DATA, not trusted to the schema. The
/// sweep's own threat model (module docs) is rows no writer of this
/// app could produce, and a session that turns the `foreign_keys`
/// pragma OFF (an external sqlite3 or an import tool — the app's own
/// connections keep it on, and the bundled library compiles with
/// `SQLITE_DEFAULT_FOREIGN_KEYS=1`) can break all three halves of the
/// sentence: a `variant_id` naming no row at all, a variant of
/// ANOTHER instruction (the composite FK's half), or a soft-deleted
/// variant. The LEFT JOIN keeps every assignment; the verdict is the
/// sentence's own conjunction, in a fixed priority (missing, then
/// foreign, then deleted — the more fundamental break first).
fn db_012(transaction: &Transaction<'_>) -> Result<Vec<Violation>, rusqlite::Error> {
    let mut statement = transaction.prepare(
        "SELECT pi.project_id, pi.instruction_id, pi.variant_id, \
                v.instruction_id, v.state \
           FROM project_instructions AS pi \
           LEFT JOIN instruction_variants AS v ON v.id = pi.variant_id \
          WHERE pi.variant_id IS NOT NULL \
          ORDER BY pi.project_id, pi.instruction_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut violations = Vec::new();
    for row in rows {
        let (project_id, instruction_id, variant_id, variant_owner, variant_state) = row?;
        // The missing arm: the join found no variant row at all.
        let detail = match (variant_owner.as_deref(), variant_state.as_deref()) {
            (None, _) => DbViolationDetail::MissingVariant {
                variant_id: variant_id.clone(),
            },
            // The foreign arm: the variant exists but lives under
            // another instruction — the same-instruction half.
            (Some(owner), _) if owner != instruction_id.as_str() => {
                DbViolationDetail::ForeignVariant {
                    variant_id: variant_id.clone(),
                    instruction_id: owner.to_owned(),
                }
            }
            // The soft-delete arm: LIB-023's move never ran over it.
            (Some(_), Some("active")) => continue,
            (Some(_), _) => DbViolationDetail::DeletedVariant {
                variant_id: variant_id.clone(),
            },
        };
        violations.push(Violation {
            invariant: "DB-012",
            kind: EntityType::Assignment,
            row_id: format!("{project_id}/{instruction_id}"),
            project_id: Some(project_id),
            detail: Some(detail),
        });
    }
    Ok(violations)
}

/// DB-014's data half: the state/`archived_at` coherence the store's
/// archive/restore pair maintains (`archived_at IS NOT NULL ⟺ state
/// = 'archived'` — projects.rs's own invariant). "Keeps its
/// assignments" is the absence of a delete path, "does not take part
/// in syncs" and "not counted as dirty" are behavior (module docs) —
/// none of those is a row this SQL could read.
fn db_014(transaction: &Transaction<'_>) -> Result<Vec<Violation>, rusqlite::Error> {
    let mut statement = transaction.prepare(
        "SELECT id, state FROM projects \
          WHERE (state = 'archived' AND archived_at IS NULL) \
             OR (state = 'active' AND archived_at IS NOT NULL) \
          ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut violations = Vec::new();
    for row in rows {
        let (id, state) = row?;
        violations.push(Violation {
            invariant: "DB-014",
            kind: EntityType::Project,
            project_id: Some(id.clone()),
            row_id: id,
            detail: Some(DbViolationDetail::ArchivedAtMismatch {
                on_active: state == "active",
            }),
        });
    }
    Ok(violations)
}

/// DB-015: "`path_key` is unique among active and archived projects."
/// The UNIQUE constraint already holds every writer to it; the sweep
/// is the data-level half — no two stored rows share a key whatever
/// wrote them. Every row of a duplicated group beyond the FIRST
/// (ordered by `created_at`, then id — deterministic) is a violation:
/// the first row is the legitimate holder, the later ones the
/// duplicates.
fn db_015(transaction: &Transaction<'_>) -> Result<Vec<Violation>, rusqlite::Error> {
    let mut statement = transaction.prepare(
        "SELECT p.id, p.path_key \
           FROM projects AS p \
           JOIN (SELECT path_key FROM projects \
                  GROUP BY path_key HAVING COUNT(*) > 1) AS d \
             ON d.path_key = p.path_key \
          ORDER BY p.path_key, p.created_at, p.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut violations = Vec::new();
    let mut current_key: Option<String> = None;
    for row in rows {
        let (id, path_key) = row?;
        let is_first_of_group = current_key.as_deref() != Some(path_key.as_str());
        if is_first_of_group {
            // The group's legitimate holder (the deterministic order's
            // first row): not itself a violation.
            current_key = Some(path_key);
            continue;
        }
        violations.push(Violation {
            invariant: "DB-015",
            kind: EntityType::Project,
            project_id: Some(id.clone()),
            row_id: id,
            detail: Some(DbViolationDetail::SharedPathKey { path_key }),
        });
    }
    Ok(violations)
}

/// DB-016's data half: "Variant text and per-project custom text
/// cannot contain a line equal to a marker" — the save-time rejection
/// (`E_CONTENT_RESERVED`) checks what ENTERS; this sweeps what IS
/// STORED. The oracle is the save path's own (the
/// `validate_variant_text` reserved-line arm): the text as the build
/// writes it — one `normalize::variant_text` pass — then
/// `classify_line` over its lines, so a marker behind a BOM or a lone
/// CR is caught exactly as saving would.
///
/// The SQL narrows the candidates first: a line classifying as a
/// marker means the literal's bytes sit contiguously in the text
/// (padding aside, `classify_line` demands byte equality), so the
/// `LIKE '%literal%'` prefilter is sound — no false negatives — and
/// the exact predicate in Rust then rejects the near-misses (a marker
/// inside a line is legal text). Linear in the stored bytes; only
/// candidate rows cross the SQL/Rust boundary.
fn db_016(transaction: &Transaction<'_>) -> Result<Vec<Violation>, rusqlite::Error> {
    use eunomia_core::markers::{BEGIN_MARKER, END_MARKER, MarkerLine};
    use eunomia_core::normalize;

    /// The exact reserved-line predicate, shared by both tables: the
    /// first (1-based line, literal) the built form holds, or none.
    fn reserved_line(text: &str) -> Option<(u32, &'static str)> {
        normalize::variant_text(text)
            .lines()
            .enumerate()
            .find_map(|(index, line)| {
                classify(line).map(|marker| (narrow_u32(index as i64 + 1), marker))
            })
    }

    // The `classify_line` half of the oracle, keeping the literal for
    // the report (a D-19 constant, never stored text).
    fn classify(line: &str) -> Option<&'static str> {
        match eunomia_core::markers::classify_line(line) {
            Some(MarkerLine::Begin) => Some(BEGIN_MARKER),
            Some(MarkerLine::End) => Some(END_MARKER),
            None => None,
        }
    }

    let mut violations = Vec::new();
    // The variant texts (any state — a soft-deleted variant's stored
    // text is stored data too; the sweep has no "live only" scope in
    // the sentence).
    let mut statement = transaction.prepare(
        "SELECT id, content FROM instruction_variants \
          WHERE content LIKE '%' || ?1 || '%' OR content LIKE '%' || ?2 || '%' \
          ORDER BY id",
    )?;
    let rows = statement.query_map(rusqlite::params![BEGIN_MARKER, END_MARKER], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, content) = row?;
        if let Some((line, marker)) = reserved_line(&content) {
            violations.push(Violation {
                invariant: "DB-016",
                kind: EntityType::Variant,
                row_id: id,
                project_id: None,
                detail: Some(DbViolationDetail::MarkerLine {
                    line,
                    marker: marker.to_owned(),
                }),
            });
        }
    }
    // The per-project custom texts (MAIN-007's replace shape — the
    // same rejection the save path applies to them).
    let mut statement = transaction.prepare(
        "SELECT project_id, instruction_id, custom_text FROM project_instructions \
          WHERE custom_text LIKE '%' || ?1 || '%' OR custom_text LIKE '%' || ?2 || '%' \
          ORDER BY project_id, instruction_id",
    )?;
    let rows = statement.query_map(rusqlite::params![BEGIN_MARKER, END_MARKER], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (project_id, instruction_id, custom_text) = row?;
        if let Some((line, marker)) = reserved_line(&custom_text) {
            violations.push(Violation {
                invariant: "DB-016",
                kind: EntityType::Assignment,
                row_id: format!("{project_id}/{instruction_id}"),
                project_id: Some(project_id),
                detail: Some(DbViolationDetail::MarkerLine {
                    line,
                    marker: marker.to_owned(),
                }),
            });
        }
    }
    Ok(violations)
}

// ---------------------------------------------------------------------------
// The one pass: checks + journal (DB-017)
// ---------------------------------------------------------------------------

/// Runs every invariant check in id order, then journals each found
/// violation inside this same transaction (D-051: the entry rides the
/// operation's own transaction — a check that fails mid-way rolls the
/// earlier entries back with everything else, an incomplete check
/// claims nothing) and records each entry into the transaction's
/// mirror buffer (T-324/D-228: the executor flushes the file-log
/// mirror only after the commit — a rollback or a busy retry leaves
/// no line for a row that never existed). A clean database writes
/// nothing (the `mark_interrupted_runs` zero-case pattern). This is
/// the one closure both callers run: the command through
/// `Db::run_with_mirror`, the startup through the same executor
/// behind `block_on`.
fn verify_and_journal(
    transaction: &Transaction<'_>,
    mirror: &mut crate::logging::MirrorBuffer,
) -> Result<Vec<Violation>, AppError> {
    let mut violations = Vec::new();
    for found in [
        db_010(transaction),
        db_011(transaction),
        db_012(transaction),
        db_014(transaction),
        db_015(transaction),
        db_016(transaction),
    ] {
        violations.extend(found.map_err(sqlite_error_to_app_error)?);
    }
    for violation in &violations {
        journal_violation(transaction, mirror, violation)?;
    }
    Ok(violations)
}

/// Writes one violation's journal entry (DB-017's durable half):
/// `error.reported` — the `Action` member documented for exactly
/// these faults — at error level, diagnostic visibility (LOG-005:
/// the file log's error-level mirror line and diagnostics), error
/// result; the entity columns name the offending row with the
/// invariant code prefixed to its id (see
/// [`Violation::journal_entity_id`]); the project column carries the
/// project-scoped correlation. No details: the closed `Details` enum
/// has no variant for a rule code (D-051), and no §10 code exists for
/// an invariant violation — the spec's own remedy vocabulary for
/// these rows is the entity columns plus the typed answer. The
/// mirror record follows the T-324/D-228 contract: recorded inside
/// the transaction, flushed only after the commit.
fn journal_violation(
    transaction: &Transaction<'_>,
    mirror: &mut crate::logging::MirrorBuffer,
    violation: &Violation,
) -> Result<(), AppError> {
    let entity_id = violation.journal_entity_id();
    let mut entry = LogEntry::new(
        Action::ErrorReported,
        Level::Error,
        Visibility::Diagnostic,
        Outcome::Error,
    );
    entry.entity = Some(Entity {
        kind: violation.kind,
        id: entity_id.as_str(),
    });
    entry.project_id = violation.project_id.as_deref();
    action_log::insert(transaction, &entry).map_err(crate::commands::activity_log_to_app_error)?;
    mirror.record(&entry);
    Ok(())
}

// ---------------------------------------------------------------------------
// The command (§7.10's System row)
// ---------------------------------------------------------------------------

/// The diagnostics section's on-demand invariant check (§6.3, §7.10
/// System row): one `Db::run_with_mirror` transaction runs every
/// check, journals each violation it finds (DB-017) with its mirror
/// line buffered for the post-commit flush (T-324/D-228), and answers
/// the typed list. A read by the §5.4 gate's own scope — no SYS-005,
/// the journal writes serialize on the one connection (module docs).
#[tauri::command]
#[specta::specta]
pub(crate) async fn db_verify<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<DbVerifyViolation>, AppError> {
    db_verify_impl(app)
        .await
        .inspect_err(crate::logging::db_verify_failed)
}

async fn db_verify_impl<R: Runtime>(app: AppHandle<R>) -> Result<Vec<DbVerifyViolation>, AppError> {
    let database = managed_db(&app)?;
    database
        .run_with_mirror(|transaction, mirror| {
            verify_and_journal(transaction, mirror)
                .map(|violations| violations.into_iter().map(Violation::into_wire).collect())
        })
        .await
}

// ---------------------------------------------------------------------------
// The startup half (§6.3: "checked at startup")
// ---------------------------------------------------------------------------

/// The session start's invariant check: the same one pass the command
/// runs — every check, the journal entries and mirror records inside
/// the one transaction — through the same `Db::run_with_mirror`
/// executor (SYS-001: blocking work off the launch thread's async
/// side; the session tail parks on `block_on` the way its other
/// synchronous housekeeping steps run their SQL —
/// `mark_interrupted_runs`'s posture, the checks being bounded
/// indexed reads after `quick_check` has already read every page at
/// open).
///
/// **Degrade, never abort** (DB-017: "no automatic data repair", and
/// the tail's own D-046/D-054 contract): a violation journals its
/// entry and the launch continues; a failure of the CHECK itself (a
/// SQL error) is returned for the caller to log through the facade —
/// the launch continues too. The violations found are journaled, not
/// returned: the command is the diagnostics surface, the journal is
/// the record.
pub(crate) fn startup_check(database: &crate::db::Db) -> Result<(), AppError> {
    tauri::async_runtime::block_on(database.run_with_mirror(verify_and_journal)).map(|_| ())
}
