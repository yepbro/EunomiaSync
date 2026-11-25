//! Integration tests of the sync pipeline on temporary Git repositories
//! in all three schemes (task T-126; spec §15.1's Integration row —
//! "the sync pipeline on temporary Git repositories in all schemes";
//! the §15.4 mapping of DoD 7, 13–15 and 17; OUT-005, OUT-011, OUT-012,
//! OUT-014, OUT-015).
//!
//! Every test drives the REAL pipeline — [`crate::commands::sync_run`]
//! over the production seams through the five §9.4 phases, a real
//! SQLite database in a temporary data root, real `.git` repositories
//! from [`TempRepos`], and the real `state://file-changed` watch
//! service live — and judges what the spec makes observable: the
//! target files on disk, the `backups/` tree, the run summary and the
//! projects' `last_synced_*` cells. The one exception is the abort
//! characterization test: it drives
//! [`crate::commands::sync_run_with`] directly, because its subject
//! (the abort clock's answer, D-197) is exactly the half the
//! production seams spell as the real clock.
//!
//! # The oracles are the spec's literals
//!
//! No expectation here is rebuilt through the builders under test
//! (`eunomia_core::target_file`, `assembly_b`, `marker_clock`, `hash`):
//! the expected bytes are spelled out in this file from ALG-020…022,
//! ALG-030…032 and decisions D-19, D-21, D-22 and D-23, so a builder
//! that drifts from the spec turns a test red instead of carrying its
//! own oracle along (Project Rule 4).
//!
//! # What pins what
//!
//! | test | spec clauses |
//! | ---- | ------------- |
//! | `a_first_sync_creates_the_missing_target_in_schemes_a_and_c_and_never_in_scheme_b` | OUT-005, §7.5 "Scheme B", D-20, T-F4, §9.4 phase 4 step 3, OUT-020 |
//! | `a_sync_fully_replaces_an_existing_a_or_c_file_and_keeps_a_copy_of_the_old_bytes` | OUT-005/015 acceptance, ALG-030 "(A only)", §7.5 "Scheme A" |
//! | `a_scheme_b_file_without_exactly_one_marker_pair_is_left_untouched_and_the_run_continues` | §7.5 "Scheme B", T-F4, OUT-006 |
//! | `a_scheme_b_file_with_bom_and_crlf_keeps_everything_outside_the_markers_byte_for_byte` | T-F12, D-22, ALG-031, D-24 |
//! | `the_file_order_is_the_library_order_not_the_creation_order_in_every_scheme` | DoD 5, OUT-001…004, ALG-020…022, LIB-035 |
//! | `the_creation_marker_matches_the_spec_template_and_closes_every_managed_block_with_one_run_timestamp` | DoD 13, DoD 14, ALG-030…032, D-23, OUT-016…018 |
//! | `scheme_a_builds_the_full_file_with_the_extra_file_lines_in_their_positions` | DoD 13, ALG-030, §7.5 "Scheme A", D-01, D-065 |
//! | `an_external_edit_is_overwritten_by_the_next_sync_and_backed_up_in_every_scheme` | OUT-015, T-F13, OUT-014 |
//! | `a_watch_event_flips_the_externally_modified_state_and_never_writes_a_file_in_any_scheme` | OUT-014, §5.7, SYS-008, T-F13 |
//! | `a_target_deleted_outside_the_app_is_recreated_in_schemes_a_and_c_while_scheme_b_skips` | SYS-009, OUT-005, §7.5 "Scheme B", OUT-020 |
//! | `library_operations_change_no_file_until_the_next_sync` | OUT-011, OUT-012, DoD 7, MAIN-011, OUT-010, ALG-020, D-11 |
//! | `a_second_sync_with_no_changes_leaves_every_file_untouched_and_makes_no_backup` | T-F9, D-24, §9.4 phase 4 step 1, OUT-020 |
//! | `last_sync_times_follow_only_the_written_and_unchanged_statuses` | DoD 17, OUT-020, DB-024, MAIN-024, MAIN-027 |
//! | `an_aborted_run_stamps_the_abort_clock_and_never_moves_the_last_success` | §9.4 phase 4 step 4 (MAIN-027), OUT-020, D-131, D-185, D-197 |
//!
//! # The "no file observer" clause
//!
//! The card lists "no file observer in the code" among the things to
//! pin. That is spec v1.0's OUT-014 ("no file watcher … There is no
//! watcher code and no conflict dialogs"); spec v1.1 (commit `16a0597`)
//! replaced it with the display-only watcher of §5.7, and the landed
//! service (`file_watch.rs`, T-102) is that watcher. So the pin here is
//! the clause the spec assigns today — "a watch event never writes a
//! file", per scheme, with the real service live — not the absence of
//! the service (DECISIONS.md D-183). The clause's other half — the
//! state is DISPLAYED, with no merge and no dialog (MAIN-028) — is UI
//! and belongs to the manual smoke checklist, not to this suite.
//!
//! # Boundaries
//!
//! The library commands of §7.10 are not landed yet, so the library
//! operations of one test go through the store writers those commands
//! will call over the same `Db::run` executor — the posture of the
//! phase-0 suite's mid-run-edit test. The run itself is always the real
//! command (one test substitutes the command's own write seam for a
//! refusal, the §15.1 fault-injection row). The harness owns its copies
//! of the mock-app shape because every sibling suite's helpers are
//! private to it; the error mappers are the consolidated seams (D-162,
//! D-171), never a new spelling.
//!
//! Watch waits are condition polls and the event assertion is a set —
//! never a pass-count slot. A fixture file written before the service
//! spawns can be delivered after the stream starts (FSEvents on
//! macOS) as one extra, silent partial pass; such a pass is
//! verdict-neutral here, so no assertion can observe it (DECISIONS.md
//! D-183).
//!
//! Two more boundaries are stated rather than implied. The creation
//! marker's zone NAME is checked for shape only (an IANA-looking token,
//! or D-23's `UTC±HH:MM` fallback); that it names the system zone is
//! `marker_clock`'s own suite's pin. And a world's teardown is
//! best-effort: the watch service task owns an `AppHandle`, so the
//! managed `Db` outlives the world, and a temp root with a file still
//! open can survive the drop (`TempDir` ignores a failed removal) — the
//! sibling suites share this shape.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use jiff::Timestamp;
use tauri::Manager;
use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
use tauri_specta::Event;

use eunomia_fs::atomic::{AtomicWriteError, WriteFailed, WriteStep, WrittenFile};
use eunomia_fs::temp_repos::{RepoRoot, TempRepos};
use eunomia_store::library::{
    ItemScope, ItemUpdate, NewAssignment, NewLibraryItem, NewVariant, VariantUpdate, assign,
    create_item, create_variant, hide_item, list_items, move_item, unassign, update_item,
    update_variant,
};
use eunomia_store::projects::{AddedVia, NewProject, Project};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use eunomia_store::sync_runs::last_success_finished_at;

use crate::commands::{
    CancelToken, SyncRunOutcome, SyncRunSeams, SyncRunSummary, projects_store_to_app_error,
};
use crate::data_dirs;
use crate::db::{Db, sqlite_error_to_app_error, sync_runs_app_error_for_tests};
use crate::dirty_recalc::library_seed_error_for_tests;
use crate::errors::AppError;
use crate::file_watch::{self, FileChangedEvent, FileWatchState, ProjectFileChange};

// ---- the spec's literals ---------------------------------------------------

/// The scheme-B marker lines, byte for byte as D-19 fixes them. Spelled
/// here rather than imported: the fixtures are files a USER pasted, so
/// a drifted production constant must fail these tests, not follow
/// them.
const BEGIN_MARKER: &str = "<!-- EUNOMIASYNC:BEGIN -->";
const END_MARKER: &str = "<!-- EUNOMIASYNC:END -->";

/// ALG-032's fixed frame: `<!-- Created by EunomiaSync … -->`.
const MARKER_PREFIX: &str = "<!-- Created by EunomiaSync ";
const MARKER_SUFFIX: &str = " -->";

/// How far the creation marker's instant may sit outside the window a
/// test observed around its run. The marker is second-granular and
/// names its offset in whole minutes — the clock truncates the zone
/// offset toward zero (`marker_clock.rs`), so an offset that carries
/// seconds (a historical LMT rule) can move the reconstructed instant
/// by less than a minute, never more. One minute covers that and
/// nothing else: a frozen stamp, a wrong field unit, or a clock shifted
/// by more than a minute lands outside.
const MARKER_SLACK_SECONDS: i64 = 60;

/// The test debounce: the production 300 ms of SYS-007 is pinned by the
/// watch service's own suite; a short window keeps this suite quick.
const DEBOUNCE: Duration = Duration::from_millis(40);

/// A quiet window for the debouncer to settle — the LOWER bound of a
/// "nothing further happens" check (a late write can only be missed,
/// never invented).
const SETTLE: Duration = Duration::from_millis(300);

/// The label of the repository group inside the `TempRepos` base.
const REPOS_LABEL: &str = "eunomia-t126";

/// The §9.2 body the standard library builds for a project assigned to
/// every one of its assigned instructions — ALG-020…022 spelled by
/// hand:
///
/// ```text
/// Preamble text.   <- the instruction before the first heading (ALG-021)
///
/// ## Wild          <- its section holds an assigned instruction (ALG-020)
///
/// Zebra text.
///
/// ### Tame
///
/// Yak text.
/// ```
///
/// `## Silent` is absent — nothing assigned to the project lives in its
/// section (ALG-020) — and no instruction NAME is in the file
/// (ALG-022).
const BODY: &str = "Preamble text.\n\n## Wild\n\nZebra text.\n\n### Tame\n\nYak text.";

/// The old managed block of a fenced scheme-B fixture.
const B_OLD_BLOCK: &str = "Old block line.\n";

/// A scheme-B target's bytes up to the begin marker: a line of the
/// user's own, then the D-19 begin marker on its line.
fn b_head() -> String {
    format!("# Notes for the team\n{BEGIN_MARKER}\n")
}

/// A scheme-B target's bytes from the end marker on: the D-19 end
/// marker, then a line of the user's own.
fn b_tail() -> String {
    format!("{END_MARKER}\n\nThanks for reading.\n")
}

/// A scheme-B target a user fenced by hand: the D-19 markers around
/// `block`, with text of their own before and after.
fn b_file(block: &str) -> String {
    format!("{}{block}{}", b_head(), b_tail())
}

