//! Behavior tests for the version-recording service (spec §6.2
//! `revisions`, §6.4 "Revisions: cleanup by limit", §7.3 LIB-026,
//! §9.8 ALG-070…072, D-16; task T-082).
//!
//! The tests run against a real database opened through `open_db` and
//! migrated with `run_migrations` (the `tests/library.rs` pattern)
//! and drive the service through its public API, reading the
//! `revisions` table back directly — the table IS the specified
//! artifact (§6.2), so its rows are the observable behavior:
//!
//! * the ALG-070 gate: an identical snapshot hash records no version
//!   — a save that changed only non-snapshot facts (`updated_at`,
//!   the main flag, the position) adds none, and an unchanged record
//!   consumes no `rev_no`;
//! * every op writes its §6.2 CHECK literal with the next `rev_no`
//!   (previous plus one), including the snapshot-blind variant
//!   delete/restore (§7.3's variant snapshot has no state field —
//!   the module docs' ruling), while instruction hide/restore ride
//!   the hash gate; a repeated hide AND a repeated blind operation
//!   (the repository's idempotent no-ops re-recorded) converge to
//!   `Unchanged`, and a genuine delete→restore→delete still records;
//! * ALG-070 compares with the **last** version only: saving back to
//!   an older snapshot records a version even though its hash
//!   already exists in the history;
//! * the ALG-072 prune: limit 10 keeps exactly the 10 newest
//!   versions, smaller (and illegal zero/negative) limits never
//!   delete the latest, `rev_no` keeps growing past deletions, the
//!   limit is per entity, and an unchanged record prunes nothing;
//! * the transaction contract: a rolled-back recording leaves
//!   neither the inserted version nor the prune;
//! * the caller-id contract: unknown ids are `NotFound` — including
//!   a variant id at the instruction entry point and vice versa —
//!   headings are `NotAnInstruction` (headings have no versions),
//!   and a corrupt `kind` cell surfaces as `Member` (D-039);
//! * the snapshot pair is stored verbatim, byte for byte as the
//!   T-055 builders produce it (their documented example literals).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::library::{
    ItemKind, ItemUpdate, LibraryItem, NewLibraryItem, NewVariant, Variant, VariantUpdate,
    create_item, create_variant, delete_variant, get_variant, hide_item, move_item, restore_item,
    restore_variant, set_main_variant, update_item, update_variant,
};
use eunomia_store::library_versions::{
    InstructionOp, Recorded, RevisionError, VariantOp, record_instruction, record_variant,
};
use eunomia_store::storage::{open_db, run_migrations};
use rusqlite::Connection;

/// A temporary database directory removed when the guard is dropped
/// (the `tests/library.rs` pattern).
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates a unique temporary directory holding a migrated database
/// (opened through `open_db`, so the DB-001 pragma set applies).
fn versions_db(label: &str) -> (Connection, TempDir) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-library-versions-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    let mut conn =
        open_db(&dir.join("db").join("eunomiasync.sqlite3")).expect("open_db should succeed");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to a fresh database");
    (conn, TempDir(dir))
}

/// Asserts the canonical UUIDv7 TEXT shape of a row id (DB-002/D-031).
fn assert_uuidv7_text(id: &str) {
    let bytes = id.as_bytes();
    assert_eq!(bytes.len(), 36, "id must be 36 characters: {id}");
    for index in [8, 13, 18, 23] {
        assert_eq!(bytes[index], b'-', "hyphen at byte {index} of {id}");
    }
    assert_eq!(bytes[14], b'7', "version nibble of {id}");
}

/// Asserts the DB-002 shape of a timestamp cell.
fn assert_db002_shape(value: &str) {
    let bytes = value.as_bytes();
    assert_eq!(bytes.len(), 24, "timestamp must be 24 characters: {value}");
    assert_eq!(bytes[10], b'T', "date/time separator of {value}");
    assert_eq!(bytes[19], b'.', "fractional separator of {value}");
    assert_eq!(bytes[23], b'Z', "UTC marker of {value}");
}

/// Creates instruction `n` with its main variant (the LIB-002
/// create-dialog shape) and returns both rows.
fn seed_instruction(conn: &Connection, n: usize) -> (LibraryItem, Variant) {
    let item = create_item(
        conn,
        &NewLibraryItem::Instruction {
            title: &format!("Instruction {n}"),
        },
    )
    .unwrap_or_else(|err| panic!("seed instruction {n} should create, got {err}"));
    let main = create_variant(
        conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Main",
            content: &format!("content {n}"),
            is_main: true,
        },
    )
    .unwrap_or_else(|err| panic!("seed main variant {n} should create, got {err}"));
    (item, main)
}

/// Creates heading `n` with level 2.
fn seed_heading(conn: &Connection, n: usize) -> LibraryItem {
    create_item(
        conn,
        &NewLibraryItem::Heading {
            title: &format!("Section {n}"),
            level: 2,
        },
    )
    .unwrap_or_else(|err| panic!("seed heading {n} should create, got {err}"))
}

