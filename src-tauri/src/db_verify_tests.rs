//! Mock-runtime tests of the `db_verify` command and the startup
//! invariant check (T-131, §6.3 DB-010…017, §7.10's System row) — the
//! `variants_tests.rs` harness shape: a real `Db` over a temp root,
//! the command registered over the mock runtime, driven through the
//! real IPC envelope, with every finding read back through the store
//! or direct SQL.
//!
//! The card's definition of done leads: **a test shows an artificially
//! violated invariant is found and lands in the journal.** One battery
//! per invariant, each with the same three halves (rule 4's
//! falsifiability — every check is proven able to go red, and proven
//! silent on a legal neighboring state):
//!
//! - **DB-010** — an active instruction with zero active variants (the
//!   main soft-deleted by raw SQL), one with zero active mains (the
//!   flag cleared — the partial index allows zero), and one with NO
//!   variant rows at all (the store allows a bare row); the legal
//!   neighbor is the two-variant instruction (one main, one not).
//! - **DB-011** — a hole in the position row, and a duplicate
//!   position carried by a HEADING (the list is one — headings
//!   count); the legal neighbor is the seeded consecutive row plus
//!   the hidden instruction (state never touches the row arithmetic).
//! - **DB-012** — the sentence verified as DATA, all three broken
//!   halves plantable: an assignment still pointing at a
//!   soft-deleted variant (the delete planted by raw SQL, no move),
//!   one naming NO variant row, and one pointing at ANOTHER
//!   instruction's variant — the last two only a foreign-keys-off
//!   session can leave, so their plants go through a RAW side
//!   connection (the external-writer world, the sweep's own threat
//!   model); the legal neighbor is the REAL `delete_variant` end
//!   state (the assignment moved to the main, LIB-023) and the
//!   custom-text cell (MAIN-007, no variant reference at all).
//! - **DB-014** — both directions of the state/`archived_at`
//!   disagreement; the legal neighbor is the store's own archive of a
//!   project that KEEPS its assignment (the "assignments are kept"
//!   half, legal rows).
//! - **DB-015** — a duplicated `path_key`, planted by rebuilding the
//!   `projects` table without its UNIQUE constraint (the 0003
//!   rebuild technique, the only honest way past a schema-level
//!   constraint — the sweep exists for exactly such data); the legal
//!   neighbor is an archived project beside an active one (different
//!   keys, either state).
//! - **DB-016** — a marker line in stored variant text and in
//!   per-project custom text, plus the padded and BOM-hidden forms
//!   (the save path's own oracle: normalize, then classify each
//!   line); the legal neighbor is the marker INSIDE a line (legal
//!   text — the prefilter's candidate that the exact predicate
//!   rejects).
//!
//! DB-013 needs no battery: it has no data invariant (the module
//! docs' recorded reading — the permission half is legal rows, the
//! build half is `eunomia_core`'s own tested behavior), and the
//! clean-world test below seeds exactly its legal state (a hidden
//! instruction WITH an assignment) to pin that the sweep does not
//! invent one.
//!
//! Cross-cutting pins: the clean database writes NO journal entry (a
//! healthy startup is silent), the found violation's journal row
//! (DB-017: `error.reported`, error level, diagnostic visibility,
//! error result, the entity columns naming rule and row), the startup
//! arm (the same findings journaled at session start, the launch
//! continuing — the non-fatal reading), the command's honest
//! no-database refusal, and the registration pin.

use eunomia_store::action_log::LogRecord;
use eunomia_store::library::{LibraryItem, NewLibraryItem, NewVariant, Variant};
use eunomia_store::projects::{AddedVia, NewProject};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use tauri::Manager;

use crate::db_verify::{DbVerifyViolation, DbViolationDetail};

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

/// A mock app in the healthy state the command serves: `db_verify`
/// registered over the mock runtime and a fresh database in a temp
/// root (the `variants_app` shape).
fn db_verify_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![crate::db_verify::db_verify])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    (temp, app)
}

/// A mock app in the state the session tail serves: no database, the
/// state services the tail arms over (the `recovery_mode_app` shape —
/// `start_database_session` expects every one of them managed).
fn session_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![crate::db_verify::db_verify])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let (dirty_state, dirty_triggers) = crate::dirty_recalc::DirtyState::new();
    app.manage(dirty_state);
    let (file_watch_state, file_watch_commands) = crate::file_watch::FileWatchState::new();
    app.manage(file_watch_state);
    app.manage(crate::availability::AvailabilityState::new());
    app.manage(crate::StateServices::parked(
        dirty_triggers,
        file_watch_commands,
    ));
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::availability::ProjectAvailabilityEvent,
            crate::dirty_recalc::DirtyEvent,
            crate::file_watch::FileChangedEvent,
        ])
        .mount_events(&app);
    (temp, app)
}

/// The database behind the app (the `variants_tests.rs` shape).
fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> crate::db::Db {
    app.state::<crate::db::Db>().inner().clone()
}

/// One invoke request for `cmd` with a JSON body (the
/// `variants_tests.rs` shape).
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

/// The mock `main` webview window the requests go through.
fn main_webview(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("failed to build the mock `main` webview window")
}