/// The D-21 reading line with `<file>` bound.
fn reading_line(stem: &str, suffix: &str) -> String {
    format!(
        "If {stem}{suffix} exists in the repository root, read it before starting work and follow its instructions."
    )
}

/// §9.4 phase 1 step 2's target name: `<root>/<base_name>` for schemes
/// A and B, `<root>/<STEM>.project.md` for scheme C.
fn target_name(scheme: Scheme, base_name: BaseName) -> &'static str {
    match (scheme, base_name) {
        (Scheme::A | Scheme::B, _) => base_name.as_str(),
        (Scheme::C, BaseName::AgentsMd) => "AGENTS.project.md",
        (Scheme::C, BaseName::ClaudeMd) => "CLAUDE.project.md",
    }
}

// ---- oracles over what a run left on disk ----------------------------------

/// Splits ALG-030's form into `(managed, marker)` — the managed block
/// and the marker line that closes it — asserting the layout's own
/// facts: exactly one line break at the end, the marker on the last
/// line, exactly one empty line between. `line_break` is the text's
/// terminator: LF (D-22) or the CRLF a scheme-B file converts its block
/// to (ALG-031).
fn split_closing_marker(text: &str, line_break: &str, what: &str) -> (String, String) {
    let body = text
        .strip_suffix(line_break)
        .unwrap_or_else(|| panic!("{what}: must end with one line break, got {text:?}"));
    assert!(
        !body.ends_with(line_break),
        "{what}: exactly one line break at the end, got {text:?}"
    );
    let (before, marker) = body.rsplit_once(line_break).unwrap_or_else(|| {
        panic!("{what}: the marker line must follow the managed block, got {text:?}")
    });
    let managed = before.strip_suffix(line_break).unwrap_or_else(|| {
        panic!(
            "{what}: exactly one empty line between the managed block and the marker, got {text:?}"
        )
    });
    (managed.to_owned(), marker.to_owned())
}

/// The lengths of `text`'s digit-only groups; empty when any group is
/// empty or holds a non-digit (a malformed field never matches).
fn digit_groups(text: &str, separator: char) -> Vec<usize> {
    let groups: Vec<&str> = text.split(separator).collect();
    if groups
        .iter()
        .any(|group| group.is_empty() || !group.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Vec::new();
    }
    groups.iter().map(|group| group.len()).collect()
}

/// The instant named by one creation-marker line, after checking the
/// line against the ALG-032 template field by field:
/// `<!-- Created by EunomiaSync YYYY-MM-DD HH:MM:SS <IANA name> <offset> -->`.
///
/// The zone field must be either an IANA-looking token or D-23's
/// `UTC±HH:MM` fallback ("Without an IANA name: `UTC+01:00`") — its
/// SHAPE: that the name is the system zone's is `marker_clock`'s own
/// suite's pin, not this validator's. The instant is rebuilt from the
/// line's own date, time and offset, so the result is the moment the
/// line claims — whatever the machine's zone is.
fn marker_instant(marker: &str) -> Timestamp {
    let between = marker
        .strip_prefix(MARKER_PREFIX)
        .and_then(|rest| rest.strip_suffix(MARKER_SUFFIX))
        .unwrap_or_else(|| {
            panic!("ALG-032's frame, expected `<!-- Created by EunomiaSync … -->`, got {marker:?}")
        });
    let fields: Vec<&str> = between.split(' ').collect();
    assert_eq!(
        fields.len(),
        4,
        "ALG-032's four fields — date, time, zone, offset — got {marker:?}"
    );
    let [date, time, zone, offset] = [fields[0], fields[1], fields[2], fields[3]];
    assert_eq!(
        digit_groups(date, '-'),
        [4, 2, 2],
        "ALG-032's `YYYY-MM-DD`, got {marker:?}"
    );
    assert_eq!(
        digit_groups(time, ':'),
        [2, 2, 2],
        "ALG-032's `HH:MM:SS`, got {marker:?}"
    );
    let offset_body = offset
        .strip_prefix(['+', '-'])
        .unwrap_or_else(|| panic!("ALG-032's signed offset, got {marker:?}"));
    assert_eq!(
        digit_groups(offset_body, ':'),
        [2, 2],
        "ALG-032's `±HH:MM`, got {marker:?}"
    );
    let is_iana_name = !zone.is_empty()
        && zone.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '/' | '+')
        });
    assert!(
        is_iana_name || zone == format!("UTC{offset}"),
        "ALG-032's zone — an IANA name, or D-23's `UTC±HH:MM` fallback — got {marker:?}"
    );
    Timestamp::from_str(&format!("{date}T{time}{offset}")).unwrap_or_else(|error| {
        panic!("ALG-032's fields must name a real instant, got {marker:?}: {error}")
    })
}

/// A whole scheme-A/C target as the run left it, split into its managed
/// block and the marker line closing it.
fn split_whole_file(bytes: &[u8], what: &str) -> (String, String) {
    split_closing_marker(&text_of(bytes, what), "\n", what)
}

/// A scheme-B target as the run left it: everything outside the marker
/// pair must still be `head` and `tail` byte for byte (§7.5, D-22), and
/// the block between them must be ALG-031's form of `block` — the
/// marker line closing that block is the answer.
fn assert_b_splice(
    actual: &[u8],
    head: &str,
    tail: &str,
    block: &str,
    line_break: &str,
    what: &str,
) -> String {
    let rest = actual
        .strip_prefix(head.as_bytes())
        .unwrap_or_else(|| panic!("{what}: the bytes before the begin marker changed"));
    let middle = rest
        .strip_suffix(tail.as_bytes())
        .unwrap_or_else(|| panic!("{what}: the bytes from the end marker on changed"));
    let (managed, marker) = split_closing_marker(&text_of(middle, what), line_break, what);
    assert_eq!(
        managed, block,
        "{what}: the managed block between the marker lines"
    );
    marker
}

/// ALG-032: the marker's time is the run's own — the instant it names
/// lies inside the window observed around the run.
fn assert_marker_within_run(marker: &str, before: Timestamp, after: Timestamp) {
    let instant = marker_instant(marker);
    assert!(
        (before.as_second() - MARKER_SLACK_SECONDS..=after.as_second() + MARKER_SLACK_SECONDS)
            .contains(&instant.as_second()),
        "ALG-032: the marker names the run's own time, got {marker:?} for a run between {before} and {after}"
    );
}

/// OUT-020: a `written` project's stamp is this run's — a DB-002 UTC
/// instant (24 characters, millisecond precision) inside the run's own
/// window. Phase 3 stamps the write instant and phase 4's
/// `finished_at` is a later one, so equality is not the contract.
fn assert_stamped_in_run(stamp: Option<&str>, summary: &SyncRunSummary, what: &str) {
    let Some(stamp) = stamp else {
        panic!("{what}: the project was never stamped");
    };
    assert_eq!(
        stamp.len(),
        24,
        "{what}: DB-002's `YYYY-MM-DDTHH:MM:SS.sssZ`, got {stamp:?}"
    );
    Timestamp::from_str(stamp)
        .unwrap_or_else(|error| panic!("{what}: DB-002's shape, got {stamp:?}: {error}"));
    let finished = summary
        .finished_at
        .as_deref()
        .expect("phase 4 stamped the run's end");
    assert!(
        summary.started_at.as_str() <= stamp && stamp <= finished,
        "{what}: stamped inside its own run, got {stamp} for a run between {} and {finished}",
        summary.started_at
    );
}

// ---- the harness -----------------------------------------------------------

/// One project's write settings as §6.5 stores them for a project.
#[derive(Debug, Clone, Copy)]
struct WriteSettings {
    scheme: Scheme,
    base_name: BaseName,
    extra_local: ExtraFile,
    extra_project: ExtraFile,
}