/// One stored `revisions` row, in the fixed order every assertion
/// helper reads it in.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RevisionRow {
    id: String,
    rev_no: i64,
    op: String,
    snapshot: String,
    snapshot_hash: String,
    reverted_from: Option<i64>,
    created_at: String,
}

/// Reads the entity's revisions ordered by `rev_no` (ascending) —
/// the History panel's own order.
fn revisions_of(conn: &Connection, entity_type: &str, entity_id: &str) -> Vec<RevisionRow> {
    let mut statement = conn
        .prepare(
            "SELECT id, rev_no, op, snapshot, snapshot_hash, reverted_from, created_at
               FROM revisions
              WHERE entity_type = ?1 AND entity_id = ?2
              ORDER BY rev_no",
        )
        .expect("the revisions query should prepare");
    statement
        .query_map(rusqlite::params![entity_type, entity_id], |row| {
            Ok(RevisionRow {
                id: row.get(0)?,
                rev_no: row.get(1)?,
                op: row.get(2)?,
                snapshot: row.get(3)?,
                snapshot_hash: row.get(4)?,
                reverted_from: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .expect("the revisions query should run")
        .collect::<Result<Vec<_>, _>>()
        .expect("the revisions rows should decode")
}

/// `(rev_no, op)` of the entity's revisions, in order — the shape
/// most assertions pin.
fn rev_ops(conn: &Connection, entity_type: &str, entity_id: &str) -> Vec<(i64, String)> {
    revisions_of(conn, entity_type, entity_id)
        .into_iter()
        .map(|row| (row.rev_no, row.op))
        .collect()
}

/// Asserts `result` is `Inserted` with this `rev_no`.
fn assert_inserted(result: Recorded, rev_no: i64) {
    assert_eq!(
        result,
        Recorded::Inserted { rev_no },
        "expected a version inserted at rev_no {rev_no}"
    );
}

// The T-055 documented example literals (D-082): a real snapshot
// pair exactly as `eunomia_core::version_snapshot` builds it —
// independently pinned there against Python's hashlib, so using
// them here proves the service stores the genuine article verbatim.
const INSTRUCTION_ACTIVE: (&str, &str) = (
    "{\"name\":\"Be concise\",\"state\":\"active\"}",
    "129ec91f397116358e6ed4b9c42844c3132d19f5aaefefa728f565244928b92a",
);
const INSTRUCTION_HIDDEN: (&str, &str) = (
    "{\"name\":\"Be concise\",\"state\":\"hidden\"}",
    "b244cb4be1c391e7db9290218536926c18f64850702b721d2180f583cdb4e3b8",
);
const VARIANT_BRIEF: (&str, &str) = (
    "{\"name\":\"Main\",\"text\":\"Be brief.\"}",
    "4f4c2ae7691504ab5e1d0876d104b7eb830f984f17b889acb688ce22bf7fa2df",
);

/// A synthetic snapshot pair for a variant text `v{n}` — the gate
/// compares hashes; the service never verifies one, so test pairs
/// only need to be distinct per content.
fn variant_pair(n: usize) -> (String, String) {
    (
        format!("{{\"name\":\"Main\",\"text\":\"v{n}\"}}"),
        format!("hash-v{n}"),
    )
}

// ---- The first version ------------------------------------------------------------

/// Creation records the entity's first version: `rev_no = 1`, the
/// op literal, the snapshot pair verbatim, a UUIDv7 id, a DB-002
/// timestamp, and no `reverted_from`.
#[test]
fn creation_records_the_first_version_with_every_column() {
    let (conn, _dir) = versions_db("create");
    let (item, _) = seed_instruction(&conn, 1);

    assert_inserted(
        record_instruction(
            &conn,
            &item.id,
            InstructionOp::Create,
            INSTRUCTION_ACTIVE.0,
            INSTRUCTION_ACTIVE.1,
            200,
        )
        .expect("the create record should succeed"),
        1,
    );

    let rows = revisions_of(&conn, "instruction", &item.id);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_uuidv7_text(&row.id);
    assert_db002_shape(&row.created_at);
    assert_eq!(row.rev_no, 1);
    assert_eq!(row.op, "create");
    assert_eq!(row.snapshot, INSTRUCTION_ACTIVE.0);
    assert_eq!(row.snapshot_hash, INSTRUCTION_ACTIVE.1);
    assert_eq!(
        row.reverted_from, None,
        "revert is the P1 flow (LIB-028/029)"
    );
}

/// The stored snapshot pair is byte-for-byte the T-055 builder
/// output (their documented example literals), on the variant side
/// too — the D-017 TEXT seam carries the genuine article.
#[test]
fn the_t055_snapshot_pair_is_stored_verbatim() {
    let (conn, _dir) = versions_db("verbatim");
    let (item, main) = seed_instruction(&conn, 1);

    record_instruction(
        &conn,
        &item.id,
        InstructionOp::Create,
        INSTRUCTION_ACTIVE.0,
        INSTRUCTION_ACTIVE.1,
        200,
    )
    .expect("instruction create should succeed");
    record_variant(
        &conn,
        &main.id,
        VariantOp::Create,
        VARIANT_BRIEF.0,
        VARIANT_BRIEF.1,
        200,
    )
    .expect("variant create should succeed");

    let instruction_rows = revisions_of(&conn, "instruction", &item.id);
    let variant_rows = revisions_of(&conn, "variant", &main.id);
    assert_eq!(instruction_rows[0].snapshot, INSTRUCTION_ACTIVE.0);
    assert_eq!(instruction_rows[0].snapshot_hash, INSTRUCTION_ACTIVE.1);
    assert_eq!(variant_rows[0].snapshot, VARIANT_BRIEF.0);
    assert_eq!(variant_rows[0].snapshot_hash, VARIANT_BRIEF.1);
    // The two histories are separate row sets keyed by their own
    // entity, each starting at its own rev_no 1:
    assert_ne!(instruction_rows[0].id, variant_rows[0].id);
    assert_eq!(instruction_rows[0].rev_no, 1);
    assert_eq!(variant_rows[0].rev_no, 1);
}

// ---- The ALG-070 gate ----------------------------------------------------------------

/// THE LIB-026 acceptance criterion: saving without changes adds no
/// version — the same snapshot pair records `Unchanged`, writes no
/// row, and consumes no `rev_no` (the next real change still gets
/// the next number).
#[test]
fn an_unchanged_save_records_no_version_and_consumes_no_rev_no() {
    let (conn, _dir) = versions_db("unchanged");
    let (item, main) = seed_instruction(&conn, 1);

    let (snap, hash) = variant_pair(1);
    assert_inserted(
        record_variant(&conn, &main.id, VariantOp::Create, &snap, &hash, 200)
            .expect("create should succeed"),
        1,
    );
    // The same save again — same name, same text, same pair:
    assert_eq!(
        record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 200)
            .expect("the unchanged record should succeed"),
        Recorded::Unchanged,
        "an identical hash must record no version"
    );
    assert_eq!(revisions_of(&conn, "variant", &main.id).len(), 1);

    // The unchanged record consumed no rev_no:
    let (snap2, hash2) = variant_pair(2);
    assert_inserted(
        record_variant(&conn, &main.id, VariantOp::Update, &snap2, &hash2, 200)
            .expect("the real change should succeed"),
        2,
    );
    assert_eq!(
        rev_ops(&conn, "variant", &main.id),
        vec![(1, String::from("create")), (2, String::from("update"))],
    );

    // The instruction side gates the same way:
    assert_inserted(
        record_instruction(
            &conn,
            &item.id,
            InstructionOp::Create,
            INSTRUCTION_ACTIVE.0,
            INSTRUCTION_ACTIVE.1,
            200,
        )
        .expect("instruction create should succeed"),
        1,
    );
    assert_eq!(
        record_instruction(
            &conn,
            &item.id,
            InstructionOp::Update,
            INSTRUCTION_ACTIVE.0,
            INSTRUCTION_ACTIVE.1,
            200
        )
        .expect("the unchanged record should succeed"),
        Recorded::Unchanged
    );
    assert_eq!(revisions_of(&conn, "instruction", &item.id).len(), 1);
}

/// A save that changed only non-snapshot facts records no version:
/// the repository restamps `updated_at` on a field-carrying no-op
/// update, a main-flag promotion and a position move write nothing
/// snapshot-visible — D-16's exclusions hold end to end.
#[test]
fn changes_outside_the_snapshot_record_no_version() {
    let (conn, _dir) = versions_db("non-snapshot");
    let (_, main) = seed_instruction(&conn, 1);
    // A second item, so the position move below is a real move:
    seed_instruction(&conn, 2);
    let second = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &main.instruction_id,
            name: "Strict",
            content: "Be very brief.",
            is_main: false,
        },
    )
    .expect("the second variant should create");

    // The pair of the variant exactly as seeded ("Main"/"content 1"):
    let (snap, hash) = ("{\"name\":\"Main\",\"text\":\"content 1\"}", "hash-seeded");
    assert_inserted(
        record_variant(&conn, &main.id, VariantOp::Create, snap, hash, 200)
            .expect("create should succeed"),
        1,
    );

    // A field-carrying update with identical values: both fields are
    // the row's own values, so nothing stored changes except the
    // repository's `updated_at` restamp — the snapshot (name + text
    // only, §7.3) does not.
    update_variant(
        &conn,
        &main.id,
        &VariantUpdate {
            name: Some("Main"),
            content: Some("content 1"),
        },
    )
    .expect("the no-op field update should succeed");
    let restamped = get_variant(&conn, &main.id).expect("the variant read should succeed");
    assert_eq!(
        restamped.expect("the variant exists").content,
        "content 1",
        "the update wrote no new content — only the restamp"
    );
    assert_eq!(
        record_variant(&conn, &main.id, VariantOp::Update, snap, hash, 200)
            .expect("the record after the restamp should succeed"),
        Recorded::Unchanged,
        "a timestamp-only change is not a real change"
    );

    // The main flag and the library order are not snapshot facts:
    set_main_variant(&conn, &second.id).expect("the promotion should succeed");
    move_item(&conn, &main.instruction_id, 2).expect("the move should succeed");
    assert_eq!(
        record_variant(&conn, &main.id, VariantOp::Update, snap, hash, 200)
            .expect("the record after the metadata writes should succeed"),
        Recorded::Unchanged
    );
    assert_eq!(
        revisions_of(&conn, "variant", &main.id).len(),
        1,
        "no version for main-flag or position changes"
    );
}

/// A repeated lifecycle record converges to `Unchanged`: hiding an
/// already-hidden instruction builds the snapshot the last hide
/// version already stored (the state field did not flip again).
#[test]
fn a_repeated_hide_records_no_version() {
    let (conn, _dir) = versions_db("repeated-hide");
    let (item, _) = seed_instruction(&conn, 1);

    record_instruction(
        &conn,
        &item.id,
        InstructionOp::Create,
        INSTRUCTION_ACTIVE.0,
        INSTRUCTION_ACTIVE.1,
        200,
    )
    .expect("create should succeed");
    hide_item(&conn, &item.id).expect("the hide should succeed");
    assert_inserted(
        record_instruction(
            &conn,
            &item.id,
            InstructionOp::Hide,
            INSTRUCTION_HIDDEN.0,
            INSTRUCTION_HIDDEN.1,
            200,
        )
        .expect("the hide record should succeed"),
        2,
    );
    // The hide ran again (the repository's idempotent no-op) and the
    // caller re-recorded it: the hash gate absorbs the re-run.
    hide_item(&conn, &item.id).expect("the repeated hide should succeed");
    assert_eq!(
        record_instruction(
            &conn,
            &item.id,
            InstructionOp::Hide,
            INSTRUCTION_HIDDEN.0,
            INSTRUCTION_HIDDEN.1,
            200
        )
        .expect("the repeated hide record should succeed"),
        Recorded::Unchanged,
        "a transition that did not happen is not a real change"
    );
    assert_eq!(
        rev_ops(&conn, "instruction", &item.id),
        vec![(1, String::from("create")), (2, String::from("hide")),]
    );
}

/// The snapshot-blind operations absorb their idempotent re-runs the
/// same way: the repository's delete/restore return no "changed"
/// signal (D-097), so a repeated command re-records a transition
/// that did not happen — and the gate suppresses it, because the
/// last version already carries that same op and hash. A genuine
/// second transition still records (its op differs from the last
/// version's).
#[test]
fn repeated_blind_operations_record_no_version() {
    let (conn, _dir) = versions_db("repeated-blind");
    let (_, main) = seed_instruction(&conn, 1);
    let second = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &main.instruction_id,
            name: "Strict",
            content: "Be very brief.",
            is_main: false,
        },
    )
    .expect("the sibling variant should create (LIB-025)");

    let (snap, hash) = VARIANT_BRIEF;
    assert_inserted(
        record_variant(&conn, &main.id, VariantOp::Create, snap, hash, 200)
            .expect("create should succeed"),
        1,
    );
    delete_variant(&conn, &main.id, Some(&second.id)).expect("the delete should succeed");
    assert_inserted(
        record_variant(&conn, &main.id, VariantOp::Delete, snap, hash, 200)
            .expect("the delete record should succeed"),
        2,
    );
    // The repository delete is idempotent; the re-recorded delete is
    // the re-run of a transition that did not happen:
    delete_variant(&conn, &main.id, Some(&second.id))
        .expect("the repeated delete is the documented no-op");
    assert_eq!(
        record_variant(&conn, &main.id, VariantOp::Delete, snap, hash, 200)
            .expect("the repeated delete record should succeed"),
        Recorded::Unchanged
    );

    restore_variant(&conn, &main.id).expect("the restore should succeed");
    assert_inserted(
        record_variant(&conn, &main.id, VariantOp::Restore, snap, hash, 200)
            .expect("the restore record should succeed"),
        3,
    );
    restore_variant(&conn, &main.id).expect("the repeated restore is the no-op");
    assert_eq!(
        record_variant(&conn, &main.id, VariantOp::Restore, snap, hash, 200)
            .expect("the repeated restore record should succeed"),
        Recorded::Unchanged
    );

    // A genuine second delete: the last version is the restore, a
    // different op — the real transition records.
    delete_variant(&conn, &main.id, Some(&second.id)).expect("the second delete should succeed");
    assert_inserted(
        record_variant(&conn, &main.id, VariantOp::Delete, snap, hash, 200)
            .expect("the second delete record should succeed"),
        4,
    );

    // The gate's hash half: an op-equal re-record with a DIFFERENT
    // pair is not the re-run the gate absorbs — the pair changed, so
    // the version records even though the op matches. Through the
    // repository this cannot arise (the content is frozen while the
    // variant is dead), so the pin is the service's own contract
    // against an op-only blind gate.
    let (other_snap, other_hash) = variant_pair(99);
    assert_inserted(
        record_variant(
            &conn,
            &main.id,
            VariantOp::Delete,
            &other_snap,
            &other_hash,
            200,
        )
        .expect("the op-equal, hash-different delete should succeed"),
        5,
    );

    assert_eq!(
        rev_ops(&conn, "variant", &main.id),
        vec![
            (1, String::from("create")),
            (2, String::from("delete")),
            (3, String::from("restore")),
            (4, String::from("delete")),
            (5, String::from("delete")),
        ]
    );
}

