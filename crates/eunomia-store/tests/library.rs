//! Behavior tests for the library repository (spec §6.2
//! `library_items`/`instruction_variants`/`project_instructions`,
//! §6.3 invariants DB-005/DB-010/DB-011/DB-012/DB-013, §6.4; task
//! T-081).
//!
//! The tests run against a real database opened through `open_db` and
//! migrated with `run_migrations` — the production configuration
//! (DB-001 pragmas, D-030) — in a temporary directory, and drive the
//! repository through its public API:
//!
//! * positions are the DB-005/DB-011 dense sequence: creation appends
//!   `1..=N` over the whole table (headings and instructions in one
//!   list), `move_item` renumbers the affected range, a heading's
//!   physical delete closes the gap, hiding keeps the position, and
//!   an out-of-range target is a typed `PositionOutOfRange` that
//!   touches nothing;
//! * variants keep DB-010 constructive: the first active variant must
//!   be main, a second active main is typed, the last active variant
//!   cannot be deleted (LIB-025), and deleting the main promotes the
//!   named replacement demote-first (never two mains mid-sequence);
//! * names are NOCASE-unique among active variants, freed by a soft
//!   delete and retaken by `restore_variant`;
//! * `delete_variant` moves assignments to the replacement before the
//!   soft delete (DB-012) — no assignment disappears silently
//!   (LIB-023) — defaulting to the main variant when none is named;
//! * assignments are one variant of one instruction per project
//!   (D-02, the composite PK), point only at an active variant of the
//!   same instruction (DB-012), survive a hidden instruction (DB-013)
//!   and change per project only (MAIN-005);
//! * `updated_at` follows the snapshot notion of a real change (D-16):
//!   renames, edits and state transitions restamp, position moves and
//!   main-flag promotions do not;
//! * by-id writers type a missing id as `NotFound`; corrupt vocabulary
//!   cells (written through the `ignore_check_constraints` seam — the
//!   only way such a cell can exist) fail reads as `Member` (D-039),
//!   and CHECK rejections (§6.9 values, DB-006, heading levels)
//!   surface as `Sql` — the store's floor;
//! * multi-statement writers roll back as sets inside an explicit
//!   transaction, and a failed statement inside a transaction keeps
//!   the transaction usable (SQLite's ABORT promise).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::library::{
    ItemKind, ItemScope, ItemState, ItemUpdate, LibraryError, LibraryItem, NewAssignment,
    NewLibraryItem, NewVariant, Variant, VariantState, VariantUpdate, assign, create_item,
    create_variant, delete_heading, delete_variant, get_assignment, get_item, get_variant,
    hide_item, list_assignments_for_project, list_assignments_for_variant, list_items,
    list_variants, move_item, reassign, restore_item, restore_variant, set_main_variant, unassign,
    update_item, update_variant,
};
use eunomia_store::projects as project_repo;
use eunomia_store::projects::{AddedVia, NewProject, Project};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use eunomia_store::storage::{open_db, run_migrations};
use rusqlite::Connection;

/// A temporary database directory removed when the guard is dropped —
/// the same teardown every sibling suite does explicitly, made
/// impossible to forget at a call site. Best-effort: a directory that
/// cannot be removed is left behind rather than failing a green test.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates a unique temporary directory holding a migrated database
/// (opened through `open_db`, so the DB-001 pragma set applies) and
/// returns the connection plus the directory guard (the
/// `tests/projects.rs` pattern).
fn library_db(label: &str) -> (Connection, TempDir) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-library-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    let mut conn =
        open_db(&dir.join("db").join("eunomiasync.sqlite3")).expect("open_db should succeed");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to a fresh database");
    (conn, TempDir(dir))
}

/// Asserts the canonical UUIDv7 TEXT shape of a row id (DB-002/D-031):
/// 36 characters, lowercase hyphenated, version nibble `7`.
fn assert_uuidv7_text(id: &str) {
    let bytes = id.as_bytes();
    assert_eq!(bytes.len(), 36, "id must be 36 characters: {id}");
    for index in [8, 13, 18, 23] {
        assert_eq!(bytes[index], b'-', "hyphen at byte {index} of {id}");
    }
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            continue;
        }
        assert!(
            byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase(),
            "lowercase hex digit at byte {index} of {id}"
        );
    }
    assert_eq!(bytes[14], b'7', "version nibble of {id}");
}

/// Asserts the DB-002 shape of a timestamp cell
/// (`YYYY-MM-DDTHH:MM:SS.sssZ`, 24 characters).
fn assert_db002_shape(value: &str) {
    let bytes = value.as_bytes();
    assert_eq!(bytes.len(), 24, "timestamp must be 24 characters: {value}");
    assert_eq!(bytes[10], b'T', "date/time separator of {value}");
    assert_eq!(bytes[19], b'.', "fractional separator of {value}");
    assert_eq!(bytes[23], b'Z', "UTC marker of {value}");
}

/// A full input for one project; every call gets a distinct
/// `path_key` (`/repos/repo-{n}`), so a set of these can be created
/// together (the `tests/projects.rs` `sample` shape).
fn project_sample(n: usize) -> NewProject<'static> {
    NewProject {
        path: Box::leak(format!("/repos/repo-{n}").into_boxed_str()),
        path_key: Box::leak(format!("/repos/repo-{n}").into_boxed_str()),
        name: Box::leak(format!("Project {n}").into_boxed_str()),
        label: Box::leak(format!("P{n}").into_boxed_str()),
        color: "blue",
        description: "",
        added_via: AddedVia::Manual,
        scheme: Scheme::A,
        base_name: BaseName::AgentsMd,
        extra_local: ExtraFile {
            enabled: false,
            position: Position::End,
        },
        extra_project: ExtraFile {
            enabled: false,
            position: Position::End,
        },
    }
}

/// Creates `project_sample(n)` and panics on failure (project rows are
/// the assignments' FK targets, not these tests' subject).
fn seed_project(conn: &Connection, n: usize) -> Project {
    project_repo::create(conn, &project_sample(n))
        .unwrap_or_else(|err| panic!("seed project {n} should create, got {err}"))
}

/// Creates instruction `n` with its main variant named `Main`
/// (the LIB-002 create-dialog shape) and returns both rows.
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

/// Creates heading `n` with the given level.
fn seed_heading(conn: &Connection, n: usize, level: u8) -> LibraryItem {
    create_item(
        conn,
        &NewLibraryItem::Heading {
            title: &format!("Section {n}"),
            level,
        },
    )
    .unwrap_or_else(|err| panic!("seed heading {n} should create, got {err}"))
}

/// The ids of `list_items(All)`, in position order.
fn all_item_ids(conn: &Connection) -> Vec<String> {
    list_items(conn, ItemScope::All)
        .expect("list_items should succeed")
        .into_iter()
        .map(|item| item.id)
        .collect()
}

/// The positions of `list_items(All)`, in position order.
fn all_positions(conn: &Connection) -> Vec<i64> {
    list_items(conn, ItemScope::All)
        .expect("list_items should succeed")
        .into_iter()
        .map(|item| item.position)
        .collect()
}

/// Writes one vocabulary-violating cell through the SQLite test seam
/// (`ignore_check_constraints`) — the simulation of the D-039
/// corruption/newer-version scenario the CHECKs otherwise make
/// unreachable.
fn corrupt_cell(conn: &Connection, update_sql: &str) {
    conn.execute_batch(&format!(
        "PRAGMA ignore_check_constraints = 1;\n{update_sql};\nPRAGMA ignore_check_constraints = 0;"
    ))
    .expect("the corruption should apply");
}

/// The escaped SQL literal of an id (UUIDv7 TEXT: nothing to escape
/// today, but the helper keeps the seam honest if that ever changes).
fn sql_literal(id: &str) -> String {
    id.replace('\'', "''")
}

// ---- Items: creation pins every column and the sequence --------------------------

#[test]
fn creating_an_instruction_pins_every_column_and_appends_the_first_position() {
    let (conn, _dir) = library_db("create-instruction");
    let created = create_item(
        &conn,
        &NewLibraryItem::Instruction {
            title: "Use concise answers",
        },
    )
    .expect("create should succeed");

    assert_uuidv7_text(&created.id);
    assert_db002_shape(&created.created_at);
    assert_eq!(created.created_at, created.updated_at);
    assert_eq!(created.kind, ItemKind::Instruction);
    assert_eq!(created.position, 1, "the first item starts the sequence");
    assert_eq!(created.heading_level, None, "an instruction has no level");
    assert_eq!(created.title, "Use concise answers");
    assert_eq!(created.state, ItemState::Active);
    assert_eq!(created.hidden_at, None, "a new row is not hidden");

    // The returned row is exactly what a fresh read returns.
    let reread = get_item(&conn, &created.id)
        .expect("get_item should succeed")
        .expect("the created row should exist");
    assert_eq!(reread, created);
}

