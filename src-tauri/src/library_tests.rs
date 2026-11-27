//! IPC round-trip tests for the `library_get` command (T-083, §7.10
//! Library module; §7.3 LIB-001/008/014, spec-table D-14, LIB-032,
//! D-187) — the mock-runtime harness the projects commands use
//! (`tauri::test::get_ipc_response`, no windowing system), with the
//! world seeded through the T-081 repository over the `Db::run`
//! executor a producing command takes.
//!
//! The battery pins the card's contract:
//!
//! - the tree is one flat vector in `position` order (the order the
//!   §7.3 library and the built file share, OUT-001), every
//!   instruction carrying the LIB-001/008 field set (name, variant
//!   count, creation and change dates, state) plus its active
//!   variants inline — each with the LIB-014 emptiness flag and the
//!   DB-010 main flag — and every heading carrying its level and the
//!   LIB-032 skip flag;
//! - the card's acceptance criterion: an instruction row's project
//!   labels are EXACTLY the active projects with an assignment on
//!   that instruction, each label naming the assigned variant — an
//!   archived project's kept assignment rows (DB-014) never label,
//!   and the label order follows the projects' `sort_order`;
//! - the skip flag marks only the heading that ARRIVES too deep, is
//!   computed over the served collection, and is scope-invariant
//!   (hidden instructions are transparent to the pair walk — the
//!   `gaps` module's own rule);
//! - soft-deleted variants stay out of the count and the list;
//! - the boundaries: a fresh database answers an empty list, the
//!   no-database state answers the honest `NoDatabase` fallback, and
//!   a corrupt vocabulary cell answers the D-037 fallback envelope
//!   with a class-only diagnostic (never the stored value);
//! - a large library (hundreds of items, thousands of rows) is
//!   served by the one read in position order — the untrusted-input
//!   complexity pin (rule 6: the passes are linear in the rows read);
//! - the registration pin (`lib.rs` registers the §7.10 name exactly
//!   once — the D-034 wiring-pin rationale).

use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};

use crate::commands::LibraryInstructionEntry;
use crate::commands::LibraryTreeItem;
use crate::data_dirs;

/// The healthy-app harness: a real `Db` over a temp root, the one
/// command registered, no watch/availability services (a pure read
/// consults none of them).
fn library_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::library_get])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    (temp, app)
}

/// One invoke request for `cmd` with a JSON body (the harness shape
/// `ipc_tests.rs` uses).
fn invoke_request(cmd: &str, body: serde_json::Value) -> tauri::webview::InvokeRequest {
    tauri::webview::InvokeRequest {
        cmd: cmd.into(),
        callback: tauri::ipc::CallbackFn(0),
        error: tauri::ipc::CallbackFn(1),
        url: if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost".parse().unwrap()
        } else {
            "tauri://localhost".parse().unwrap()
        },
        body: tauri::ipc::InvokeBody::Json(body),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.to_string(),
    }
}

/// The mock `main` webview window the requests are delivered through.
fn main_webview(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("failed to create the mock `main` webview window")
}

/// Invokes `library_get` with the given `includeHidden` wire value
/// (`null` omitted is the default active-only read), deserializing
/// the success payload into the typed tree.
fn invoke_library_get(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    include_hidden: serde_json::Value,
) -> Result<crate::commands::LibraryGetResult, serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request(
            "library_get",
            serde_json::json!({ "includeHidden": include_hidden }),
        ),
    );
    match response {
        Ok(payload) => Ok(payload
            .deserialize::<crate::commands::LibraryGetResult>()
            .expect("the success payload must deserialize into the answer DTO")),
        Err(error) => Err(error),
    }
}

// ---- seeding helpers (the repositories a producing command takes) ---------------

/// Seeds one project row with the default write configuration and
/// returns its id (the `ipc_tests` `column_project` shape; the path
/// needs no real folder — `library_get` probes nothing).
fn seed_project(
    database: &crate::db::Db,
    path: String,
    path_key: &'static str,
    name: &'static str,
    label: &'static str,
    color: &'static str,
) -> String {
    use eunomia_store::projects::AddedVia;
    use eunomia_store::projects::NewProject;
    use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let new = NewProject {
            path: &path,
            path_key,
            name,
            label,
            color,
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
        };
        eunomia_store::projects::create(tx, &new)
            .map(|project| project.id)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work")
}

/// Appends one heading (LIB-030) and returns its id.
fn seed_heading(database: &crate::db::Db, title: &'static str, level: u8) -> String {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::create_item(
            tx,
            &eunomia_store::library::NewLibraryItem::Heading { title, level },
        )
        .map(|item| item.id)
        .map_err(crate::commands::library_store_to_app_error)
    }))
    .expect("seeding a heading through the repository must work")
}