/// ALG-070 compares against the **last** version, not any stored
/// one: saving back to an older snapshot is a real change and must
/// record, even though its hash already exists further down the
/// history.
#[test]
fn saving_back_to_an_older_snapshot_still_records_a_version() {
    let (conn, _dir) = versions_db("older-hash");
    let (_, main) = seed_instruction(&conn, 1);

    let (first_snap, first_hash) = variant_pair(1);
    let (second_snap, second_hash) = variant_pair(2);
    assert_inserted(
        record_variant(
            &conn,
            &main.id,
            VariantOp::Create,
            &first_snap,
            &first_hash,
            200,
        )
        .expect("create should succeed"),
        1,
    );
    assert_inserted(
        record_variant(
            &conn,
            &main.id,
            VariantOp::Update,
            &second_snap,
            &second_hash,
            200,
        )
        .expect("the first update should succeed"),
        2,
    );
    // Back to the very first content: the v1 hash exists in the
    // history, but the comparison is with the last version only —
    // returning to an old value is a real change of the current
    // state.
    assert_inserted(
        record_variant(
            &conn,
            &main.id,
            VariantOp::Update,
            &first_snap,
            &first_hash,
            200,
        )
        .expect("the edit back should succeed"),
        3,
    );
    assert_eq!(
        rev_ops(&conn, "variant", &main.id),
        vec![
            (1, String::from("create")),
            (2, String::from("update")),
            (3, String::from("update")),
        ]
    );
}