#[test]
fn creating_a_heading_pins_the_level_and_items_share_one_dense_sequence() {
    // §7.3: headings and instructions are one shared ordered list, so
    // DB-011's 1..=N runs over both kinds together.
    let (conn, _dir) = library_db("create-heading");
    let h2 = seed_heading(&conn, 1, 2);
    let (i1, _) = seed_instruction(&conn, 1);
    let h3 = seed_heading(&conn, 2, 3);
    let (i2, _) = seed_instruction(&conn, 2);

    assert_eq!(h2.heading_level, Some(2));
    assert_eq!(h3.heading_level, Some(3));
    assert_eq!(i1.heading_level, None);
    assert_eq!(
        all_item_ids(&conn),
        vec![h2.id.clone(), i1.id.clone(), h3.id.clone(), i2.id.clone()],
        "creation order is position order across both kinds"
    );
    assert_eq!(all_positions(&conn), vec![1, 2, 3, 4], "gapless 1..=N");
}

#[test]
fn get_item_returns_none_for_an_unknown_id() {
    let (conn, _dir) = library_db("get-unknown");
    assert_eq!(
        get_item(&conn, "no-such-id").expect("get_item should succeed"),
        None
    );
}

#[test]
fn instruction_titles_are_unique_among_visible_instructions_nocase() {
    // §6.9's main-instruction-name clause: "unique among visible
    // instructions, case-insensitive" — a cross-row query, so it is
    // this layer's (D-076), not the field validators'.
    let (conn, _dir) = library_db("title-nocase");
    let (item, _) = seed_instruction(&conn, 1); // titled "Instruction 1"

    for twin in ["instruction 1", "INSTRUCTION 1"] {
        let err = create_item(&conn, &NewLibraryItem::Instruction { title: twin })
            .map(|_| ())
            .expect_err("a visible NOCASE twin must be rejected");
        assert!(
            matches!(
                err,
                LibraryError::DuplicateInstructionTitle { ref title } if title == twin
            ),
            "expected DuplicateInstructionTitle for {twin:?}, got {err:?}"
        );
    }

    // Heading titles are exempt: the §6.9 row is the instruction
    // name's, and headings coexist with same-titled instructions.
    create_item(
        &conn,
        &NewLibraryItem::Heading {
            title: "Instruction 1",
            level: 2,
        },
    )
    .expect("a heading may share an instruction's title");

    // A hidden twin does not block (it is not visible); the conflict
    // surfaces when the twin is restored.
    hide_item(&conn, &item.id).expect("hide should succeed");
    let twin_item = create_item(
        &conn,
        &NewLibraryItem::Instruction {
            title: "Instruction 1",
        },
    )
    .expect("a hidden twin does not block the title");
    let err = restore_item(&conn, &item.id)
        .map(|_| ())
        .expect_err("restoring into the taken title must conflict");
    assert!(
        matches!(err, LibraryError::DuplicateInstructionTitle { .. }),
        "expected DuplicateInstructionTitle, got {err:?}"
    );

    // The rename path is covered too, and keeping the own title is
    // exempt.
    let (third, _) = seed_instruction(&conn, 2); // "Instruction 2"
    let err = update_item(
        &conn,
        &twin_item.id,
        &ItemUpdate {
            title: Some("instruction 2"),
            ..ItemUpdate::default()
        },
    )
    .map(|_| ())
    .expect_err("renaming onto a visible twin must be rejected");
    assert!(matches!(
        err,
        LibraryError::DuplicateInstructionTitle { .. }
    ));
    update_item(
        &conn,
        &twin_item.id,
        &ItemUpdate {
            title: Some("INSTRUCTION 1"),
            ..ItemUpdate::default()
        },
    )
    .expect("keeping the own title NOCASE-equal is legal");
    let _ = third;
}

#[test]
fn a_heading_level_outside_two_to_six_hits_the_check_floor() {
    // §6.9/LIB-030 validation belongs to eunomia-core; the §6.2 CHECK
    // is the store's floor and must not let a bypassed level through.
    let (conn, _dir) = library_db("level-floor");
    for level in [0u8, 1, 7] {
        let err = create_item(
            &conn,
            &NewLibraryItem::Heading {
                title: "Bad section",
                level,
            },
        )
        .map(|_| ())
        .expect_err("a level outside 2..=6 must be rejected");
        assert!(
            matches!(err, LibraryError::Sql(_)),
            "expected the raw SQL (CHECK) error for level {level}, got {err:?}"
        );
    }
    assert_eq!(
        list_items(&conn, ItemScope::All)
            .expect("list_items should succeed")
            .len(),
        0,
        "the rejected rows left nothing behind"
    );
}

// ---- Variants: creation and the DB-010 constructive rules -------------------------

#[test]
fn creating_a_variant_pins_every_column_and_round_trips() {
    let (conn, _dir) = library_db("create-variant");
    let (item, _) = seed_instruction(&conn, 1);
    let created = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Strict",
            content: "Answer in one sentence.",
            is_main: false,
        },
    )
    .expect("create should succeed");

    assert_uuidv7_text(&created.id);
    assert_db002_shape(&created.created_at);
    assert_eq!(created.created_at, created.updated_at);
    assert_eq!(created.instruction_id, item.id);
    assert_eq!(created.name, "Strict");
    assert_eq!(created.content, "Answer in one sentence.");
    assert!(!created.is_main);
    assert_eq!(created.state, VariantState::Active);
    assert_eq!(created.deleted_at, None);

    let reread = get_variant(&conn, &created.id)
        .expect("get_variant should succeed")
        .expect("the created row should exist");
    assert_eq!(reread, created);
    let listed = list_variants(&conn, &item.id).expect("list_variants should succeed");
    assert_eq!(listed.len(), 2, "main + the new alternative");
    assert_eq!(
        listed
            .iter()
            .map(|variant| variant.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Main", "Strict"],
        "creation order (created_at, id)"
    );
}

#[test]
fn the_first_variant_of_an_instruction_must_be_its_main() {
    // DB-010's constructive half: creating an instruction's first
    // active variant without the main flag would leave DB-010
    // unsatisfiable by later API calls.
    let (conn, _dir) = library_db("first-must-be-main");
    let item = create_item(
        &conn,
        &NewLibraryItem::Instruction {
            title: "Bare instruction",
        },
    )
    .expect("create should succeed");

    let err = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .map(|_| ())
    .expect_err("a first non-main variant must be rejected");
    assert!(
        matches!(
            err,
            LibraryError::MainRequired { ref instruction_id } if instruction_id == &item.id
        ),
        "expected MainRequired, got {err:?}"
    );
    assert_eq!(
        list_variants(&conn, &item.id)
            .expect("list_variants should succeed")
            .len(),
        0,
        "the rejected row left nothing behind"
    );

    create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Main",
            content: "",
            is_main: true,
        },
    )
    .expect("the first variant as main is accepted");
}

#[test]
fn a_second_active_main_variant_is_a_typed_conflict() {
    let (conn, _dir) = library_db("second-main");
    let (item, main) = seed_instruction(&conn, 1);

    let err = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Second main",
            content: "",
            is_main: true,
        },
    )
    .map(|_| ())
    .expect_err("a second active main must be rejected");
    assert!(
        matches!(
            err,
            LibraryError::MainVariantExists { ref instruction_id } if instruction_id == &item.id
        ),
        "expected MainVariantExists, got {err:?}"
    );
    // The schema's partial index stays the floor behind the typed
    // pre-check: the surviving main is untouched.
    let reread = get_variant(&conn, &main.id)
        .expect("get_variant should succeed")
        .expect("the main variant should exist");
    assert!(reread.is_main);
}

#[test]
fn active_variant_names_are_unique_case_insensitively() {
    // §6.9: unique among the instruction's active variants, NOCASE —
    // the `ux_variant_name` partial index's scope.
    let (conn, _dir) = library_db("name-nocase");
    let (item, _) = seed_instruction(&conn, 1);

    for twin in ["main", "MAIN", "MaIn"] {
        let err = create_variant(
            &conn,
            &NewVariant {
                instruction_id: &item.id,
                name: twin,
                content: "",
                is_main: false,
            },
        )
        .map(|_| ())
        .expect_err("the NOCASE twin must be rejected");
        assert!(
            matches!(
                err,
                LibraryError::DuplicateVariantName { ref name, .. } if name == twin
            ),
            "expected DuplicateVariantName for {twin:?}, got {err:?}"
        );
    }
    // Different instructions are separate namespaces.
    let (other, _) = seed_instruction(&conn, 2);
    create_variant(
        &conn,
        &NewVariant {
            instruction_id: &other.id,
            name: "Twin",
            content: "",
            is_main: false,
        },
    )
    .expect("the same name on another instruction is legal");
}