/// Appends one instruction WITH its main variant (the DB-010 pair the
/// §7.3 create dialog produces) and returns the item's id.
fn seed_instruction(database: &crate::db::Db, title: &'static str) -> String {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let item = eunomia_store::library::create_item(
            tx,
            &eunomia_store::library::NewLibraryItem::Instruction { title },
        )
        .map_err(crate::commands::library_store_to_app_error)?;
        eunomia_store::library::create_variant(
            tx,
            &eunomia_store::library::NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content: "Be brief.",
                is_main: true,
            },
        )
        .map_err(crate::commands::library_store_to_app_error)?;
        Ok(item.id)
    }))
    .expect("seeding an instruction with its main variant must work")
}

/// Appends one non-main variant to an instruction and returns its id.
fn seed_variant(
    database: &crate::db::Db,
    instruction_id: String,
    name: &'static str,
    content: &'static str,
) -> String {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::create_variant(
            tx,
            &eunomia_store::library::NewVariant {
                instruction_id: &instruction_id,
                name,
                content,
                is_main: false,
            },
        )
        .map(|variant| variant.id)
        .map_err(crate::commands::library_store_to_app_error)
    }))
    .expect("seeding a variant through the repository must work")
}

/// Assigns one variant of one instruction to one project.
fn seed_assignment(
    database: &crate::db::Db,
    project_id: String,
    instruction_id: String,
    variant_id: String,
) {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::assign(
            tx,
            &eunomia_store::library::NewAssignment {
                project_id: &project_id,
                instruction_id: &instruction_id,
                variant_id: &variant_id,
            },
        )
        .map(|_| ())
        .map_err(crate::commands::library_store_to_app_error)
    }))
    .expect("seeding an assignment through the repository must work");
}

// ---- assertion helpers -----------------------------------------------------------

/// The instruction half of an entry, or a panic naming the violation.
fn instruction_of(entry: &LibraryTreeItem) -> &LibraryInstructionEntry {
    match entry {
        LibraryTreeItem::Instruction(instruction) => instruction,
        LibraryTreeItem::Heading(heading) => {
            panic!(
                "expected an instruction entry, found the heading {}",
                heading.title
            )
        }
    }
}

/// The heading half of an entry, or a panic naming the violation.
fn heading_of(entry: &LibraryTreeItem) -> &crate::commands::LibraryHeadingEntry {
    match entry {
        LibraryTreeItem::Heading(heading) => heading,
        LibraryTreeItem::Instruction(instruction) => panic!(
            "expected a heading entry, found the instruction {}",
            instruction.title
        ),
    }
}

/// The instruction entry with exactly this title, or a panic.
fn instruction_by_title<'a>(
    tree: &'a [LibraryTreeItem],
    title: &str,
) -> &'a LibraryInstructionEntry {
    tree.iter()
        .filter_map(|entry| match entry {
            LibraryTreeItem::Instruction(instruction) => Some(instruction),
            LibraryTreeItem::Heading(_) => None,
        })
        .find(|instruction| instruction.title == title)
        .unwrap_or_else(|| panic!("no instruction titled {title} in the tree"))
}

/// The stored row of an item, read through the store's own by-id read
/// (an oracle independent of the command's list query).
fn stored_item(database: &crate::db::Db, id: &str) -> eunomia_store::library::LibraryItem {
    let id = id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::get_item(tx, &id)
            .map_err(crate::commands::library_store_to_app_error)
    }))
    .expect("reading the stored item through the executor must work")
    .expect("the seeded item must be readable")
}

// ---- the battery ------------------------------------------------------------------