// ---- The op vocabulary ------------------------------------------------------------

/// Every instruction operation writes its op literal with
/// `rev_no + 1`: create, update (a rename), hide, restore.
#[test]
fn every_instruction_operation_writes_its_op_and_rev_no_plus_one() {
    let (conn, _dir) = versions_db("instruction-ops");
    let (item, _) = seed_instruction(&conn, 1);

    let renamed = (
        "{\"name\":\"Be terse\",\"state\":\"active\"}",
        "hash-renamed",
    );
    assert_inserted(
        record_instruction(
            &conn,
            &item.id,
            InstructionOp::Create,
            INSTRUCTION_ACTIVE.0,
            INSTRUCTION_ACTIVE.1,
            200,
        )
        .expect("create should succeed"),
        1,
    );
    update_item(
        &conn,
        &item.id,
        &ItemUpdate {
            title: Some("Be terse"),
            heading_level: None,
        },
    )
    .expect("the rename should succeed");
    assert_inserted(
        record_instruction(
            &conn,
            &item.id,
            InstructionOp::Update,
            renamed.0,
            renamed.1,
            200,
        )
        .expect("the update record should succeed"),
        2,
    );
    hide_item(&conn, &item.id).expect("the hide should succeed");
    assert_inserted(
        record_instruction(
            &conn,
            &item.id,
            InstructionOp::Hide,
            "{\"name\":\"Be terse\",\"state\":\"hidden\"}",
            "hash-terse-hidden",
            200,
        )
        .expect("the hide record should succeed"),
        3,
    );
    restore_item(&conn, &item.id).expect("the restore should succeed");
    assert_inserted(
        record_instruction(
            &conn,
            &item.id,
            InstructionOp::Restore,
            renamed.0,
            renamed.1,
            200,
        )
        .expect("the restore record should succeed"),
        4,
    );

    assert_eq!(
        rev_ops(&conn, "instruction", &item.id),
        vec![
            (1, String::from("create")),
            (2, String::from("update")),
            (3, String::from("hide")),
            (4, String::from("restore")),
        ]
    );
}