#[test]
fn a_variant_name_freed_by_a_soft_delete_becomes_reusable() {
    // The NOCASE index covers active rows only (§6.2): soft-deleting
    // the holder frees the name.
    let (conn, _dir) = library_db("name-reuse");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Strict",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    let _ = main;

    let err = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "strict",
            content: "",
            is_main: false,
        },
    )
    .map(|_| ())
    .expect_err("the NOCASE twin must be rejected while Strict is active");
    assert!(matches!(err, LibraryError::DuplicateVariantName { .. }));

    delete_variant(&conn, &alt.id, None).expect("delete should succeed");
    create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "strict",
            content: "",
            is_main: false,
        },
    )
    .expect("the freed name is reusable in any case shape");
}

#[test]
fn creating_a_variant_on_a_heading_or_an_unknown_instruction_is_typed() {
    let (conn, _dir) = library_db("variant-on-heading");
    let heading = seed_heading(&conn, 1, 2);
    let err = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &heading.id,
            name: "Main",
            content: "",
            is_main: true,
        },
    )
    .map(|_| ())
    .expect_err("a heading must not gain variants");
    assert!(
        matches!(
            err,
            LibraryError::NotAnInstruction { ref id } if id == &heading.id
        ),
        "expected NotAnInstruction, got {err:?}"
    );

    let err = create_variant(
        &conn,
        &NewVariant {
            instruction_id: "no-such-id",
            name: "Main",
            content: "",
            is_main: true,
        },
    )
    .map(|_| ())
    .expect_err("an unknown instruction must be rejected");
    assert!(
        matches!(
            err,
            LibraryError::NotFound { ref id } if id == "no-such-id"
        ),
        "expected NotFound, got {err:?}"
    );
}

#[test]
fn the_db006_content_boundary_is_the_check_floor() {
    let (conn, _dir) = library_db("content-boundary");
    let (item, _) = seed_instruction(&conn, 1);
    let max = "x".repeat(262_144);
    let created = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Max",
            content: &max,
            is_main: false,
        },
    )
    .expect("exactly 262144 characters fit (DB-006)");
    assert_eq!(created.content.len(), 262_144);

    let over = "x".repeat(262_145);
    let err = update_variant(
        &conn,
        &created.id,
        &VariantUpdate {
            content: Some(&over),
            ..VariantUpdate::default()
        },
    )
    .map(|_| ())
    .expect_err("262145 characters must be rejected");
    assert!(
        matches!(err, LibraryError::Sql(_)),
        "expected the raw SQL (CHECK) error, got {err:?}"
    );

    // The §6.9 name-length floor on the variant side (the validators
    // own the friendly message; the store must not let a bypassed
    // value through).
    let err = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "   ",
            content: "",
            is_main: false,
        },
    )
    .map(|_| ())
    .expect_err("a whitespace-only name must be rejected");
    assert!(
        matches!(err, LibraryError::Sql(_)),
        "expected the raw SQL (CHECK) error, got {err:?}"
    );
    assert_eq!(
        list_variants(&conn, &item.id)
            .expect("list_variants should succeed")
            .len(),
        2,
        "the rejected row left nothing behind"
    );
}

#[test]
fn variant_text_is_stored_crlf_folded_and_the_fold_measures_db006() {
    // §6.9's variant-text row: "CRLF is changed to LF on save" — this
    // layer's own clause (D-076's validation/normalization split).
    // The stored form is the folded form, so the schema CHECK
    // measures what is actually stored.
    let (conn, _dir) = library_db("crlf-fold");
    let (item, _) = seed_instruction(&conn, 1);

    let created = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Folded",
            content: "a\r\nb\r\nc",
            is_main: false,
        },
    )
    .expect("create should succeed");
    assert_eq!(created.content, "a\nb\nc", "CRLF is folded on save");

    // CRLF only: a lone CR is deliberately untouched (the lone-CR
    // conversion is ALG-002's build-time pass, D-076's scope note).
    let lone = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Lone",
            content: "x\ry",
            is_main: false,
        },
    )
    .expect("create should succeed");
    assert_eq!(lone.content, "x\ry", "a lone CR survives the save fold");

    let updated = update_variant(
        &conn,
        &created.id,
        &VariantUpdate {
            content: Some("d\r\ne"),
            ..VariantUpdate::default()
        },
    )
    .expect("update should succeed");
    assert_eq!(updated.content, "d\ne", "the update path folds alike");

    // D-076's recorded asymmetry: a raw text over the limit whose
    // folded form fits is a legal stored text — 131072 `x\r\n` pairs
    // are 393216 raw code points but exactly 262144 folded.
    let raw = "x\r\n".repeat(131_072);
    assert_eq!(raw.chars().count(), 393_216);
    let fits_folded = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Big",
            content: &raw,
            is_main: false,
        },
    )
    .expect("the folded form fits DB-006, so the raw form is accepted");
    assert_eq!(
        fits_folded.content.chars().count(),
        262_144,
        "exactly the limit is stored"
    );
}

// ---- Variants: update and the main flag -------------------------------------------

#[test]
fn updating_a_variant_writes_exactly_the_present_fields_and_restamps() {
    let (conn, _dir) = library_db("update-variant");
    let (item, _) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Strict",
            content: "old text",
            is_main: false,
        },
    )
    .expect("create should succeed");

    // Sleeping past a millisecond boundary before the update (the
    // D-031 test precedent) makes the stamp assertion below
    // discriminating.
    std::thread::sleep(std::time::Duration::from_millis(2));

    let updated = update_variant(
        &conn,
        &alt.id,
        &VariantUpdate {
            content: Some("new text"),
            ..VariantUpdate::default()
        },
    )
    .expect("update should succeed");
    assert_eq!(updated.content, "new text");
    assert_eq!(updated.name, "Strict", "absent fields keep their values");
    assert_eq!(updated.state, VariantState::Active);
    assert!(
        updated.updated_at.as_str() > updated.created_at.as_str(),
        "updated_at {} must be strictly later than created_at {}",
        updated.updated_at,
        updated.created_at
    );

    // A rename onto the own name is exempt from the twin check;
    // onto a sibling's it is typed.
    update_variant(
        &conn,
        &alt.id,
        &VariantUpdate {
            name: Some("STRICT"),
            ..VariantUpdate::default()
        },
    )
    .expect("keeping the own name NOCASE-equal is legal");
    let err = update_variant(
        &conn,
        &alt.id,
        &VariantUpdate {
            name: Some("main"),
            ..VariantUpdate::default()
        },
    )
    .map(|_| ())
    .expect_err("renaming onto the sibling's name must be rejected");
    assert!(
        matches!(err, LibraryError::DuplicateVariantName { ref name, .. } if name == "main"),
        "expected DuplicateVariantName, got {err:?}"
    );

    let err = update_variant(&conn, "no-such-id", &VariantUpdate::default())
        .map(|_| ())
        .expect_err("an unknown id must be rejected");
    assert!(matches!(err, LibraryError::NotFound { ref id } if id == "no-such-id"));
}

#[test]
fn a_no_op_variant_update_changes_nothing_including_the_stamp() {
    let (conn, _dir) = library_db("update-variant-noop");
    let (item, _) = seed_instruction(&conn, 1);
    let listed = list_variants(&conn, &item.id).expect("list_variants should succeed");
    let original = &listed[0];

    let unchanged = update_variant(&conn, &original.id, &VariantUpdate::default())
        .expect("a fieldless update should succeed");
    assert_eq!(
        unchanged,
        original.clone(),
        "identical row: no churn, no restamp"
    );
}

#[test]
fn updating_a_deleted_variant_is_a_typed_error() {
    let (conn, _dir) = library_db("update-deleted");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    delete_variant(&conn, &alt.id, None).expect("delete should succeed");

    let err = update_variant(
        &conn,
        &alt.id,
        &VariantUpdate {
            content: Some("zombie edit"),
            ..VariantUpdate::default()
        },
    )
    .map(|_| ())
    .expect_err("editing a soft-deleted variant must be rejected");
    assert!(
        matches!(err, LibraryError::VariantNotActive { ref id } if id == &alt.id),
        "expected VariantNotActive, got {err:?}"
    );
    let _ = main;
}

#[test]
fn set_main_switches_the_flag_without_restamping_and_leaves_other_instructions_alone() {
    let (conn, _dir) = library_db("set-main");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    let (other, other_main) = seed_instruction(&conn, 2);

    let promoted = set_main_variant(&conn, &alt.id).expect("set_main should succeed");
    assert!(promoted.is_main);
    // The flag flip is metadata (D-16): no restamp.
    assert_eq!(promoted.updated_at, alt.updated_at);
    let demoted = get_variant(&conn, &main.id)
        .expect("get_variant should succeed")
        .expect("the old main should exist");
    assert!(!demoted.is_main);
    assert_eq!(
        demoted.updated_at, main.updated_at,
        "demotion restamps nothing"
    );

    // Other instructions keep their mains (the demote is scoped).
    let untouched = get_variant(&conn, &other_main.id)
        .expect("get_variant should succeed")
        .expect("the other main should exist");
    assert!(untouched.is_main);
    let _ = other;

    // Promoting the current main again is a net no-op.
    let again = set_main_variant(&conn, &alt.id).expect("re-promotion should succeed");
    assert!(again.is_main);

    let err = set_main_variant(&conn, "no-such-id")
        .map(|_| ())
        .expect_err("an unknown id must be rejected");
    assert!(matches!(err, LibraryError::NotFound { .. }));
}

