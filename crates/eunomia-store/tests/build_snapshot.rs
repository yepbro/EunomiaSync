//! Behavior tests for the build snapshot read (spec §9.1 ALG-001,
//! §5.4 SYS-005; task T-100).
//!
//! The tests run against a real database opened through `open_db` and
//! migrated with `run_migrations` — the production configuration
//! (DB-001 pragmas, D-030) — seeded through the repositories' public
//! API and read back through `build_snapshot::read`:
//!
//! * the snapshot holds exactly ALG-001's four collections in their
//!   spec orders: active projects by `sort_order` (archived projects
//!   and their assignments excluded, DB-014), all library items by
//!   `position` (headings and hidden instructions included), the
//!   active variants of every instruction (soft-deleted ones never
//!   reach a build), and the assignments of the active projects;
//! * the row→domain mapping carries the fields the build needs
//!   verbatim — the canonical root path, the write settings with
//!   every vocabulary member, and the variant text exactly as stored
//!   (the save-time CRLF fold already happened at the write boundary;
//!   ALG-002 normalization is downstream of the snapshot);
//! * the SYS-005 criterion: a library edit committed by a **second
//!   connection** while the read transaction is open does not enter
//!   the snapshot — pinned deterministically (WAL pins a transaction's
//!   view at its first read; the edit commits between two full
//!   snapshot reads inside one still-open transaction), not by racing
//!   a fast reader against a slow writer;
//! * an inconsistent graph (a bare active instruction, DB-010) and a
//!   corrupt vocabulary cell (D-039) surface as typed errors at the
//!   read, not as a torn value.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_core::library::{Instruction as CoreInstruction, LibraryItem as CoreLibraryItem};
use eunomia_core::project::{
    BaseName as CoreBaseName, ExtraFile as CoreExtraFile, ExtraFilePosition as CorePosition,
    ProjectWriteSettings, Scheme as CoreScheme,
};
use eunomia_core::snapshot::BuildSnapshot;
use eunomia_store::build_snapshot::{self, BuildSnapshotError};
use eunomia_store::library::{
    ItemScope, NewAssignment, NewLibraryItem, NewVariant, VariantUpdate, assign, create_item,
    create_variant, get_variant, hide_item, update_variant,
};
use eunomia_store::projects as project_repo;
use eunomia_store::projects::{AddedVia, NewProject, Project};
use eunomia_store::projects::{ListScope, ProjectsError};
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
/// (opened through `open_db`, so the DB-001 pragma set — WAL included
/// — applies) and returns the connection plus the directory guard (the
/// `tests/library.rs` pattern).
fn snapshot_db(label: &str) -> (Connection, TempDir) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-snapshot-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    let mut conn =
        open_db(&dir.join("db").join("eunomiasync.sqlite3")).expect("open_db should succeed");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to a fresh database");
    (conn, TempDir(dir))
}

/// The database file path of a `snapshot_db` fixture (the second
/// connection of the isolation test opens it directly).
fn db_path(dir: &TempDir) -> PathBuf {
    dir.0.join("db").join("eunomiasync.sqlite3")
}

/// A default-shape project input; every call gets a distinct
/// `path_key` (`/repos/repo-{n}`), so a set of these can be created
/// together (the `tests/library.rs` `project_sample` shape).
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

/// Creates instruction `n` with its main variant named `Main` (the
/// LIB-002 create-dialog shape) and returns both rows.
fn seed_instruction(
    conn: &Connection,
    n: usize,
) -> (
    eunomia_store::library::LibraryItem,
    eunomia_store::library::Variant,
) {
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
fn seed_heading(conn: &Connection, n: usize, level: u8) -> eunomia_store::library::LibraryItem {
    create_item(
        conn,
        &NewLibraryItem::Heading {
            title: &format!("Section {n}"),
            level,
        },
    )
    .unwrap_or_else(|err| panic!("seed heading {n} should create, got {err}"))
}

/// Reads the snapshot and panics on failure (the read is the subject
/// of every test here; a failure is the assertion).
fn read_snapshot(conn: &Connection) -> BuildSnapshot {
    build_snapshot::read(conn).unwrap_or_else(|err| panic!("snapshot read failed: {err}"))
}

/// The instruction with this id from the snapshot, panicking when it
/// is absent or is a heading — the shape every assertion below walks.
fn instruction_of<'a>(snapshot: &'a BuildSnapshot, id: &str) -> &'a CoreInstruction {
    for item in snapshot.items() {
        if let CoreLibraryItem::Instruction(instruction) = item
            && instruction.id.as_str() == id
        {
            return instruction;
        }
    }
    panic!("instruction {id} not found in the snapshot");
}