/// Every variant operation writes its op literal with `rev_no + 1` —
/// including delete and restore, whose snapshots are byte-identical
/// to the last version's (§7.3's variant snapshot has no state
/// field): the two snapshot-blind operations record anyway.
#[test]
fn every_variant_operation_writes_its_op_including_the_blind_ones() {
    let (conn, _dir) = versions_db("variant-ops");
    let (_, main) = seed_instruction(&conn, 1);
    let second = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &main.instruction_id,
            name: "Strict",
            content: "Be very brief.",
            is_main: false,
        },
    )
    .expect("the sibling variant should create (LIB-025)");

    let (snap, hash) = VARIANT_BRIEF;
    assert_inserted(
        record_variant(&conn, &main.id, VariantOp::Create, snap, hash, 200)
            .expect("create should succeed"),
        1,
    );
    // Delete: the snapshot did not change, the lifecycle did. The
    // deleted variant is the main one, so the sibling is named the
    // successor (LIB-023; `None` would be `ReplacementRequired`).
    delete_variant(&conn, &main.id, Some(&second.id)).expect("the delete should succeed");
    assert_inserted(
        record_variant(&conn, &main.id, VariantOp::Delete, snap, hash, 200)
            .expect("the delete record should succeed"),
        2,
        // The blind operations carry the unchanged pair faithfully —
        // what the variant was at its deletion.
    );
    assert_eq!(
        revisions_of(&conn, "variant", &main.id)[1].snapshot_hash,
        hash
    );
    // Restore: still snapshot-blind, still a real change.
    restore_variant(&conn, &main.id).expect("the restore should succeed");
    assert_inserted(
        record_variant(&conn, &main.id, VariantOp::Restore, snap, hash, 200)
            .expect("the restore record should succeed"),
        3,
    );

    assert_eq!(
        rev_ops(&conn, "variant", &main.id),
        vec![
            (1, String::from("create")),
            (2, String::from("delete")),
            (3, String::from("restore")),
        ]
    );
}

