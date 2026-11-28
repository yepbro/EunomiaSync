//! Mock-IPC behavior tests for `project_build_preview` (T-358,
//! MAIN-033, §7.10).
//!
//! The oracle is the card's own done-when: the preview's text equals
//! what the NEXT SYNC would write to the managed area, ignoring the
//! marker — so the three scheme tests drive a REAL run (phases 0–3,
//! the §9.4 order, the real §9.5 write and the fixed marker clock)
//! over a real temp repository and compare the written FILE byte for
//! byte with the answer the command gave BEFORE the run (nothing
//! changes between the two, so the identity is exact). The extend
//! composition, the empty column, the refusal shapes, the held-sync
//! read and the recovery-mode answer complete the battery.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};

use eunomia_core::marker_clock::MarkerClock;
use eunomia_store::projects::NewProject;
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};

use crate::sync_phase0;
use crate::sync_phase1;
use crate::sync_phase2;
use crate::sync_phase3;

// ---- the fixed clock (the phase-2/3 suites' value) -------------------

/// The run's fixed write instant: the marker line's Prague civil
/// time, sub-second precision dropped (the phase-3 suite's value).
fn fixed_now() -> jiff::Timestamp {
    "2026-11-21T10:30:00.123Z"
        .parse()
        .expect("the fixed instant must parse")
}

/// The run's ONE marker line, pinned as a literal — the byte the
/// phase-3 drive writes after the preview's text (ALG-032's shape at
/// the fixed instant; the preview itself never carries it).
const MARKER_LINE: &str =
    "<!-- Created by EunomiaSync 2026-11-21 11:30:00 Europe/Prague +01:00 -->";

/// The fixed marker clock (the phase-3 suite's shape).
fn fixed_marker_clock() -> MarkerClock {
    MarkerClock::from_instant(
        fixed_now(),
        jiff::tz::TimeZone::get("Europe/Prague").expect("Europe/Prague is in the tzdb"),
    )
}

// ---- the harness ------------------------------------------------------

/// A mock app with the preview command registered over a fresh
/// database in a temp data root, plus the states a real sync drive
/// needs (the `SyncRunning` flag and phase 3's write-in-flight gate),
/// and the `repos/` root the fixtures live under.
fn preview_app() -> (
    tempfile::TempDir,
    tauri::App<tauri::test::MockRuntime>,
    PathBuf,
) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path()).expect("the data layout must be creatable");
    let database =
        crate::db::Db::open(temp.path()).expect("a fresh database must open in the temp root");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_build_preview::project_build_preview
        ])
        .build(mock_context(noop_assets()))
        .expect("the mock app must build");
    app.manage(database);
    app.manage(crate::sync_running::SyncRunning::new());
    // The write-in-flight gate phase 3's per-project guard reads
    // (T-123/D-219) — present so the real-sync drives below run the
    // production composition.
    app.manage(crate::commands::SyncWriteGate::new());
    let repos = temp.path().join("repos");
    fs::create_dir(&repos).expect("the repos root must be creatable");
    (temp, app, repos)
}

/// A mock app with the command registered and NO database managed —
/// the recovery-mode shape.
fn databaseless_app() -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_build_preview::project_build_preview
        ])
        .build(mock_context(noop_assets()))
        .expect("the mock app must build")
}

fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> crate::db::Db {
    app.state::<crate::db::Db>().inner().clone()
}

fn main_webview(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("the mock main webview must build")
}

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

fn invoke<T: serde::de::DeserializeOwned>(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    body: serde_json::Value,
) -> Result<T, serde_json::Value> {
    match tauri::test::get_ipc_response(webview, invoke_request(cmd, body)) {
        Ok(payload) => Ok(payload
            .deserialize::<T>()
            .expect("the success payload deserializes into the command DTO")),
        Err(error) => Err(error),
    }
}

/// Invokes the preview command over mock IPC.
fn invoke_preview(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    project_id: &str,
) -> Result<crate::project_build_preview::BuildPreview, serde_json::Value> {
    invoke(
        webview,
        "project_build_preview",
        serde_json::json!({ "projectId": project_id }),
    )
}