/// Invokes `db_verify` over the app's `main` webview and answers the
/// typed halves.
fn invoke_verify(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
) -> Result<Vec<DbVerifyViolation>, serde_json::Value> {
    match tauri::test::get_ipc_response(webview, invoke_request("db_verify", serde_json::json!({})))
    {
        Ok(payload) => Ok(payload
            .deserialize::<Vec<DbVerifyViolation>>()
            .expect("the success payload must deserialize into the DTO")),
        Err(error) => Err(error),
    }
}

// ---------------------------------------------------------------------------
// The seeds (legal rows through the store, the `variants_tests.rs` shape)
// ---------------------------------------------------------------------------

/// Maps a seed-time store failure onto the wire shape the executor
/// speaks: the seeds use legal input, so any failure here is a test
/// bug — the fallback carries the store's own error class, never used
/// as an assertion target.
fn seed_error(error: eunomia_store::library::LibraryError) -> crate::errors::AppError {
    crate::commands::rejected_field("seed", "library", &error.to_string())
}

fn projects_seed_error(error: eunomia_store::projects::ProjectsError) -> crate::errors::AppError {
    crate::commands::rejected_field("seed", "projects", &error.to_string())
}

/// Seeds one project and answers its row id.
fn seed_project(database: &crate::db::Db, n: usize) -> String {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let path = format!("/repos/repo-{n}");
        let name = format!("Project {n}");
        let label = format!("P{n}");
        eunomia_store::projects::create(
            tx,
            &NewProject {
                path: &path,
                path_key: &path,
                name: &name,
                label: &label,
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
            },
        )
        .map(|project| project.id)
        .map_err(projects_seed_error)
    }))
    .expect("seeding a project must work")
}

/// Creates a BARE instruction — no variant at all. The store allows
/// it (library.rs's own wording: "a bare instruction row violates
/// DB-010 until its main variant lands"); only the app layer's create
/// flow guarantees the pair.
fn seed_bare_instruction(database: &crate::db::Db, n: usize) -> LibraryItem {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::create_item(
            tx,
            &NewLibraryItem::Instruction {
                title: &format!("Instruction {n}"),
            },
        )
        .map_err(seed_error)
    }))
    .expect("seeding a bare instruction must work")
}

/// Creates instruction `n` with its main variant named `Main` and
/// answers both rows — the world every check starts from.
fn seed_instruction(database: &crate::db::Db, n: usize) -> (LibraryItem, Variant) {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let item = eunomia_store::library::create_item(
            tx,
            &NewLibraryItem::Instruction {
                title: &format!("Instruction {n}"),
            },
        )
        .map_err(seed_error)?;
        let main = eunomia_store::library::create_variant(
            tx,
            &NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content: &format!("main content {n}"),
                is_main: true,
            },
        )
        .map_err(seed_error)?;
        Ok((item, main))
    }))
    .expect("seeding an instruction with its main variant must work")
}

/// Seeds one more (non-main) variant of the instruction.
fn seed_variant(
    database: &crate::db::Db,
    instruction_id: &str,
    name: &str,
    content: &str,
) -> Variant {
    let instruction_id = instruction_id.to_owned();
    let name = name.to_owned();
    let content = content.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::create_variant(
            tx,
            &NewVariant {
                instruction_id: &instruction_id,
                name: &name,
                content: &content,
                is_main: false,
            },
        )
        .map_err(seed_error)
    }))
    .expect("seeding a variant must work")
}

/// Seeds one heading (the §7.3 list's other kind — DB-011's row is
/// one list, headings included).
fn seed_heading(database: &crate::db::Db) -> LibraryItem {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::create_item(
            tx,
            &NewLibraryItem::Heading {
                title: "Section",
                level: 2,
            },
        )
        .map_err(seed_error)
    }))
    .expect("seeding a heading must work")
}

/// Assigns `variant_id` of `instruction_id` to `project_id`.
fn seed_assignment(
    database: &crate::db::Db,
    project_id: &str,
    instruction_id: &str,
    variant_id: &str,
) {
    let project_id = project_id.to_owned();
    let instruction_id = instruction_id.to_owned();
    let variant_id = variant_id.to_owned();
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
        .map_err(seed_error)
        .map(|_| ())
    }))
    .expect("seeding an assignment must work")
}

/// Sets per-project custom text (MAIN-007's replace cell — legal
/// store rows; the store layer itself does not validate content, the
/// command boundary does, which is exactly why the sweep exists).
fn seed_custom(database: &crate::db::Db, project_id: &str, instruction_id: &str, text: &str) {
    let project_id = project_id.to_owned();
    let instruction_id = instruction_id.to_owned();
    let text = text.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::set_custom(tx, &project_id, &instruction_id, Some(&text))
            .map_err(seed_error)
            .map(|_| ())
    }))
    .expect("seeding custom text must work")
}

// ---------------------------------------------------------------------------
// The plants and the reads
// ---------------------------------------------------------------------------

/// Plants raw SQL over the executor — the "another writer broke it"
/// seam: data no service path of this app could have produced, the
/// world the sweep exists for. Parameters are text (every plant binds
/// row ids; literals ride the SQL itself).
fn plant(database: &crate::db::Db, sql: &str, params: Vec<&str>) {
    let sql = sql.to_owned();
    let params: Vec<String> = params.into_iter().map(str::to_owned).collect();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute(
            sql.as_str(),
            rusqlite::params_from_iter(params.iter().cloned()),
        )
        .map_err(crate::db::sqlite_error_to_app_error)
        .map(|_| ())
    }))
    .expect("planting the violation must work")
}