#[test]
fn library_get_serves_the_position_ordered_tree_with_the_full_instruction_shape() {
    // Arrange: H2 "Style", an instruction with a filled main variant
    // and an empty second variant, H4 "Tone", one more instruction —
    // the §7.3 shape with every field the card names.
    let (temp, app) = library_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let style = seed_heading(&database, "Style", 2);
    let concise = seed_instruction(&database, "Be concise");
    let empty_one = seed_variant(&database, concise.clone(), "Empty one", "");
    let blank = seed_variant(&database, concise.clone(), "Blank", " \n\t ");
    let tone = seed_heading(&database, "Tone", 4);
    let friendly = seed_instruction(&database, "Be friendly");

    // Act: the default read (includeHidden omitted).
    let webview = main_webview(&app);
    let tree = invoke_library_get(&webview, serde_json::Value::Null)
        .expect("the default read must succeed")
        .tree;

    // Assert: position order — one flat vector, headings and
    // instructions interleaved the way they were appended.
    assert_eq!(tree.len(), 4, "every item is served");
    assert_eq!(heading_of(&tree[0]).id, style);
    assert_eq!(instruction_of(&tree[1]).id, concise);
    assert_eq!(heading_of(&tree[2]).id, tone);
    assert_eq!(instruction_of(&tree[3]).id, friendly);

    // The heading half of the shape: level, title, and the skip facts
    // (H2 → H4 across one transparent instruction is a skip — the
    // dedicated battery below owns the rule; here it pins the fields):
    // the boolean AND the skipped-from level (D-199) — the flag says
    // "a skip", the level says WHICH level the skip leaves behind.
    let style_entry = heading_of(&tree[0]);
    assert_eq!(style_entry.title, "Style");
    assert_eq!(style_entry.level, 2);
    assert!(!style_entry.level_skipped, "the first heading has no pair");
    assert_eq!(
        style_entry.from_level, None,
        "no pair, no skipped-from level"
    );
    let tone_entry = heading_of(&tree[2]);
    assert_eq!(tone_entry.title, "Tone");
    assert_eq!(tone_entry.level, 4);
    assert!(tone_entry.level_skipped, "H4 arriving after H2 is a skip");
    assert_eq!(
        tone_entry.from_level,
        Some(2),
        "the skip's other half: the H2 it arrived from"
    );

    // The instruction half (LIB-001/008/014): name, state, variant
    // count, dates, the variants inline with both flags, no labels
    // (nothing is assigned in this world). The main variant's id is
    // the stored row's own (the store's by-instruction read is the
    // independent oracle).
    let stored_variants = {
        let database = database.clone();
        let concise_for_read = concise.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::library::list_variants(tx, &concise_for_read)
                .map(|variants| {
                    variants
                        .into_iter()
                        .map(|variant| variant.id)
                        .collect::<Vec<_>>()
                })
                .map_err(crate::commands::library_store_to_app_error)
        }))
        .expect("reading the stored variants through the executor must work")
    };
    let entry = instruction_of(&tree[1]);
    assert_eq!(entry.title, "Be concise");
    assert_eq!(entry.state, "active");
    assert_eq!(entry.variant_count, 3);
    assert!(entry.project_labels.is_empty());
    assert_eq!(entry.variants.len(), 3);
    assert_eq!(entry.variants[0].id, stored_variants[0]);
    assert_eq!(entry.variants[0].name, "Main");
    assert!(entry.variants[0].is_main);
    assert!(!entry.variants[0].is_empty, "the main variant carries text");
    assert_eq!(entry.variants[1].id, empty_one);
    assert_eq!(entry.variants[1].name, "Empty one");
    assert!(!entry.variants[1].is_main);
    assert!(
        entry.variants[1].is_empty,
        "LIB-014: an empty variant is marked"
    );
    // D-13's reading (round-1 P2-2): a whitespace-only stored body
    // OUTPUTS no text after ALG-002's normalization, so it is an
    // empty variant exactly as it is an empty block in a build.
    assert_eq!(entry.variants[2].id, blank);
    assert_eq!(entry.variants[2].name, "Blank");
    assert!(!entry.variants[2].is_main);
    assert!(
        entry.variants[2].is_empty,
        "a whitespace-only variant outputs no text (D-13)"
    );

    // The dates are the row's own DB-002 values (the store's by-id
    // read is the independent oracle), in the fixed shape — checked
    // by the store's own public helper (round-1 P3-4: the shipped
    // oracle, not a hand-rolled weaker twin).
    let row = stored_item(&database, &concise);
    assert_eq!(entry.created_at, row.created_at);
    assert_eq!(entry.updated_at, row.updated_at);
    assert!(eunomia_store::values::is_db002_shape(&entry.created_at));
    assert!(eunomia_store::values::is_db002_shape(&entry.updated_at));

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn library_get_labels_match_the_assignments() {
    // The card's acceptance criterion: an instruction's project labels
    // are exactly the ACTIVE projects with an assignment on it — each
    // label naming the assigned variant — in the projects' own
    // `sort_order`, and an archived project's kept assignment rows
    // (DB-014) never label.
    let (temp, app) = library_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let root = temp.path().join("repo").display().to_string();
    let alpha = seed_project(
        &database,
        format!("{root}/alpha"),
        "/alpha",
        "Alpha",
        "ALP",
        "red",
    );
    let beta = seed_project(
        &database,
        format!("{root}/beta"),
        "/beta",
        "Beta",
        "BET",
        "green",
    );
    let archived = seed_project(
        &database,
        format!("{root}/gone"),
        "/gone",
        "Gone",
        "GON",
        "amber",
    );
    // Column order becomes Beta, Alpha, Gone — labels must follow the
    // projects' sort_order, not the seeding order.
    let order = [beta.clone(), alpha.clone(), archived.clone()];
    let database_for_reorder = database.clone();
    tauri::async_runtime::block_on(database_for_reorder.run(move |tx| {
        eunomia_store::projects::reorder(tx, &order.iter().map(String::as_str).collect::<Vec<_>>())
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("reordering the projects through the repository must work");
    let archived_for_archive = archived.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::archive(tx, &archived_for_archive)
            .map(|_| ())
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("archiving the project through the repository must work");

    // One instruction assigned to a single project through its EMPTY
    // second variant (LIB-015: an empty variant is assignable); one
    // assigned to both active projects and the archived one; one with
    // no assignment at all.
    let terse = seed_instruction(&database, "Assigned terse");
    let terse_empty = seed_variant(&database, terse.clone(), "Terse", "");
    let wide = seed_instruction(&database, "Assigned wide");
    seed_instruction(&database, "Orphan");
    seed_assignment(&database, alpha.clone(), terse.clone(), terse_empty.clone());
    let wide_main = {
        // The main variant's id, read through the store.
        let database = database.clone();
        let wide_for_read = wide.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::library::list_variants(tx, &wide_for_read)
                .map(|variants| {
                    variants
                        .iter()
                        .find(|variant| variant.is_main)
                        .expect("the seeded instruction has a main variant")
                        .id
                        .clone()
                })
                .map_err(crate::commands::library_store_to_app_error)
        }))
        .expect("reading the main variant through the executor must work")
    };
    seed_assignment(&database, alpha.clone(), wide.clone(), wide_main.clone());
    seed_assignment(&database, beta.clone(), wide.clone(), wide_main.clone());
    // The archived project's row (DB-014: archive keeps assignments)
    // — the same variant is fine; the row must simply never label.
    seed_assignment(&database, archived.clone(), wide.clone(), wide_main);

    // Act
    let webview = main_webview(&app);
    let tree = invoke_library_get(&webview, serde_json::Value::Null)
        .expect("the default read must succeed")
        .tree;

    // Assert: "Assigned terse" carries exactly Alpha's chip naming the
    // empty variant it was assigned through.
    let terse_entry = instruction_by_title(&tree, "Assigned terse");
    assert_eq!(
        terse_entry.project_labels,
        vec![crate::commands::ProjectLabelEntry {
            project_id: alpha.clone(),
            name: "Alpha".to_owned(),
            label: "ALP".to_owned(),
            color: "red".to_owned(),
            variant_id: Some(terse_empty),
        }],
        "labels are exactly the active projects with an assignment, naming the assigned variant"
    );

    // "Assigned wide" carries both ACTIVE chips in sort_order
    // (Beta first after the reorder); the archived project's kept
    // assignment row (DB-014) labels nothing.
    let wide_entry = instruction_by_title(&tree, "Assigned wide");
    assert_eq!(
        wide_entry
            .project_labels
            .iter()
            .map(|label| label.project_id.as_str())
            .collect::<Vec<_>>(),
        [beta.as_str(), alpha.as_str()],
        "labels follow the projects' sort_order, never the seeding order"
    );
    assert!(
        wide_entry
            .project_labels
            .iter()
            .all(|label| label.project_id != archived),
        "an archived project's assignment rows never label (DB-014)"
    );
    assert_eq!(wide_entry.project_labels[0].name, "Beta");
    assert_eq!(wide_entry.project_labels[0].label, "BET");
    assert_eq!(wide_entry.project_labels[0].color, "green");
    assert_eq!(wide_entry.project_labels[1].label, "ALP");

    // "Orphan" carries no labels at all.
    let orphan_entry = instruction_by_title(&tree, "Orphan");
    assert!(
        orphan_entry.project_labels.is_empty(),
        "an instruction with no assignment carries no labels"
    );

    drop(app);
    drop(temp);
}