#[test]
fn empty_database_yields_an_empty_snapshot() {
    let (conn, _dir) = snapshot_db("empty");
    let snapshot = read_snapshot(&conn);
    assert!(snapshot.projects().is_empty());
    assert!(snapshot.items().is_empty());
    assert!(snapshot.assignments().is_empty());
}

#[test]
fn snapshot_holds_active_projects_in_sort_order_with_the_write_settings() {
    let (conn, _dir) = snapshot_db("projects");
    // A non-default write-settings carrier: scheme B, CLAUDE.md, both
    // extra files on, one at each position — every vocabulary member
    // the mapping translates.
    let curated = project_repo::create(
        &conn,
        &NewProject {
            path: "/repos/curated",
            path_key: "/repos/curated",
            name: "Curated",
            label: "CUR",
            color: "teal",
            description: "mapped fields",
            added_via: AddedVia::Scan,
            scheme: Scheme::B,
            base_name: BaseName::ClaudeMd,
            extra_local: ExtraFile {
                enabled: true,
                position: Position::Start,
            },
            extra_project: ExtraFile {
                enabled: true,
                position: Position::End,
            },
        },
    )
    .expect("create should succeed");
    let middle = seed_project(&conn, 1);
    let last = seed_project(&conn, 2);
    // Decouple `sort_order` from creation (and name and path) order, so
    // the assertion below can only pass through the real ORDER BY: the
    // full-table permutation puts `last` first (D-093's `reorder`).
    project_repo::reorder(&conn, &[&last.id, &curated.id, &middle.id])
        .expect("reorder should succeed");
    project_repo::archive(&conn, &middle.id).expect("archive should succeed");

    let snapshot = read_snapshot(&conn);

    // Active projects only, in `sort_order` order: the archived middle
    // row keeps its position in the table (D-093) but leaves the
    // snapshot (ALG-001, DB-014) — and the surviving order is the
    // reordered one, not the creation order.
    let ids: Vec<&str> = snapshot.projects().iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec![last.id.as_str(), curated.id.as_str()]);

    let first = &snapshot.projects()[0];
    assert_eq!(first.name, "Project 2");
    assert_eq!(first.root, PathBuf::from("/repos/repo-2"));
    assert_eq!(
        first.settings,
        ProjectWriteSettings {
            scheme: CoreScheme::A,
            base_name: CoreBaseName::AgentsMd,
            extra_local: CoreExtraFile {
                enabled: false,
                position: CorePosition::End,
            },
            extra_project: CoreExtraFile {
                enabled: false,
                position: CorePosition::End,
            },
        }
    );

    // The custom-shaped project maps member-by-member too.
    let second = &snapshot.projects()[1];
    assert_eq!(second.name, "Curated");
    // The canonical root, verbatim as stored (DB-030).
    assert_eq!(second.root, PathBuf::from("/repos/curated"));
    assert_eq!(
        second.settings,
        ProjectWriteSettings {
            scheme: CoreScheme::B,
            base_name: CoreBaseName::ClaudeMd,
            extra_local: CoreExtraFile {
                enabled: true,
                position: CorePosition::Start,
            },
            extra_project: CoreExtraFile {
                enabled: true,
                position: CorePosition::End,
            },
        }
    );
}