/// Maps a store seeding failure onto the app's wire shape (the
/// assignments suite's `seed_error`).
fn seed_error(error: eunomia_store::library::LibraryError) -> crate::errors::AppError {
    crate::db::sqlite_error_to_app_error(match error {
        eunomia_store::library::LibraryError::Sql(source) => source,
        other => rusqlite::Error::InvalidParameterName(other.to_string()),
    })
}

/// Seeds a project row over a real repo folder (`repos/repo-<n>` with
/// a `.git` directory), with the scheme, base name and the two
/// extra-file options the caller pins — the settings the §9.3
/// assembly branches on. Returns `(project_id, canonical_repo)`.
fn seed_repo_project(
    app: &tauri::App<tauri::test::MockRuntime>,
    repos: &Path,
    n: usize,
    scheme: Scheme,
    base_name: BaseName,
    extra_project: ExtraFile,
    extra_local: ExtraFile,
) -> (String, PathBuf) {
    let repo = repos.join(format!("repo-{n}"));
    fs::create_dir(&repo).expect("the repository folder must be creatable");
    fs::create_dir(repo.join(".git")).expect("the .git directory must be creatable");
    let canonical =
        eunomia_fs::paths::canonicalize(&repo).expect("canonicalizing the fresh repo must work");
    let path = Box::leak(canonical.to_string_lossy().into_owned().into_boxed_str());
    let id = tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        eunomia_store::projects::create(
            tx,
            &NewProject {
                path,
                path_key: path,
                name: Box::leak(format!("Project {n}").into_boxed_str()),
                label: Box::leak(format!("P{n}").into_boxed_str()),
                color: "blue",
                description: "",
                added_via: eunomia_store::projects::AddedVia::Manual,
                scheme,
                base_name,
                extra_local,
                extra_project,
            },
        )
        .map(|project| project.id)
        .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work");
    (id, canonical)
}

/// Seeds one active instruction with its main variant carrying
/// `content`, assigned to the project (the phase-3 harness shape).
/// Returns `(instruction_id, variant_id)`.
fn seed_assigned_instruction(
    app: &tauri::App<tauri::test::MockRuntime>,
    project_id: &str,
    title: &'static str,
    content: &'static str,
) -> (String, String) {
    let project_id = project_id.to_owned();
    tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        let item = eunomia_store::library::create_item(
            tx,
            &eunomia_store::library::NewLibraryItem::Instruction { title },
        )
        .map_err(seed_error)?;
        let variant = eunomia_store::library::create_variant(
            tx,
            &eunomia_store::library::NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content,
                is_main: true,
            },
        )
        .map_err(seed_error)?;
        eunomia_store::library::assign(
            tx,
            &eunomia_store::library::NewAssignment {
                project_id: &project_id,
                instruction_id: &item.id,
                variant_id: &variant.id,
            },
        )
        .map_err(seed_error)?;
        Ok((item.id, variant.id))
    }))
    .expect("seeding the assigned instruction must work")
}

/// Seeds an extend cell on `instruction` (the store's own write — the
/// base is resolved exactly the way the real command resolves it).
fn seed_extend_cell(
    app: &tauri::App<tauri::test::MockRuntime>,
    project_id: &str,
    instruction_id: &str,
    supplement: &str,
) {
    let project_id = project_id.to_owned();
    let instruction_id = instruction_id.to_owned();
    let supplement = supplement.to_owned();
    tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        eunomia_store::library::set_custom(
            tx,
            &project_id,
            &instruction_id,
            Some(&supplement),
            eunomia_store::library::CustomMode::Extend,
        )
        .map_err(seed_error)?;
        Ok(())
    }))
    .expect("seeding the extend cell must work");
}

/// Seeds one section heading (LIB-030) — the ALG-020 fixture half the
/// heading arm of the scheme-A oracle needs: the heading lands before
/// the instructions seeded after it, so its section includes them.
fn seed_heading(
    app: &tauri::App<tauri::test::MockRuntime>,
    title: &'static str,
    level: u8,
) -> String {
    tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        eunomia_store::library::create_item(
            tx,
            &eunomia_store::library::NewLibraryItem::Heading { title, level },
        )
        .map_err(seed_error)
        .map(|item| item.id)
    }))
    .expect("seeding the heading must work")
}