impl WriteSettings {
    /// Both extra-file options off.
    fn plain(scheme: Scheme, base_name: BaseName) -> Self {
        Self {
            scheme,
            base_name,
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

    /// Both extra-file options on, each at its own position. The
    /// arguments follow the production signature's order —
    /// project-then-local — so a call site reads like the spec's
    /// bullets (D-152's swapped-arguments trap).
    fn with_reading_lines(
        scheme: Scheme,
        base_name: BaseName,
        extra_project: Position,
        extra_local: Position,
    ) -> Self {
        Self {
            scheme,
            base_name,
            extra_local: ExtraFile {
                enabled: true,
                position: extra_local,
            },
            extra_project: ExtraFile {
                enabled: true,
                position: extra_project,
            },
        }
    }
}

/// A project row seeded over a real repository.
struct SeededProject {
    id: String,
    root: PathBuf,
    target: PathBuf,
}

/// The ids of the standard library's items the operation tests address.
struct Library {
    xray_variant: String,
    zebra: String,
    yak: String,
    silent: String,
    mute: String,
    mute_variant: String,
}

/// One world: a mock app in the state `setup` produces (the
/// `sync_watch_app` shape of `ipc_tests.rs`), a temp data root, real
/// repositories, and the projects seeded over them in `sort_order`.
struct World {
    /// The mock app. Declared first so it drops before the temp roots;
    /// the live watch service task owns an `AppHandle` and the managed
    /// `Db`, so [`World`]'s `Drop` stops the service first (T-342's
    /// teardown seam) — without it the task outlives the world, and on
    /// Windows a temp root with a file still open survives its own drop
    /// (there the open handle blocks the removal; on Unix unlinking an
    /// open file succeeds, so only the Windows legs would notice).
    app: tauri::App<MockRuntime>,
    /// The watch command bus, held until [`go_live`] hands it to the
    /// service — a receiver nobody consumes is the D-137 hang shape.
    bus: Option<tauri::async_runtime::Receiver<file_watch::Command>>,
    data_root: tempfile::TempDir,
    /// Underscore-named: holding the fixture IS the effect — it removes
    /// the repositories with the world — nothing reads it back.
    _repos: TempRepos,
    projects: Vec<SeededProject>,
}

impl Drop for World {
    fn drop(&mut self) {
        // T-342: a live service task holds an `AppHandle` and the `Db`,
        // so stopping it (and awaiting its return) is what lets the
        // temp roots go with the world. A world that never went live
        // still owns the bus's receiver (`bus` is `Some`), where a
        // stop would wait for a consumer that does not exist — hence
        // the guard.
        if self.bus.is_none() {
            tauri::async_runtime::block_on(self.app.state::<FileWatchState>().shutdown());
        }
    }
}

impl World {
    fn project(&self, index: usize) -> &SeededProject {
        &self.projects[index]
    }

    fn data_root(&self) -> &Path {
        self.data_root.path()
    }
}

/// A project's `last_synced_*` cells, OUT-020's observable.
type Stamps = (Option<String>, Option<String>);

/// The watch service's verdicts by project id: `(external_changed,
/// file_deleted)`; an unobserved project is the clean baseline.
type Verdicts = BTreeMap<String, (bool, bool)>;

fn database_of(world: &World) -> Db {
    world.app.state::<Db>().inner().clone()
}

/// Builds the world and seeds one project per entry of `specs`, in
/// order (`sort_order` follows it). Not live yet: the caller seeds its
/// fixture files, then calls [`go_live`].
fn build(specs: &[WriteSettings]) -> World {
    let data_root = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(data_root.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database =
        Db::open(data_root.path()).expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(data_dirs::DataRoot(data_root.path().to_path_buf()));
    app.manage(crate::sync_running::SyncRunning::new());
    app.manage(crate::commands::SyncCancelSlot::new());
    app.manage(crate::availability::AvailabilityState::new());
    app.manage(crate::dirty_recalc::DirtyState::new().0);
    let (watch_state, bus) = FileWatchState::new();
    app.manage(watch_state);
    tauri_specta::Builder::<MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::availability::ProjectAvailabilityEvent,
            crate::dirty_recalc::DirtyEvent,
            FileChangedEvent,
        ])
        .mount_events(&app);
    let repos =
        TempRepos::create(specs.len(), REPOS_LABEL).expect("the repository fixture must create");
    let projects = specs
        .iter()
        .zip(repos.roots())
        .enumerate()
        .map(|(index, (spec, root))| seed_project(&app, index, *spec, root))
        .collect();
    World {
        app,
        bus: Some(bus),
        data_root,
        _repos: repos,
        projects,
    }
}

/// Registers the project over its repository with the canonical path —
/// the DB-030 spelling the target resolution contracts on (and, on
/// macOS, the `/private/var` one: the fixture canonicalizes, never the
/// raw temp spelling — D-160).
fn seed_project(
    app: &tauri::App<MockRuntime>,
    index: usize,
    spec: WriteSettings,
    root: &RepoRoot,
) -> SeededProject {
    let database = app.state::<Db>().inner().clone();
    let path = root.path.to_string_lossy().into_owned();
    let path_key = root.path_key.clone();
    let name = format!("Project {}", index + 1);
    let label = format!("P{}", index + 1);
    let id = tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::create(
            tx,
            &NewProject {
                path: &path,
                path_key: &path_key,
                name: &name,
                label: &label,
                color: "blue",
                description: "",
                added_via: AddedVia::Manual,
                scheme: spec.scheme,
                base_name: spec.base_name,
                extra_local: spec.extra_local,
                extra_project: spec.extra_project,
            },
        )
        .map(|project| project.id)
        .map_err(projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work");
    SeededProject {
        id,
        root: root.path.clone(),
        target: root.path.join(target_name(spec.scheme, spec.base_name)),
    }
}

/// Seeds the standard library in one transaction and assigns Xray,
/// Zebra and Yak to every project: each project's built body is then
/// [`BODY`]. Creation order is the library order:
/// Xray, `## Wild`, Zebra, `### Tame`, Yak, `## Silent`, Mute — Mute
/// (inside `## Silent`) is left unassigned for the operation tests.
fn seed_standard_library(world: &World) -> Library {
    let database = database_of(world);
    let project_ids: Vec<String> = world
        .projects
        .iter()
        .map(|project| project.id.clone())
        .collect();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let (xray, xray_variant) = instruction(tx, "Xray preamble", "Preamble text.")?;
        heading(tx, "Wild", 2)?;
        let (zebra, zebra_variant) = instruction(tx, "Zebra", "Zebra text.")?;
        heading(tx, "Tame", 3)?;
        let (yak, yak_variant) = instruction(tx, "Yak", "Yak text.")?;
        let silent = heading(tx, "Silent", 2)?;
        let (mute, mute_variant) = instruction(tx, "Mute", "Mute text.")?;
        for project_id in &project_ids {
            for (instruction_id, variant_id) in [
                (&xray, &xray_variant),
                (&zebra, &zebra_variant),
                (&yak, &yak_variant),
            ] {
                assign(
                    tx,
                    &NewAssignment {
                        project_id,
                        instruction_id,
                        variant_id,
                    },
                )
                .map_err(library_seed_error_for_tests)?;
            }
        }
        Ok(Library {
            xray_variant,
            zebra,
            yak,
            silent,
            mute,
            mute_variant,
        })
    }))
    .expect("seeding the standard library must work")
}

/// One active instruction with its main variant, created at the end of
/// the library: `(item id, variant id)`.
fn instruction(
    tx: &rusqlite::Transaction<'_>,
    title: &str,
    content: &str,
) -> Result<(String, String), AppError> {
    let item = create_item(tx, &NewLibraryItem::Instruction { title })
        .map_err(library_seed_error_for_tests)?;
    let variant = create_variant(
        tx,
        &NewVariant {
            instruction_id: &item.id,
            name: "Main",
            content,
            is_main: true,
        },
    )
    .map_err(library_seed_error_for_tests)?;
    Ok((item.id, variant.id))
}

/// One section heading created at the end of the library.
fn heading(tx: &rusqlite::Transaction<'_>, title: &str, level: u8) -> Result<String, AppError> {
    create_item(tx, &NewLibraryItem::Heading { title, level })
        .map(|item| item.id)
        .map_err(library_seed_error_for_tests)
}

/// Starts the real watch service behind the world and waits for its
/// startup pass. The service must be live BEFORE the first sync: a run
/// awaits the after-sync refresh's ack, and that await carries no
/// budget (the D-137 hang note). Returns the `state://file-changed`
/// wire every emitted payload lands on.
fn go_live(world: &mut World) -> Receiver<FileChangedEvent> {
    let bus = world.bus.take().expect("a world goes live exactly once");
    file_watch::spawn_service(world.app.handle().clone(), bus, DEBOUNCE)
        .expect("the watcher must start");
    let (sender, events) = std::sync::mpsc::channel();
    FileChangedEvent::listen(&world.app, move |event| {
        let _ = sender.send(event.payload);
    });
    let passes = world.app.state::<FileWatchState>();
    wait_until("the watch service's startup pass", || {
        passes.pass_count() >= 1
    });
    events
}

/// The standard world, live: one project per scheme in `sort_order`
/// (`a`, `b`, `c` are their base names, so both stems run through the
/// suite), the standard library assigned to all three, and — when
/// `b_block` is `Some` — a scheme-B target the user fenced by hand
/// around that old block; `None` leaves the scheme-B target missing.
fn three_scheme_world(
    a: BaseName,
    b: BaseName,
    c: BaseName,
    b_block: Option<&str>,
) -> (World, Library, Receiver<FileChangedEvent>) {
    let mut world = build(&[
        WriteSettings::plain(Scheme::A, a),
        WriteSettings::plain(Scheme::B, b),
        WriteSettings::plain(Scheme::C, c),
    ]);
    if let Some(block) = b_block {
        write(&world.project(1).target, b_file(block));
    }
    let library = seed_standard_library(&world);
    let events = go_live(&mut world);
    (world, library, events)
}

/// Runs the real `sync_run` command — the production seams, the whole
/// §9.4 pipeline — and answers the finished run's summary. A
/// confirmation request is a panic: with the standard library the
/// builds are never empty, so the only interactive gate (D-25) must not
/// open — the run answers "without any dialog".
fn run_sync(world: &World, confirm_empty: Option<bool>) -> SyncRunSummary {
    completed(
        tauri::async_runtime::block_on(crate::commands::sync_run(
            world.app.handle().clone(),
            confirm_empty,
        ))
        .expect("the run must answer Ok"),
    )
}

fn completed(outcome: SyncRunOutcome) -> SyncRunSummary {
    let SyncRunOutcome::Completed { summary } = outcome else {
        panic!("the run must complete, not ask for the empty-build confirmation");
    };
    summary
}

/// One library operation, committed on its own through the `Db::run`
/// executor — the way the library commands will write.
fn library_write<T: Send + 'static>(
    world: &World,
    operation: impl FnMut(&rusqlite::Transaction<'_>) -> Result<T, AppError> + Send + 'static,
) -> T {
    let database = database_of(world);
    tauri::async_runtime::block_on(database.run(operation))
        .expect("the library operation must commit")
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).expect("the file must be readable")
}

fn text_of(bytes: &[u8], what: &str) -> String {
    String::from_utf8(bytes.to_owned())
        .unwrap_or_else(|_| panic!("{what}: the app writes UTF-8 (D-22)"))
}

fn write(path: &Path, contents: impl AsRef<[u8]>) {
    fs::write(path, contents).expect("writing the file must work");
}

/// The sorted entry names of a folder.
fn entries_of(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("the folder must be listable")
        .map(|entry| {
            entry
                .expect("every entry must be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn modified_of(path: &Path) -> SystemTime {
    fs::metadata(path)
        .expect("stating the file must work")
        .modified()
        .expect("the platform reports a modification time")
}

/// Every backup copy kept for one project — the files under
/// `backups/<project_id>__<slug>/` (§5.5), their bytes, oldest first
/// (the copies are named by their UTC time).
fn backup_bytes(world: &World, index: usize) -> Vec<Vec<u8>> {
    fn collect(folder: &Path, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(folder).expect("the backups tree must be listable") {
            let path = entry.expect("every entry must be readable").path();
            if path.is_dir() {
                collect(&path, found);
            } else {
                found.push(path);
            }
        }
    }
    let root = world.data_root().join("backups");
    let prefix = format!("{}__", world.project(index).id);
    let mut paths = Vec::new();
    if root.is_dir() {
        for entry in fs::read_dir(&root).expect("the backups tree must be listable") {
            let entry = entry.expect("every entry must be readable");
            if entry.file_name().to_string_lossy().starts_with(&prefix) && entry.path().is_dir() {
                collect(&entry.path(), &mut paths);
            }
        }
    }
    paths.sort();
    paths.iter().map(|path| read(path)).collect()
}

/// One project row, read straight back. The row's presence is checked
/// OUTSIDE the executor closure on purpose: a panic inside `Db::run` is
/// remapped to an `E_DB_LOCKED` envelope with the payload in its
/// details, and the named premise would surface as the wrong layer's
/// failure.
fn project_row(world: &World, index: usize) -> Project {
    let database = database_of(world);
    let project_id = world.project(index).id.clone();
    let row = tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::get(tx, &project_id).map_err(projects_store_to_app_error)
    }))
    .expect("reading the project must work");
    row.expect("the seeded project must persist")
}