#[test]
fn snapshot_holds_all_items_in_position_order_headings_and_hidden() {
    let (conn, _dir) = snapshot_db("items");
    let style = seed_heading(&conn, 1, 2);
    let (first, _) = seed_instruction(&conn, 1);
    let tests = seed_heading(&conn, 2, 3);
    let (second, _) = seed_instruction(&conn, 2);
    let (third, _) = seed_instruction(&conn, 3);
    hide_item(&conn, &third.id).expect("hide should succeed");
    // A middle move renumbers the DB-005 range; the snapshot must
    // carry the resulting order, not creation order.
    eunomia_store::library::move_item(&conn, &tests.id, 4).expect("move should succeed");

    let snapshot = read_snapshot(&conn);

    let ids: Vec<&str> = snapshot
        .items()
        .iter()
        .map(|i| i.item_id().as_str())
        .collect();
    assert_eq!(
        ids,
        vec![
            style.id.as_str(),
            first.id.as_str(),
            second.id.as_str(),
            tests.id.as_str(),
            third.id.as_str()
        ],
        "items follow the current position sequence"
    );

    // The heading carries id, level and title.
    let CoreLibraryItem::Heading(heading) = &snapshot.items()[0] else {
        panic!("the first item must be the heading");
    };
    assert_eq!(heading.title, "Section 1");
    assert_eq!(u8::from(heading.level), 2);

    // The moved heading keeps its level at its new place.
    let CoreLibraryItem::Heading(moved) = &snapshot.items()[3] else {
        panic!("the fourth item must be the moved heading");
    };
    assert_eq!(moved.title, "Section 2");
    assert_eq!(u8::from(moved.level), 3);

    // The hidden instruction stays in the snapshot (ALG-001 reads "all
    // library items"; the build excludes it per ALG-020, DB-013).
    assert!(!instruction_of(&snapshot, &third.id).is_active());
    assert!(instruction_of(&snapshot, &first.id).is_active());
}

#[test]
fn only_active_variants_reach_the_snapshot_in_creation_order() {
    let (conn, _dir) = snapshot_db("variants");
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

    let snapshot = read_snapshot(&conn);
    let instruction = instruction_of(&snapshot, &item.id);
    let names: Vec<&str> = instruction
        .variants
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["Main", "Alt"],
        "creation order within the instruction"
    );
    assert_eq!(instruction.variants[0].id.as_str(), main.id.as_str());

    // Soft-deleting the alternative (its assignment-free delete needs
    // no replacement, but one is named for the shape) removes exactly
    // it from the snapshot; the deleted row stays in the table.
    eunomia_store::library::delete_variant(&conn, &alt.id, Some(&main.id))
        .expect("delete should succeed");
    let snapshot = read_snapshot(&conn);
    let instruction = instruction_of(&snapshot, &item.id);
    let names: Vec<&str> = instruction
        .variants
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["Main"],
        "soft-deleted variants never reach a build"
    );
}

#[test]
fn variant_text_rides_the_snapshot_exactly_as_stored() {
    let (conn, _dir) = snapshot_db("text");
    // The save-time fold (D-076/D-097) turns CRLF into LF at the write
    // boundary; everything else — trailing spaces, blank edge lines —
    // is stored verbatim, and ALG-002 normalization happens downstream
    // of the snapshot. The expected value is the stored cell (read
    // back through the repository), not a re-derived string.
    let item = create_item(
        &conn,
        &NewLibraryItem::Instruction {
            title: "Text carrier",
        },
    )
    .expect("seed item should create");
    let stored = create_variant(
        &conn,
        &NewVariant {
            instruction_id: &item.id,
            name: "Main",
            content: "line one\r\nline two  \n\n",
            is_main: true,
        },
    )
    .expect("create should succeed");

    let snapshot = read_snapshot(&conn);
    let instruction = instruction_of(&snapshot, &item.id);
    assert_eq!(instruction.variants[0].content, stored.content);
    assert_eq!(
        instruction.variants[0].content, "line one\nline two  \n\n",
        "the stored (CRLF-folded) form, edge whitespace untouched"
    );
}