/// The action_log row count — the read's no-journal pin's oracle.
fn journal_size(app: &tauri::App<tauri::test::MockRuntime>) -> i64 {
    tauri::async_runtime::block_on(database_of(app).run(|tx| {
        tx.query_row("SELECT COUNT(*) FROM action_log", [], |row| row.get(0))
            .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("the journal count must read")
}

/// Runs a REAL sync (§9.4's phases 0–3, the real §9.5 write and the
/// fixed marker clock) over the app's current database and drops the
/// flag lease — the composition whose disk output is the oracle.
fn run_real_sync_through_phase3(app: &tauri::App<tauri::test::MockRuntime>, data_dir: &Path) {
    let phase0 = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect("phase 0 must run over the seeded database");
    let phase1 = tauri::async_runtime::block_on(sync_phase1::run(
        app.handle(),
        &phase0,
        || Ok(u64::MAX),
        SystemTime::now(),
    ))
    .expect("phase 1 must pass");
    tauri::async_runtime::block_on(sync_phase2::run_with_abort_clock(
        app.handle(),
        &phase1,
        data_dir,
        eunomia_fs::backup::backup_target_file,
        fixed_now(),
        jiff::Timestamp::now,
    ))
    .expect("phase 2 must succeed over the healthy fixtures");
    tauri::async_runtime::block_on(sync_phase3::run(
        app.handle(),
        &phase1,
        sync_phase3::production_write,
        fixed_marker_clock,
        fixed_now(),
    ))
    .expect("phase 3 must succeed over the healthy fixtures");
    drop(phase0);
}

/// The ALG-030 file the written A/C target must hold: the preview's
/// text, an empty line, the run's marker line, one line break (the
/// phase-3 suite's `old_sync_file` shape — the spec's own layout,
/// spelled as the literal it is).
fn alg_030_file(preview_text: &str) -> String {
    if preview_text.is_empty() {
        format!("{MARKER_LINE}\n")
    } else {
        format!("{preview_text}\n\n{MARKER_LINE}\n")
    }
}

// ---- the real-sync oracle, per scheme ---------------------------------

#[test]
fn the_preview_text_is_what_a_real_sync_writes_scheme_a() {
    // MAIN-033's acceptance: the preview matches the next sync's
    // managed content. Scheme A with BOTH reading lines enabled — the
    // D-21 lines belong to the managed text (ALG-030's "A only"
    // halves), so the drive proves they flow through the preview and
    // the real write alike — and a `## Style` heading over the
    // assigned instruction, so ALG-020's heading block rides the same
    // oracle (MAIN-033's "headings by ALG-020" sentence). The file a
    // real run leaves must be the preview's text framed by ALG-030's
    // empty line + marker + break — byte for byte, the fixed clock
    // spelling the marker. And the READ half: the preview leaves no
    // file and no journal row behind (the P3-3 pin — a preview-side
    // write or journal call reddens here, before phase 3 could mask
    // it).
    let (temp, app, repos) = preview_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        Scheme::A,
        BaseName::AgentsMd,
        ExtraFile {
            enabled: true,
            position: Position::Start,
        },
        ExtraFile {
            enabled: true,
            position: Position::End,
        },
    );
    let _heading = seed_heading(&app, "Style", 2);
    seed_assigned_instruction(&app, &project, "Be brief one", "Body text.");
    let webview = main_webview(&app);
    let journal_before = journal_size(&app);

    let preview = invoke_preview(&webview, &project)
        .expect("the preview over the active project must succeed");
    assert_eq!(
        journal_size(&app),
        journal_before,
        "a read journals nothing (D-051: reading is not an event)"
    );
    assert!(
        !repo.join("AGENTS.md").exists(),
        "the preview writes no file — the next sync owns the target"
    );
    assert_eq!(
        preview.target_path,
        repo.join("AGENTS.md")
            .to_str()
            .expect("the target is UTF-8"),
        "the answer carries §9.4 phase 1 step 2's path spelling"
    );
    assert_eq!(
        preview.text,
        "If AGENTS.project.md exists in the repository root, read it before starting \
         work and follow its instructions.\n\
         \n\
         ## Style\n\
         \n\
         Body text.\n\
         \n\
         If AGENTS.local.md exists in the repository root, read it before starting \
         work and follow its instructions.",
        "scheme A's managed text splices the D-21 reading lines around the body, \
         the heading included by ALG-020"
    );
    assert!(
        !preview.text.contains("Created by EunomiaSync"),
        "the preview never carries the creation marker"
    );

    run_real_sync_through_phase3(&app, temp.path());
    let written = fs::read_to_string(repo.join("AGENTS.md"))
        .expect("the real run must have written the target");
    assert_eq!(
        written,
        alg_030_file(&preview.text),
        "the written file is the preview's text plus ALG-030's marker framing"
    );
}

#[test]
fn the_preview_text_is_what_a_real_sync_writes_scheme_c() {
    // The same oracle over scheme C: the extras are stored on the row
    // (§6.2's columns are NOT NULL) but ALG-030's "(A only)" gate
    // drops them from the build — the preview's text has no reading
    // line, and the run writes `<STEM>.project.md` with exactly that
    // text.
    let (temp, app, repos) = preview_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        2,
        Scheme::C,
        BaseName::AgentsMd,
        ExtraFile {
            enabled: true,
            position: Position::Start,
        },
        ExtraFile {
            enabled: true,
            position: Position::End,
        },
    );
    seed_assigned_instruction(&app, &project, "Be brief two", "Scheme C body.");
    let webview = main_webview(&app);

    let preview = invoke_preview(&webview, &project)
        .expect("the preview over the active project must succeed");
    assert!(
        !repo.join("AGENTS.project.md").exists(),
        "the preview writes no file — the next sync owns the target"
    );
    assert_eq!(
        preview.target_path,
        repo.join("AGENTS.project.md")
            .to_str()
            .expect("the target is UTF-8"),
        "scheme C's target is the <STEM>.project.md spelling"
    );
    assert_eq!(preview.text, "Scheme C body.");
    assert!(
        !preview.text.contains("read it before starting work"),
        "ALG-030's (A only) gate keeps the reading lines out of a scheme-C build"
    );

    run_real_sync_through_phase3(&app, temp.path());
    let written = fs::read_to_string(repo.join("AGENTS.project.md"))
        .expect("the real run must have written the scheme-C target");
    assert_eq!(
        written,
        alg_030_file(&preview.text),
        "the scheme-C file is the preview's text plus the marker framing"
    );
}

#[test]
fn the_preview_text_is_the_managed_block_a_real_sync_writes_scheme_b() {
    // Scheme B answers the managed block only (MAIN-033's own
    // clause): the real run splices the block between the marker
    // lines, and the preview's text is exactly the block — never the
    // file's outside, never the markers, never the reading lines (the
    // row carries both enabled). The written file is the phase-3
    // suite's byte-exact splice shape over the preview's text.
    let (temp, app, repos) = preview_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        3,
        Scheme::B,
        BaseName::AgentsMd,
        ExtraFile {
            enabled: true,
            position: Position::Start,
        },
        ExtraFile {
            enabled: true,
            position: Position::End,
        },
    );
    seed_assigned_instruction(&app, &project, "Be brief three", "Block body.");
    let target = repo.join("AGENTS.md");
    fs::write(
        &target,
        "Intro.\n<!-- EUNOMIASYNC:BEGIN -->\nOld body.\n<!-- EUNOMIASYNC:END -->\nTail.\n",
    )
    .expect("writing the marked fixture must work");
    let webview = main_webview(&app);

    let preview = invoke_preview(&webview, &project)
        .expect("the preview over the active project must succeed");
    // The read leaves the existing target's bytes untouched (the P3-3
    // pin's scheme-B arm: the file already exists here, so the honest
    // no-write assertion is byte equality around the call).
    assert_eq!(
        fs::read_to_string(&target).expect("the marked fixture must read"),
        "Intro.\n<!-- EUNOMIASYNC:BEGIN -->\nOld body.\n<!-- EUNOMIASYNC:END -->\nTail.\n",
        "the preview writes no file — the next sync owns the splice"
    );
    assert_eq!(
        preview.target_path,
        target.to_str().expect("the target is UTF-8"),
        "scheme B's target is the <root>/<base_name> spelling"
    );
    assert_eq!(preview.text, "Block body.");
    assert!(
        !preview.text.contains("EUNOMIASYNC"),
        "the managed block carries neither the run marker nor the fence markers"
    );

    run_real_sync_through_phase3(&app, temp.path());
    let written = fs::read_to_string(&target).expect("the real run must have spliced the target");
    assert_eq!(
        written,
        format!(
            "Intro.\n<!-- EUNOMIASYNC:BEGIN -->\n{}\n\n{MARKER_LINE}\n\
             <!-- EUNOMIASYNC:END -->\nTail.\n",
            preview.text
        ),
        "the splice writes exactly the preview's text as the managed block"
    );
}