#[test]
fn library_get_labels_a_custom_assignment_without_inventing_a_variant_id() {
    let (temp, app) = library_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let root = temp.path().join("repo").display().to_string();
    let project = seed_project(
        &database,
        format!("{root}/custom"),
        "/custom",
        "Custom project",
        "CUSTOM",
        "blue",
    );
    let instruction = seed_instruction(&database, "Custom target");
    let database_for_custom = database.clone();
    let project_for_custom = project.clone();
    let instruction_for_custom = instruction.clone();
    tauri::async_runtime::block_on(database_for_custom.run(move |tx| {
        eunomia_store::library::set_custom(
            tx,
            &project_for_custom,
            &instruction_for_custom,
            Some("per-project text"),
            eunomia_store::library::CustomMode::Replace,
        )
        .map(|_| ())
        .map_err(crate::commands::library_store_to_app_error)
    }))
    .expect("custom assignment seed succeeds");

    let webview = main_webview(&app);
    let tree = invoke_library_get(&webview, serde_json::Value::Null)
        .expect("library read succeeds")
        .tree;
    let entry = instruction_by_title(&tree, "Custom target");
    assert_eq!(entry.project_labels.len(), 1);
    assert_eq!(entry.project_labels[0].project_id, project);
    assert_eq!(entry.project_labels[0].variant_id, None);

    // MAIN-034: an extend cell's label carries its BASE variant id
    // ("Labels match the assignments", LIB-001): the assignment names
    // the base beside the supplement. The base is a NON-main variant so
    // the assertion distinguishes the base from the main (a mapping to
    // the active main would fail it).
    let alt = seed_variant(&database, instruction.clone(), "Alt", "alt shared text");
    let database_for_assign = database.clone();
    let project_for_assign = project.clone();
    let instruction_for_assign = instruction.clone();
    let alt_for_assign = alt.clone();
    tauri::async_runtime::block_on(database_for_assign.run(move |tx| {
        // reassign converts the replace cell to the alt variant (the
        // explicit-variant conversion), so the extend write below keeps
        // the alt as its base.
        eunomia_store::library::reassign(
            tx,
            &project_for_assign,
            &instruction_for_assign,
            &alt_for_assign,
        )
        .map_err(crate::commands::library_store_to_app_error)
        .map(|_| ())
    }))
    .expect("alt reassignment seed succeeds");
    let database_for_extend = database.clone();
    let project_for_extend = project.clone();
    let instruction_for_extend = instruction.clone();
    tauri::async_runtime::block_on(database_for_extend.run(move |tx| {
        eunomia_store::library::set_custom(
            tx,
            &project_for_extend,
            &instruction_for_extend,
            Some("supplement"),
            eunomia_store::library::CustomMode::Extend,
        )
        .map(|_| ())
        .map_err(crate::commands::library_store_to_app_error)
    }))
    .expect("extend assignment seed succeeds");

    let tree = invoke_library_get(&webview, serde_json::Value::Null)
        .expect("library read succeeds")
        .tree;
    let entry = instruction_by_title(&tree, "Custom target");
    assert_eq!(entry.project_labels.len(), 1);
    assert_eq!(entry.project_labels[0].project_id, project);
    assert_eq!(entry.project_labels[0].variant_id, Some(alt));
}