#[test]
fn assignments_are_scoped_to_active_projects() {
    let (conn, _dir) = snapshot_db("assignments");
    let first = seed_project(&conn, 1);
    let doomed = seed_project(&conn, 2);
    let third = seed_project(&conn, 3);
    let (a, a_main) = seed_instruction(&conn, 1);
    let (b, b_main) = seed_instruction(&conn, 2);
    let (c, c_main) = seed_instruction(&conn, 3);
    // Library order is creation order here: A before B before C.
    //
    // The fixture deliberately makes every order disagree: assignments
    // are created C→first, A→first, A→doomed, B→third (not in library
    // order), and the project permutation below puts `third` before
    // `first` in `sort_order`. Only the read's real ORDER BY —
    // `p.sort_order, i.position` — can produce the asserted sequence;
    // creation order, position alone, `sort_order` alone and no
    // ordering at all each produce a different one.
    assign(
        &conn,
        &NewAssignment {
            project_id: &first.id,
            instruction_id: &c.id,
            variant_id: &c_main.id,
        },
    )
    .expect("assign C to first");
    assign(
        &conn,
        &NewAssignment {
            project_id: &first.id,
            instruction_id: &a.id,
            variant_id: &a_main.id,
        },
    )
    .expect("assign A to first");
    assign(
        &conn,
        &NewAssignment {
            project_id: &doomed.id,
            instruction_id: &a.id,
            variant_id: &a_main.id,
        },
    )
    .expect("assign A to the soon-archived project");
    // A hidden instruction keeps its assignment (DB-013) and the
    // snapshot carries it (the build excludes it, D-11).
    assign(
        &conn,
        &NewAssignment {
            project_id: &third.id,
            instruction_id: &b.id,
            variant_id: &b_main.id,
        },
    )
    .expect("assign B to third");
    hide_item(&conn, &b.id).expect("hide should succeed");
    project_repo::reorder(&conn, &[&third.id, &first.id, &doomed.id])
        .expect("reorder should succeed");
    project_repo::archive(&conn, &doomed.id).expect("archive should succeed");

    let snapshot = read_snapshot(&conn);

    // Three assignments, not four: the archived project's row stays in
    // `project_instructions` (DB-014) but is outside the snapshot. The
    // order is the snapshot's reading order: projects by the reordered
    // `sort_order` (third, first), within a project by library
    // position (A before C — the reverse of their assignment creation
    // order).
    let carried: Vec<(&str, &str)> = snapshot
        .assignments()
        .iter()
        .map(|a| (a.project_id.as_str(), a.instruction_id.as_str()))
        .collect();
    assert_eq!(
        carried,
        vec![
            (third.id.as_str(), b.id.as_str()),
            (first.id.as_str(), a.id.as_str()),
            (first.id.as_str(), c.id.as_str()),
        ]
    );
    // The variant ids ride along — the build resolves the text through
    // them.
    assert_eq!(
        snapshot.assignments()[1].variant_id.as_str(),
        a_main.id.as_str()
    );
}

#[test]
fn library_edit_committed_mid_read_stays_out_of_the_snapshot() {
    // The SYS-005 criterion, held honestly: the read transaction is
    // open and has already read, the edit commits through a second
    // connection, and a full snapshot read inside the same transaction
    // still returns the pre-edit value. WAL (DB-001, applied by
    // `open_db`) pins a transaction's view of the database at its
    // first read statement, so this is deterministic — no race between
    // a fast reader and a slow writer.
    let (conn, dir) = snapshot_db("isolation");
    let project = seed_project(&conn, 1);
    let (item, main) = seed_instruction(&conn, 1);
    assign(
        &conn,
        &NewAssignment {
            project_id: &project.id,
            instruction_id: &item.id,
            variant_id: &main.id,
        },
    )
    .expect("assign should succeed");

    // The scenario's precondition: WAL is what lets the writer commit
    // while the reader holds its snapshot (in rollback-journal mode
    // the commit would wait on the reader's lock instead).
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal_mode should be readable");
    assert_eq!(mode.to_ascii_lowercase(), "wal");

    let tx = conn.unchecked_transaction().expect("BEGIN should succeed");
    let first =
        build_snapshot::read(&tx).unwrap_or_else(|err| panic!("first snapshot read failed: {err}"));

    // The concurrent library edit: a second connection, its own
    // transaction scope, committed while the reader's is open. The
    // bare repository call runs in autocommit mode, so the UPDATE is
    // committed when it returns.
    let writer = open_db(&db_path(&dir)).expect("the second connection should open");
    update_variant(
        &writer,
        &main.id,
        &VariantUpdate {
            name: None,
            content: Some("edited mid-read"),
        },
    )
    .expect("the mid-read edit should commit");
    let committed = get_variant(&writer, &main.id)
        .expect("get_variant should succeed")
        .expect("the variant should exist");
    assert_eq!(committed.content, "edited mid-read");

    // The criterion: the still-open read transaction does not see the
    // committed edit. Equality of the two snapshots covers every
    // collection; the content assertion names the edited fact.
    let second = build_snapshot::read(&tx)
        .unwrap_or_else(|err| panic!("second snapshot read failed: {err}"));
    assert_eq!(
        second, first,
        "the mid-read edit must not enter the snapshot"
    );
    assert_eq!(
        instruction_of(&second, &item.id).variants[0].content,
        "content 1",
        "the pre-edit text is what the in-flight snapshot holds"
    );

    // Ending the read transaction releases the snapshot: a fresh read
    // sees the edit — the proof the edit was really committed and the
    // assertions above were not vacuous.
    tx.rollback().expect("ROLLBACK should succeed");
    let after = read_snapshot(&conn);
    assert_eq!(
        instruction_of(&after, &item.id).variants[0].content,
        "edited mid-read",
        "after the read transaction ends the edit is visible"
    );
}