// ---- the ALG-023 composition ------------------------------------------

#[test]
fn an_extend_cell_composes_per_alg_023_in_the_preview() {
    // The T-356 composition flows through the preview unchanged: an
    // extend cell contributes the non-empty halves of the normalized
    // base and supplement joined by exactly one blank line, and an
    // empty half contributes nothing (D-13) — the same `build_body`
    // the sync builds with, over the same store state.
    let (_temp, app, repos) = preview_app();
    let (project, _repo) = seed_repo_project(
        &app,
        &repos,
        4,
        Scheme::B,
        BaseName::ClaudeMd,
        ExtraFile {
            enabled: false,
            position: Position::End,
        },
        ExtraFile {
            enabled: false,
            position: Position::End,
        },
    );
    let (instruction, _variant) =
        seed_assigned_instruction(&app, &project, "Be brief four", "Base text.");
    // The store's own write: the unassigned cell adopts the main as
    // the base (D-203 reading 4c).
    seed_extend_cell(&app, &project, &instruction, "supplement text");
    let webview = main_webview(&app);

    let preview =
        invoke_preview(&webview, &project).expect("the preview over the extend cell must succeed");
    assert_eq!(
        preview.text, "Base text.\n\nsupplement text",
        "ALG-023 joins the base and the supplement with exactly one blank line"
    );

    // The empty half: a whitespace-only supplement contributes
    // nothing — the base alone is the cell's output.
    seed_extend_cell(&app, &project, &instruction, " \n\t ");
    let second = invoke_preview(&webview, &project)
        .expect("the second preview over the same cell must succeed");
    assert_eq!(
        second.text, "Base text.",
        "an empty half contributes nothing (D-13)"
    );
    assert_eq!(second.target_path, preview.target_path);
}