fn stamps(world: &World, index: usize) -> Stamps {
    let project = project_row(world, index);
    (project.last_synced_at, project.last_synced_hash)
}

fn run_count(world: &World) -> i64 {
    let database = database_of(world);
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.query_row("SELECT COUNT(*) FROM sync_runs", [], |row| row.get(0))
            .map_err(sqlite_error_to_app_error)
    }))
    .expect("counting the runs must work")
}

/// MAIN-027's overall time: the `finished_at` of the last `success` run.
fn last_success(world: &World) -> Option<String> {
    let database = database_of(world);
    tauri::async_runtime::block_on(
        database.run(move |tx| last_success_finished_at(tx).map_err(sync_runs_app_error_for_tests)),
    )
    .expect("reading the last successful sync must work")
}

/// The library's `(position, title)` rows — the order the build reads.
fn library_rows(world: &World) -> Vec<(i64, String)> {
    let database = database_of(world);
    let mut rows = tauri::async_runtime::block_on(database.run(move |tx| {
        list_items(tx, ItemScope::All)
            .map_err(library_seed_error_for_tests)
            .map(|items| {
                items
                    .into_iter()
                    .map(|item| (item.position, item.title))
                    .collect::<Vec<_>>()
            })
    }))
    .expect("reading the library must work");
    rows.sort();
    rows
}

/// The per-project statuses of a summary, in `sort_order`.
fn statuses(summary: &SyncRunSummary) -> Vec<&str> {
    summary
        .projects
        .iter()
        .map(|entry| entry.status.as_str())
        .collect()
}

fn assert_counters(
    summary: &SyncRunSummary,
    total: i32,
    written: i32,
    unchanged: i32,
    skipped: i32,
    failed: i32,
) {
    assert_eq!(
        (
            summary.projects_total,
            summary.projects_written,
            summary.projects_unchanged,
            summary.projects_skipped,
            summary.projects_failed,
        ),
        (total, written, unchanged, skipped, failed),
        "MAIN-024's counter chips"
    );
    assert_eq!(
        summary.projects.len(),
        usize::try_from(total).expect("a project count fits usize"),
        "one summary row per project"
    );
}

/// The bytes of every target of the world, in `sort_order`.
fn all_targets(world: &World) -> Vec<Vec<u8>> {
    world
        .projects
        .iter()
        .map(|project| read(&project.target))
        .collect()
}

/// Polls `condition` until it holds; panics after 30 s. The bound is a
/// failure ceiling — a healthy watch pass answers in milliseconds, and
/// a loaded runner only stretches the wait (D-140).
fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn file_verdicts(world: &World) -> Verdicts {
    world
        .app
        .state::<FileWatchState>()
        .snapshot()
        .into_iter()
        .map(|change| {
            (
                change.project_id,
                (change.external_changed, change.file_deleted),
            )
        })
        .collect()
}

/// Whether the watch service's recorded verdict for each `(project
/// index, external_changed, file_deleted)` triple is already the
/// expected one; an unobserved project is the clean baseline.
fn verdicts_hold(world: &World, expected: &[(usize, bool, bool)]) -> bool {
    let verdicts = file_verdicts(world);
    expected
        .iter()
        .all(|&(index, external_changed, file_deleted)| {
            verdicts
                .get(&world.project(index).id)
                .copied()
                .unwrap_or((false, false))
                == (external_changed, file_deleted)
        })
}

/// Waits until the watch service recorded the expected verdicts.
fn wait_for_verdicts(world: &World, expected: &[(usize, bool, bool)]) {
    wait_until("the watch service to record the expected verdicts", || {
        verdicts_hold(world, expected)
    });
}

/// Asserts the watch service's verdicts are ALREADY the expected ones —
/// for a service that has settled (the run's own awaited after-sync
/// pass, or a debounce window that has elapsed).
fn assert_verdicts(world: &World, expected: &[(usize, bool, bool)], what: &str) {
    assert!(
        verdicts_hold(world, expected),
        "{what}: expected {expected:?}, the service holds {:?}",
        file_verdicts(world)
    );
}

/// Everything the `state://file-changed` wire holds so far.
fn drain(events: &Receiver<FileChangedEvent>) -> Vec<ProjectFileChange> {
    let mut changes = Vec::new();
    while let Ok(event) = events.try_recv() {
        changes.extend(event.changes);
    }
    changes
}

// ---- creation and overwrite ------------------------------------------------

#[test]
fn a_first_sync_creates_the_missing_target_in_schemes_a_and_c_and_never_in_scheme_b() {
    // OUT-005: "A missing file is created (schemes A and C)". §7.5
    // "Scheme B": "The app does not create the file and does not insert
    // the markers by itself … No markers …: the project gets the
    // `skipped` status with the `E_MARKERS_INVALID` error"; T-F4: "the
    // other projects are written" — which makes the run §9.4 phase 4
    // step 3's `partial`. (The file a sync once wrote and somebody
    // then deleted is the deletion test's subject, below.)
    let mut world = build(&[
        WriteSettings::plain(Scheme::A, BaseName::AgentsMd),
        WriteSettings::plain(Scheme::B, BaseName::ClaudeMd),
        WriteSettings::plain(Scheme::C, BaseName::ClaudeMd),
    ]);
    // The scheme-C project has a main file of its own before the app
    // ever runs: D-20 — "The app does not edit the main file" — is a
    // promise about a file that exists.
    const C_MAIN_FILE: &str = "# Written by the team, never by the app.\n";
    let c_main = world.project(2).root.join("CLAUDE.md");
    write(&c_main, C_MAIN_FILE);
    seed_standard_library(&world);
    let _events = go_live(&mut world);

    let before = Timestamp::now();
    let summary = run_sync(&world, None);
    let after = Timestamp::now();

    assert_eq!(
        summary.verdict, "partial",
        "§9.4 phase 4 step 3: a skipped project beside written ones"
    );
    assert_counters(&summary, 3, 2, 0, 1, 0);
    assert_eq!(statuses(&summary), ["written", "skipped", "written"]);
    assert_eq!(
        summary.projects[1].error_code.as_deref(),
        Some("E_MARKERS_INVALID"),
        "the skip carries its §10 code"
    );

    // Scheme A: `AGENTS.md` is created as ALG-030's whole file.
    let (managed, marker) = split_whole_file(&read(&world.project(0).target), "scheme A");
    assert_eq!(managed, BODY, "ALG-020…022 over the standard library");
    assert_marker_within_run(&marker, before, after);

    // Scheme C: `<STEM>.project.md` is created, and the main file the
    // team wrote is not touched (D-20: "The app does not edit the main
    // file").
    let (managed_c, marker_c) = split_whole_file(&read(&world.project(2).target), "scheme C");
    assert_eq!(managed_c, BODY);
    assert_eq!(
        marker_c, marker,
        "ALG-032: one marker for every project of the run"
    );
    assert_eq!(
        read(&c_main),
        C_MAIN_FILE.as_bytes(),
        "scheme C leaves the existing main file byte for byte"
    );
    assert_eq!(
        entries_of(&world.project(2).root),
        [".git", "CLAUDE.md", "CLAUDE.project.md"],
        "scheme C writes its own file and creates nothing else"
    );

    // Scheme B: nothing is created — no file, no markers.
    assert_eq!(
        entries_of(&world.project(1).root),
        [".git"],
        "§7.5: the app does not create a scheme-B file"
    );
    assert!(
        fs::metadata(&world.project(1).target)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "the scheme-B target must still be missing"
    );

    // Creation has nothing to back up.
    for index in 0..3 {
        assert!(
            backup_bytes(&world, index).is_empty(),
            "project {index}: a created or skipped file makes no backup"
        );
    }

    // OUT-020 / DoD 17: the stamp follows the status.
    assert_stamped_in_run(
        stamps(&world, 0).0.as_deref(),
        &summary,
        "the written scheme-A project",
    );
    assert_stamped_in_run(
        stamps(&world, 2).0.as_deref(),
        &summary,
        "the written scheme-C project",
    );
    assert_eq!(
        stamps(&world, 1),
        (None, None),
        "a skipped project is never stamped"
    );
}

#[test]
fn a_sync_fully_replaces_an_existing_a_or_c_file_and_keeps_a_copy_of_the_old_bytes() {
    // OUT-005, 015: "An existing target file is fully replaced (except
    // the area outside the markers in scheme B)" — and the acceptance:
    // "Content added to the file by hand disappears after a sync
    // (scheme A), and a copy stays in the backup". ALG-030's "(A
    // only)": the reading lines exist in scheme A alone, so B and C
    // carry the body alone with every option enabled.
    let specs = [
        WriteSettings::with_reading_lines(
            Scheme::A,
            BaseName::ClaudeMd,
            Position::Start,
            Position::End,
        ),
        WriteSettings::with_reading_lines(
            Scheme::B,
            BaseName::AgentsMd,
            Position::Start,
            Position::Start,
        ),
        WriteSettings::with_reading_lines(
            Scheme::C,
            BaseName::AgentsMd,
            Position::End,
            Position::End,
        ),
    ];
    let old = [
        b"Hand-written notes.\nPlease keep me.\n".to_vec(),
        b_file(B_OLD_BLOCK).into_bytes(),
        b"Old project file.\n".to_vec(),
    ];
    let mut world = build(&specs);
    for (project, bytes) in world.projects.iter().zip(&old) {
        write(&project.target, bytes);
    }
    seed_standard_library(&world);
    let _events = go_live(&mut world);

    let summary = run_sync(&world, None);
    assert_eq!(summary.verdict, "success");
    assert_eq!(statuses(&summary), ["written", "written", "written"]);

    // Scheme A — the full ALG-030 composition on disk: the start lines,
    // the body, the end lines, the marker. The hand-written text is
    // gone.
    let (managed_a, marker) =
        split_whole_file(&read(&world.project(0).target), "the scheme-A target");
    assert_eq!(
        managed_a,
        format!(
            "{}\n\n{BODY}\n\n{}",
            reading_line("CLAUDE", ".project.md"),
            reading_line("CLAUDE", ".local.md")
        ),
        "ALG-030: the start lines, the body, the end lines, one empty line between"
    );
    marker_instant(&marker);
    let (managed_c, _) = split_whole_file(&read(&world.project(2).target), "the scheme-C target");
    assert_eq!(
        managed_c, BODY,
        "ALG-030's reading lines are scheme A's alone"
    );
    assert_b_splice(
        &read(&world.project(1).target),
        &b_head(),
        &b_tail(),
        BODY,
        "\n",
        "the scheme-B target",
    );

    // The overwritten bytes survive as exactly one backup copy each.
    for (index, bytes) in old.iter().enumerate() {
        assert_eq!(
            backup_bytes(&world, index),
            [bytes.as_slice()],
            "project {index}: one copy of the overwritten bytes"
        );
    }
}