#[test]
fn a_bare_active_instruction_fails_the_read_as_no_active_variants() {
    // DB-010's constructive rule lives in the library repository; this
    // read's job is to surface a committed bare instruction (a
    // producer bug or out-of-band data) as core's named cause at sync
    // start, not to hand the build a torn value.
    let (conn, _dir) = snapshot_db("bare-instruction");
    let item = create_item(
        &conn,
        &NewLibraryItem::Instruction {
            title: "Bare instruction",
        },
    )
    .expect("create_item alone should succeed (the DB-010 window)");

    let err = build_snapshot::read(&conn)
        .expect_err("a bare active instruction must fail the snapshot read");
    assert!(
        matches!(
            &err,
            BuildSnapshotError::Invalid(eunomia_core::snapshot::SnapshotError::NoActiveVariants {
                item_id
            }) if item_id.as_str() == item.id
        ),
        "expected Invalid(NoActiveVariants) for {}, got {err:?}",
        item.id
    );
}

#[test]
fn a_corrupt_vocabulary_cell_surfaces_through_the_projects_error() {
    // D-039's corrupt-cells policy, propagated: the repository's
    // `Member` error arrives wrapped, with the column and the
    // offending value intact. The corrupted column is `scheme` — a
    // cell the active-scoped read still selects and decodes. (A
    // corrupt `state` cell cannot surface here at all: the
    // `WHERE state = 'active'` filter of the Active scope drops the
    // row before any decode, so the project silently leaves the
    // snapshot — the D-093 scope semantics, recorded with this task.)
    let (conn, _dir) = snapshot_db("corrupt-cell");
    let project = seed_project(&conn, 1);
    conn.pragma_update(None, "ignore_check_constraints", true)
        .expect("the corruption seam should enable");
    conn.execute(
        "UPDATE projects SET scheme = 'D' WHERE id = ?1",
        [&project.id],
    )
    .expect("the corrupting update should run");
    conn.pragma_update(None, "ignore_check_constraints", false)
        .expect("the corruption seam should disable");

    let err =
        build_snapshot::read(&conn).expect_err("a corrupt scheme cell must fail the snapshot read");
    assert!(
        matches!(
            &err,
            BuildSnapshotError::Projects(ProjectsError::Member {
                column: "scheme",
                found
            }) if found == "D"
        ),
        "expected Projects(Member) with scheme/D, got {err:?}"
    );
}

#[test]
fn read_runs_against_the_repositories_it_composes() {
    // A cross-check against the repositories' own reads (not part of
    // the criterion set): the snapshot's project set equals the
    // projects repository's active list and its item list equals the
    // library repository's all-items list, so a filter drift in either
    // direction fails here. The library side needs real rows — a
    // heading, a visible instruction and a hidden one — or the item
    // comparison would be `[] == []`.
    let (conn, _dir) = snapshot_db("cross-check");
    seed_project(&conn, 1);
    let archived = seed_project(&conn, 2);
    seed_project(&conn, 3);
    project_repo::archive(&conn, &archived.id).expect("archive should succeed");
    seed_heading(&conn, 1, 2);
    let (visible, _) = seed_instruction(&conn, 1);
    let (hidden, _) = seed_instruction(&conn, 2);
    hide_item(&conn, &hidden.id).expect("hide should succeed");

    let snapshot = read_snapshot(&conn);
    let listed = project_repo::list(&conn, ListScope::Active).expect("list should succeed");
    let snapshot_ids: Vec<String> = snapshot
        .projects()
        .iter()
        .map(|p| p.id.as_str().to_owned())
        .collect();
    let listed_ids: Vec<String> = listed.into_iter().map(|p| p.id).collect();
    assert_eq!(snapshot_ids, listed_ids);

    let all_items = eunomia_store::library::list_items(&conn, ItemScope::All)
        .expect("list_items should succeed");
    let snapshot_items: Vec<String> = snapshot
        .items()
        .iter()
        .map(|i| i.item_id().as_str().to_owned())
        .collect();
    let listed_items: Vec<String> = all_items.into_iter().map(|i| i.id).collect();
    assert_eq!(snapshot_items, listed_items);
    assert_eq!(snapshot_items.len(), 3);
    assert!(snapshot_items.contains(&visible.id));
    assert!(snapshot_items.contains(&hidden.id));
}