// ---- the empty column ---------------------------------------------------

#[test]
fn an_empty_column_answers_empty_text() {
    // MAIN-033's "a column with no assignments shows a note instead
    // of an empty document": the NOTE is the modal's own — the
    // command's honest answer over a column with no assignments (and
    // no reading lines) is the empty text, the build D-25 would
    // confirm before wiping.
    let (_temp, app, repos) = preview_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        5,
        Scheme::A,
        BaseName::AgentsMd,
        ExtraFile {
            enabled: false,
            position: Position::End,
        },
        ExtraFile {
            enabled: false,
            position: Position::End,
        },
    );
    // The library is non-empty and another column IS assigned — the
    // empty verdict belongs to THIS column alone, not to an empty
    // library.
    let (other, _other_repo) = seed_repo_project(
        &app,
        &repos,
        6,
        Scheme::A,
        BaseName::AgentsMd,
        ExtraFile {
            enabled: false,
            position: Position::End,
        },
        ExtraFile {
            enabled: false,
            position: Position::End,
        },
    );
    seed_assigned_instruction(&app, &other, "Be brief six", "Assigned elsewhere.");
    let webview = main_webview(&app);

    let preview = invoke_preview(&webview, &project)
        .expect("the preview over the assignment-free column must succeed");
    assert!(
        !repo.join("AGENTS.md").exists(),
        "the preview writes no file — the next sync owns the target"
    );
    assert_eq!(preview.text, "", "an empty column builds the empty text");
    assert_eq!(
        preview.target_path,
        repo.join("AGENTS.md")
            .to_str()
            .expect("the target is UTF-8"),
        "the target path is answered even over an empty column"
    );
}