#[test]
fn library_get_marks_level_skips_on_the_arriving_headings_only() {
    // LIB-032 over the `gaps` rule: the flag marks the heading that
    // ARRIVES more than one level deeper than the heading before it —
    // a close, a sibling or a single step down is not a skip, the
    // first heading has no pair, and a hidden instruction between two
    // headings cannot interrupt one (transparent by the module's own
    // rule). The flag set must be identical under either scope: the
    // served collection is what the indices address, and the heading
    // PAIRS do not change with the scope.
    let (temp, app) = library_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    seed_heading(&database, "Style", 2); // 0 — first, no pair
    let shy = seed_instruction(&database, "Shy"); // transparent, hidden below
    seed_heading(&database, "Tone", 4); // 2 — ARRIVES two deeper: skip
    seed_heading(&database, "Deep", 5); // 3 — one step: no skip
    seed_heading(&database, "Release", 3); // 4 — close: no skip
    seed_heading(&database, "Max", 6); // 5 — ARRIVES three deeper: skip (from H3)
    seed_heading(&database, "Back", 3); // 6 — close: no skip
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::hide_item(tx, &shy)
            .map(|_| ())
            .map_err(crate::commands::library_store_to_app_error)
    }))
    .expect("hiding the instruction through the repository must work");

    let expected = [false, true, false, false, true, false];
    // D-199: the second half of every pair — the skipped-from level.
    // The skips' pairs are Tone←Style (H2) and Max←Release (H3) — two
    // DIFFERENT skipped-from values, so a constant or first-gap
    // implementation cannot pass; every non-skip row carries no level
    // at all.
    let expected_from = [None, Some(2u8), None, None, Some(3u8), None];
    let webview = main_webview(&app);
    for scope in [serde_json::Value::Null, serde_json::json!(true)] {
        let tree = invoke_library_get(&webview, scope.clone())
            .expect("the read must succeed under either scope")
            .tree;
        let flags = tree
            .iter()
            .filter_map(|entry| match entry {
                LibraryTreeItem::Heading(heading) => Some(heading.level_skipped),
                LibraryTreeItem::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            flags, expected,
            "the skip flag set is the same under includeHidden={scope}"
        );
        let from_levels = tree
            .iter()
            .filter_map(|entry| match entry {
                LibraryTreeItem::Heading(heading) => Some(heading.from_level),
                LibraryTreeItem::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            from_levels, expected_from,
            "the skipped-from level rides exactly the skip rows, under includeHidden={scope}"
        );
        let titles = tree
            .iter()
            .filter_map(|entry| match entry {
                LibraryTreeItem::Heading(heading) => Some(heading.title.as_str()),
                LibraryTreeItem::Instruction(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            titles,
            ["Style", "Tone", "Deep", "Release", "Max", "Back"],
            "the six headings keep their position order"
        );
    }

    // The widened read also serves the hidden instruction itself
    // (the default read's absence is the next test's own subject).
    let widened = invoke_library_get(&webview, serde_json::json!(true))
        .expect("the widened read must succeed")
        .tree;
    assert_eq!(widened.len(), 7);
    assert_eq!(instruction_by_title(&widened, "Shy").state, "hidden");

    drop(app);
    drop(temp);
}

#[test]
fn library_get_counts_only_active_variants() {
    // A soft-deleted variant (§6.4) leaves both the count and the
    // served list — the tree's variants are the ACTIVE ones
    // (DB-010's "active variant" reading, the snapshot's own scope).
    let (temp, app) = library_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let two_ways = seed_instruction(&database, "Two ways");
    let extra = seed_variant(&database, two_ways.clone(), "Extra", "More text.");
    seed_instruction(&database, "Solo");
    let extra_for_delete = extra.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::delete_variant(tx, &extra_for_delete, None)
            .map(|_| ())
            .map_err(crate::commands::library_store_to_app_error)
    }))
    .expect("soft-deleting the variant through the repository must work");

    let webview = main_webview(&app);
    let tree = invoke_library_get(&webview, serde_json::Value::Null)
        .expect("the read must succeed")
        .tree;

    let two_ways_entry = instruction_by_title(&tree, "Two ways");
    assert_eq!(
        two_ways_entry.variant_count, 1,
        "the deleted variant is not counted"
    );
    assert_eq!(two_ways_entry.variants.len(), 1);
    assert_eq!(two_ways_entry.variants[0].name, "Main");
    assert!(
        two_ways_entry
            .variants
            .iter()
            .all(|variant| variant.id != extra),
        "the soft-deleted variant's id is not served"
    );
    let solo_entry = instruction_by_title(&tree, "Solo");
    assert_eq!(solo_entry.variant_count, 1);
    assert_eq!(solo_entry.variants.len(), 1);

    drop(app);
    drop(temp);
}