// ---- The ALG-072 prune --------------------------------------------------------------

/// THE acceptance criterion: a limit of 10 keeps exactly the 10
/// newest versions — the oldest extras are deleted after every
/// insert, and the survivor set is the newest by `rev_no`.
#[test]
fn a_limit_of_ten_keeps_exactly_the_ten_newest_versions() {
    let (conn, _dir) = versions_db("limit-ten");
    let (_, main) = seed_instruction(&conn, 1);

    for n in 1..=15 {
        let (snap, hash) = variant_pair(n);
        let result = record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 10)
            .unwrap_or_else(|err| panic!("record {n} should succeed, got {err}"));
        assert_inserted(result, n as i64);
    }

    let rows = revisions_of(&conn, "variant", &main.id);
    assert_eq!(rows.len(), 10, "exactly the limit survives");
    assert_eq!(
        rows.iter().map(|row| row.rev_no).collect::<Vec<_>>(),
        (6..=15).collect::<Vec<_>>(),
        "the newest ten by rev_no, the oldest five deleted"
    );
    // The newest survivor is the just-inserted version:
    assert_eq!(rows.last().expect("ten rows").snapshot_hash, "hash-v15");
}

/// Under the limit nothing is deleted — the prune is a no-op while
/// the history is shorter than the keep count.
#[test]
fn under_the_limit_nothing_is_deleted() {
    let (conn, _dir) = versions_db("under-limit");
    let (_, main) = seed_instruction(&conn, 1);

    for n in 1..=3 {
        let (snap, hash) = variant_pair(n);
        record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 10)
            .unwrap_or_else(|err| panic!("record {n} should succeed, got {err}"));
    }
    assert_eq!(
        revisions_of(&conn, "variant", &main.id)
            .iter()
            .map(|row| row.rev_no)
            .collect::<Vec<_>>(),
        vec![1, 2, 3],
        "a history of three under a limit of ten keeps all three"
    );
}

/// The latest version survives every small limit — including the
/// zero and negative values §6.5 forbids: the clamp keeps the
/// newest row, never wiping the entity's whole history.
#[test]
fn the_latest_version_survives_every_small_limit() {
    let (conn, _dir) = versions_db("small-limits");
    let (_, main) = seed_instruction(&conn, 1);

    for n in 1..=4 {
        let (snap, hash) = variant_pair(n);
        record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 200)
            .unwrap_or_else(|err| panic!("seed record {n} should succeed, got {err}"));
    }
    assert_eq!(revisions_of(&conn, "variant", &main.id).len(), 4);

    let (snap, hash) = variant_pair(5);
    record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 1)
        .expect("a limit of one should succeed");
    assert_eq!(
        rev_ops(&conn, "variant", &main.id),
        vec![(5, String::from("update"))],
        "a limit of one keeps only the latest"
    );

    let (snap, hash) = variant_pair(6);
    record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 0)
        .expect("a zero limit should not wipe the entity");
    assert_eq!(
        rev_ops(&conn, "variant", &main.id),
        vec![(6, String::from("update"))],
        "the clamp turns zero into keep-the-latest"
    );

    let (snap, hash) = variant_pair(7);
    record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, -3)
        .expect("a negative limit should not wipe the entity");
    assert_eq!(
        rev_ops(&conn, "variant", &main.id),
        vec![(7, String::from("update"))],
        "the clamp turns negative into keep-the-latest"
    );
}

