//! Version recording for library instructions and variants (spec §6.2
//! `revisions`, §6.4 "Revisions: cleanup by limit", §7.3 LIB-026,
//! §9.8 ALG-070…072, owner decision D-16; task T-082).
//!
//! This module is the *bookkeeping* half of version history: it owns
//! the `revisions` table's write path — the ALG-070 comparison
//! against the last recorded version, the ALG-071 `rev_no` sequence
//! and `op` column, and the ALG-072 retention prune. It does **not**
//! build snapshots: `eunomia_core::version_snapshot` (T-055, D-082)
//! owns the canonical JSON and its SHA-256, and the caller hands the
//! pair in as TEXT. Two entry points mirror the §6.4 lifecycle split
//! at the type level — [`record_instruction`] (create, update, hide,
//! restore) and [`record_variant`] (create, update, delete,
//! restore): a `hide` version of a *variant* or a `delete` version of
//! an *instruction* is a compile error, not a stored oddity. Headings
//! have no versions at all (§6.2's closing note) and are rejected
//! with [`RevisionError::NotAnInstruction`].
//!
//! # What "a real change" means (ALG-070, D-16)
//!
//! A version is created only on a real change. The service decides
//! that by the snapshot's SHA-256, exactly as ALG-070 words it: if
//! the new hash equals the last version's hash, no version is
//! created — the acceptance criterion of LIB-026 ("saving without
//! changes adds none"). The comparison is against the entity's last
//! *recorded version*, never against the library row, and the
//! snapshot's fields are the only measure: §7.3 fixes them (an
//! instruction's name and state; a variant's name and text) and
//! D-016/D-097 exclude everything else, so a save that restamps
//! `updated_at` while touching nothing snapshot-visible (a no-op
//! field write, a pure position move, a main-flag promotion, a
//! CRLF-fold that was already folded on the previous save) builds
//! the same snapshot, hashes it the same, and records no version.
//!
//! One class of real change is structurally invisible to that
//! comparison: the **variant delete and restore**. §7.3 pins the
//! variant snapshot to name and text — there is no state field — so
//! a soft delete (§6.4) or its LIB-026/027 restore builds a snapshot
//! byte-identical to the last version's, yet the entity's lifecycle
//! really did change, LIB-026's acceptance criterion says "each real
//! change adds one version", and ALG-071's `op` vocabulary exists
//! precisely so the History panel can show these operations.
//! ALG-070's hash rule, read in its own scope — "when an instruction
//! name, or a variant name or text, changes" — governs content
//! saves, not lifecycle transitions. Those two variant operations
//! therefore pass the gate whenever they represent a transition: a
//! delete records unless the last version is *already* the same
//! delete (same `op`, same hash), and a restore likewise. That
//! op-plus-hash pairing is what keeps D-16 whole on the blind side —
//! the repository's `delete_variant`/`restore_variant` are
//! idempotent and return no "changed" signal (D-097), so a repeated
//! command (double click, retry) re-records a transition that did
//! not happen, and the pairing suppresses it: while a variant is
//! deleted its content cannot change (the repository refuses to edit
//! the dead), so an op-equal, hash-equal re-run is definitionally a
//! re-run, while a genuine transition always differs (a second real
//! delete follows a `restore`, a different `op`). That reasoning
//! carries one caller invariant, stated here for the command tasks
//! that compose this service: **every** delete/restore transition is
//! recorded through this seam — a state mutation that skips the
//! recording would leave a stale `(op, hash)` as the last version
//! and could wrongly absorb the next genuine transition. Everything else —
//! including instruction hide/restore, whose snapshot *does* carry
//! the state field, so a genuine transition always changes the
//! hash — is hash-gated. The useful corollary: re-recording any
//! transition that did not happen (a `hide` for an already-hidden
//! instruction, a repeated variant delete) converges to
//! [`Recorded::Unchanged`] — the no-version rule absorbs idempotent
//! re-runs on both sides.
//!
//! # The retention prune (ALG-072)
//!
//! After an insert — and only after one; an unchanged record is a
//! pure read-plus-nothing — the oldest extra versions of that one
//! entity are deleted until `max_revisions` remain, newest by
//! `rev_no`. The latest version is never deleted: the keep-set is
//! clamped to at least one row inside the prune itself, so the
//! invariant holds for any `max_revisions` a caller passes, not just
//! §6.5-legal ones. `rev_no` is never renumbered by the prune: the
//! next version is always `MAX(rev_no) + 1` (the [`crate::library`]
//! `create_item` scalar-subquery shape), so numbers keep growing
//! monotonically past deletions and stay unique per entity (the
//! §6.2 UNIQUE constraint). The prune is idempotent: running it
//! again over a pruned state removes nothing. Lowering the setting
//! takes effect per entity at that entity's next recorded change —
//! ALG-072 ties the prune to an insertion, and no other sweep exists
//! in v1.
//!
//! # The limit parameter
//!
//! `max_revisions` arrives as a plain `i64` parameter on purpose —
//! the `crate::action_log::cleanup` `LogLimits` precedent: the
//! §6.5 range (10–1000) has exactly one owner, the settings
//! repository ([`crate::settings::get`] with
//! [`crate::settings::SettingKey::HistoryMaxRevisions`]), and
//! re-validating it here would duplicate that canon. The caller —
//! the command layer — reads the key in the *same transaction* as
//! the save it is recording, then passes the value on. Out-of-range
//! values are the caller's bug and behave as the prune's own floor
//! dictates: zero and negatives clamp to "keep the latest only";
//! a value larger than the stored history keeps everything.
//!
//! # The snapshot boundary (D-017)
//!
//! This module stays TEXT-in/TEXT-out — the crate's one
//! `eunomia-core` edge lives in `crate::build_snapshot` (wired by its
//! first core consumer, D-105), and this module does not use it:
//! it compares and stores the pair, it never builds one. The contract
//! on the caller is that `snapshot_hash` is the SHA-256 of
//! `snapshot` in the T-055 canonical form (the core builders produce
//! the pair together; the service cannot verify it and does not try
//! — a wrong pair is the command layer's bug, surfaced by its own
//! tests). This seam is also what the P1 revert (LIB-028/029) needs:
//! its `op=revert` version carries a *historical* snapshot that no
//! current row can supply, so the pass-through shape is
//! forward-compatible; `revert` and `reverted_from` are deliberately
//! absent from this module's vocabulary until that task lands.
//!
//! # Transactions
//!
//! Every call joins the caller's transaction (D-037; D-051's
//! executor-agnostic rule). The operation is multi-statement — gate
//! read, insert, prune — so it is atomic **only** inside the
//! caller's transaction; the integration suite pins the rollback
//! (a rolled-back recording leaves neither the new version nor the
//! prune). The action-log row for the recorded change is the
//! producer's to write at its own `Db::run` boundary, the
//! [`crate::settings::set`] and [`crate::library`] precedent.
//!
//! # What this module does not do
//!
//! * It does not read versions back: the History panel's list
//!   (LIB-026) and the P1 diff/revert views (LIB-028/029) are later
//!   tasks with their own ordering and limit needs.
//! * It does not derive `op` from the row: which user action a save
//!   represents is the command layer's knowledge; the service trusts
//!   the typed `op` and the snapshot's hash.
//! * It does not journal (D-051) and does not touch the library
//!   tables beyond the existence/kind probes that type the caller's
//!   id (the [`crate::library::assign`] raw-EXISTS precedent: this
//!   module's error type is its own, and the probe needs only the
//!   existence fact).