#[test]
fn library_get_hides_hidden_instructions_behind_the_flag_and_keeps_their_labels() {
    // D-14's "Show hidden" half: the default read serves the visible
    // library; the explicit flag widens it; a hidden instruction
    // keeps its assignments (DB-013) and therefore its labels, with
    // the state spelled for the dashed chip (MAIN-013).
    let (temp, app) = library_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let root = temp.path().join("repo").display().to_string();
    let alpha = seed_project(
        &database,
        format!("{root}/alpha"),
        "/alpha",
        "Alpha",
        "ALP",
        "red",
    );
    let keeper = seed_instruction(&database, "Keeper");
    seed_instruction(&database, "Dropper");
    // The keeper's assignment: the main variant.
    let keeper_main = {
        let database = database.clone();
        let keeper_for_read = keeper.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::library::list_variants(tx, &keeper_for_read)
                .map(|variants| variants[0].id.clone())
                .map_err(crate::commands::library_store_to_app_error)
        }))
        .expect("reading the main variant through the executor must work")
    };
    seed_assignment(&database, alpha.clone(), keeper.clone(), keeper_main);
    let keeper_for_hide = keeper.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::hide_item(tx, &keeper_for_hide)
            .map(|_| ())
            .map_err(crate::commands::library_store_to_app_error)
    }))
    .expect("hiding the instruction through the repository must work");

    // Default: only the visible instruction is served — both the
    // omitted flag and the EXPLICIT `false` spell the default scope
    // (the `projects_list` reading, pinned on both spellings).
    let webview = main_webview(&app);
    let visible = invoke_library_get(&webview, serde_json::Value::Null)
        .expect("the default read must succeed")
        .tree;
    assert_eq!(
        visible.len(),
        1,
        "the hidden instruction is absent by default"
    );
    assert_eq!(instruction_of(&visible[0]).title, "Dropper");
    let explicitly_visible = invoke_library_get(&webview, serde_json::json!(false))
        .expect("the explicit-false read must succeed")
        .tree;
    assert_eq!(
        explicitly_visible, visible,
        "an explicit includeHidden=false is the default scope"
    );

    // Widened: both, the hidden one marked, its labels kept (DB-013).
    let widened = invoke_library_get(&webview, serde_json::json!(true))
        .expect("the widened read must succeed")
        .tree;
    assert_eq!(widened.len(), 2);
    let keeper_entry = instruction_by_title(&widened, "Keeper");
    assert_eq!(keeper_entry.state, "hidden");
    assert_eq!(
        keeper_entry
            .project_labels
            .iter()
            .map(|label| label.project_id.as_str())
            .collect::<Vec<_>>(),
        [alpha.as_str()],
        "a hidden instruction keeps its assignments and its labels (DB-013)"
    );

    drop(app);
    drop(temp);
}