// ---- the refusals -------------------------------------------------------

#[test]
fn an_archived_or_unknown_id_is_the_project_update_refusal() {
    // §7.10's sentence: an id naming no active project (unknown, or
    // archived) is the typed `E_VALIDATION` refusal naming
    // `project_id`, the `project_update` shape — the envelope's
    // field/value/reason triple, the value echoed bounded (D-115's
    // cap on the caller's unbounded webview string).
    let (_temp, app, repos) = preview_app();
    let (project, _repo) = seed_repo_project(
        &app,
        &repos,
        7,
        Scheme::A,
        BaseName::AgentsMd,
        ExtraFile {
            enabled: false,
            position: Position::End,
        },
        ExtraFile {
            enabled: false,
            position: Position::End,
        },
    );
    let project_id = project.clone();
    let _ = tauri::async_runtime::block_on(database_of(&app).run(move |tx| {
        eunomia_store::projects::archive(tx, &project)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("archiving the seeded project must work");
    let webview = main_webview(&app);

    for (id, expected_value) in [
        (project_id.as_str(), project_id.as_str()),
        ("no-such-project", "no-such-project"),
        // A 70-character id: the echo is bounded to 64 code points
        // plus the count (D-115's arithmetic).
        (
            &"x".repeat(70),
            &format!("{} (+6 more code points)", "x".repeat(64)),
        ),
    ] {
        let error = invoke_preview(&webview, id)
            .expect_err("an id naming no active project must be refused");
        assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
        assert_eq!(error["params"]["field"], serde_json::json!("project_id"));
        assert_eq!(
            error["params"]["value"],
            serde_json::json!(expected_value),
            "the refusal echoes the caller's id bounded"
        );
        assert_eq!(
            error["params"]["reason"],
            serde_json::json!("no active project has this id")
        );
    }
}

// ---- cross-cutting pins -------------------------------------------------

#[test]
fn the_preview_is_readable_during_a_held_sync() {
    // §7.10's "a read, no SYS-005 gate": while a sync holds the
    // global flag, the preview still answers — the moment-of-read
    // fact a mid-run `projects_list` serves the same way (the next
    // run's snapshot may differ; that is the flag's own semantics,
    // not this command's refusal).
    let (_temp, app, repos) = preview_app();
    let (project, _repo) = seed_repo_project(
        &app,
        &repos,
        8,
        Scheme::C,
        BaseName::AgentsMd,
        ExtraFile {
            enabled: false,
            position: Position::End,
        },
        ExtraFile {
            enabled: false,
            position: Position::End,
        },
    );
    seed_assigned_instruction(&app, &project, "Be brief eight", "During a sync.");
    let webview = main_webview(&app);
    let guard =
        crate::commands::begin_sync(app.handle()).expect("the test's flag must be free to take");

    let preview = invoke_preview(&webview, &project)
        .expect("the preview is a read: no SYS-005 refusal while a sync runs");
    assert_eq!(preview.text, "During a sync.");

    drop(guard);
}

#[test]
fn the_preview_answers_honestly_without_a_database() {
    // Recovery mode (no `Db` managed): the honest `NoDatabase`
    // fallback every database read shares — never a pretend-empty
    // preview about a database that is not running.
    let app = databaseless_app();
    let webview = main_webview(&app);

    let error = invoke_preview(&webview, "any-project")
        .expect_err("the preview must refuse without a database");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{error}");
    assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
}

#[test]
fn the_project_build_preview_command_is_registered_with_the_specta_builder() {
    // The §7.10 Projects-module name, registered exactly once — the
    // D-034 wiring-pin rationale: without this, a dropped registration
    // would leave `export_bindings_are_up_to_date` green while it
    // silently rewrote the committed bindings without the command.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("project_build_preview::project_build_preview")
            .count(),
        1,
        "lib.rs must register the `project_build_preview` command exactly once (§7.10, MAIN-033)"
    );
}