use std::fmt;

use rusqlite::Connection;
use rusqlite::OptionalExtension;

use crate::values::{new_id, now_utc};

// ---- Vocabularies -----------------------------------------------------------------

/// Which library operation an instruction version records (§9.8
/// ALG-071's op vocabulary, instruction half; §6.4: an instruction's
/// lifecycle is create, edit, hide and restore — v1 has no physical
/// instruction delete).
///
/// `revert` is deliberately absent: it is the P1 "Restore this
/// version" flow (LIB-028/029), which also carries `reverted_from`,
/// and lands with its own task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstructionOp {
    /// The instruction was created (LIB-002); the first version,
    /// `rev_no = 1`.
    Create,
    /// The instruction's name was saved (a content change; ALG-070's
    /// own trigger).
    Update,
    /// The instruction was hidden (LIB-005; §6.4: data, variants and
    /// assignments stay).
    Hide,
    /// The hidden instruction was restored (LIB-005…007).
    Restore,
}

impl InstructionOp {
    /// The stored member string (§6.2 `revisions.op` CHECK
    /// vocabulary).
    pub const fn as_str(&self) -> &'static str {
        match self {
            InstructionOp::Create => "create",
            InstructionOp::Update => "update",
            InstructionOp::Hide => "hide",
            InstructionOp::Restore => "restore",
        }
    }
}