#[test]
fn library_get_answers_an_empty_tree_for_a_fresh_database() {
    // The empty library's honest answer: an empty tree with a zero
    // count, not an error and not a null payload the frontend has to
    // special-case.
    let (temp, app) = library_app();
    let webview = main_webview(&app);
    let answer = invoke_library_get(&webview, serde_json::Value::Null)
        .expect("a fresh database must answer successfully");
    assert_eq!(answer.tree, Vec::new());
    assert_eq!(answer.hidden_count, 0, "nothing is hidden either");
    drop(app);
    drop(temp);
}

#[test]
fn library_get_answers_honestly_without_a_database() {
    // Recovery mode (the `projects_list` pin): no `Db` managed — the
    // recorded D-037 fallback with the `NoDatabase` diagnostic, never
    // a pretend-empty tree that would lie about the library tables
    // being empty.
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::library_get])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    let webview = main_webview(&app);
    let error = invoke_library_get(&webview, serde_json::Value::Null)
        .expect_err("the no-database state must fail honestly");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["message_key"], serde_json::json!("errors.db_locked"));
    assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
}

#[test]
fn a_corrupt_library_cell_answers_the_fallback_envelope_not_validation() {
    // The corrupt-cells wire contract (the T-027 round-2 rule, applied
    // to the library tables): a stored vocabulary cell outside its
    // CHECK vocabulary is corruption, NOT caller input — the read
    // fails with the D-037 fallback, a `LibraryCorrupt` diagnostic,
    // empty params, and a class-only `os_message` (a tampered cell is
    // arbitrary database text; `params` and `os_message` feed the log
    // line and the localized message — rule 8). Written through the
    // `ignore_check_constraints` seam — the schema CHECKs make such a
    // cell unreachable through the API.
    let (temp, app) = library_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    seed_instruction(&database, "Poisoned");
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "PRAGMA ignore_check_constraints = 1;\n\
             UPDATE library_items SET kind = 'chapter';\n\
             PRAGMA ignore_check_constraints = 0;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("tampering the kind cell through the executor must work");

    // Act: the default read already decodes `kind` (no state filter
    // shields the column).
    let webview = main_webview(&app);
    let error = invoke_library_get(&webview, serde_json::Value::Null)
        .expect_err("a corrupt cell must fail the read");

    // Assert
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["message_key"], serde_json::json!("errors.db_locked"));
    assert_eq!(
        error["params"],
        serde_json::json!({}),
        "no raw cell travels"
    );
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("LibraryCorrupt")
    );
    assert_eq!(
        error["details"]["os_message"],
        serde_json::json!("a stored kind cell is outside its vocabulary")
    );

    drop(app);
    drop(temp);
}

#[test]
fn the_library_get_command_is_registered_with_the_specta_builder() {
    // The §7.10 Library-module name, registered exactly once — the
    // D-034 wiring-pin rationale: without this pin, dropping the
    // registration would leave `export_bindings_are_up_to_date` green
    // while it silently rewrote the committed bindings without the
    // command.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("commands::library_get").count(),
        1,
        "lib.rs must register the `library_get` command exactly once (§7.10)"
    );
}