/// The prune is a rolling window and `rev_no` keeps growing past
/// deletions: after the window slides by one, the numbers stay
/// monotonic and unique — never renumbered.
#[test]
fn the_prune_is_a_rolling_window_and_rev_no_keeps_growing() {
    let (conn, _dir) = versions_db("rolling");
    let (_, main) = seed_instruction(&conn, 1);

    for n in 1..=12 {
        let (snap, hash) = variant_pair(n);
        record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 10)
            .unwrap_or_else(|err| panic!("record {n} should succeed, got {err}"));
    }
    assert_eq!(
        revisions_of(&conn, "variant", &main.id)
            .first()
            .expect("ten rows")
            .rev_no,
        3,
        "the window slid to 3..=12"
    );

    let (snap, hash) = variant_pair(13);
    // rev_no continues from MAX(rev_no), not from a re-count:
    assert_inserted(
        record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 10)
            .expect("the next record should succeed"),
        13,
    );
    assert_eq!(
        revisions_of(&conn, "variant", &main.id)
            .iter()
            .map(|row| row.rev_no)
            .collect::<Vec<_>>(),
        (4..=13).collect::<Vec<_>>()
    );
}

/// An unchanged record runs no cleanup: ALG-072 ties the prune to
/// an insertion, so a no-change record leaves an over-limit history
/// exactly as it is until the entity's next real change (which then
/// prunes). The repeated-prune behavior itself — a prune over an
/// already-pruned state removing nothing — is pinned at the private
/// seam by the module's unit test, and through the public API by the
/// rolling-window test.
#[test]
fn an_unchanged_record_runs_no_cleanup() {
    let (conn, _dir) = versions_db("no-cleanup-unchanged");
    let (_, main) = seed_instruction(&conn, 1);

    // Twelve versions under a wide limit — over the limit of ten
    // the entity is about to be recorded with.
    for n in 1..=12 {
        let (snap, hash) = variant_pair(n);
        record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 200)
            .unwrap_or_else(|err| panic!("record {n} should succeed, got {err}"));
    }
    assert_eq!(revisions_of(&conn, "variant", &main.id).len(), 12);

    // The unchanged record (the last pair, again) does not prune:
    let (snap, hash) = variant_pair(12);
    assert_eq!(
        record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 10)
            .expect("the unchanged record should succeed"),
        Recorded::Unchanged
    );
    assert_eq!(
        revisions_of(&conn, "variant", &main.id).len(),
        12,
        "no insertion, no prune (ALG-072)"
    );

    // The next real change prunes once — to exactly ten:
    let (snap, hash) = variant_pair(13);
    record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 10)
        .expect("the real change should succeed");
    assert_eq!(revisions_of(&conn, "variant", &main.id).len(), 10);
}

/// The limit is per entity: one entity churning under a tight limit
/// leaves its neighbors' histories — and its own instruction's
/// history — untouched.
#[test]
fn the_limit_is_per_entity() {
    let (conn, _dir) = versions_db("per-entity");
    let (item_a, main_a) = seed_instruction(&conn, 1);
    let (item_b, _) = seed_instruction(&conn, 2);

    // A's variant churns under a limit of ten…
    for n in 1..=15 {
        let (snap, hash) = variant_pair(n);
        record_variant(&conn, &main_a.id, VariantOp::Update, &snap, &hash, 10)
            .unwrap_or_else(|err| panic!("record {n} should succeed, got {err}"));
    }
    // …while A's own instruction and B's instruction stay short:
    record_instruction(
        &conn,
        &item_a.id,
        InstructionOp::Create,
        INSTRUCTION_ACTIVE.0,
        INSTRUCTION_ACTIVE.1,
        200,
    )
    .expect("A's instruction create should succeed");
    record_instruction(
        &conn,
        &item_b.id,
        InstructionOp::Create,
        INSTRUCTION_ACTIVE.0,
        INSTRUCTION_ACTIVE.1,
        200,
    )
    .expect("B's instruction create should succeed");
    record_instruction(
        &conn,
        &item_b.id,
        InstructionOp::Update,
        "{\"name\":\"Be terse\",\"state\":\"active\"}",
        "hash-b-renamed",
        200,
    )
    .expect("B's instruction update should succeed");

    assert_eq!(revisions_of(&conn, "variant", &main_a.id).len(), 10);
    assert_eq!(
        revisions_of(&conn, "instruction", &item_a.id).len(),
        1,
        "the variant's prune touches no other entity"
    );
    assert_eq!(
        revisions_of(&conn, "instruction", &item_b.id).len(),
        2,
        "an unchurned neighbor keeps its whole history"
    );
}

// ---- The transaction contract --------------------------------------------------------