/// Which library operation a variant version records (§9.8 ALG-071's
/// op vocabulary, variant half; §6.4: a variant's lifecycle is
/// create, edit, soft delete and restore — see [`InstructionOp`]
/// for the absent `revert`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VariantOp {
    /// The variant was created; the first version, `rev_no = 1`.
    Create,
    /// The variant's name or text was saved (ALG-070's own trigger).
    Update,
    /// The variant was soft-deleted (LIB-023…025; §6.4). Records
    /// whenever it represents a transition — see the module docs for
    /// the snapshot-blind ruling.
    Delete,
    /// The soft-deleted variant was brought back (LIB-026/027).
    /// Records like [`VariantOp::Delete`].
    Restore,
}

impl VariantOp {
    /// The stored member string (§6.2 `revisions.op` CHECK
    /// vocabulary).
    pub const fn as_str(&self) -> &'static str {
        match self {
            VariantOp::Create => "create",
            VariantOp::Update => "update",
            VariantOp::Delete => "delete",
            VariantOp::Restore => "restore",
        }
    }
}

// ---- Result ------------------------------------------------------------------------

/// The outcome of one [`record_instruction`]/[`record_variant`]
/// call: whether a version row was inserted, and its `rev_no`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// A new version was inserted with this `rev_no` (ALG-071: the
    /// previous maximum plus one, or 1 for the entity's first
    /// version), and the ALG-072 prune ran after it.
    Inserted {
        /// The inserted row's `rev_no`.
        rev_no: i64,
    },
    /// The snapshot's hash equals the last version's: no row was
    /// written and no prune ran (ALG-070, D-16 — the LIB-026
    /// acceptance criterion "saving without changes adds none").
    Unchanged,
}

// ---- Errors ------------------------------------------------------------------------

/// Why a version recording failed.
///
/// Returned **directly** by every function of this module, the
/// D-093 boundary shape shared with [`crate::library::LibraryError`]
/// and [`crate::projects::ProjectsError`]: the app layer matches it
/// beside its other error enums until a task allowed to edit
/// `src-tauri` normalizes the shapes.
#[derive(Debug)]
pub enum RevisionError {
    /// No library row with this id: the instruction
    /// (`library_items`) or variant (`instruction_variants`) the
    /// caller wants to version does not exist. A variant row always
    /// exists in a legal recording flow — deletion is soft (§6.4) —
    /// so this is a caller bug or out-of-band data.
    NotFound {
        /// The id that matched nothing.
        id: String,
    },
    /// The `library_items` row exists but is a heading: headings have
    /// no versions (§6.2's closing note; LIB-030 renames are
    /// action-log events, not versions).
    NotAnInstruction {
        /// The heading's id.
        id: String,
    },
    /// The stored `library_items.kind` cell is outside its closed
    /// vocabulary — corruption or a row written by a newer app
    /// version (the D-039 policy, surfaced at this module's own
    /// probe rather than guessed at).
    Member {
        /// The column the value came from.
        column: &'static str,
        /// The offending value.
        found: String,
    },
    /// The statement or query itself failed — a closed connection, a
    /// tampered schema. Everything SQLite reports that is not one of
    /// the typed outcomes above. The §6.2 CHECK floor is unreachable
    /// here: the op and entity-type members this module writes come
    /// from enums that produce exactly the CHECK literals.
    Sql(rusqlite::Error),
}