/// Plants SQL the app's own connections cannot produce: a RAW second
/// connection to the same database file with the `foreign_keys`
/// pragma explicitly OFF — the external-writer world the sweep's
/// threat model names (a hand-driven sqlite3 session or an import
/// tool that turns the pragma off; the bundled library compiles with
/// `SQLITE_DEFAULT_FOREIGN_KEYS=1`, so even a raw rusqlite connection
/// enforces the keys — the plant opts out the same way that writer
/// does). One statement per call; runs between the executor's
/// transactions, so the two connections never contend.
fn plant_foreign(root: &std::path::Path, sql: &str, params: &[&str]) {
    let db_path = crate::data_dirs::db_file_path(root);
    let connection =
        rusqlite::Connection::open(&db_path).expect("opening the raw side connection must work");
    connection
        .execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("turning the raw connection's foreign keys off must work");
    let owned: Vec<String> = params.iter().map(|value| (*value).to_owned()).collect();
    connection
        .execute(sql, rusqlite::params_from_iter(owned))
        .expect("planting through the raw side connection must work");
}

/// Reads the journal's newest entries (up to 100) through the
/// executor — the assertions' window into what a check journaled.
fn journal_of(database: &crate::db::Db) -> Vec<LogRecord> {
    tauri::async_runtime::block_on(database.run(|tx| {
        eunomia_store::action_log::recent(tx, 100)
            .map_err(crate::commands::activity_log_to_app_error)
    }))
    .expect("reading the journal must work")
}

// ---------------------------------------------------------------------------
// DB-010: variant health of active instructions
// ---------------------------------------------------------------------------