#[test]
fn making_a_deleted_variant_main_is_a_typed_error() {
    let (conn, _dir) = library_db("set-main-deleted");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    delete_variant(&conn, &alt.id, None).expect("delete should succeed");

    let err = set_main_variant(&conn, &alt.id)
        .map(|_| ())
        .expect_err("a deleted variant must not become main");
    assert!(
        matches!(err, LibraryError::VariantNotActive { ref id } if id == &alt.id),
        "expected VariantNotActive, got {err:?}"
    );
    let _ = main;
}

// ---- Variants: delete and restore (LIB-023/025, DB-012) ---------------------------

#[test]
fn deleting_an_unassigned_non_main_variant_soft_deletes_it() {
    let (conn, _dir) = library_db("delete-soft");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "text",
            is_main: false,
        },
    )
    .expect("create should succeed");

    std::thread::sleep(std::time::Duration::from_millis(2));
    let deleted = delete_variant(&conn, &alt.id, None).expect("delete should succeed");
    assert_eq!(deleted.state, VariantState::Deleted);
    let stamp = deleted.deleted_at.clone().expect("deleted_at must be set");
    assert_db002_shape(&stamp);
    assert!(!deleted.is_main, "the main flag goes with the delete");
    assert!(
        deleted.updated_at.as_str() > deleted.created_at.as_str(),
        "the state change restamps"
    );
    // §6.4: the row stays (the Restore surface), and the main keeps
    // DB-010.
    let main_reread = get_variant(&conn, &main.id)
        .expect("get_variant should succeed")
        .expect("the main should exist");
    assert!(main_reread.is_main);
    assert_eq!(main_reread.state, VariantState::Active);

    // Idempotent: a repeated delete returns the row unchanged.
    let again = delete_variant(&conn, &alt.id, None).expect("a repeated delete should succeed");
    assert_eq!(again, deleted);

    let err = delete_variant(&conn, "no-such-id", None)
        .map(|_| ())
        .expect_err("an unknown id must be rejected");
    assert!(matches!(err, LibraryError::NotFound { .. }));
}

#[test]
fn the_last_active_variant_cannot_be_deleted_whatever_the_replacement() {
    // LIB-025 is the absolute refusal; it fires even when the caller
    // names a replacement (which could only be a non-sibling here).
    let (conn, _dir) = library_db("delete-last");
    let (item, main) = seed_instruction(&conn, 1);
    let (other, other_main) = seed_instruction(&conn, 2);

    for replacement in [None, Some(other_main.id.as_str())] {
        let err = delete_variant(&conn, &main.id, replacement)
            .map(|_| ())
            .expect_err("the last active variant must not be deletable");
        assert!(
            matches!(
                err,
                LibraryError::LastVariant { ref instruction_id } if instruction_id == &item.id
            ),
            "expected LastVariant, got {err:?}"
        );
    }
    let reread = get_variant(&conn, &main.id)
        .expect("get_variant should succeed")
        .expect("the variant should survive");
    assert_eq!(reread.state, VariantState::Active);
    let _ = other;
}

#[test]
fn deleting_the_main_variant_requires_a_named_replacement() {
    let (conn, _dir) = library_db("delete-main-required");
    let (item, main) = seed_instruction(&conn, 1);
    create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");

    let err = delete_variant(&conn, &main.id, None)
        .map(|_| ())
        .expect_err("deleting the main without a successor must be rejected");
    assert!(
        matches!(
            err,
            LibraryError::ReplacementRequired { ref instruction_id } if instruction_id == &item.id
        ),
        "expected ReplacementRequired, got {err:?}"
    );
    let reread = get_variant(&conn, &main.id)
        .expect("get_variant should succeed")
        .expect("the main should survive");
    assert!(reread.is_main);
}

#[test]
fn a_missing_main_surfaces_as_a_typed_error_not_a_guess() {
    // The main flag is reachable by a plain out-of-band UPDATE that
    // violates no CHECK — DB-010 can be broken without corruption of
    // any cell. The default replacement then refuses to guess
    // (DB-017: surfaced, not repaired) instead of panicking.
    let (conn, _dir) = library_db("no-active-main");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");

    conn.execute(
        "UPDATE instruction_variants SET is_main = 0 WHERE id = ?1",
        [main.id.as_str()],
    )
    .expect("the out-of-band flag clear applies (no CHECK is violated)");

    let err = delete_variant(&conn, &alt.id, None)
        .map(|_| ())
        .expect_err("a missing main must not be guessed around");
    assert!(
        matches!(
            err,
            LibraryError::NoActiveMain { ref instruction_id } if instruction_id == &item.id
        ),
        "expected NoActiveMain, got {err:?}"
    );
    // Nothing was written: the variant survives.
    let survived = get_variant(&conn, &alt.id)
        .expect("get_variant should succeed")
        .expect("the variant should survive");
    assert_eq!(survived.state, VariantState::Active);
}

#[test]
fn deleting_the_main_variant_promotes_the_replacement_and_moves_assignments() {
    let (conn, _dir) = library_db("delete-main-promotes");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "alt text",
            is_main: false,
        },
    )
    .expect("create should succeed");
    let project = seed_project(&conn, 1);
    let assignment = assign(
        &conn,
        &NewAssignment {
            project_id: &project.id,
            instruction_id: &item.id,
            variant_id: &main.id,
        },
    )
    .expect("assign should succeed");

    std::thread::sleep(std::time::Duration::from_millis(2));
    let deleted = delete_variant(&conn, &main.id, Some(&alt.id)).expect("delete should succeed");
    assert_eq!(deleted.state, VariantState::Deleted);
    assert!(!deleted.is_main);

    // The replacement is the new main; the promotion restamps nothing
    // (metadata, D-16).
    let promoted = get_variant(&conn, &alt.id)
        .expect("get_variant should succeed")
        .expect("the replacement should exist");
    assert!(promoted.is_main);
    assert_eq!(promoted.state, VariantState::Active);
    assert_eq!(
        promoted.updated_at, alt.updated_at,
        "promotion restamps nothing"
    );

    // LIB-023's acceptance: no assignment disappeared silently — the
    // assignment now points at the replacement.
    let moved = get_assignment(&conn, &project.id, &item.id)
        .expect("get_assignment should succeed")
        .expect("the assignment should survive");
    assert_eq!(moved.variant_id, alt.id);
    assert!(
        moved.updated_at.as_str() > assignment.created_at.as_str(),
        "the move restamps the assignment row"
    );
    assert_eq!(
        list_assignments_for_variant(&conn, &main.id)
            .expect("list should succeed")
            .len(),
        0,
        "the deleted variant holds no assignments"
    );
}

#[test]
fn deleting_an_assigned_variant_defaults_the_replacement_to_the_main() {
    // LIB-023: the dialog's default is "the main one" — so is the
    // repository's.
    let (conn, _dir) = library_db("delete-default-main");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    let project = seed_project(&conn, 1);
    assign(
        &conn,
        &NewAssignment {
            project_id: &project.id,
            instruction_id: &item.id,
            variant_id: &alt.id,
        },
    )
    .expect("assign should succeed");

    delete_variant(&conn, &alt.id, None).expect("delete should succeed");
    let moved = get_assignment(&conn, &project.id, &item.id)
        .expect("get_assignment should succeed")
        .expect("the assignment should survive");
    assert_eq!(
        moved.variant_id, main.id,
        "the default replacement is the main"
    );
    let main_reread = get_variant(&conn, &main.id)
        .expect("get_variant should succeed")
        .expect("the main should exist");
    assert!(main_reread.is_main, "the untouched main keeps the flag");
}