#[test]
fn a_scheme_b_file_without_exactly_one_marker_pair_is_left_untouched_and_the_run_continues() {
    // §7.5 "Scheme B": "The target file must exist and contain exactly
    // one pair of markers from D-19 … No markers, several of them,
    // reversed order, or one of them missing: the project gets the
    // `skipped` status with the `E_MARKERS_INVALID` error, nothing is
    // written, and the other projects continue" — T-F4's three shapes
    // plus the two missing halves. "Nothing is written" is a promise
    // about the user's file: the app never inserts the markers by
    // itself (G5), so every refused file must stay byte for byte.
    let refused = [
        "Just notes.\nNo markers in this file.\n".to_owned(),
        format!("{BEGIN_MARKER}\nfirst\n{END_MARKER}\n{BEGIN_MARKER}\nsecond\n{END_MARKER}\n"),
        format!("{END_MARKER}\nold block\n{BEGIN_MARKER}\n"),
        format!("{BEGIN_MARKER}\nold block\n"),
        format!("old block\n{END_MARKER}\n"),
    ];
    let mut specs = vec![WriteSettings::plain(Scheme::A, BaseName::AgentsMd)];
    specs.extend(std::iter::repeat_n(
        WriteSettings::plain(Scheme::B, BaseName::ClaudeMd),
        refused.len(),
    ));
    let mut world = build(&specs);
    for (project, bytes) in world.projects.iter().skip(1).zip(&refused) {
        write(&project.target, bytes);
    }
    seed_standard_library(&world);
    let _events = go_live(&mut world);

    let summary = run_sync(&world, None);

    // The promise about the user's file comes first: every refused
    // file is byte for byte what the user left — no markers inserted,
    // no block spliced in — and nothing was copied or stamped for it.
    for index in 1..=refused.len() {
        assert_eq!(
            read(&world.project(index).target),
            refused[index - 1].as_bytes(),
            "project {index}: nothing is written into a refused scheme-B file"
        );
        assert!(
            backup_bytes(&world, index).is_empty(),
            "project {index}: a refused file makes no backup"
        );
        assert_eq!(
            stamps(&world, index),
            (None, None),
            "project {index}: a skipped project is never stamped"
        );
    }
    // Each refusal is the `skipped` row with its §10 code, the run is
    // `partial`, and the other project is written (T-F4).
    assert_eq!(
        summary.verdict, "partial",
        "§9.4 phase 4 step 3: five skipped projects beside a written one"
    );
    assert_counters(&summary, 6, 1, 0, 5, 0);
    assert_eq!(
        statuses(&summary),
        [
            "written", "skipped", "skipped", "skipped", "skipped", "skipped"
        ]
    );
    for entry in &summary.projects[1..] {
        assert_eq!(
            entry.error_code.as_deref(),
            Some("E_MARKERS_INVALID"),
            "the skip carries its §10 code"
        );
    }
    let (managed, _) = split_whole_file(&read(&world.project(0).target), "scheme A");
    assert_eq!(managed, BODY, "T-F4: the other projects are written");
}

#[test]
fn a_scheme_b_file_with_bom_and_crlf_keeps_everything_outside_the_markers_byte_for_byte() {
    // T-F12: "A scheme B file with BOM and CRLF — the area outside the
    // markers is kept byte for byte." D-22: "In scheme B, the file's
    // BOM and line breaks are kept for the content outside the block";
    // ALG-031: "The block's line breaks are converted to the ones most
    // used in the file". And D-24: the file this run writes must read
    // back as matching, or the next sync would rewrite an unchanged
    // file.
    let head = format!(
        "\u{feff}# Team notes\r\n\r\nText before the block, with trailing spaces.   \r\n{BEGIN_MARKER}\r\n"
    );
    let tail = format!("{END_MARKER}\r\n\r\nText after the block.\r\n");
    let old_block = "Old managed line.\r\n\r\n<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\r\n";
    let mut world = build(&[WriteSettings::plain(Scheme::B, BaseName::AgentsMd)]);
    let original = format!("{head}{old_block}{tail}");
    write(&world.project(0).target, &original);
    seed_standard_library(&world);
    let _events = go_live(&mut world);

    let before = Timestamp::now();
    let summary = run_sync(&world, None);
    let after = Timestamp::now();
    assert_eq!(statuses(&summary), ["written"]);

    let written = read(&world.project(0).target);
    let marker = assert_b_splice(
        &written,
        &head,
        &tail,
        &BODY.replace('\n', "\r\n"),
        "\r\n",
        "the T-F12 target after the sync",
    );
    assert_marker_within_run(&marker, before, after);
    assert_eq!(
        backup_bytes(&world, 0),
        [original.into_bytes()],
        "the copy holds the file as it was, BOM and CRLF included"
    );

    // The second sync finds the file matching by ALG-033: not
    // overwritten, not backed up again.
    let again = run_sync(&world, None);
    assert_eq!(again.verdict, "success");
    assert_counters(&again, 1, 0, 1, 0, 0);
    assert_eq!(
        read(&world.project(0).target),
        written,
        "D-24: the unchanged CRLF file is not overwritten"
    );
    assert_eq!(backup_bytes(&world, 0).len(), 1, "no second copy");
}

// ---- order, marker, the full scheme-A build --------------------------------

#[test]
fn the_file_order_is_the_library_order_not_the_creation_order_in_every_scheme() {
    // DoD 5 / OUT-001…004: "The order in the file equals the order in
    // the library". Xray is created first and Zebra third; a library
    // reorder (LIB-004) moves Zebra to position 1, and the next sync
    // must follow the position — not the creation order, not the
    // alphabetical one.
    const ZEBRA_FIRST: &str = "Zebra text.\n\nPreamble text.\n\n## Wild\n\n### Tame\n\nYak text.";
    let (world, library, _events) = three_scheme_world(
        BaseName::AgentsMd,
        BaseName::ClaudeMd,
        BaseName::ClaudeMd,
        Some(B_OLD_BLOCK),
    );

    let first = run_sync(&world, None);
    assert_eq!(statuses(&first), ["written", "written", "written"]);
    let (managed, _) = split_whole_file(&read(&world.project(0).target), "scheme A");
    assert_eq!(managed, BODY, "the baseline: the creation order");

    let zebra = library.zebra.clone();
    library_write(&world, move |tx| {
        move_item(tx, &zebra, 1)
            .map(|_| ())
            .map_err(library_seed_error_for_tests)
    });
    assert_eq!(
        library_rows(&world)[0].1,
        "Zebra",
        "the reorder was saved right away"
    );

    let second = run_sync(&world, None);
    assert_eq!(statuses(&second), ["written", "written", "written"]);
    let (managed, _) = split_whole_file(
        &read(&world.project(0).target),
        "scheme A after the reorder",
    );
    assert_eq!(
        managed, ZEBRA_FIRST,
        "the file follows the library position"
    );
    assert_b_splice(
        &read(&world.project(1).target),
        &b_head(),
        &b_tail(),
        ZEBRA_FIRST,
        "\n",
        "scheme B after the reorder",
    );
    let (managed, _) = split_whole_file(
        &read(&world.project(2).target),
        "scheme C after the reorder",
    );
    assert_eq!(managed, ZEBRA_FIRST);
}

#[test]
fn the_creation_marker_matches_the_spec_template_and_closes_every_managed_block_with_one_run_timestamp()
 {
    // DoD 13 ("Full build with marker") and DoD 14 ("marker at the end
    // of the block in scheme B"); ALG-030…032; D-23; OUT-016…018 ("A
    // creation marker at the end of each built content, local time
    // with time zone").
    let (world, _library, _events) = three_scheme_world(
        BaseName::AgentsMd,
        BaseName::ClaudeMd,
        BaseName::AgentsMd,
        Some(B_OLD_BLOCK),
    );

    let before = Timestamp::now();
    let summary = run_sync(&world, None);
    let after = Timestamp::now();
    assert_eq!(statuses(&summary), ["written", "written", "written"]);

    // Schemes A and C (ALG-030): the marker is the file's last line.
    let a_text = text_of(&read(&world.project(0).target), "scheme A");
    let c_text = text_of(&read(&world.project(2).target), "scheme C");
    let (_, marker_a) = split_closing_marker(&a_text, "\n", "scheme A");
    let (_, marker_c) = split_closing_marker(&c_text, "\n", "scheme C");

    // Scheme B (ALG-031, DoD 14): the marker closes the managed block,
    // on the line right before the end marker.
    let b_text = text_of(&read(&world.project(1).target), "scheme B");
    let lines: Vec<&str> = b_text.lines().collect();
    let end_at = lines
        .iter()
        .position(|line| *line == END_MARKER)
        .expect("the end marker is still there");
    let marker_b = lines[end_at - 1];

    for text in [&a_text, &b_text, &c_text] {
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with(MARKER_PREFIX))
                .count(),
            1,
            "exactly one creation marker per file, got {text:?}"
        );
    }
    assert_marker_within_run(marker_b, before, after);
    assert_eq!(
        marker_a, marker_b,
        "ALG-032: one time per run, the same for all projects"
    );
    assert_eq!(marker_c, marker_b);
}