/// A rolled-back recording leaves no partial history: neither the
/// inserted version nor the prune survives the rollback — the whole
/// operation is atomic inside the caller's transaction (D-037).
#[test]
fn a_rolled_back_recording_leaves_neither_version_nor_prune() {
    let (conn, _dir) = versions_db("rollback");
    let (_, main) = seed_instruction(&conn, 1);

    // Twelve committed versions under a wide limit.
    for n in 1..=12 {
        let (snap, hash) = variant_pair(n);
        record_variant(&conn, &main.id, VariantOp::Update, &snap, &hash, 200)
            .unwrap_or_else(|err| panic!("record {n} should succeed, got {err}"));
    }

    {
        let tx = conn.unchecked_transaction().expect("BEGIN should succeed");
        let (snap, hash) = variant_pair(13);
        assert_inserted(
            record_variant(&tx, &main.id, VariantOp::Update, &snap, &hash, 10)
                .expect("the recording inside the tx should succeed"),
            13,
        );
        // Inside the transaction the prune already ran…
        assert_eq!(
            revisions_of(&tx, "variant", &main.id).len(),
            10,
            "the pruned state is visible inside the tx"
        );
        // …and the drop of `tx` rolls it all back.
    }
    assert_eq!(
        revisions_of(&conn, "variant", &main.id)
            .iter()
            .map(|row| row.rev_no)
            .collect::<Vec<_>>(),
        (1..=12).collect::<Vec<_>>(),
        "the rollback erased both the insert and the prune"
    );
}

// ---- The caller-id contract ----------------------------------------------------------

/// Unknown ids are typed `NotFound` and write nothing — including
/// the cross-entity case: a variant id carries no `library_items`
/// row and an instruction id no `instruction_variants` row, so each
/// entry point rejects the other's ids through its own probe.
#[test]
fn unknown_entity_ids_are_not_found() {
    let (conn, _dir) = versions_db("unknown-ids");
    let (item, main) = seed_instruction(&conn, 1);

    match record_instruction(
        &conn,
        "no-such-instruction",
        InstructionOp::Create,
        INSTRUCTION_ACTIVE.0,
        INSTRUCTION_ACTIVE.1,
        200,
    ) {
        Err(RevisionError::NotFound { id }) => assert_eq!(id, "no-such-instruction"),
        other => panic!("expected NotFound, got {other:?}"),
    }
    match record_variant(
        &conn,
        "no-such-variant",
        VariantOp::Create,
        VARIANT_BRIEF.0,
        VARIANT_BRIEF.1,
        200,
    ) {
        Err(RevisionError::NotFound { id }) => assert_eq!(id, "no-such-variant"),
        other => panic!("expected NotFound, got {other:?}"),
    }
    // A variant id at the instruction entry point…
    match record_instruction(
        &conn,
        &main.id,
        InstructionOp::Create,
        INSTRUCTION_ACTIVE.0,
        INSTRUCTION_ACTIVE.1,
        200,
    ) {
        Err(RevisionError::NotFound { id }) => assert_eq!(id, main.id),
        other => panic!("expected NotFound for the variant id, got {other:?}"),
    }
    // …and an instruction id at the variant entry point:
    match record_variant(
        &conn,
        &item.id,
        VariantOp::Create,
        VARIANT_BRIEF.0,
        VARIANT_BRIEF.1,
        200,
    ) {
        Err(RevisionError::NotFound { id }) => assert_eq!(id, item.id),
        other => panic!("expected NotFound for the instruction id, got {other:?}"),
    }
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM revisions", [], |row| row.get(0))
        .expect("the count should run");
    assert_eq!(total, 0, "a rejected recording writes no row");
}

/// Headings have no versions (§6.2's closing note): recording one is
/// a typed `NotAnInstruction`, not a stored oddity.
#[test]
fn headings_have_no_versions() {
    let (conn, _dir) = versions_db("headings");
    let heading = seed_heading(&conn, 1);

    match record_instruction(
        &conn,
        &heading.id,
        InstructionOp::Create,
        INSTRUCTION_ACTIVE.0,
        INSTRUCTION_ACTIVE.1,
        200,
    ) {
        Err(RevisionError::NotAnInstruction { id }) => assert_eq!(id, heading.id),
        other => panic!("expected NotAnInstruction, got {other:?}"),
    }
    assert!(
        revisions_of(&conn, "instruction", &heading.id).is_empty(),
        "the heading gained no version"
    );
    assert_eq!(heading.kind, ItemKind::Heading);
}

/// A `kind` cell outside its closed vocabulary surfaces as `Member`
/// (D-039) — written through the `ignore_check_constraints` seam,
/// the only way such a cell can exist.
#[test]
fn a_corrupt_kind_cell_surfaces_as_member() {
    let (conn, _dir) = versions_db("corrupt-kind");
    let heading = seed_heading(&conn, 1);
    conn.execute_batch(&format!(
        "PRAGMA ignore_check_constraints = 1;\n\
         UPDATE library_items SET kind = 'chapter' WHERE id = '{}';\n\
         PRAGMA ignore_check_constraints = 0;",
        heading.id.replace('\'', "''"),
    ))
    .expect("the corruption should apply");

    match record_instruction(
        &conn,
        &heading.id,
        InstructionOp::Create,
        INSTRUCTION_ACTIVE.0,
        INSTRUCTION_ACTIVE.1,
        200,
    ) {
        Err(RevisionError::Member {
            column: "kind",
            found,
        }) => assert_eq!(found, "chapter"),
        other => panic!("expected Member, got {other:?}"),
    }
}