#[test]
fn a_replacement_that_is_not_an_active_sibling_is_a_typed_error() {
    let (conn, _dir) = library_db("delete-bad-replacement");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    let (other, other_main) = seed_instruction(&conn, 2);

    // Another instruction's variant, an unknown id, and the deleted
    // variant itself: three ways to fail the sibling check.
    for bad in [other_main.id.as_str(), "no-such-id", main.id.as_str()] {
        let err = delete_variant(&conn, &main.id, Some(bad))
            .map(|_| ())
            .expect_err("the bad replacement must be rejected");
        assert!(
            matches!(
                err,
                LibraryError::ReplacementInvalid {
                    ref instruction_id,
                    ref replacement,
                } if instruction_id == &item.id && replacement.as_str() == bad
            ),
            "expected ReplacementInvalid naming {bad:?}, got {err:?}"
        );
    }
    // A soft-deleted sibling does not qualify either. A third variant
    // keeps the instruction off the LIB-025 floor so the rejection can
    // be specifically about the deleted sibling.
    let third = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Third",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    delete_variant(&conn, &alt.id, None).expect("delete should succeed");
    let err = delete_variant(&conn, &main.id, Some(&alt.id))
        .map(|_| ())
        .expect_err("a deleted sibling must be rejected");
    assert!(matches!(err, LibraryError::ReplacementInvalid { .. }));
    let _ = third;

    let reread = get_variant(&conn, &main.id)
        .expect("get_variant should succeed")
        .expect("the main should survive every rejection");
    assert!(reread.is_main);
    let _ = other;
}

#[test]
fn restore_variant_reactivates_and_a_reused_name_is_a_typed_conflict() {
    let (conn, _dir) = library_db("restore-variant");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Strict",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    delete_variant(&conn, &alt.id, None).expect("delete should succeed");
    let heir = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "strict",
            content: "",
            is_main: false,
        },
    )
    .expect("the freed name was reused");

    // The name is taken again: the restore is a typed conflict.
    let err = restore_variant(&conn, &alt.id)
        .map(|_| ())
        .expect_err("the reused name must block the restore");
    assert!(
        matches!(
            err,
            LibraryError::DuplicateVariantName { ref name, .. } if name == "Strict"
        ),
        "expected DuplicateVariantName, got {err:?}"
    );

    delete_variant(&conn, &heir.id, None).expect("delete should succeed");
    std::thread::sleep(std::time::Duration::from_millis(2));
    let restored = restore_variant(&conn, &alt.id).expect("restore should succeed");
    assert_eq!(restored.state, VariantState::Active);
    assert_eq!(restored.deleted_at, None);
    assert_eq!(restored.name, "Strict");
    assert!(!restored.is_main, "the old main flag is not restored");
    assert!(
        restored.updated_at.as_str() > restored.created_at.as_str(),
        "the state change restamps"
    );
    // Exactly one active main remains (DB-010).
    let mains = list_variants(&conn, &item.id)
        .expect("list_variants should succeed")
        .into_iter()
        .filter(|variant| variant.is_main && variant.state == VariantState::Active)
        .count();
    assert_eq!(mains, 1);

    // Idempotent: a repeated restore returns the row unchanged.
    let again = restore_variant(&conn, &alt.id).expect("a repeated restore should succeed");
    assert_eq!(again, restored);
    let _ = main;

    let err = restore_variant(&conn, "no-such-id")
        .map(|_| ())
        .expect_err("an unknown id must be rejected");
    assert!(matches!(err, LibraryError::NotFound { .. }));
}

#[test]
fn per_project_replacements_compose_reassign_after_the_delete() {
    // LIB-023's dialog allows a different replacement per affected
    // project; the repository's replacement is one-for-all, and the
    // per-project choices compose `reassign` calls in the same
    // transaction — nothing is lost either way.
    let (conn, _dir) = library_db("per-project-replacement");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    let third = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Third",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    let p1 = seed_project(&conn, 1);
    let p2 = seed_project(&conn, 2);
    for project_id in [&p1.id, &p2.id] {
        assign(
            &conn,
            &NewAssignment {
                project_id,
                instruction_id: &item.id,
                variant_id: &alt.id,
            },
        )
        .expect("assign should succeed");
    }

    {
        let tx = conn.unchecked_transaction().expect("BEGIN should succeed");
        // The delete's default moves everything to the main…
        delete_variant(&tx, &alt.id, None).expect("delete should succeed inside the tx");
        // …then the dialog's per-project choice re-points p2.
        reassign(&tx, &p2.id, &item.id, &third.id).expect("reassign should succeed inside the tx");
        tx.commit().expect("commit should succeed");
    }

    let p1_assignment = get_assignment(&conn, &p1.id, &item.id)
        .expect("get_assignment should succeed")
        .expect("p1 keeps its assignment");
    assert_eq!(p1_assignment.variant_id, main.id, "p1 took the default");
    let p2_assignment = get_assignment(&conn, &p2.id, &item.id)
        .expect("get_assignment should succeed")
        .expect("p2 keeps its assignment");
    assert_eq!(p2_assignment.variant_id, third.id, "p2 took its own choice");
}

// ---- Items: hide/restore (§6.4, DB-013) -------------------------------------------

#[test]
fn hiding_keeps_variants_assignments_and_position_and_is_idempotent() {
    let (conn, _dir) = library_db("hide");
    let (item, main) = seed_instruction(&conn, 1);
    let heading = seed_heading(&conn, 1, 2);
    let (after, _) = seed_instruction(&conn, 2);
    let project = seed_project(&conn, 1);
    assign(
        &conn,
        &NewAssignment {
            project_id: &project.id,
            instruction_id: &item.id,
            variant_id: &main.id,
        },
    )
    .expect("assign should succeed");

    let hidden = hide_item(&conn, &item.id).expect("hide should succeed");
    assert_eq!(hidden.state, ItemState::Hidden);
    let stamp = hidden.hidden_at.clone().expect("hidden_at must be set");
    assert_db002_shape(&stamp);
    assert_eq!(hidden.position, item.position, "hide keeps the position");
    assert!(
        hidden.updated_at.as_str() > hidden.created_at.as_str(),
        "the state change restamps"
    );

    // DB-013: a hidden instruction keeps its assignments; §6.4: the
    // variants stay.
    assert!(
        get_assignment(&conn, &project.id, &item.id)
            .expect("get_assignment should succeed")
            .is_some()
    );
    let variants = list_variants(&conn, &item.id).expect("list_variants should succeed");
    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].state, VariantState::Active);

    // The scoped lists split correctly; the table-wide sequence is
    // untouched (positions stay 1, 2, 3 in creation order). Headings
    // are always active (§6.2 CHECK), so the heading stays visible.
    let active_ids: Vec<String> = list_items(&conn, ItemScope::Active)
        .expect("list should succeed")
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(
        active_ids,
        vec![heading.id.clone(), after.id.clone()],
        "the heading and the other instruction stay active"
    );
    let hidden_ids: Vec<String> = list_items(&conn, ItemScope::Hidden)
        .expect("list should succeed")
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(hidden_ids, vec![item.id.clone()]);
    assert_eq!(all_positions(&conn), vec![1, 2, 3], "table-wide and dense");

    // Idempotent, without timestamp drift.
    let again = hide_item(&conn, &item.id).expect("a repeated hide should succeed");
    assert_eq!(again, hidden);

    // DB-013's "gains": a hidden instruction takes a new assignment
    // (it is excluded from the build, not from the matrix data).
    let second_project = seed_project(&conn, 2);
    assign(
        &conn,
        &NewAssignment {
            project_id: &second_project.id,
            instruction_id: &item.id,
            variant_id: &main.id,
        },
    )
    .expect("a hidden instruction gains assignments");

    // A hidden row moves like any other: position is table-wide
    // metadata, indifferent to the item's state.
    let moved = move_item(&conn, &item.id, 3).expect("a hidden row moves");
    assert_eq!(moved.position, 3);
    assert_eq!(
        all_positions(&conn),
        vec![1, 2, 3],
        "the sequence stays dense when the mover is hidden"
    );

    let restored = restore_item(&conn, &item.id).expect("restore should succeed");
    assert_eq!(restored.state, ItemState::Active);
    assert_eq!(restored.hidden_at, None);
    assert_eq!(
        restored.position, moved.position,
        "restore returns the position the row last had"
    );

    for err in [
        hide_item(&conn, "no-such-id")
            .map(|_| ())
            .expect_err("must be rejected"),
        restore_item(&conn, "no-such-id")
            .map(|_| ())
            .expect_err("must be rejected"),
    ] {
        assert!(matches!(err, LibraryError::NotFound { ref id } if id == "no-such-id"));
    }
}

#[test]
fn hiding_a_heading_hits_the_check_floor() {
    // §6.2 CHECK: headings cannot be hidden; the floor surfaces as
    // the raw SQL error (the store does not re-validate §6.9).
    let (conn, _dir) = library_db("hide-heading");
    let heading = seed_heading(&conn, 1, 2);
    let err = hide_item(&conn, &heading.id)
        .map(|_| ())
        .expect_err("a heading must not be hideable");
    assert!(
        matches!(err, LibraryError::Sql(_)),
        "expected the raw SQL (CHECK) error, got {err:?}"
    );
    let reread = get_item(&conn, &heading.id)
        .expect("get_item should succeed")
        .expect("the heading should survive");
    assert_eq!(reread.state, ItemState::Active);

    // restore_item on a heading is the documented idempotent no-op: a
    // heading is always active, so it is returned unchanged (the
    // restore-on-active path, not a silent state change).
    let restored = restore_item(&conn, &heading.id).expect("restore should succeed");
    assert_eq!(restored, reread);
}