#[test]
fn scheme_a_builds_the_full_file_with_the_extra_file_lines_in_their_positions() {
    // DoD 13 — scheme A's full build (ALG-030: "the extra-file reading
    // lines with position `start` (A only), the body (if not empty),
    // and the reading lines with position `end` (A only)"). §7.5
    // "Scheme A": "If both lines are in the same position, the line for
    // `.project.md` goes first, then the one for `.local.md`" — one
    // block of adjacent lines (D-065). D-01: "The app does not create,
    // read, or change these files."
    let specs = [
        WriteSettings::with_reading_lines(
            Scheme::A,
            BaseName::AgentsMd,
            Position::End,
            Position::End,
        ),
        WriteSettings::with_reading_lines(
            Scheme::A,
            BaseName::ClaudeMd,
            Position::Start,
            Position::Start,
        ),
        WriteSettings::with_reading_lines(
            Scheme::A,
            BaseName::AgentsMd,
            Position::End,
            Position::Start,
        ),
    ];
    let mut world = build(&specs);
    // Project 0 already has both extra files of its stem before the app
    // runs: D-01 — "The app does not create, read, or change these
    // files" — is a promise about files that exist.
    const TEAM_OWNED: &str = "# The team's own file; the app never writes here.\n";
    let extra_files = [
        world.project(0).root.join("AGENTS.project.md"),
        world.project(0).root.join("AGENTS.local.md"),
    ];
    for path in &extra_files {
        write(path, TEAM_OWNED);
    }
    seed_standard_library(&world);
    let _events = go_live(&mut world);

    let summary = run_sync(&world, None);
    assert_eq!(summary.verdict, "success");
    assert_eq!(statuses(&summary), ["written", "written", "written"]);
    for path in &extra_files {
        assert_eq!(
            read(path),
            TEAM_OWNED.as_bytes(),
            "D-01: the app does not change an extra file that already exists"
        );
    }

    let expected_managed = [
        format!(
            "{BODY}\n\n{}\n{}",
            reading_line("AGENTS", ".project.md"),
            reading_line("AGENTS", ".local.md")
        ),
        format!(
            "{}\n{}\n\n{BODY}",
            reading_line("CLAUDE", ".project.md"),
            reading_line("CLAUDE", ".local.md")
        ),
        format!(
            "{}\n\n{BODY}\n\n{}",
            reading_line("AGENTS", ".local.md"),
            reading_line("AGENTS", ".project.md")
        ),
    ];
    let expected_names = ["AGENTS.md", "CLAUDE.md", "AGENTS.md"];
    for index in 0..3 {
        let what = format!("scheme-A project {index}");
        let (managed, marker) = split_whole_file(&read(&world.project(index).target), &what);
        assert_eq!(
            managed, expected_managed[index],
            "{what}: ALG-030's composition"
        );
        marker_instant(&marker);
        let mut expected_entries = vec![".git"];
        if index == 0 {
            expected_entries.extend(["AGENTS.local.md", "AGENTS.md", "AGENTS.project.md"]);
        } else {
            expected_entries.push(expected_names[index]);
        }
        assert_eq!(
            entries_of(&world.project(index).root),
            expected_entries,
            "{what}: only the target is written — the reading lines name files the app never creates (D-01)"
        );
    }
}

// ---- external edits and the watcher ----------------------------------------

#[test]
fn an_external_edit_is_overwritten_by_the_next_sync_and_backed_up_in_every_scheme() {
    // T-F13, second sentence — "the next sync overwrites the file and
    // makes a backup" — and OUT-015: "The next sync overwrites them as
    // before". OUT-014's acceptance ends "a sync still overwrites the
    // file and makes a backup" — without any dialog: the run answers
    // its summary directly. In scheme B only the marked block is the
    // app's: a hand-added line after the end marker survives the sync.
    let (world, _library, _events) = three_scheme_world(
        BaseName::AgentsMd,
        BaseName::ClaudeMd,
        BaseName::ClaudeMd,
        Some(B_OLD_BLOCK),
    );
    let first = run_sync(&world, None);
    assert_eq!(statuses(&first), ["written", "written", "written"]);

    // The external edits, completely outside the app.
    let a_external = b"Edited by hand in an editor.\n".to_vec();
    let b_external = format!(
        "{}Hand-added footer.\n",
        text_of(&read(&world.project(1).target), "scheme B")
            .replace("Zebra text.", "Zebra text, edited by hand.")
    )
    .into_bytes();
    let c_external = b"Changed outside the app.\n\nBye.\n".to_vec();
    write(&world.project(0).target, &a_external);
    write(&world.project(1).target, &b_external);
    write(&world.project(2).target, &c_external);

    // T-F13's first sentence — "the project shows the externally
    // modified state" — through the service's own verdicts.
    wait_for_verdicts(
        &world,
        &[(0, true, false), (1, true, false), (2, true, false)],
    );

    let second = run_sync(&world, None);
    assert_eq!(second.verdict, "success");
    assert_eq!(statuses(&second), ["written", "written", "written"]);

    // A and C: the hand-written content is gone, the build is back.
    let (managed, _) = split_whole_file(
        &read(&world.project(0).target),
        "scheme A after the overwrite",
    );
    assert_eq!(managed, BODY);
    let (managed, _) = split_whole_file(
        &read(&world.project(2).target),
        "scheme C after the overwrite",
    );
    assert_eq!(managed, BODY);
    // B: the edited block is rebuilt; the footer outside the markers
    // survives (OUT-005's "except the area outside the markers").
    assert_b_splice(
        &read(&world.project(1).target),
        &b_head(),
        &format!("{}Hand-added footer.\n", b_tail()),
        BODY,
        "\n",
        "scheme B after the overwrite",
    );

    // … and makes a backup — of the externally edited bytes. A and C
    // had no file before their first sync, so this is their only copy;
    // B's fenced file was already copied by the first run, so the new
    // copy lands beside that one.
    assert_eq!(backup_bytes(&world, 0), [a_external]);
    assert_eq!(backup_bytes(&world, 2), [c_external]);
    let mut b_copies = backup_bytes(&world, 1);
    assert_eq!(
        b_copies.pop(),
        Some(b_external),
        "scheme B: the newest copy holds the externally edited bytes"
    );
    assert_eq!(
        b_copies,
        [b_file(B_OLD_BLOCK).into_bytes()],
        "scheme B: the first run's copy of the fenced file stays"
    );

    // The run's own after-sync pass cleared the state — it is awaited
    // inside the run, so the clearing is already on record.
    assert_verdicts(
        &world,
        &[(0, false, false), (1, false, false), (2, false, false)],
        "the run's own after-sync pass cleared the state",
    );
}

#[test]
fn a_watch_event_flips_the_externally_modified_state_and_never_writes_a_file_in_any_scheme() {
    // OUT-014 / §5.7: "Watching is display-only: a watch event never
    // writes a file, and there is no merge and no conflict dialog";
    // SYS-008: "A project is externally modified when its target file
    // on disk does not match the project's current build"; the
    // acceptance: "An external edit flips the state without any
    // dialog". Pinned per scheme with the real service live.
    let (world, _library, events) = three_scheme_world(
        BaseName::AgentsMd,
        BaseName::ClaudeMd,
        BaseName::ClaudeMd,
        Some(B_OLD_BLOCK),
    );
    run_sync(&world, None);
    let synced = all_targets(&world);
    // The run's own writes raised watch events too. Let the debouncer
    // settle on them: a settled service reads every project clean, and
    // the files are exactly what the run left — those events wrote
    // nothing back either.
    std::thread::sleep(SETTLE);
    assert_verdicts(
        &world,
        &[(0, false, false), (1, false, false), (2, false, false)],
        "the watch events of the run's own writes changed no verdict",
    );
    assert_eq!(
        all_targets(&world),
        synced,
        "the watch events of the run's own writes wrote nothing back"
    );
    // Clear the wire of everything the first run reported.
    drain(&events);
    let backups: Vec<Vec<Vec<u8>>> = (0..3).map(|index| backup_bytes(&world, index)).collect();
    let stamps_before: Vec<Stamps> = (0..3).map(|index| stamps(&world, index)).collect();
    let runs_before = run_count(&world);

    // The external edits, completely outside the app: whole files for A
    // and C; inside the block for B.
    let external = [
        b"Edited by hand in an editor.\n".to_vec(),
        text_of(&read(&world.project(1).target), "scheme B")
            .replace("Yak text.", "Yak text, tweaked outside.")
            .into_bytes(),
        b"Changed outside the app.\n\nBye.\n".to_vec(),
    ];
    for (index, bytes) in external.iter().enumerate() {
        write(&world.project(index).target, bytes);
    }

    wait_for_verdicts(
        &world,
        &[(0, true, false), (1, true, false), (2, true, false)],
    );
    let seen: BTreeSet<(String, bool, bool)> = drain(&events)
        .into_iter()
        .map(|change| {
            (
                change.project_id,
                change.external_changed,
                change.file_deleted,
            )
        })
        .collect();
    assert_eq!(
        seen,
        BTreeSet::from([
            (world.project(0).id.clone(), true, false),
            (world.project(1).id.clone(), true, false),
            (world.project(2).id.clone(), true, false),
        ]),
        "SYS-008: the `state://file-changed` wire carries exactly the three flips"
    );

    // Display-only: the files still hold the external bytes — nothing
    // merged in, nothing written back — also after the debouncer had
    // every chance to react.
    std::thread::sleep(SETTLE);
    assert_eq!(
        all_targets(&world),
        external,
        "a watch event never writes a file"
    );
    assert!(
        drain(&events).is_empty(),
        "the settled verdicts emit nothing more"
    );
    // … and the service starts no sync: no run, no copy, no stamp.
    assert_eq!(
        run_count(&world),
        runs_before,
        "a watch event never runs a sync"
    );
    for index in 0..3 {
        assert_eq!(
            backup_bytes(&world, index),
            backups[index],
            "project {index}: no new copy"
        );
        assert_eq!(
            stamps(&world, index),
            stamps_before[index],
            "project {index}: the sync stamp is untouched"
        );
    }
}