impl fmt::Display for RevisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RevisionError::NotFound { id } => write!(f, "no row with id {id:?}"),
            RevisionError::NotAnInstruction { id } => {
                write!(f, "library item {id:?} is not an instruction")
            }
            RevisionError::Member { column, found } => {
                write!(f, "{found:?} is not a valid {column} value")
            }
            RevisionError::Sql(err) => write!(f, "database error: {err}"),
        }
    }
}

impl std::error::Error for RevisionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RevisionError::Sql(err) => Some(err),
            _ => None,
        }
    }
}

/// Lets `?` lift a rusqlite failure into the typed error.
impl From<rusqlite::Error> for RevisionError {
    fn from(err: rusqlite::Error) -> Self {
        RevisionError::Sql(err)
    }
}

// ---- Entity probes ------------------------------------------------------------------

/// Verifies that `id` names an existing instruction: the raw
/// existence/kind probe (the [`crate::library::assign`] precedent —
/// this module's error type is its own, and the Foreign-Key-free
/// `revisions` table does not enforce the pairing itself).
fn ensure_instruction(conn: &Connection, id: &str) -> Result<(), RevisionError> {
    let kind: Option<String> = conn
        .query_row(
            "SELECT kind FROM library_items WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    match kind.as_deref() {
        Some("instruction") => Ok(()),
        Some("heading") => Err(RevisionError::NotAnInstruction { id: id.to_owned() }),
        Some(other) => Err(RevisionError::Member {
            column: "kind",
            found: other.to_owned(),
        }),
        None => Err(RevisionError::NotFound { id: id.to_owned() }),
    }
}

/// Verifies that `id` names an existing variant row (active or
/// soft-deleted — both are versionable, §6.4).
fn ensure_variant(conn: &Connection, id: &str) -> Result<(), RevisionError> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM instruction_variants WHERE id = ?1)",
        [id],
        |row| row.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(RevisionError::NotFound { id: id.to_owned() })
    }
}

// ---- The recording core --------------------------------------------------------------

/// The per-call inputs of [`record`] beyond the connection: the
/// already-encoded row members plus the gate flag, grouped so the
/// write path reads by name (the crate's `NewVariant` shape at
/// private scale).
struct Recording<'a> {
    /// The §6.2 `entity_type` CHECK literal.
    entity_type: &'static str,
    /// The library row the version belongs to.
    entity_id: &'a str,
    /// The §6.2 `op` CHECK literal.
    op: &'static str,
    /// The T-055 snapshot JSON, stored verbatim.
    snapshot: &'a str,
    /// The T-055 snapshot hash, compared and stored verbatim.
    snapshot_hash: &'a str,
    /// Whether the op is one of the two variant lifecycle operations
    /// §7.3's snapshot cannot express (see the module docs) — gated
    /// against the last version's `op` *and* hash instead of the
    /// hash alone.
    snapshot_blind: bool,
}