// ---- Items: rename and level changes ----------------------------------------------

#[test]
fn renaming_and_level_changes_restamp_and_a_no_op_changes_nothing() {
    let (conn, _dir) = library_db("update-item");
    let heading = seed_heading(&conn, 1, 2);

    std::thread::sleep(std::time::Duration::from_millis(2));
    let renamed = update_item(
        &conn,
        &heading.id,
        &ItemUpdate {
            title: Some("Renamed section"),
            heading_level: Some(3),
        },
    )
    .expect("update should succeed");
    assert_eq!(renamed.title, "Renamed section");
    assert_eq!(renamed.heading_level, Some(3));
    assert!(
        renamed.updated_at.as_str() > renamed.created_at.as_str(),
        "rename and level changes restamp"
    );

    let unchanged = update_item(&conn, &heading.id, &ItemUpdate::default())
        .expect("a fieldless update should succeed");
    assert_eq!(unchanged, renamed, "no churn, no restamp");

    // An out-of-range level hits the CHECK floor.
    let err = update_item(
        &conn,
        &heading.id,
        &ItemUpdate {
            heading_level: Some(7),
            ..ItemUpdate::default()
        },
    )
    .map(|_| ())
    .expect_err("level 7 must be rejected");
    assert!(matches!(err, LibraryError::Sql(_)));

    // The §6.2 CHECK matrix floor through the dynamic update: a
    // level written onto an instruction row is rejected (the kind is
    // immutable, but the column mapping must not bypass the matrix).
    let (instruction, _) = seed_instruction(&conn, 3);
    let err = update_item(
        &conn,
        &instruction.id,
        &ItemUpdate {
            heading_level: Some(3),
            ..ItemUpdate::default()
        },
    )
    .map(|_| ())
    .expect_err("a level on an instruction must be rejected");
    assert!(
        matches!(err, LibraryError::Sql(_)),
        "expected the raw SQL (CHECK) error, got {err:?}"
    );

    // A whitespace-only title hits the §6.2 trim CHECK floor.
    let err = update_item(
        &conn,
        &heading.id,
        &ItemUpdate {
            title: Some("   "),
            ..ItemUpdate::default()
        },
    )
    .map(|_| ())
    .expect_err("a blank title must be rejected");
    assert!(matches!(err, LibraryError::Sql(_)));

    let err = update_item(&conn, "no-such-id", &ItemUpdate::default())
        .map(|_| ())
        .expect_err("an unknown id must be rejected");
    assert!(matches!(err, LibraryError::NotFound { .. }));
}

// ---- Items: positions under move and delete (DB-005/DB-011) ----------------------

#[test]
fn moving_an_item_renumbers_the_range_and_keeps_the_sequence_dense() {
    let (conn, _dir) = library_db("move");
    let a = seed_heading(&conn, 1, 2);
    let (b, _) = seed_instruction(&conn, 1);
    let (c, _) = seed_instruction(&conn, 2);
    let d = seed_heading(&conn, 2, 2);

    // Middle → front: the head shifts down by one.
    let moved = move_item(&conn, &c.id, 1).expect("move should succeed");
    assert_eq!(moved.position, 1);
    assert_eq!(
        all_item_ids(&conn),
        vec![c.id.clone(), a.id.clone(), b.id.clone(), d.id.clone()]
    );
    assert_eq!(
        all_positions(&conn),
        vec![1, 2, 3, 4],
        "dense after the move"
    );
    // A position move is metadata: no restamp (D-16).
    assert_eq!(moved.updated_at, c.updated_at);

    // Front area → end: the tail shifts up.
    move_item(&conn, &a.id, 4).expect("move should succeed");
    assert_eq!(
        all_item_ids(&conn),
        vec![c.id.clone(), b.id.clone(), d.id.clone(), a.id.clone()]
    );
    assert_eq!(all_positions(&conn), vec![1, 2, 3, 4]);

    // A move to the current position writes nothing.
    let before = all_positions(&conn);
    let stationary = move_item(&conn, &b.id, 2).expect("move should succeed");
    assert_eq!(stationary.position, 2);
    assert_eq!(all_positions(&conn), before);
}

#[test]
fn moving_out_of_the_table_range_is_a_typed_error_and_touches_nothing() {
    let (conn, _dir) = library_db("move-range");
    let a = seed_heading(&conn, 1, 2);
    let (b, _) = seed_instruction(&conn, 1);

    for requested in [0, 3] {
        let err = move_item(&conn, &a.id, requested)
            .map(|_| ())
            .expect_err("a position outside 1..=N must be rejected");
        assert!(
            matches!(
                err,
                LibraryError::PositionOutOfRange {
                    requested: denied,
                    max: 2,
                } if denied == requested
            ),
            "expected PositionOutOfRange({requested}), got {err:?}"
        );
    }
    assert_eq!(all_positions(&conn), vec![1, 2], "the table is untouched");

    let err = move_item(&conn, "no-such-id", 1)
        .map(|_| ())
        .expect_err("an unknown id must be rejected");
    assert!(matches!(err, LibraryError::NotFound { .. }));
    let _ = b;
}

#[test]
fn a_move_inside_a_transaction_rolls_back_as_a_set() {
    let (conn, _dir) = library_db("move-rollback");
    let a = seed_heading(&conn, 1, 2);
    let (b, _) = seed_instruction(&conn, 1);
    let (c, _) = seed_instruction(&conn, 2);

    {
        let tx = conn.unchecked_transaction().expect("BEGIN should succeed");
        move_item(&tx, &c.id, 1).expect("move should succeed inside the tx");
        // The change is visible inside the transaction…
        assert_eq!(
            all_item_ids(&tx),
            vec![c.id.as_str(), a.id.as_str(), b.id.as_str()]
        );
        // …and vanishes with the rollback (the Transaction drop).
    }
    assert_eq!(
        all_item_ids(&conn),
        vec![a.id, b.id, c.id],
        "the rollback erased the whole renumbering"
    );
    assert_eq!(all_positions(&conn), vec![1, 2, 3]);
}

#[test]
fn deleting_a_heading_closes_the_gap_and_an_instruction_is_not_a_heading() {
    let (conn, _dir) = library_db("delete-heading");
    let a = seed_heading(&conn, 1, 2);
    let (b, _) = seed_instruction(&conn, 1);
    let h = seed_heading(&conn, 2, 3);
    let (c, _) = seed_instruction(&conn, 2);

    delete_heading(&conn, &h.id).expect("delete_heading should succeed");
    assert_eq!(
        get_item(&conn, &h.id).expect("get_item should succeed"),
        None
    );
    // §6.4: the section's instructions stay in the list — positionally
    // shifted up, the sequence dense again.
    assert_eq!(
        all_item_ids(&conn),
        vec![a.id.clone(), b.id.clone(), c.id.clone()]
    );
    assert_eq!(all_positions(&conn), vec![1, 2, 3], "the gap closed");

    // Instructions have no physical delete in v1 (§6.4).
    let err = delete_heading(&conn, &b.id)
        .map(|_| ())
        .expect_err("an instruction must not be physically deleted");
    assert!(
        matches!(err, LibraryError::NotAHeading { ref id } if id == &b.id),
        "expected NotAHeading, got {err:?}"
    );
    let err = delete_heading(&conn, "no-such-id")
        .map(|_| ())
        .expect_err("an unknown id must be rejected");
    assert!(matches!(err, LibraryError::NotFound { .. }));
}

#[test]
fn deleting_the_last_item_empties_the_table_and_a_blank_title_hits_the_check_floor() {
    // The gap-close with an empty tail (no `position > ?` row), and
    // the §6.2 trim CHECK floor at creation time (the update path is
    // pinned elsewhere).
    let (conn, _dir) = library_db("delete-last-item");
    let heading = seed_heading(&conn, 1, 2);

    delete_heading(&conn, &heading.id).expect("the last row deletes");
    assert_eq!(
        list_items(&conn, ItemScope::All)
            .expect("list_items should succeed")
            .len(),
        0,
        "the table is empty"
    );
    assert_eq!(all_positions(&conn), Vec::<i64>::new());

    let err = create_item(&conn, &NewLibraryItem::Instruction { title: "   " })
        .map(|_| ())
        .expect_err("a whitespace-only title must be rejected");
    assert!(
        matches!(err, LibraryError::Sql(_)),
        "expected the raw SQL (CHECK) error, got {err:?}"
    );
}

// ---- Assignments: assign, reassign, unassign (DB-012, D-02, MAIN-005) ------------