#[test]
fn a_target_deleted_outside_the_app_is_recreated_in_schemes_a_and_c_while_scheme_b_skips() {
    // SYS-009: "If the target file is deleted externally, the project
    // shows a 'target deleted' hint. The next sync recreates the file
    // in schemes A and C; scheme B skips it with `E_MARKERS_INVALID`
    // as usual" — over files a sync once wrote, which is a different
    // state from the creation test's never-written target. OUT-005
    // covers it — a missing A/C target is written, not skipped — and a
    // deleted file has nothing to back up. OUT-020: the skipped scheme-B
    // project keeps the sync state the run that wrote it left.
    let (world, _library, events) = three_scheme_world(
        BaseName::AgentsMd,
        BaseName::ClaudeMd,
        BaseName::ClaudeMd,
        Some(B_OLD_BLOCK),
    );
    let first = run_sync(&world, None);
    assert_eq!(statuses(&first), ["written", "written", "written"]);
    let b_stamps = stamps(&world, 1);
    assert!(
        b_stamps.0.is_some(),
        "the premise: scheme B was stamped by the run that wrote it"
    );
    std::thread::sleep(SETTLE);
    drain(&events);

    // The deletions, completely outside the app.
    for project in &world.projects {
        fs::remove_file(&project.target).expect("the external deletion must work");
    }
    // SYS-009's first sentence — the hint — through the service's own
    // verdicts; the watcher recreates nothing itself.
    wait_for_verdicts(
        &world,
        &[(0, false, true), (1, false, true), (2, false, true)],
    );
    for project in &world.projects {
        assert!(
            fs::metadata(&project.target)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
            "a watch event never writes a file — not a deleted one back"
        );
    }

    let before = Timestamp::now();
    let second = run_sync(&world, None);
    let after = Timestamp::now();
    assert_eq!(
        second.verdict, "partial",
        "§9.4 phase 4 step 3: the skipped scheme-B project beside two written ones"
    );
    assert_eq!(statuses(&second), ["written", "skipped", "written"]);
    assert_eq!(
        second.projects[1].error_code.as_deref(),
        Some("E_MARKERS_INVALID"),
        "scheme B skips it as usual: the app does not create the file"
    );
    for index in [0, 2] {
        let what = format!("the recreated target of project {index}");
        let (managed, marker) = split_whole_file(&read(&world.project(index).target), &what);
        assert_eq!(managed, BODY, "{what}: the build again");
        assert_marker_within_run(&marker, before, after);
        assert_stamped_in_run(stamps(&world, index).0.as_deref(), &second, &what);
        assert!(
            backup_bytes(&world, index).is_empty(),
            "{what}: a deleted file has nothing to back up"
        );
    }
    assert!(
        fs::metadata(&world.project(1).target)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "the scheme-B target stays missing"
    );
    assert_eq!(
        stamps(&world, 1),
        b_stamps,
        "OUT-020: a skipped project keeps the sync state of the run that wrote it"
    );
    // The run's own after-sync pass cleared the hint of the recreated
    // targets; scheme B keeps its hint.
    assert_verdicts(
        &world,
        &[(0, false, false), (1, false, true), (2, false, false)],
        "the recreated targets read clean again, scheme B keeps its hint",
    );
}

// ---- library operations, unchanged runs, the sync times --------------------

#[test]
fn library_operations_change_no_file_until_the_next_sync() {
    // DoD 7 — "Saved right away, files not changed" — over OUT-011,
    // 012 ("Files change only with the 'Run sync' command"), MAIN-011
    // / OUT-010 ("Any change is saved to the DB right away; project
    // files are not touched") and G5. Seven library operations change
    // the builds — six of all three schemes, one of scheme C alone;
    // the files must not move before the next sync, and that sync must
    // apply every one of them — which is what proves the operations
    // really changed something.
    let (world, library, _events) = three_scheme_world(
        BaseName::AgentsMd,
        BaseName::ClaudeMd,
        BaseName::AgentsMd,
        Some(B_OLD_BLOCK),
    );
    run_sync(&world, None);
    let targets_before = all_targets(&world);
    let modified_before: Vec<SystemTime> = world
        .projects
        .iter()
        .map(|project| modified_of(&project.target))
        .collect();
    let backups_before: Vec<Vec<Vec<u8>>> =
        (0..3).map(|index| backup_bytes(&world, index)).collect();
    let runs_before = run_count(&world);

    // 1. The text of Xray's main variant is edited.
    let xray_variant = library.xray_variant.clone();
    library_write(&world, move |tx| {
        update_variant(
            tx,
            &xray_variant,
            &VariantUpdate {
                name: None,
                content: Some("Preamble text, revised."),
            },
        )
        .map(|_| ())
        .map_err(library_seed_error_for_tests)
    });
    // 2. Mute — the unassigned instruction inside `## Silent` — is
    //    assigned to all three projects; 3. Zebra is unassigned from
    //    all three (one transaction each, the bulk-operation rule).
    let ids: Vec<String> = world
        .projects
        .iter()
        .map(|project| project.id.clone())
        .collect();
    let mute = library.mute.clone();
    let mute_variant = library.mute_variant.clone();
    library_write(&world, move |tx| {
        for project_id in &ids {
            assign(
                tx,
                &NewAssignment {
                    project_id,
                    instruction_id: &mute,
                    variant_id: &mute_variant,
                },
            )
            .map_err(library_seed_error_for_tests)?;
        }
        Ok(())
    });
    let ids: Vec<String> = world
        .projects
        .iter()
        .map(|project| project.id.clone())
        .collect();
    let zebra = library.zebra.clone();
    library_write(&world, move |tx| {
        for project_id in &ids {
            unassign(tx, project_id, &zebra).map_err(library_seed_error_for_tests)?;
        }
        Ok(())
    });
    // 4. Yak is hidden (D-11: out of the build, assignments kept);
    //    5. Mute moves up into the Tame section; 6. the `## Silent`
    //    heading is renamed.
    let yak = library.yak.clone();
    library_write(&world, move |tx| {
        hide_item(tx, &yak)
            .map(|_| ())
            .map_err(library_seed_error_for_tests)
    });
    let mute = library.mute.clone();
    library_write(&world, move |tx| {
        move_item(tx, &mute, 5)
            .map(|_| ())
            .map_err(library_seed_error_for_tests)
    });
    let silent = library.silent.clone();
    library_write(&world, move |tx| {
        update_item(
            tx,
            &silent,
            &ItemUpdate {
                title: Some("Release"),
                heading_level: None,
            },
        )
        .map(|_| ())
        .map_err(library_seed_error_for_tests)
    });
    // 7. A new instruction is created at the end — inside `## Release`
    //    now — and assigned to the scheme-C project alone.
    let c_id = world.project(2).id.clone();
    library_write(&world, move |tx| {
        let (novel, novel_variant) = instruction(tx, "Novel", "Novel text.")?;
        assign(
            tx,
            &NewAssignment {
                project_id: &c_id,
                instruction_id: &novel,
                variant_id: &novel_variant,
            },
        )
        .map_err(library_seed_error_for_tests)?;
        Ok(())
    });

    // Saved right away: the library reads back in its new order …
    let rows = library_rows(&world);
    let titles: Vec<&str> = rows.iter().map(|(_, title)| title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Xray preamble",
            "Wild",
            "Zebra",
            "Tame",
            "Mute",
            "Yak",
            "Release",
            "Novel"
        ],
        "MAIN-011: every change is in the database"
    );
    // … and no project file moved: same bytes, same modification time,
    // no run, no copy.
    assert_eq!(
        all_targets(&world),
        targets_before,
        "OUT-011: files change only with a sync"
    );
    for index in 0..3 {
        assert_eq!(
            modified_of(&world.project(index).target),
            modified_before[index],
            "project {index}: not even rewritten with the same bytes"
        );
        assert_eq!(backup_bytes(&world, index), backups_before[index]);
    }
    assert_eq!(run_count(&world), runs_before, "no run happened");

    // The next sync applies every operation. ALG-020 over the new
    // library: `## Wild` keeps its nested `### Tame`, which now holds
    // the assigned Mute; the hidden Yak and the unassigned Zebra are
    // out (D-11, ALG-020); `## Release` has an assigned instruction in
    // the scheme-C project alone.
    const REVISED: &str = "Preamble text, revised.\n\n## Wild\n\n### Tame\n\nMute text.";
    let second = run_sync(&world, None);
    assert_eq!(statuses(&second), ["written", "written", "written"]);
    let (managed, _) = split_whole_file(
        &read(&world.project(0).target),
        "scheme A after the library changes",
    );
    assert_eq!(managed, REVISED);
    assert_b_splice(
        &read(&world.project(1).target),
        &b_head(),
        &b_tail(),
        REVISED,
        "\n",
        "scheme B after the library changes",
    );
    let (managed, _) = split_whole_file(
        &read(&world.project(2).target),
        "scheme C after the library changes",
    );
    assert_eq!(managed, format!("{REVISED}\n\n## Release\n\nNovel text."));
}

#[test]
fn a_second_sync_with_no_changes_leaves_every_file_untouched_and_makes_no_backup() {
    // T-F9 — "A second sync with no changes: All projects `unchanged`;
    // no backups created" — and D-24: "A file that did not change
    // compared to the new build (ignoring the marker line) is not
    // overwritten and not backed up". §9.4 phase 4 step 1: the
    // `unchanged` projects' stamps are updated all the same.
    let (world, _library, _events) = three_scheme_world(
        BaseName::AgentsMd,
        BaseName::ClaudeMd,
        BaseName::ClaudeMd,
        Some(B_OLD_BLOCK),
    );
    let first = run_sync(&world, None);
    assert_eq!(statuses(&first), ["written", "written", "written"]);
    let targets_before = all_targets(&world);
    let modified_before: Vec<SystemTime> = world
        .projects
        .iter()
        .map(|project| modified_of(&project.target))
        .collect();
    let backups_before: Vec<Vec<Vec<u8>>> =
        (0..3).map(|index| backup_bytes(&world, index)).collect();
    let stamps_before: Vec<Stamps> = (0..3).map(|index| stamps(&world, index)).collect();
    assert_eq!(
        backups_before[1].len(),
        1,
        "the first run backed up the one file it overwrote"
    );

    let second = run_sync(&world, None);
    assert_eq!(second.verdict, "success");
    assert_counters(&second, 3, 0, 3, 0, 0);
    for index in 0..3 {
        assert_eq!(
            read(&world.project(index).target),
            targets_before[index],
            "project {index}: not overwritten — not even the marker's timestamp"
        );
        assert_eq!(
            modified_of(&world.project(index).target),
            modified_before[index],
            "project {index}: the file was not rewritten"
        );
        assert_eq!(
            backup_bytes(&world, index),
            backups_before[index],
            "project {index}: no backup created"
        );
        let (stamped_at, stamped_hash) = stamps(&world, index);
        assert_eq!(
            stamped_at.as_deref(),
            second.finished_at.as_deref(),
            "project {index}: §9.4 phase 4 step 1 stamps the unchanged project"
        );
        assert_eq!(
            stamped_hash, stamps_before[index].1,
            "project {index}: the build never changed"
        );
    }
}