#[test]
fn db_010_flags_an_active_instruction_without_active_variants() {
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let (instruction, main) = seed_instruction(&database, 1);
    // The plant: the only active variant soft-deleted by another
    // writer, no LIB-023 move — the instruction left alone.
    plant(
        &database,
        "UPDATE instruction_variants SET state = 'deleted', deleted_at = '2026-11-27T00:00:00.000Z' WHERE id = ?1",
        vec![&main.id],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    // The finding: exactly the sentence's broken pair.
    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-010".to_owned(),
            entity_type: "instruction".to_owned(),
            entity_id: instruction.id.clone(),
            project_id: None,
            detail: Some(DbViolationDetail::VariantCounts {
                active: 0,
                active_main: 0,
            }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_010_flags_an_active_instruction_without_an_active_main() {
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let (instruction, main) = seed_instruction(&database, 1);
    seed_variant(&database, &instruction.id, "Extra", "extra content");
    // The plant: the main flag cleared — two active variants remain
    // (the "at least one" half holds), but no active main (the
    // partial unique index allows zero, it only caps at one).
    plant(
        &database,
        "UPDATE instruction_variants SET is_main = 0 WHERE id = ?1",
        vec![&main.id],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-010".to_owned(),
            entity_type: "instruction".to_owned(),
            entity_id: instruction.id,
            project_id: None,
            detail: Some(DbViolationDetail::VariantCounts {
                active: 2,
                active_main: 0,
            }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_010_flags_an_active_instruction_with_no_variants_at_all() {
    // The LEFT JOIN's zero row: an instruction with NO variant rows
    // (the store allows it — "violates DB-010 until its main variant
    // lands", library.rs's own wording). Round-1 review's P2-2: both
    // other DB-010 plants leave a variant row behind, so an
    // INNER-JOIN regression would stay green without this one.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let item = seed_bare_instruction(&database, 1);

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-010".to_owned(),
            entity_type: "instruction".to_owned(),
            entity_id: item.id,
            project_id: None,
            detail: Some(DbViolationDetail::VariantCounts {
                active: 0,
                active_main: 0,
            }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_010_stays_silent_over_a_two_variant_instruction() {
    // The legal neighbor: one main plus one extra variant — the
    // counts the sentence allows.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let (instruction, _) = seed_instruction(&database, 1);
    seed_variant(&database, &instruction.id, "Extra", "extra content");

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(violations, Vec::new());
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// DB-011: the 1..=N position row
// ---------------------------------------------------------------------------

#[test]
fn db_011_flags_a_hole_in_the_position_row() {
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let (first, _) = seed_instruction(&database, 1);
    let (second, _) = seed_instruction(&database, 2);
    let (third, _) = seed_instruction(&database, 3);
    // The plant: the third row moved past a hole (position 5 where
    // the 1..=N row puts 3 — a gap AND an overshoot in one number).
    plant(
        &database,
        "UPDATE library_items SET position = 5 WHERE id = ?1",
        vec![&third.id],
    );
    let _ = (first, second);

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-011".to_owned(),
            entity_type: "instruction".to_owned(),
            entity_id: third.id,
            project_id: None,
            detail: Some(DbViolationDetail::Position {
                position: 5,
                expected: 3,
            }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_011_flags_a_duplicate_position_on_a_heading() {
    // The duplicate plant rides a HEADING: the list is one (§7.3),
    // headings included in the row arithmetic.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    seed_instruction(&database, 1);
    seed_instruction(&database, 2);
    let heading = seed_heading(&database);
    // The plant: the heading moved onto the second instruction's
    // position — two rows at 2, the walk's third row expected at 3.
    plant(
        &database,
        "UPDATE library_items SET position = 2 WHERE id = ?1",
        vec![&heading.id],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-011".to_owned(),
            entity_type: "heading".to_owned(),
            entity_id: heading.id,
            project_id: None,
            detail: Some(DbViolationDetail::Position {
                position: 2,
                expected: 3,
            }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_011_stays_silent_over_the_seeded_row_and_hidden_rows() {
    // The legal neighbor: consecutive positions, and a HIDDEN
    // instruction among them (state never leaves the row arithmetic —
    // hiding keeps the position).
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let (first, _) = seed_instruction(&database, 1);
    let (second, _) = seed_instruction(&database, 2);
    let second_id = second.id.clone();
    let database_hide = database.clone();
    tauri::async_runtime::block_on(database_hide.run(move |tx| {
        eunomia_store::library::hide_item(tx, &second_id)
            .map_err(seed_error)
            .map(|_| ())
    }))
    .expect("hiding the instruction must work");
    let _ = first;

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(violations, Vec::new());
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// DB-012: an assignment's variant must be an active one
// ---------------------------------------------------------------------------

#[test]
fn db_012_flags_an_assignment_to_a_soft_deleted_variant() {
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, _) = seed_instruction(&database, 1);
    // The assignment rides the EXTRA variant, the main stays healthy:
    // the plant below breaks DB-012 alone, not DB-010 with it.
    let extra = seed_variant(&database, &instruction.id, "Extra", "extra content");
    seed_assignment(&database, &project, &instruction.id, &extra.id);
    // The plant: the variant soft-deleted by another writer with NO
    // LIB-023 move — the assignment left pointing at a deleted row.
    plant(
        &database,
        "UPDATE instruction_variants SET state = 'deleted', deleted_at = '2026-11-27T00:00:00.000Z' WHERE id = ?1",
        vec![&extra.id],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-012".to_owned(),
            entity_type: "assignment".to_owned(),
            entity_id: format!("{project}/{}", instruction.id),
            project_id: Some(project),
            detail: Some(DbViolationDetail::DeletedVariant {
                variant_id: extra.id,
            }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_012_flags_an_assignment_to_a_missing_variant() {
    // The external writer's first shape (round-1 review's P1-2): the
    // assignment's variant_id names NO variant row — data only a
    // foreign-keys-off session can leave (the composite FK holds
    // every writer this app has), planted through the raw side
    // connection.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, _) = seed_instruction(&database, 1);
    let extra = seed_variant(&database, &instruction.id, "Extra", "extra content");
    seed_assignment(&database, &project, &instruction.id, &extra.id);
    plant_foreign(
        temp.path(),
        "UPDATE project_instructions SET variant_id = 'ghost-variant' \
          WHERE project_id = ?1 AND instruction_id = ?2",
        &[project.as_str(), instruction.id.as_str()],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-012".to_owned(),
            entity_type: "assignment".to_owned(),
            entity_id: format!("{project}/{}", instruction.id),
            project_id: Some(project),
            detail: Some(DbViolationDetail::MissingVariant {
                variant_id: "ghost-variant".to_owned(),
            }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_012_flags_an_assignment_to_a_foreign_variants_row() {
    // The external writer's second shape (round-1 review's P1-2): the
    // assignment points at ANOTHER instruction's variant — the
    // same-instruction half of the sentence, verified as data instead
    // of trusted to the composite FK.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, _) = seed_instruction(&database, 1);
    let extra = seed_variant(&database, &instruction.id, "Extra", "extra content");
    seed_assignment(&database, &project, &instruction.id, &extra.id);
    let (other, other_main) = seed_instruction(&database, 2);
    plant_foreign(
        temp.path(),
        "UPDATE project_instructions SET variant_id = ?3 \
          WHERE project_id = ?1 AND instruction_id = ?2",
        &[
            project.as_str(),
            instruction.id.as_str(),
            other_main.id.as_str(),
        ],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-012".to_owned(),
            entity_type: "assignment".to_owned(),
            entity_id: format!("{project}/{}", instruction.id),
            project_id: Some(project),
            detail: Some(DbViolationDetail::ForeignVariant {
                variant_id: other_main.id,
                instruction_id: other.id,
            }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_012_stays_silent_over_the_real_delete_and_a_custom_cell() {
    // The legal neighbors: the store's own `delete_variant` end state
    // (the assignment moved to the main, LIB-023 — the deleted
    // variant ends UNASSIGNED), and a MAIN-007 custom cell (no
    // variant reference at all).
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, main) = seed_instruction(&database, 1);
    let _ = main;
    let extra = seed_variant(&database, &instruction.id, "Extra", "extra content");
    seed_assignment(&database, &project, &instruction.id, &extra.id);
    let move_db = database.clone();
    let extra_id = extra.id.clone();
    tauri::async_runtime::block_on(move_db.run(move |tx| {
        eunomia_store::library::delete_variant(tx, &extra_id, None)
            .map_err(seed_error)
            .map(|_| ())
    }))
    .expect("deleting the assigned variant through the store must work");
    let (custom_instruction, custom_main) = seed_instruction(&database, 3);
    seed_custom(
        &database,
        &project,
        &custom_instruction.id,
        "custom cell text",
    );
    let _ = custom_main;

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(violations, Vec::new());
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// DB-014: the archived row's state coherence
// ---------------------------------------------------------------------------

#[test]
fn db_014_flags_an_archived_row_without_a_stamp() {
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    seed_project(&database, 1);
    let archived = seed_project(&database, 2);
    let archive_db = database.clone();
    let archived_id = archived.clone();
    tauri::async_runtime::block_on(archive_db.run(move |tx| {
        eunomia_store::projects::archive(tx, &archived_id)
            .map_err(projects_seed_error)
            .map(|_| ())
    }))
    .expect("archiving the project must work");
    // The plant: the stamp cleared off the archived row — the store's
    // own `archived_at IS NOT NULL ⟺ state = 'archived'` invariant,
    // broken from outside.
    plant(
        &database,
        "UPDATE projects SET archived_at = NULL WHERE id = ?1",
        vec![&archived],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-014".to_owned(),
            entity_type: "project".to_owned(),
            entity_id: archived.clone(),
            project_id: Some(archived),
            detail: Some(DbViolationDetail::ArchivedAtMismatch { on_active: false }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_014_flags_an_active_row_with_a_stamp() {
    // The disagreement's other direction: an ACTIVE row carrying a
    // stamped `archived_at`.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let active = seed_project(&database, 1);
    plant(
        &database,
        "UPDATE projects SET archived_at = '2026-11-27T00:00:00.000Z' WHERE id = ?1",
        vec![&active],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-014".to_owned(),
            entity_type: "project".to_owned(),
            entity_id: active.clone(),
            project_id: Some(active),
            detail: Some(DbViolationDetail::ArchivedAtMismatch { on_active: true }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_014_stays_silent_over_an_archived_project_keeping_its_assignment() {
    // The legal neighbor: the store's own archive of a project that
    // KEEPS its assignment (the "assignments are kept" half, legal
    // rows — nothing about the pair disagrees).
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, main) = seed_instruction(&database, 1);
    seed_assignment(&database, &project, &instruction.id, &main.id);
    let archive_db = database.clone();
    tauri::async_runtime::block_on(archive_db.run(move |tx| {
        eunomia_store::projects::archive(tx, &project)
            .map_err(projects_seed_error)
            .map(|_| ())
    }))
    .expect("archiving the assignment-keeping project must work");

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(violations, Vec::new());
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// DB-015: one path_key per project row, either state
// ---------------------------------------------------------------------------

#[test]
fn db_015_flags_a_duplicated_path_key() {
    // The UNIQUE constraint holds every writer this app has, so the
    // duplicate can only be planted the way foreign data arrives: the
    // `projects` table rebuilt WITHOUT the constraint (the migration
    // 0003 rebuild technique), the rows copied, a duplicate added.
    // The sweep exists for exactly this data.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let first = seed_project(&database, 1);
    seed_project(&database, 2);
    plant(
        &database,
        "ALTER TABLE projects RENAME TO projects_v2",
        vec![],
    );
    plant(
        &database,
        "CREATE TABLE projects_rebuilt ( \
             id TEXT PRIMARY KEY, path TEXT NOT NULL, \
             path_key TEXT NOT NULL, \
             name TEXT NOT NULL, label TEXT NOT NULL, color TEXT NOT NULL, \
             description TEXT NOT NULL DEFAULT '', \
             state TEXT NOT NULL DEFAULT 'active', \
             added_via TEXT NOT NULL, sort_order INTEGER NOT NULL, \
             scheme TEXT NOT NULL, base_name TEXT NOT NULL, \
             extra_local_enabled INTEGER NOT NULL DEFAULT 0, \
             extra_local_position TEXT NOT NULL DEFAULT 'end', \
             extra_project_enabled INTEGER NOT NULL DEFAULT 0, \
             extra_project_position TEXT NOT NULL DEFAULT 'end', \
             last_synced_at TEXT, last_synced_hash TEXT, \
             created_at TEXT NOT NULL, updated_at TEXT NOT NULL, \
             archived_at TEXT \
         )",
        vec![],
    );
    plant(
        &database,
        "INSERT INTO projects_rebuilt \
             SELECT * FROM projects_v2",
        vec![],
    );
    // The duplicate: project 1's path_key under a new id, stamped
    // LAST (`created_at` 9999) so the deterministic "first row is the
    // legitimate holder" rule has one outcome.
    plant(
        &database,
        "INSERT INTO projects_rebuilt ( \
             id, path, path_key, name, label, color, description, state, \
             added_via, sort_order, scheme, base_name, \
             extra_local_enabled, extra_local_position, \
             extra_project_enabled, extra_project_position, \
             last_synced_at, last_synced_hash, created_at, updated_at, archived_at \
         ) SELECT \
             'plant-duplicate', path, path_key, name, label, color, description, \
             state, added_via, sort_order, scheme, base_name, \
             extra_local_enabled, extra_local_position, \
             extra_project_enabled, extra_project_position, \
             last_synced_at, last_synced_hash, \
             '9999-01-01T00:00:00.000Z', '9999-01-01T00:00:00.000Z', archived_at \
           FROM projects_v2 WHERE id = ?1",
        vec![first.as_str()],
    );
    plant(&database, "DROP TABLE projects_v2", vec![]);
    plant(
        &database,
        "ALTER TABLE projects_rebuilt RENAME TO projects",
        vec![],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-015".to_owned(),
            entity_type: "project".to_owned(),
            entity_id: "plant-duplicate".to_owned(),
            project_id: Some("plant-duplicate".to_owned()),
            detail: Some(DbViolationDetail::SharedPathKey {
                path_key: "/repos/repo-1".to_owned(),
            }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_015_stays_silent_over_an_archived_project_beside_an_active_one() {
    // The legal neighbor: an archived row and an active row with
    // different keys — "unique among active AND archived" holds over
    // both states coexisting.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let first = seed_project(&database, 1);
    seed_project(&database, 2);
    let archive_db = database.clone();
    tauri::async_runtime::block_on(archive_db.run(move |tx| {
        eunomia_store::projects::archive(tx, &first)
            .map_err(projects_seed_error)
            .map(|_| ())
    }))
    .expect("archiving the project must work");

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(violations, Vec::new());
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// DB-016: no stored text holds a marker line
// ---------------------------------------------------------------------------

#[test]
fn db_016_flags_a_marker_line_in_stored_variant_text() {
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let (_, main) = seed_instruction(&database, 1);
    // The plant: the END marker as its own second line — the exact
    // line the save path rejects with E_CONTENT_RESERVED, now stored.
    plant(
        &database,
        "UPDATE instruction_variants SET content = 'x' || char(10) || '<!-- EUNOMIASYNC:END -->' WHERE id = ?1",
        vec![&main.id],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-016".to_owned(),
            entity_type: "variant".to_owned(),
            entity_id: main.id.clone(),
            project_id: None,
            detail: Some(DbViolationDetail::MarkerLine {
                line: 2,
                marker: "<!-- EUNOMIASYNC:END -->".to_owned(),
            }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_016_flags_the_padded_and_bom_hidden_marker_forms() {
    // The save path's oracle normalizes first (ALG-002: the BOM
    // drops, edge whitespace trims) — the sweep must agree: a marker
    // behind a BOM, and a padded one, are violations too.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let (instruction, main) = seed_instruction(&database, 1);
    plant(
        &database,
        "UPDATE instruction_variants SET content = char(65279) || '<!-- EUNOMIASYNC:BEGIN -->' WHERE id = ?1",
        vec![&main.id],
    );
    let padded = seed_variant(
        &database,
        &instruction.id,
        "Padded",
        "  \t<!-- EUNOMIASYNC:END -->\t\t",
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![
            DbVerifyViolation {
                invariant: "DB-016".to_owned(),
                entity_type: "variant".to_owned(),
                entity_id: main.id,
                project_id: None,
                detail: Some(DbViolationDetail::MarkerLine {
                    line: 1,
                    marker: "<!-- EUNOMIASYNC:BEGIN -->".to_owned(),
                }),
            },
            DbVerifyViolation {
                invariant: "DB-016".to_owned(),
                entity_type: "variant".to_owned(),
                entity_id: padded.id,
                project_id: None,
                detail: Some(DbViolationDetail::MarkerLine {
                    line: 1,
                    marker: "<!-- EUNOMIASYNC:END -->".to_owned(),
                }),
            },
        ]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_016_flags_a_marker_line_in_custom_text() {
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, _) = seed_instruction(&database, 1);
    // The store layer writes what it is handed (the command boundary
    // validates) — the MAIN-007 cell seeded with a marker line is
    // exactly the "another writer" world the sweep exists for.
    seed_custom(
        &database,
        &project,
        &instruction.id,
        "y\n<!-- EUNOMIASYNC:BEGIN -->",
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(
        violations,
        vec![DbVerifyViolation {
            invariant: "DB-016".to_owned(),
            entity_type: "assignment".to_owned(),
            entity_id: format!("{project}/{}", instruction.id),
            project_id: Some(project),
            detail: Some(DbViolationDetail::MarkerLine {
                line: 2,
                marker: "<!-- EUNOMIASYNC:BEGIN -->".to_owned(),
            }),
        }]
    );
    drop(app);
    drop(temp);
}

#[test]
fn db_016_stays_silent_over_a_marker_inside_a_line() {
    // The legal neighbor: the marker NOT as a whole line — legal
    // text the save path accepts; the prefilter still selects the
    // row (the literal is present), and the exact predicate must
    // reject it.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let (_, main) = seed_instruction(&database, 1);
    plant(
        &database,
        "UPDATE instruction_variants SET content = 'see <!-- EUNOMIASYNC:END --> inline' WHERE id = ?1",
        vec![&main.id],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    assert_eq!(violations, Vec::new());
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// The clean world: silence, and no journal noise
// ---------------------------------------------------------------------------

#[test]
fn a_clean_database_finds_nothing_and_journals_nothing() {
    // The full legal world: an active project with a variant
    // assignment and a custom cell, an archived project KEEPING its
    // assignment (DB-014's "assignments are kept"), a hidden
    // instruction WITH its assignment (DB-013's permission), a
    // soft-deleted variant whose assignment moved (LIB-023's legal
    // end state), and a heading — every neighboring legal state the
    // per-invariant batteries name, in one database.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let archived = seed_project(&database, 2);
    let (instruction, main) = seed_instruction(&database, 1);
    seed_assignment(&database, &project, &instruction.id, &main.id);
    let (custom_instruction, _) = seed_instruction(&database, 2);
    seed_custom(
        &database,
        &project,
        &custom_instruction.id,
        "custom cell text",
    );
    let (archived_instruction, archived_main) = seed_instruction(&database, 3);
    seed_assignment(
        &database,
        &archived,
        &archived_instruction.id,
        &archived_main.id,
    );
    let archive_db = database.clone();
    let archived_id = archived.clone();
    tauri::async_runtime::block_on(archive_db.run(move |tx| {
        eunomia_store::projects::archive(tx, &archived_id)
            .map_err(projects_seed_error)
            .map(|_| ())
    }))
    .expect("archiving the assignment-keeping project must work");
    let (hidden, hidden_main) = seed_instruction(&database, 4);
    seed_assignment(&database, &project, &hidden.id, &hidden_main.id);
    let hide_db = database.clone();
    let hidden_id = hidden.id.clone();
    tauri::async_runtime::block_on(hide_db.run(move |tx| {
        eunomia_store::library::hide_item(tx, &hidden_id)
            .map_err(seed_error)
            .map(|_| ())
    }))
    .expect("hiding the assigned instruction must work");
    let (deleted_pair, _) = seed_instruction(&database, 5);
    let extra = seed_variant(&database, &deleted_pair.id, "Extra", "extra content");
    seed_assignment(&database, &project, &deleted_pair.id, &extra.id);
    let delete_db = database.clone();
    let extra_id = extra.id.clone();
    tauri::async_runtime::block_on(delete_db.run(move |tx| {
        eunomia_store::library::delete_variant(tx, &extra_id, None)
            .map_err(seed_error)
            .map(|_| ())
    }))
    .expect("deleting the assigned variant through the store must work");
    seed_heading(&database);

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");

    // Nothing found, and nothing journaled: a healthy check is silent
    // (the `mark_interrupted_runs` zero-case pattern).
    assert_eq!(violations, Vec::new());
    assert!(
        journal_of(&database).is_empty(),
        "a clean database must journal no invariant entry"
    );
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// DB-017: the journal entry
// ---------------------------------------------------------------------------

#[test]
fn a_found_violation_lands_in_the_journal_at_the_error_level() {
    // The card's definition of done, both halves at once: the
    // artificially violated invariant is FOUND (the answer above)
    // and LANDS IN THE JOURNAL (this assertion) — one `error.reported`
    // row at the error level, diagnostic visibility, error result,
    // the entity columns naming the rule and the row, the project
    // column carrying the correlation.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, _) = seed_instruction(&database, 1);
    // The assignment rides the extra variant (the main stays healthy —
    // the plant below breaks DB-012 alone).
    let extra = seed_variant(&database, &instruction.id, "Extra", "extra content");
    seed_assignment(&database, &project, &instruction.id, &extra.id);
    plant(
        &database,
        "UPDATE instruction_variants SET state = 'deleted', deleted_at = '2026-11-27T00:00:00.000Z' WHERE id = ?1",
        vec![&extra.id],
    );

    let violations = invoke_verify(&main_webview(&app)).expect("the check must answer");
    assert_eq!(violations.len(), 1);

    let journal = journal_of(&database);
    assert_eq!(journal.len(), 1, "exactly one entry for one violation");
    let entry = &journal[0];
    assert_eq!(entry.action.as_str(), "error.reported");
    assert_eq!(entry.level.as_str(), "error");
    assert_eq!(entry.visibility.as_str(), "diagnostic");
    assert_eq!(entry.result.as_str(), "error");
    let (entity_type, entity_id) = entry.entity.as_ref().expect("the entry names the row");
    assert_eq!(entity_type.as_str(), "assignment");
    assert_eq!(
        entity_id,
        &format!("DB-012/{project}/{}", instruction.id),
        "the journal entity names the rule and the row"
    );
    assert_eq!(entry.project_id.as_deref(), Some(project.as_str()));
    assert_eq!(entry.details, None);
    drop(app);
    drop(temp);
}

#[test]
fn a_repeat_run_re_finds_and_re_journals_the_violation() {
    // DB-017 has no repair and no dedup state: the journal is the
    // history of checks, so the second run records the same finding
    // again — pinned so the shape is a recorded decision, not an
    // accident.
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, _) = seed_instruction(&database, 1);
    // The assignment rides the extra variant (the main stays healthy —
    // the plant below breaks DB-012 alone).
    let extra = seed_variant(&database, &instruction.id, "Extra", "extra content");
    seed_assignment(&database, &project, &instruction.id, &extra.id);
    plant(
        &database,
        "UPDATE instruction_variants SET state = 'deleted', deleted_at = '2026-11-27T00:00:00.000Z' WHERE id = ?1",
        vec![&extra.id],
    );
    let webview = main_webview(&app);
    let first = invoke_verify(&webview).expect("the first check must answer");
    let second = invoke_verify(&webview).expect("the second check must answer");

    assert_eq!(first, second, "the finding is stable across runs");
    assert_eq!(
        journal_of(&database).len(),
        2,
        "each check records what it found"
    );
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// The startup half: the session tail runs the check
// ---------------------------------------------------------------------------

#[test]
fn the_session_start_runs_the_invariant_check_and_journals_its_findings() {
    // §6.3's "checked at startup": the tail's own arm, driven the way
    // the `setup` hook drives it. A planted violation must land in
    // the journal, and the launch must CONTINUE — the database is
    // published and serving (DB-017's non-fatal reading: a violation
    // is a recorded fact, never an abort).
    let (temp, app) = session_app();
    let database =
        crate::db::Db::open(temp.path()).expect("opening the session database must work");
    let project = seed_project(&database, 1);
    let (instruction, _) = seed_instruction(&database, 1);
    // The assignment rides the extra variant (the main stays healthy —
    // the plant below breaks DB-012 alone).
    let extra = seed_variant(&database, &instruction.id, "Extra", "extra content");
    seed_assignment(&database, &project, &instruction.id, &extra.id);
    plant(
        &database,
        "UPDATE instruction_variants SET state = 'deleted', deleted_at = '2026-11-27T00:00:00.000Z' WHERE id = ?1",
        vec![&extra.id],
    );

    // Act: the startup-shaped call (the tail takes ownership).
    crate::start_database_session(app.handle(), database);

    // Assert 1: the journal holds the finding.
    let journal = journal_of(&database_of(&app));
    assert_eq!(
        journal.len(),
        1,
        "the startup check journaled the violation"
    );
    let entry = &journal[0];
    assert_eq!(entry.action.as_str(), "error.reported");
    assert_eq!(entry.level.as_str(), "error");
    let (_, entity_id) = entry.entity.as_ref().expect("the entry names the row");
    assert_eq!(entity_id, &format!("DB-012/{project}/{}", instruction.id));
    // Assert 2: the launch continued — the database is published and
    // the command answers through it (the on-demand half, too).
    let violations =
        invoke_verify(&main_webview(&app)).expect("the command must answer over the session");
    assert_eq!(violations.len(), 1);
    drop(app);
    drop(temp);
}

#[test]
fn a_clean_session_start_journals_nothing() {
    // The healthy startup is silent: no violation, no entry (the
    // zero-case pattern), and the launch proceeds.
    let (temp, app) = session_app();
    let database =
        crate::db::Db::open(temp.path()).expect("opening the session database must work");
    let project = seed_project(&database, 1);
    let (instruction, main) = seed_instruction(&database, 1);
    seed_assignment(&database, &project, &instruction.id, &main.id);

    crate::start_database_session(app.handle(), database);

    assert!(
        journal_of(&database_of(&app)).is_empty(),
        "a clean startup must journal no invariant entry"
    );
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// The command's contract
// ---------------------------------------------------------------------------

#[test]
fn db_verify_refuses_honestly_without_a_database() {
    // Recovery mode: no `Db` managed — the `managed_db` refusal
    // (E_DB_LOCKED + NoDatabase), never a fake report.
    let (temp, app) = session_app();
    let error =
        invoke_verify(&main_webview(&app)).expect_err("without a database the command must refuse");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
    drop(app);
    drop(temp);
}

#[test]
fn the_violation_row_travels_the_wire_in_its_tagged_shape() {
    // The §7.10 envelope's payload shape, pinned as raw JSON: the
    // snake_case fields (the house DTO convention) and the
    // `kind`-tagged detail (the `SyncRunOutcome` enum convention).
    let (temp, app) = db_verify_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, _) = seed_instruction(&database, 1);
    // The assignment rides the extra variant (the main stays healthy —
    // the plant below breaks DB-012 alone).
    let extra = seed_variant(&database, &instruction.id, "Extra", "extra content");
    seed_assignment(&database, &project, &instruction.id, &extra.id);
    plant(
        &database,
        "UPDATE instruction_variants SET state = 'deleted', deleted_at = '2026-11-27T00:00:00.000Z' WHERE id = ?1",
        vec![&extra.id],
    );

    let response = tauri::test::get_ipc_response(
        &main_webview(&app),
        invoke_request("db_verify", serde_json::json!({})),
    )
    .expect("the check must respond successfully");
    let payload: serde_json::Value = response
        .deserialize()
        .expect("the payload must deserialize as JSON");

    assert_eq!(
        payload,
        serde_json::json!([{
            "invariant": "DB-012",
            "entity_type": "assignment",
            "entity_id": format!("{project}/{instruction_id}", instruction_id = instruction.id),
            "project_id": project,
            "detail": {
                "kind": "deletedVariant",
                "variant_id": extra.id,
            },
        }])
    );
    drop(app);
    drop(temp);
}

#[test]
fn the_db_verify_command_is_registered_with_the_specta_builder() {
    // The D-034 wiring-pin rationale: without this, a dropped
    // registration would silently rewrite the committed bindings
    // without the command.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("db_verify::db_verify").count(),
        1,
        "lib.rs must register `db_verify::db_verify` exactly once (§7.10, D-009)"
    );
}