#[test]
fn assign_pins_every_column_and_round_trips() {
    let (conn, _dir) = library_db("assign");
    let (item, main) = seed_instruction(&conn, 1);
    let project = seed_project(&conn, 1);

    let created = assign(
        &conn,
        &NewAssignment {
            project_id: &project.id,
            instruction_id: &item.id,
            variant_id: &main.id,
        },
    )
    .expect("assign should succeed");
    assert_eq!(created.project_id, project.id);
    assert_eq!(created.instruction_id, item.id);
    assert_eq!(created.variant_id, main.id);
    assert_db002_shape(&created.created_at);
    assert_eq!(created.created_at, created.updated_at);

    let reread = get_assignment(&conn, &project.id, &item.id)
        .expect("get_assignment should succeed")
        .expect("the assignment should exist");
    assert_eq!(reread, created);
    assert_eq!(
        get_assignment(&conn, &project.id, "no-such-instruction")
            .expect("get_assignment should succeed"),
        None
    );
}

#[test]
fn assigning_the_same_instruction_twice_is_a_typed_error() {
    // D-02: a project can have only one variant of an instruction
    // (the composite PRIMARY KEY); the change path is reassign.
    let (conn, _dir) = library_db("assign-duplicate");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    let project = seed_project(&conn, 1);
    assign(
        &conn,
        &NewAssignment {
            project_id: &project.id,
            instruction_id: &item.id,
            variant_id: &alt.id,
        },
    )
    .expect("assign should succeed");

    let err = assign(
        &conn,
        &NewAssignment {
            project_id: &project.id,
            instruction_id: &item.id,
            variant_id: &main.id,
        },
    )
    .map(|_| ())
    .expect_err("the second assignment must be rejected");
    assert!(
        matches!(
            err,
            LibraryError::AlreadyAssigned {
                ref project_id,
                ref instruction_id,
            } if project_id == &project.id && instruction_id == &item.id
        ),
        "expected AlreadyAssigned, got {err:?}"
    );
    // The surviving assignment is untouched.
    let survivor = get_assignment(&conn, &project.id, &item.id)
        .expect("get_assignment should succeed")
        .expect("the assignment should exist");
    assert_eq!(survivor.variant_id, alt.id);
}

#[test]
fn an_assignment_requires_an_active_variant_of_the_same_instruction() {
    // DB-012's three failure causes: an unknown variant, another
    // instruction's variant, and a soft-deleted variant.
    let (conn, _dir) = library_db("assign-variant-rules");
    let (item, _) = seed_instruction(&conn, 1);
    let (other, _) = seed_instruction(&conn, 2);
    let other_alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &other.id,
            name: "Other alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    let project = seed_project(&conn, 1);

    let dead = {
        // A soft-deleted variant of `item` itself.
        let doomed = create_variant(
            &conn,
            &NewVariant {
                instruction_id: &item.id,
                name: "Doomed",
                content: "",
                is_main: false,
            },
        )
        .expect("create should succeed");
        delete_variant(&conn, &doomed.id, None).expect("delete should succeed");
        doomed
    };

    for bad in ["no-such-variant", other_alt.id.as_str(), dead.id.as_str()] {
        let err = assign(
            &conn,
            &NewAssignment {
                project_id: &project.id,
                instruction_id: &item.id,
                variant_id: bad,
            },
        )
        .map(|_| ())
        .expect_err("the non-assignable variant must be rejected");
        assert!(
            matches!(
                err,
                LibraryError::VariantNotAssignable {
                    ref instruction_id,
                    ref variant_id,
                } if instruction_id == &item.id && variant_id.as_str() == bad
            ),
            "expected VariantNotAssignable for {bad:?}, got {err:?}"
        );
    }
    assert_eq!(
        list_assignments_for_project(&conn, &project.id)
            .expect("list should succeed")
            .len(),
        0,
        "no assignment was written"
    );
}

#[test]
fn assigning_to_a_heading_an_unknown_project_or_an_unknown_instruction_is_typed() {
    let (conn, _dir) = library_db("assign-targets");
    let (item, main) = seed_instruction(&conn, 1);
    let heading = seed_heading(&conn, 1, 2);
    let project = seed_project(&conn, 1);

    let err = assign(
        &conn,
        &NewAssignment {
            project_id: "no-such-project",
            instruction_id: &item.id,
            variant_id: &main.id,
        },
    )
    .map(|_| ())
    .expect_err("an unknown project must be rejected");
    assert!(matches!(err, LibraryError::NotFound { ref id } if id == "no-such-project"));

    let err = assign(
        &conn,
        &NewAssignment {
            project_id: &project.id,
            instruction_id: "no-such-instruction",
            variant_id: &main.id,
        },
    )
    .map(|_| ())
    .expect_err("an unknown instruction must be rejected");
    assert!(
        matches!(err, LibraryError::NotFound { ref id } if id == "no-such-instruction"),
        "the instruction check is reachable with an existing project"
    );

    let err = assign(
        &conn,
        &NewAssignment {
            project_id: &project.id,
            instruction_id: &heading.id,
            variant_id: &main.id,
        },
    )
    .map(|_| ())
    .expect_err("a heading must be rejected");
    assert!(
        matches!(err, LibraryError::NotAnInstruction { ref id } if id == &heading.id),
        "expected NotAnInstruction, got {err:?}"
    );
}

#[test]
fn reassign_changes_only_the_calling_projects_variant() {
    // MAIN-005: changing the variant in one project does not change
    // other projects.
    let (conn, _dir) = library_db("reassign");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    let p1 = seed_project(&conn, 1);
    let p2 = seed_project(&conn, 2);
    assign(
        &conn,
        &NewAssignment {
            project_id: &p1.id,
            instruction_id: &item.id,
            variant_id: &main.id,
        },
    )
    .expect("assign should succeed");
    let p2_assignment = assign(
        &conn,
        &NewAssignment {
            project_id: &p2.id,
            instruction_id: &item.id,
            variant_id: &main.id,
        },
    )
    .expect("assign should succeed");

    std::thread::sleep(std::time::Duration::from_millis(2));
    let changed = reassign(&conn, &p1.id, &item.id, &alt.id).expect("reassign should succeed");
    assert_eq!(changed.variant_id, alt.id);
    assert!(
        changed.updated_at.as_str() > changed.created_at.as_str(),
        "the change restamps"
    );
    let other = get_assignment(&conn, &p2.id, &item.id)
        .expect("get_assignment should succeed")
        .expect("the other project's assignment should exist");
    assert_eq!(other, p2_assignment, "the other project keeps its row");

    // A non-sibling variant is typed; an absent assignment is NotFound.
    let (stranger, stranger_main) = seed_instruction(&conn, 2);
    let err = reassign(&conn, &p1.id, &item.id, &stranger_main.id)
        .map(|_| ())
        .expect_err("another instruction's variant must be rejected");
    assert!(
        matches!(err, LibraryError::VariantNotAssignable { .. }),
        "expected VariantNotAssignable, got {err:?}"
    );
    let err = reassign(&conn, &p1.id, &stranger.id, &stranger_main.id)
        .map(|_| ())
        .expect_err("an absent assignment must be rejected");
    assert!(
        matches!(err, LibraryError::NotFound { ref id } if id == &format!("{}/{}", p1.id, stranger.id)),
        "expected NotFound with the pair key, got {err:?}"
    );
    // The existence check runs before the variant check: an absent
    // assignment is reported as absent even when the variant id is
    // garbage too.
    let err = reassign(&conn, &p1.id, &stranger.id, "garbage-variant")
        .map(|_| ())
        .expect_err("an absent assignment must be reported as absent");
    assert!(
        matches!(err, LibraryError::NotFound { .. }),
        "expected NotFound before VariantNotAssignable, got {err:?}"
    );
    // Re-assigning the held variant is a verified no-op: nothing is
    // written, `updated_at` does not churn.
    let before = get_assignment(&conn, &p1.id, &item.id)
        .expect("get_assignment should succeed")
        .expect("the assignment should exist");
    let unchanged = reassign(&conn, &p1.id, &item.id, &alt.id)
        .expect("re-assigning the held variant should succeed");
    assert_eq!(unchanged, before, "identical row: no churn, no restamp");
}

#[test]
fn unassign_removes_the_row_and_keeps_the_variant() {
    // LIB-020: an assignment can be removed without deleting the
    // variant.
    let (conn, _dir) = library_db("unassign");
    let (item, main) = seed_instruction(&conn, 1);
    let project = seed_project(&conn, 1);
    assign(
        &conn,
        &NewAssignment {
            project_id: &project.id,
            instruction_id: &item.id,
            variant_id: &main.id,
        },
    )
    .expect("assign should succeed");

    unassign(&conn, &project.id, &item.id).expect("unassign should succeed");
    assert_eq!(
        get_assignment(&conn, &project.id, &item.id).expect("get_assignment should succeed"),
        None
    );
    let variant = get_variant(&conn, &main.id)
        .expect("get_variant should succeed")
        .expect("the variant should survive");
    assert_eq!(variant.state, VariantState::Active);
    assert!(variant.is_main);

    let err = unassign(&conn, &project.id, &item.id)
        .map(|_| ())
        .expect_err("an absent assignment must be rejected");
    assert!(
        matches!(err, LibraryError::NotFound { ref id } if id == &format!("{}/{}", project.id, item.id)),
        "expected NotFound with the pair key, got {err:?}"
    );
}