#[test]
fn last_sync_times_follow_only_the_written_and_unchanged_statuses() {
    // DoD 17 ("Last sync times") over OUT-020: `last_synced_at` and
    // `last_synced_hash` "are updated only for the `written` and
    // `unchanged` statuses — A skipped or failed project keeps its
    // previous sync state (MAIN-024, S7)". MAIN-027: the overall time
    // is the `finished_at` of the last `success` run — "For `partial`,
    // the overall time is not updated".
    let (world, _library, _events) = three_scheme_world(
        BaseName::AgentsMd,
        BaseName::ClaudeMd,
        BaseName::AgentsMd,
        None,
    );

    // Run 1: the scheme-B target is missing, so B is skipped while A
    // and C are written.
    let first = run_sync(&world, None);
    assert_eq!(statuses(&first), ["written", "skipped", "written"]);
    let a1 = stamps(&world, 0);
    let c1 = stamps(&world, 2);
    assert_stamped_in_run(a1.0.as_deref(), &first, "the written scheme-A project");
    assert_stamped_in_run(c1.0.as_deref(), &first, "the written scheme-C project");
    assert_eq!(
        stamps(&world, 1),
        (None, None),
        "the skipped project is never stamped"
    );
    assert_eq!(
        last_success(&world),
        None,
        "MAIN-027: a `partial` run is nobody's last successful sync"
    );

    // Between the runs, outside the app: B gets a valid marked file,
    // and C's file is edited.
    write(&world.project(1).target, b_file(B_OLD_BLOCK));
    let c_external = b"Changed outside the app.\n".to_vec();
    write(&world.project(2).target, &c_external);

    // Run 2 through the command's own seam: C's write refuses (the
    // §15.1 fault-injection row — a refusing write), everything else
    // is the production pipeline.
    let refusing_c = Arc::new(
        |target: &Path, contents: &[u8]| -> Result<WrittenFile, AtomicWriteError> {
            if target
                .file_name()
                .is_some_and(|name| name == "AGENTS.project.md")
            {
                Err(AtomicWriteError::WriteFailed(WriteFailed {
                    step: WriteStep::Replace,
                    path: target.to_path_buf(),
                    source: std::io::Error::other("injected by the test"),
                    cleanup: None,
                }))
            } else {
                crate::sync_phase3::production_write(target, contents)
            }
        },
    );
    let seams = SyncRunSeams {
        backup: Arc::new(eunomia_fs::backup::backup_target_file),
        write: refusing_c,
        abort_clock: SyncRunSeams::production().abort_clock,
    };
    let second = completed(
        tauri::async_runtime::block_on(crate::commands::sync_run_with(
            world.app.handle().clone(),
            None,
            CancelToken::new(),
            seams,
        ))
        .expect("the run must answer Ok"),
    );
    assert_eq!(second.verdict, "partial");
    assert_eq!(statuses(&second), ["unchanged", "written", "failed"]);
    assert_eq!(
        second.projects[2].error_code.as_deref(),
        Some("E_WRITE_FAILED"),
        "§10: a refused write is `E_WRITE_FAILED`"
    );
    let a2 = stamps(&world, 0);
    let b2 = stamps(&world, 1);
    let c2 = stamps(&world, 2);
    assert_eq!(
        a2.0.as_deref(),
        second.finished_at.as_deref(),
        "§9.4 phase 4 step 1: an unchanged project is stamped with the run's time"
    );
    assert_eq!(a2.1, a1.1, "the build never changed");
    assert_stamped_in_run(b2.0.as_deref(), &second, "the written scheme-B project");
    assert_eq!(
        c2, c1,
        "OUT-020: a failed project keeps its previous sync state"
    );
    assert_eq!(
        read(&world.project(2).target),
        c_external,
        "§9.5 step 7: the original stays unchanged"
    );
    assert_eq!(
        backup_bytes(&world, 2),
        [c_external],
        "§9.4 phase 3 step 3: the backup stays"
    );
    assert_eq!(last_success(&world), None, "MAIN-027: still `partial`");

    // Run 3 — the production seams: everything converges, and the
    // overall time follows the first `success`.
    let third = run_sync(&world, None);
    assert_eq!(third.verdict, "success");
    assert_eq!(statuses(&third), ["unchanged", "unchanged", "written"]);
    assert_eq!(
        stamps(&world, 0).0.as_deref(),
        third.finished_at.as_deref(),
        "the unchanged scheme-A project"
    );
    assert_eq!(
        stamps(&world, 1).0.as_deref(),
        third.finished_at.as_deref(),
        "the unchanged scheme-B project"
    );
    assert_stamped_in_run(
        stamps(&world, 2).0.as_deref(),
        &third,
        "the written scheme-C project",
    );
    assert_eq!(last_success(&world), third.finished_at);

    // Run 4 — a project that WAS stamped skips: scheme B's target is
    // replaced, outside the app, by a file without a marker pair. The
    // skipped half of OUT-020 is only a promise while the previous
    // state is not empty — an unstamped project has nothing to keep.
    let b_stamped = stamps(&world, 1);
    assert!(b_stamped.0.is_some(), "the premise: scheme B is stamped");
    const B_MARKERLESS: &[u8] = b"No markers any more.\n";
    write(&world.project(1).target, B_MARKERLESS);
    let fourth = run_sync(&world, None);
    assert_eq!(fourth.verdict, "partial");
    assert_eq!(
        statuses(&fourth),
        ["unchanged", "skipped", "unchanged"],
        "the refused scheme-B file beside two unchanged ones"
    );
    assert_eq!(
        stamps(&world, 1),
        b_stamped,
        "OUT-020: a skipped project keeps its previous sync state"
    );
    assert_eq!(
        read(&world.project(1).target),
        B_MARKERLESS,
        "nothing is written into the refused file"
    );
    assert_eq!(
        stamps(&world, 0).0.as_deref(),
        fourth.finished_at.as_deref(),
        "the unchanged scheme-A project is stamped with the fourth run"
    );
    assert_eq!(
        last_success(&world),
        third.finished_at,
        "MAIN-027: the `partial` run does not move the overall time"
    );
}

#[test]
fn an_aborted_run_stamps_the_abort_clock_and_never_moves_the_last_success() {
    // T-346's characterization pin, executable for the first time
    // (D-197): an interrupted run — the cancel checkpoint's stop, the
    // D-131 tail shape — stamps its row with the ABORT's OWN instant
    // (D-185's semantics, now observable through the command), and
    // MAIN-027's derived read (§9.4 phase 4 step 4: the last `success`
    // run's `finished_at`) is UNMOVED, as is the project's OUT-020
    // stamp: nothing about an aborted run claims a sync.
    //
    // The seam makes the pause MEASURABLE without a sleep: the abort
    // clock answers a fixed instant far from the first run's, so a
    // stamp derived from the run's own clock, from the real clock, or
    // absent entirely all fail the equality.
    let (world, _library, _events) = three_scheme_world(
        BaseName::AgentsMd,
        BaseName::ClaudeMd,
        BaseName::AgentsMd,
        Some(B_OLD_BLOCK),
    );

    // Run 1 — success: MAIN-027's baseline is this run's own stamp.
    // (The scheme-B target is fenced in by hand — `Some(B_OLD_BLOCK)` —
    // so nothing skips and the first run IS a success.)
    let first = run_sync(&world, None);
    assert_eq!(first.verdict, "success");
    let baseline = last_success(&world);
    assert_eq!(
        baseline, first.finished_at,
        "the success run IS the overall time (§9.4 phase 4 step 4)"
    );
    // A WRITTEN project is stamped by phase 3 with its own instant —
    // the run stamp is phase 4's — so the house window helper, not an
    // equality. Every project's pair is captured per index: the build
    // hashes differ per project, so no single snapshot can stand for
    // all three.
    let stamped: Vec<Stamps> = (0..3).map(|index| stamps(&world, index)).collect();
    assert_stamped_in_run(
        stamped[0].0.as_deref(),
        &first,
        "the written scheme-A project",
    );

    // Run 2 — the PRE-LATCHED token: steps 1–3 consult nothing, step 4
    // is the one check point, so the stop lands deterministically
    // before the backups; no timing, no sleep. The injected instant is
    // run 1's stamp plus one second: distinct from everything the run
    // could otherwise produce, and AFTER its own started_at, a state
    // production could hold.
    let cancel = CancelToken::new();
    cancel.cancel();
    let abort_instant = Timestamp::from_str(first.finished_at.as_deref().expect("run 1 finished"))
        .expect("run 1's stamp parses")
        + std::time::Duration::from_secs(1);
    let expected_stamp = abort_instant.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let seams = SyncRunSeams {
        backup: Arc::new(eunomia_fs::backup::backup_target_file),
        write: Arc::new(crate::sync_phase3::production_write),
        abort_clock: Arc::new(move || abort_instant),
    };
    let second = completed(
        tauri::async_runtime::block_on(crate::commands::sync_run_with(
            world.app.handle().clone(),
            None,
            cancel,
            seams,
        ))
        .expect("the aborted run answers Ok: the user asked for this"),
    );

    assert_eq!(
        second.verdict, "aborted",
        "the cancel checkpoint's stop is the row's own verdict"
    );
    assert_eq!(
        second.finished_at.as_deref(),
        Some(expected_stamp.as_str()),
        "the aborted row is stamped with the SEAM's instant, verbatim"
    );
    assert_eq!(
        statuses(&second),
        ["unchanged"; 3],
        "the D-131 tail: phase 1's verdict rows stay as phase 1 wrote them — nothing \
         was backed up or written (the backups never started)"
    );
    assert_eq!(
        last_success(&world),
        baseline,
        "MAIN-027: an aborted run does not move the overall time"
    );
    for (index, before_run) in stamped.iter().enumerate() {
        assert_eq!(
            stamps(&world, index),
            *before_run,
            "OUT-020: an aborted run leaves project {index}'s sync state alone"
        );
    }
}