/// The shared write path behind both public entry points: the
/// ALG-070 gate, the ALG-071 insert, the ALG-072 prune.
///
/// `entity_type` and `op` arrive as the already-encoded CHECK
/// literals (enum `as_str` output), so the §6.2 CHECK floor is
/// unreachable.
fn record(
    conn: &Connection,
    recording: &Recording<'_>,
    max_revisions: i64,
) -> Result<Recorded, RevisionError> {
    let Recording {
        entity_type,
        entity_id,
        op,
        snapshot,
        snapshot_hash,
        snapshot_blind,
    } = recording;
    // ALG-070: compare with the entity's last recorded version —
    // `ORDER BY rev_no DESC LIMIT 1` over the (entity_type,
    // entity_id, rev_no) UNIQUE index.
    let last: Option<(String, String)> = conn
        .query_row(
            "SELECT snapshot_hash, op FROM revisions
              WHERE entity_type = ?1 AND entity_id = ?2
              ORDER BY rev_no DESC LIMIT 1",
            rusqlite::params![entity_type, entity_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((last_hash, last_op)) = last {
        // A snapshot-visible op is suppressed by the hash alone; a
        // snapshot-blind one (variant delete/restore) also demands
        // the same op as the last version — the repository's
        // delete/restore are idempotent and hand the caller no
        // "changed" signal, so an op-equal, hash-equal re-run is the
        // re-record of a transition that did not happen, while a
        // genuine transition always differs (content can only change
        // through an update in between, and delete/restore alternate
        // ops).
        let unchanged = if *snapshot_blind {
            last_hash == *snapshot_hash && last_op == *op
        } else {
            last_hash == *snapshot_hash
        };
        if unchanged {
            // D-16: no version without a real change — and no prune:
            // ALG-072 ties the cleanup to an insertion.
            return Ok(Recorded::Unchanged);
        }
    }
    let id = new_id();
    // ALG-071: rev_no is the previous maximum plus one, computed by a
    // scalar subquery *inside* the INSERT (the `create_item` shape),
    // so the append is one statement. Numbers keep growing past the
    // prune; the §6.2 UNIQUE constraint keeps them distinct.
    conn.execute(
        "INSERT INTO revisions (
             id, entity_type, entity_id, rev_no, op,
             snapshot, snapshot_hash, reverted_from, created_at
         ) VALUES (
             ?1, ?2, ?3,
             (SELECT COALESCE(MAX(rev_no), 0) + 1 FROM revisions
               WHERE entity_type = ?2 AND entity_id = ?3),
             ?4, ?5, ?6, NULL, ?7
         )",
        rusqlite::params![
            id,
            entity_type,
            entity_id,
            op,
            snapshot,
            snapshot_hash,
            now_utc()
        ],
    )?;
    let rev_no: i64 = conn
        .query_row("SELECT rev_no FROM revisions WHERE id = ?1", [&id], |row| {
            row.get(0)
        })
        .optional()?
        // Unreachable while the INSERT above succeeded (one
        // connection, D-037); a reasoned panic instead of a
        // misleading rev_no.
        .expect("the row just inserted must be readable");
    prune(conn, entity_type, entity_id, max_revisions)?;
    Ok(Recorded::Inserted { rev_no })
}

/// Enforces the ALG-072 retention for one entity: after an insert,
/// delete the oldest versions beyond the newest `max_revisions`.
///
/// "The latest version is never deleted" is unconditional here, not
/// a precondition: the keep count is clamped to at least one, so a
/// zero or negative `max_revisions` (a caller bug — §6.5 validates
/// 10–1000) collapses the history to the latest version instead of
/// wiping it. The comparison is pure `rev_no` arithmetic against
/// `MAX(rev_no)`, so the prune is idempotent and touches no other
/// entity's rows.
fn prune(
    conn: &Connection,
    entity_type: &'static str,
    entity_id: &str,
    max_revisions: i64,
) -> Result<(), RevisionError> {
    let keep = max_revisions.max(1);
    conn.execute(
        "DELETE FROM revisions
          WHERE entity_type = ?1 AND entity_id = ?2
            AND rev_no <= (
                SELECT COALESCE(MAX(rev_no), 0) - ?3 FROM revisions
                 WHERE entity_type = ?1 AND entity_id = ?2
            )",
        rusqlite::params![entity_type, entity_id, keep],
    )?;
    Ok(())
}

// ---- Public entry points --------------------------------------------------------------

/// Records one version of the instruction with this id (spec §9.8
/// ALG-070/071/072, LIB-026; D-16) and prunes the entity's history
/// to `max_revisions`, all inside the caller's transaction.
///
/// `snapshot` and `snapshot_hash` are the T-055 pair for the
/// instruction's current name and state — the caller builds them
/// with `eunomia_core::version_snapshot` over the just-saved row
/// (see the module docs for the TEXT boundary). If the hash equals
/// the instruction's last recorded version's, nothing is written and
/// [`Recorded::Unchanged`] returns; otherwise a row is inserted with
/// the given `op`, `rev_no` = previous maximum + 1 (1 for the first
/// version), the snapshot pair stored verbatim, and `created_at` in
/// the DB-002 UTC form.
///
/// # Errors
///
/// Returns [`RevisionError::NotFound`] when no `library_items` row
/// has this id, [`RevisionError::NotAnInstruction`] for a heading
/// (headings have no versions), [`RevisionError::Member`] when the
/// stored `kind` cell is outside its closed vocabulary (D-039), and
/// [`RevisionError::Sql`] when a statement fails.
pub fn record_instruction(
    conn: &Connection,
    id: &str,
    op: InstructionOp,
    snapshot: &str,
    snapshot_hash: &str,
    max_revisions: i64,
) -> Result<Recorded, RevisionError> {
    ensure_instruction(conn, id)?;
    // Instruction hide/restore are snapshot-visible (§7.3's state
    // field), so every op here goes through the ALG-070 hash gate.
    record(
        conn,
        &Recording {
            entity_type: "instruction",
            entity_id: id,
            op: op.as_str(),
            snapshot,
            snapshot_hash,
            snapshot_blind: false,
        },
        max_revisions,
    )
}

/// Records one version of the variant with this id — the variant
/// twin of [`record_instruction`] (spec §9.8, LIB-026; D-16).
///
/// `snapshot` and `snapshot_hash` are the T-055 pair for the
/// variant's current name and text; the caller supplies them over
/// the just-saved (or just-deleted, or just-restored) row. The
/// ALG-070 hash gate applies to [`VariantOp::Create`] and
/// [`VariantOp::Update`]; [`VariantOp::Delete`] and
/// [`VariantOp::Restore`] pass it whenever they represent a real
/// transition — the last version carrying both the same `op` and the
/// same hash is the idempotent re-run of a transition that did not
/// happen — because §7.3's variant snapshot has no state field to
/// carry the transition; see the module docs for the ruling.
///
/// # Errors
///
/// Returns [`RevisionError::NotFound`] when no
/// `instruction_variants` row has this id (a soft-deleted variant
/// still has one — the "Restore variant" surface), and
/// [`RevisionError::Sql`] when a statement fails.
pub fn record_variant(
    conn: &Connection,
    id: &str,
    op: VariantOp,
    snapshot: &str,
    snapshot_hash: &str,
    max_revisions: i64,
) -> Result<Recorded, RevisionError> {
    ensure_variant(conn, id)?;
    let snapshot_blind = matches!(op, VariantOp::Delete | VariantOp::Restore);
    record(
        conn,
        &Recording {
            entity_type: "variant",
            entity_id: id,
            op: op.as_str(),
            snapshot,
            snapshot_hash,
            snapshot_blind,
        },
        max_revisions,
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// The prune over an already-pruned state removes nothing —
    /// pinned at the private seam with an in-memory migrated
    /// database, because the public API only prunes after an insert
    /// (the rolling-window integration test covers that path).
    #[test]
    fn prune_over_a_pruned_state_removes_nothing() {
        let mut conn =
            rusqlite::Connection::open_in_memory().expect("the in-memory database should open");
        crate::storage::run_migrations(&mut conn, Path::new("unused-backup-dir"))
            .expect("migrations should apply to a fresh database");
        for n in 1..=12i64 {
            conn.execute(
                "INSERT INTO revisions (
                     id, entity_type, entity_id, rev_no, op,
                     snapshot, snapshot_hash, reverted_from, created_at
                 ) VALUES (?1, 'variant', 'v', ?2, 'update', 's', 'h', NULL,
                           '2026-11-20T00:00:00.000Z')",
                rusqlite::params![format!("id-{n}"), n],
            )
            .expect("the seed row should insert");
        }

        prune(&conn, "variant", "v", 10).expect("the first prune should succeed");
        let after_first: Vec<i64> = rev_nos(&conn);
        assert_eq!(after_first, (3..=12).collect::<Vec<_>>());

        prune(&conn, "variant", "v", 10).expect("the second prune should succeed");
        assert_eq!(
            rev_nos(&conn),
            after_first,
            "the idempotent re-run removed nothing"
        );

        // A second entity's rows are not this prune's business:
        conn.execute(
            "INSERT INTO revisions (
                 id, entity_type, entity_id, rev_no, op,
                 snapshot, snapshot_hash, reverted_from, created_at
             ) VALUES ('other', 'instruction', 'v', 1, 'create', 's', 'h', NULL,
                       '2026-11-20T00:00:00.000Z')",
            [],
        )
        .expect("the other entity's row should insert");
        prune(&conn, "variant", "v", 10).expect("the third prune should succeed");
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM revisions WHERE entity_type = 'instruction'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("the count should run"),
            1,
            "the prune left the other entity's rows alone"
        );
    }

    /// The entity's stored `rev_no` values, ascending.
    fn rev_nos(conn: &rusqlite::Connection) -> Vec<i64> {
        let mut statement = conn
            .prepare(
                "SELECT rev_no FROM revisions
                  WHERE entity_type = 'variant' AND entity_id = 'v' ORDER BY rev_no",
            )
            .expect("the query should prepare");
        statement
            .query_map([], |row| row.get(0))
            .expect("the query should run")
            .collect::<Result<Vec<i64>, _>>()
            .expect("the rows should decode")
    }

    /// The op members this module stores must be exactly the §6.2
    /// CHECK literals — a drifted member would make the INSERT hit
    /// the CHECK floor, so the round trip is pinned here (the
    /// `library` module's precedent).
    #[test]
    fn op_members_are_the_schema_check_literals() {
        for (op, literal) in [
            (InstructionOp::Create, "create"),
            (InstructionOp::Update, "update"),
            (InstructionOp::Hide, "hide"),
            (InstructionOp::Restore, "restore"),
        ] {
            assert_eq!(op.as_str(), literal);
        }
        for (op, literal) in [
            (VariantOp::Create, "create"),
            (VariantOp::Update, "update"),
            (VariantOp::Delete, "delete"),
            (VariantOp::Restore, "restore"),
        ] {
            assert_eq!(op.as_str(), literal);
        }
        // The §6.2 CHECK vocabulary also allows 'revert'; it is the
        // P1 LIB-028/029 flow and deliberately not expressible here.
    }

    /// `Display` stays lowercase and sentence-shaped without trailing
    /// punctuation (the store's error-message convention).
    #[test]
    fn error_display_is_lowercase_and_specific() {
        assert_eq!(
            RevisionError::NotFound {
                id: String::from("x")
            }
            .to_string(),
            "no row with id \"x\""
        );
        assert_eq!(
            RevisionError::NotAnInstruction {
                id: String::from("h")
            }
            .to_string(),
            "library item \"h\" is not an instruction"
        );
        assert_eq!(
            RevisionError::Member {
                column: "kind",
                found: String::from("bogus"),
            }
            .to_string(),
            "\"bogus\" is not a valid kind value"
        );
    }
}