#[test]
fn list_assignments_for_project_orders_by_library_position() {
    // OUT-001: the order in the built file equals the order in the
    // library — the per-project read follows `library_items.position`,
    // not insertion order.
    let (conn, _dir) = library_db("list-by-position");
    let (first, first_main) = seed_instruction(&conn, 1);
    seed_heading(&conn, 1, 2);
    let (second, second_main) = seed_instruction(&conn, 2);
    let (third, third_main) = seed_instruction(&conn, 3);
    let project = seed_project(&conn, 1);

    // Assign out of library order.
    for (instruction_id, variant_id) in [
        (&third.id, &third_main.id),
        (&first.id, &first_main.id),
        (&second.id, &second_main.id),
    ] {
        assign(
            &conn,
            &NewAssignment {
                project_id: &project.id,
                instruction_id,
                variant_id,
            },
        )
        .expect("assign should succeed");
    }

    let listed = list_assignments_for_project(&conn, &project.id).expect("list should succeed");
    let listed_ids: Vec<&String> = listed.iter().map(|a| &a.instruction_id).collect();
    assert_eq!(
        listed_ids,
        vec![&first.id, &second.id, &third.id],
        "library position order, headings never present"
    );
}

#[test]
fn list_assignments_for_variant_lists_the_affected_projects() {
    // The LIB-023 delete dialog's read: which projects hold this
    // variant.
    let (conn, _dir) = library_db("list-by-variant");
    let (item, main) = seed_instruction(&conn, 1);
    let p1 = seed_project(&conn, 1);
    let p2 = seed_project(&conn, 2);
    for project_id in [&p1.id, &p2.id] {
        assign(
            &conn,
            &NewAssignment {
                project_id,
                instruction_id: &item.id,
                variant_id: &main.id,
            },
        )
        .expect("assign should succeed");
    }

    let listed = list_assignments_for_variant(&conn, &main.id).expect("list should succeed");
    let project_ids: Vec<&String> = listed.iter().map(|a| &a.project_id).collect();
    assert_eq!(project_ids, vec![&p1.id, &p2.id]);
    assert!(
        listed.iter().all(|a| a.variant_id == main.id),
        "only this variant's assignments"
    );
}

// ---- Corrupt cells and transaction contracts --------------------------------------

#[test]
fn a_corrupt_vocabulary_cell_fails_the_reads_as_typed_member() {
    let (conn, _dir) = library_db("corrupt-cell");
    let (item, main) = seed_instruction(&conn, 1);
    let heading = seed_heading(&conn, 1, 2);

    corrupt_cell(
        &conn,
        &format!(
            "UPDATE library_items SET kind = 'bogus' WHERE id = '{}'",
            sql_literal(&item.id)
        ),
    );
    let err = get_item(&conn, &item.id)
        .map(|_| ())
        .expect_err("a corrupt kind cell must fail the read");
    assert!(
        matches!(err, LibraryError::Member { column: "kind", .. }),
        "expected Member(kind), got {err:?}"
    );
    let list_err = list_items(&conn, ItemScope::All)
        .map(|_| ())
        .expect_err("list_items must fail on the corrupt cell too");
    assert!(
        matches!(list_err, LibraryError::Member { column: "kind", .. }),
        "every read path rejects the corrupt cell"
    );

    corrupt_cell(
        &conn,
        &format!(
            "UPDATE library_items SET kind = 'instruction', state = 'bogus' WHERE id = '{}'",
            sql_literal(&item.id)
        ),
    );
    let err = get_item(&conn, &item.id)
        .map(|_| ())
        .expect_err("a corrupt state cell must fail the read");
    assert!(matches!(
        err,
        LibraryError::Member {
            column: "state",
            ..
        }
    ));

    corrupt_cell(
        &conn,
        &format!(
            "UPDATE library_items SET heading_level = 9 WHERE id = '{}'",
            sql_literal(&heading.id)
        ),
    );
    let err = get_item(&conn, &heading.id)
        .map(|_| ())
        .expect_err("a corrupt heading_level cell must fail the read");
    assert!(
        matches!(
            err,
            LibraryError::Member {
                column: "heading_level",
                ..
            }
        ),
        "expected Member(heading_level), got {err:?}"
    );

    corrupt_cell(
        &conn,
        &format!(
            "UPDATE instruction_variants SET state = 'bogus' WHERE id = '{}'",
            sql_literal(&main.id)
        ),
    );
    let err = get_variant(&conn, &main.id)
        .map(|_| ())
        .expect_err("a corrupt variant state cell must fail the read");
    assert!(
        matches!(
            err,
            LibraryError::Member {
                column: "state",
                ..
            }
        ),
        "expected Member(state), got {err:?}"
    );
    assert!(
        list_variants(&conn, &item.id)
            .map(|_| ())
            .is_err_and(|err| matches!(
                err,
                LibraryError::Member {
                    column: "state",
                    ..
                }
            ))
    );
}

#[test]
fn a_failed_statement_inside_a_transaction_keeps_it_usable() {
    // The ABORT promise the composing command tasks build on: a failed
    // statement rolls back alone and the surrounding transaction goes
    // on to commit real work. Two failure kinds: a typed pre-check
    // rejection (no statement fails) and a statement SQLite itself
    // rejects (the §6.2 CHECK floor — the actual ABORT path).
    let (conn, _dir) = library_db("abort-keeps-transaction");
    let (item, _) = seed_instruction(&conn, 1);

    let tx = conn.unchecked_transaction().expect("BEGIN should succeed");
    create_variant(
        &tx,
        &NewVariant {
            instruction_id: &item.id,
            name: "main",
            content: "",
            is_main: false,
        },
    )
    .map(|_| ())
    .expect_err("the NOCASE twin must conflict inside the transaction too");
    let err = create_item(
        &tx,
        &NewLibraryItem::Heading {
            title: "Bad level",
            level: 7,
        },
    )
    .map(|_| ())
    .expect_err("the CHECK must reject the statement inside the transaction");
    assert!(
        matches!(err, LibraryError::Sql(_)),
        "expected the raw SQL (CHECK) error, got {err:?}"
    );
    let alt = create_variant(
        &tx,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("the transaction must stay usable");
    tx.commit().expect("commit should succeed");

    assert_eq!(
        list_variants(&conn, &item.id)
            .expect("list_variants should succeed")
            .len(),
        2,
        "the committed work survived the earlier failed statements"
    );
    assert_eq!(
        list_items(&conn, ItemScope::All)
            .expect("list_items should succeed")
            .len(),
        1,
        "the CHECK-rejected heading left nothing behind"
    );
    let _ = alt;
}

#[test]
fn delete_variant_inside_a_transaction_rolls_back_everything() {
    // The LIB-023 "everything runs in one transaction" promise: a
    // rolled-back delete leaves the variant, its main flag and every
    // assignment exactly as they were.
    let (conn, _dir) = library_db("delete-rollback");
    let (item, main) = seed_instruction(&conn, 1);
    let alt = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Alt",
            content: "",
            is_main: false,
        },
    )
    .expect("create should succeed");
    let project = seed_project(&conn, 1);
    let assignment = assign(
        &conn,
        &NewAssignment {
            project_id: &project.id,
            instruction_id: &item.id,
            variant_id: &alt.id,
        },
    )
    .expect("assign should succeed");

    {
        let tx = conn.unchecked_transaction().expect("BEGIN should succeed");
        delete_variant(&tx, &alt.id, Some(&main.id)).expect("delete should succeed inside the tx");
        assert_eq!(
            get_assignment(&tx, &project.id, &item.id)
                .expect("get_assignment should succeed")
                .expect("moved inside the tx")
                .variant_id,
            main.id
        );
    }

    let survived = get_variant(&conn, &alt.id)
        .expect("get_variant should succeed")
        .expect("the variant should survive the rollback");
    assert_eq!(survived.state, VariantState::Active);
    assert_eq!(survived.deleted_at, None);
    assert_eq!(
        get_assignment(&conn, &project.id, &item.id)
            .expect("get_assignment should succeed")
            .expect("the assignment should survive the rollback"),
        assignment,
        "the assignment is exactly the pre-transaction row"
    );
    // The main keeps the flag (the promotion was rolled back).
    let main_after = get_variant(&conn, &main.id)
        .expect("get_variant should succeed")
        .expect("the main should exist");
    assert!(main_after.is_main, "the rollback undid the promotion");
}