#[test]
fn a_large_library_is_served_by_one_read_in_position_order() {
    // The untrusted-input complexity pin (rule 6): the tree is served
    // by ONE transaction whose passes are linear in the rows read —
    // 660 items, 1 320 variants and 1 200 assignments over four
    // projects, with the store's own whole-table read as the
    // independent order oracle (the ALG-001 read, a different code
    // path from the command).
    let (temp, app) = library_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let root = temp.path().join("repo").display().to_string();
    let projects: Vec<String> = (0..4)
        .map(|index| {
            seed_project(
                &database,
                format!("{root}/p{index}"),
                Box::leak(format!("/p{index}").into_boxed_str()),
                Box::leak(format!("Project {index}").into_boxed_str()),
                Box::leak(format!("P{index:03}").into_boxed_str()),
                "blue",
            )
        })
        .collect();
    let instructions = 600;
    let headings = 60;
    for index in 0..instructions {
        if index % (instructions / headings) == 0 {
            let level = 2 + (index % 5) as u8;
            let title = Box::leak(format!("Heading {index:04}").into_boxed_str());
            seed_heading(&database, title, level);
        }
        let title = Box::leak(format!("Instruction {index:04}").into_boxed_str());
        let id = seed_instruction(&database, title);
        let extra = seed_variant(&database, id.clone(), "Extra", "More text.");
        let project = &projects[index % projects.len()];
        // Every second instruction: the empty variant to that project,
        // every third (offset) one: the main variant to the next.
        if index % 2 == 0 {
            seed_assignment(&database, project.clone(), id.clone(), extra);
        } else if index % 3 == 0 {
            let database = database.clone();
            let id_for_read = id.clone();
            let main_id = tauri::async_runtime::block_on(database.run(move |tx| {
                eunomia_store::library::list_variants(tx, &id_for_read)
                    .map(|variants| variants[0].id.clone())
                    .map_err(crate::commands::library_store_to_app_error)
            }))
            .expect("reading the main variant through the executor must work");
            seed_assignment(
                &database,
                projects[(index + 1) % projects.len()].clone(),
                id,
                main_id,
            );
        }
    }

    // Act
    let webview = main_webview(&app);
    let tree = invoke_library_get(&webview, serde_json::Value::Null)
        .expect("the large read must succeed")
        .tree;

    // Assert: every item served, in exactly the store's position
    // order, with the labels joined for every assigned instruction.
    assert_eq!(
        tree.len(),
        instructions + headings,
        "every item of the large library is served"
    );
    let served_ids = tree
        .iter()
        .map(|entry| match entry {
            LibraryTreeItem::Instruction(instruction) => instruction.id.as_str(),
            LibraryTreeItem::Heading(heading) => heading.id.as_str(),
        })
        .collect::<Vec<_>>();
    let stored_ids = {
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::library::list_items(tx, eunomia_store::library::ItemScope::All)
                .map(|items| items.into_iter().map(|item| item.id).collect::<Vec<_>>())
                .map_err(crate::commands::library_store_to_app_error)
        }))
        .expect("the store's whole-table read must work")
    };
    assert_eq!(served_ids, stored_ids, "the tree is the position order");
    let labeled = tree
        .iter()
        .filter_map(|entry| match entry {
            LibraryTreeItem::Instruction(instruction) => Some(instruction),
            LibraryTreeItem::Heading(_) => None,
        })
        .filter(|instruction| !instruction.project_labels.is_empty())
        .count();
    let assigned = instructions / 2 + instructions / 6;
    assert_eq!(
        labeled, assigned,
        "every assigned instruction carries its labels ({assigned} of {instructions})"
    );

    drop(app);
    drop(temp);
}

#[test]
fn library_get_counts_the_hidden_instructions_in_the_default_answer() {
    // D-200/D-14: the default answer carries the number of hidden
    // instructions — the "Show hidden" toggle's number — computed in
    // the same read transaction, under BOTH scopes (the count is
    // scope-independent; the default scope does not serve the rows it
    // counts). Headings are never hidden (§6.2: they are always
    // active), so the count is the instructions' alone.
    let (temp, app) = library_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    seed_instruction(&database, "Kept");
    seed_instruction(&database, "Second kept");
    let gone_one = seed_instruction(&database, "Gone one");
    let gone_two = seed_instruction(&database, "Gone two");
    seed_heading(&database, "Style", 2);
    for id in [&gone_one, &gone_two] {
        let id = id.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::library::hide_item(tx, &id)
                .map(|_| ())
                .map_err(crate::commands::library_store_to_app_error)
        }))
        .expect("hiding the instruction through the repository must work");
    }

    let webview = main_webview(&app);
    let default_read = invoke_library_get(&webview, serde_json::Value::Null)
        .expect("the default read must succeed");
    assert_eq!(
        default_read.hidden_count, 2,
        "the D-14 number: two hidden instructions"
    );
    assert_eq!(
        default_read.tree.len(),
        3,
        "the default scope serves neither hidden row"
    );
    let widened = invoke_library_get(&webview, serde_json::json!(true))
        .expect("the widened read must succeed");
    assert_eq!(widened.hidden_count, 2, "the count is scope-independent");
    assert_eq!(widened.tree.len(), 5, "the widened scope serves them");

    drop(app);
    drop(temp);
}
