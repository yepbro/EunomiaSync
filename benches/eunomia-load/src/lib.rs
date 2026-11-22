//! The load-benchmark harness of spec §12.2 (task T-137): the fixtures
//! and the pass compositions the two criterion benchmarks measure.
//!
//! §15.1's Load row names this crate's tooling — "Section 12.2 metrics
//! on sets M and L — `criterion` benchmarks and e2e scripts" — and the
//! two budget rows measured here are "Building one project with up to
//! 1000 items: no more than 20 ms" (the `project_build` bench) and
//! "Dirty recalculation for all projects (set L): no more than 500 ms,
//! in the background" (the `dirty_recalc` bench). The benchmarks are a
//! **local tool**: CI compiles the bench targets and runs this
//! crate's pin tests like any member's (no `cargo bench` step exists
//! in `.github/workflows/ci.yml`), but the benchmarks themselves run
//! only where `cargo bench -p eunomia-load` is invoked, and
//! §12.2's "reference machine" is therefore honestly read as *the
//! machine running `cargo bench`* — the numbers travel with the
//! DECISIONS.md entry of this task, they are not a CI gate.
//!
//! # Why a crate of its own
//!
//! The two library halves of a fixture refuse to know each other: the
//! store stays file-system-free and the fs layer stays database-free
//! (D-109's two-crate split), so the composition the benchmarks need —
//! temporary Git roots from `eunomia_fs::temp_repos` feeding
//! `eunomia_store::data_sets::generate` — cannot live in either. That
//! entry already named this place: "composed in the benchmark crate".
//! A workspace member (not benches inside an existing crate) also
//! keeps the harness out of every production build graph while `cargo
//! clippy --workspace --all-targets` still lints it.
//!
//! # What is measured, and the seams that are cut
//!
//! The benchmarks mirror the two production passes that own §12.2's
//! rows — `src-tauri/src/dirty_recalc.rs` (T-101/D-121, the dirty
//! recalculation) and `src-tauri/src/sync_phase0.rs` (T-112/D-130, the
//! pure per-project content build) — recomposed here from the public
//! seams only, because both modules are `pub(crate)` (and frozen zones
//! of parallel tasks besides). Three seams are cut, and each is part
//! of the card's definition of the measurement:
//!
//! * **No event emit.** `state://dirty` delivery, the diff cache and
//!   the debounce are UI plumbing around the computation; the budget
//!   row is about the pass's cost (read + build + hash + compare).
//! * **No Tauri runtime.** Production spreads the build chunks over
//!   `tauri::async_runtime::spawn_blocking`; the parallel spelling here
//!   spreads the same chunks over `std::thread::scope` — the same
//!   shape (one OS task per chunk, collected in chunk order), without
//!   an app handle. The sequential spelling is kept beside it as the
//!   reference the tests pin the parallel one against (D-121's own
//!   cross-check pattern).
//! * **A plain `rusqlite::Connection`.** Production reads through the
//!   D-037 executor; here the caller's transaction is opened directly
//!   on the fixture's connection — the standalone-connection seam
//!   D-109 built the generator for. The reads themselves are the same
//!   repository calls in the same one-transaction shape (SYS-005).
//!
//! # The mirrors, and how they cannot drift
//!
//! `managed_text` (D-064's markerless seam) and the chunked
//! mini-snapshots of 25 projects (D-121's set-L batching) already exist
//! twice — in `dirty_recalc.rs` and `sync_phase0.rs`, each pinned
//! against the same oracles because the private `managed_block` they
//! mirror is unreachable. This crate carries the third spelling, with
//! the same discipline: `tests/mirror_pins.rs` pins the mirror against
//! `eunomia_core::target_file::build_content`'s real output (schemes A
//! and C, the file minus its marker tail) and against the body (scheme
//! B, where `build_content` panics by design), pins the chunk scoping
//! against an independent full-snapshot build on a boundary-crossing
//! fixture, and pins the parallel pass against the sequential one
//! id-for-id. D-130's follow-up card (consolidating the spellings
//! behind one shared builder) covers this third copy too.
//!
//! # Data
//!
//! Every fixture is generated in **setup**, never inside a measured
//! iteration: `TempRepos::create` lays out the roots, `open_db` +
//! `run_migrations` give the production DB-001 configuration, and
//! `data_sets::generate` fills the rows through the repositories. The
//! named profiles are `data_sets::L` (§12.2's upper limit, 500
//! projects / 1000 instructions / 100,000 assignments) and this
//! crate's [`ONE_PROJECT_1000_ITEMS`] for the single-project row (see
//! its docs for the shape). Stored hashes stay `NULL` in the fixtures
//! — the honest startup pass, where DB-021's first clause marks every
//! project with assignments dirty (D-121's L probe measured the same
//! state).

use std::collections::HashMap;
use std::fmt;

use rusqlite::Connection;
use tempfile::TempDir;

use eunomia_core::assignment::Assignment;
use eunomia_core::backup_name::target_file_name;
use eunomia_core::body::build_body;
use eunomia_core::dirty::{ProjectState, is_dirty};
use eunomia_core::extra_files::{ExtraFileLines, extra_file_lines};
use eunomia_core::hash::build_hash;
use eunomia_core::project::Scheme;
use eunomia_core::snapshot::{BuildSnapshot, SnapshotProject};
use eunomia_fs::temp_repos::TempRepos;
use eunomia_store::build_snapshot::{self, BuildSnapshotError};
use eunomia_store::data_sets::{self, DataSetError, DataSetSpec, GeneratedDataSet, ProjectRoot};
use eunomia_store::projects::{self, ProjectsError};
use eunomia_store::storage::{self, StorageError};

/// The single-project profile of §12.2's "Building one project with up
/// to 1000 items" row: data set L's library (1000 instructions, 100
/// headings, 3 variants per instruction) with **one** project that
/// carries **all** 1000 instructions — the upper-limit file the row
/// names, between the two named sets' per-project densities (M: 40
/// assignments per project, L: 200; the row's ceiling is the library
/// size itself, reachable only by assigning everything).
///
/// The built managed text is therefore ~1000 items (~52 KB — the
/// generator's templates average ≈52 characters), and the benchmark's
/// measured iteration is the whole production slice of one project:
/// one read transaction for the ALG-001 snapshot, then the §9.2 body,
/// the ALG-030 managed text, the §9.4 target path and the DB-020 hash
/// (the T-112 build mechanics, one chunk wide).
pub const ONE_PROJECT_1000_ITEMS: DataSetSpec = DataSetSpec {
    projects: 1,
    headings: 100,
    instructions: 1000,
    variants_per_instruction: 3,
    assignments_per_project: 1000,
};

/// How many projects share one scoped mini-snapshot — D-121's batching
/// constant, mirrored verbatim (`dirty_recalc::MINI_SNAPSHOT_PROJECTS`
/// and `sync_phase0`'s copy): `build_body` rescans the assignment list
/// per project, so the pass scopes each chunk to its own projects'
/// assignments over the shared item list. 25 held the §12.2 set-L
/// budget in D-121's measurements; anything from ~10 to ~50 measured
/// within noise there.
pub const MINI_SNAPSHOT_PROJECTS: usize = 25;

/// The stored-hash half of the dirty pass: `last_synced_hash` per
/// project row id, `None` ≡ never synced (DB-024's NULL until the
/// first writer, D-068).
pub type StoredHashes = HashMap<String, Option<String>>;

/// The assignment half of the batching: row indices into the
/// snapshot's assignment list, grouped by project id (D-121's map,
/// mirrored). Also answers DB-021's `has_assignments` exception.
pub type AssignmentsByProject = HashMap<String, Vec<usize>>;

/// Why a fixture or a read failed. Hand-rolled impls — the house
/// boundary style (D-093/D-097): no `thiserror`, sources carried by
/// the std `Error` impl.
#[derive(Debug)]
pub enum LoadError {
    /// A temporary directory or repository root could not be created
    /// (`TempRepos`/`TempDir`).
    Io(std::io::Error),
    /// `open_db` or `run_migrations` failed — the DB-001 pragma chain
    /// or the schema, the production configuration the fixture insists
    /// on.
    Storage(StorageError),
    /// The data set generator refused or failed (D-109's typed
    /// refusals or a repository failure inside its transaction).
    DataSet(DataSetError),
    /// A snapshot read failed inside a measured pass.
    Snapshot(BuildSnapshotError),
    /// The projects read behind the stored hashes failed.
    Projects(ProjectsError),
    /// The read transaction itself failed to begin or commit.
    Transaction(rusqlite::Error),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(err) => write!(f, "temporary fixture directory: {err}"),
            LoadError::Storage(err) => write!(f, "fixture database: {err}"),
            LoadError::DataSet(err) => write!(f, "data set generation: {err}"),
            LoadError::Snapshot(err) => write!(f, "reading the build snapshot: {err}"),
            LoadError::Projects(err) => write!(f, "reading the projects: {err}"),
            LoadError::Transaction(err) => write!(f, "fixture read transaction: {err}"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Io(err) => Some(err),
            LoadError::Storage(err) => Some(err),
            LoadError::DataSet(err) => Some(err),
            LoadError::Snapshot(err) => Some(err),
            LoadError::Projects(err) => Some(err),
            LoadError::Transaction(err) => Some(err),
        }
    }
}

/// One benchmark fixture: temporary Git roots, a migrated production-
/// shaped database filled through the generator, and the created row
/// ids. Everything self-cleans when the fixture drops (both guards are
/// held here); nothing is created inside a measured iteration.
#[derive(Debug)]
pub struct LoadFixture {
    /// The fixture's own connection (the standalone seam; production
    /// goes through the D-037 executor, the benches through this).
    /// Declared FIRST on purpose: fields drop in declaration order,
    /// and the connection must close before the database's home
    /// directory is removed — on Windows a directory removal under an
    /// open SQLite file fails silently (`TempDir::drop` swallows the
    /// error), leaving the fixture behind in `%TEMP%` (round-1
    /// review's P3-4; Unix unlink-while-open would have hidden it).
    conn: Connection,
    /// The database home — held (not read) so its guard keeps the
    /// directory alive for the fixture's lifetime; drops after
    /// `conn`, per the order note above.
    _db_home: TempDir,
    /// The repository roots — same holding rule; drops last.
    _repos: TempRepos,
    /// What [`data_sets::generate`] created, for addressing rows.
    generated: GeneratedDataSet,
}

impl LoadFixture {
    /// Creates the roots, the database and the rows for `spec`: one
    /// `TempRepos::create(spec.projects, label)` layout, one database
    /// under a self-cleaning directory (`open_db` then
    /// `run_migrations`), one `generate` run through the repositories
    /// (D-109's composition, spelled in `temp_repos`'s own docs).
    ///
    /// # Errors
    ///
    /// [`LoadError::Io`] for a temporary layout failure,
    /// [`LoadError::Storage`] for the open or the migrations,
    /// [`LoadError::DataSet`] for the generator's typed refusals or a
    /// repository failure (the whole run rolled back — a retry
    /// converges on a fresh fixture).
    pub fn create(spec: &DataSetSpec, label: &str) -> Result<Self, LoadError> {
        let repos = TempRepos::create(spec.projects, label).map_err(LoadError::Io)?;
        // Owned lossy spellings first, borrowed afterwards — the
        // `ProjectRoot` borrows, so the strings it points at must
        // outlive the call (the roots are ASCII `repo-NNNN` and key
        // losslessly; the fixture never degrades a path).
        let owned: Vec<(String, String)> = repos
            .roots()
            .iter()
            .map(|root| {
                (
                    root.path.to_string_lossy().into_owned(),
                    root.path_key.clone(),
                )
            })
            .collect();
        let roots: Vec<ProjectRoot<'_>> = owned
            .iter()
            .map(|(path, path_key)| ProjectRoot { path, path_key })
            .collect();
        let db_home = TempDir::new().map_err(LoadError::Io)?;
        let mut conn = storage::open_db(&db_home.path().join("eunomiasync.sqlite3"))
            .map_err(LoadError::Storage)?;
        // A fresh database takes the single embedded migration with no
        // upgrade backups to write (the store suite's own fixture
        // spelling); the directory exists, so a future multi-step set
        // would land its copies inside the guard too.
        storage::run_migrations(&mut conn, &db_home.path().join("migration-backups"))
            .map_err(LoadError::Storage)?;
        let generated = data_sets::generate(&mut conn, spec, &roots).map_err(LoadError::DataSet)?;
        Ok(Self {
            _repos: repos,
            _db_home: db_home,
            conn,
            generated,
        })
    }

    /// The created row ids in creation order (D-109's
    /// [`GeneratedDataSet`]).
    #[must_use]
    pub fn generated(&self) -> &GeneratedDataSet {
        &self.generated
    }

    /// The fixture's connection, mutably — the reads take a
    /// transaction, and `Connection::transaction` needs `&mut`.
    pub fn connection(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

/// Reads the ALG-001 build snapshot in **one read transaction**
/// (SYS-005's shape): the same repository read production composes
/// under the executor, committed before the build half starts (the
/// D-096 ordering — the transaction never spans the CPU work).
///
/// # Errors
///
/// [`LoadError::Snapshot`] when the read fails,
/// [`LoadError::Transaction`] when the transaction itself cannot begin
/// or commit.
pub fn read_snapshot(conn: &mut Connection) -> Result<BuildSnapshot, LoadError> {
    let tx = conn.transaction().map_err(LoadError::Transaction)?;
    let snapshot = build_snapshot::read(&tx).map_err(LoadError::Snapshot)?;
    tx.commit().map_err(LoadError::Transaction)?;
    Ok(snapshot)
}

/// Reads the dirty pass's two halves in **one** read transaction
/// (SYS-005/D-105, mirrored from `recalc_and_emit`: the snapshot and
/// the stored hashes must describe the same committed state) — the
/// ALG-001 snapshot plus the active projects' `last_synced_hash` cells
/// as [`StoredHashes`].
///
/// # Errors
///
/// [`LoadError::Snapshot`] or [`LoadError::Projects`] for the reads,
/// [`LoadError::Transaction`] for the transaction itself.
pub fn read_snapshot_and_hashes(
    conn: &mut Connection,
) -> Result<(BuildSnapshot, StoredHashes), LoadError> {
    let tx = conn.transaction().map_err(LoadError::Transaction)?;
    let snapshot = build_snapshot::read(&tx).map_err(LoadError::Snapshot)?;
    let rows = projects::list(&tx, projects::ListScope::Active).map_err(LoadError::Projects)?;
    tx.commit().map_err(LoadError::Transaction)?;
    let stored = rows
        .into_iter()
        .map(|project| (project.id, project.last_synced_hash))
        .collect();
    Ok((snapshot, stored))
}

/// Groups the snapshot's assignment row indices by project id (D-121's
/// `group_assignments`, mirrored verbatim): one pass over the
/// assignments instead of a rescan per chunk.
#[must_use]
pub fn group_assignments(snapshot: &BuildSnapshot) -> AssignmentsByProject {
    let mut grouped: AssignmentsByProject = HashMap::with_capacity(snapshot.projects().len());
    for (index, assignment) in snapshot.assignments().iter().enumerate() {
        grouped
            .entry(assignment.project_id.as_str().to_owned())
            .or_default()
            .push(index);
    }
    grouped
}

/// The markerless managed text of one project — D-064's seam, the
/// deliberate byte-exact mirror of `target_file`'s private
/// `managed_block` (D-065) and of the two src-tauri spellings
/// (`dirty_recalc::managed_text`, `sync_phase0`'s copy — frozen zones
/// of parallel tasks, hence this third): ALG-030's parts joined with
/// one empty line between neighbours, scheme A splicing the reading
/// lines around the body, schemes B and C taking the body alone.
/// Pinned against `build_content`'s real output by
/// `tests/mirror_pins.rs`, so the mirror cannot drift from the truth.
#[must_use]
pub fn managed_text(scheme: Scheme, body: &str, lines: &ExtraFileLines) -> String {
    /// One reading-lines position as a single owned block: a
    /// position's lines are ONE part (adjacent lines joined with
    /// single breaks), and only neighbouring parts get the empty line
    /// — the same two-level split `managed_block` performs.
    fn lines_part(block: &[String]) -> Option<String> {
        (!block.is_empty()).then(|| block.join("\n"))
    }

    let mut parts: Vec<String> = Vec::with_capacity(3);
    match scheme {
        Scheme::A => {
            if let Some(start) = lines_part(&lines.start) {
                parts.push(start);
            }
            if !body.is_empty() {
                parts.push(body.to_owned());
            }
            if let Some(end) = lines_part(&lines.end) {
                parts.push(end);
            }
        }
        Scheme::B | Scheme::C => {
            if !body.is_empty() {
                parts.push(body.to_owned());
            }
        }
    }
    parts.join("\n\n")
}

/// One project's built content, the T-112 product: the markerless
/// managed text and its DB-020 hash (the fingerprint the sync run and
/// the dirty verdict both read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltProject {
    /// The project row id.
    pub project_id: String,
    /// The ALG-030 managed text, marker excluded.
    pub managed: String,
    /// The DB-020 hash of the build.
    pub build_hash: String,
}

/// The per-project production slice both passes share (the §9.2 body
/// over the mini-snapshot is the caller's — `build_body` needs the
/// scoped assignment list): the ALG-030 managed text, the §9.4 target
/// path and the DB-020 hash. The `expect` is D-061's reasoned-panic
/// posture carried over from the mirrors: a `None` body names a
/// broken invariant of the composition, never a silent default.
fn prepare_project(project: &SnapshotProject, body: String) -> BuiltProject {
    let settings = &project.settings;
    // D-064's seam: the assembled managed block, marker excluded,
    // scheme A's reading lines included. The argument order is
    // project-then-local — §7.5's tie order, the reverse of the
    // settings struct's field order.
    let lines = extra_file_lines(
        settings.base_name,
        settings.extra_project,
        settings.extra_local,
    );
    let managed = managed_text(settings.scheme, &body, &lines);
    // The DB-020 target path: `<root>/<base_name>` (A, B) or
    // `<root>/<STEM>.project.md` (C), through the one public
    // file-name owner.
    let target_path = project
        .root
        .join(target_file_name(settings.scheme, settings.base_name));
    let build_hash = build_hash(
        settings.scheme,
        &target_path,
        settings.extra_local,
        settings.extra_project,
        &managed,
    );
    BuiltProject {
        project_id: project.id.as_str().to_owned(),
        managed,
        build_hash,
    }
}

/// The chunk scoping both chunk mirrors share: the chunk's project
/// slice and its fresh scoped mini-snapshot — the same items, only the
/// chunk's assignments (D-121's equivalence: filtering by project id
/// is exactly what `selected_variants` does inside, so every built
/// byte equals the full snapshot's). `None` names a `start` past the
/// project list's end.
fn scope_chunk<'a>(
    snapshot: &'a BuildSnapshot,
    grouped: &AssignmentsByProject,
    start: usize,
) -> Option<(&'a [SnapshotProject], BuildSnapshot)> {
    // The last chunk may be shorter — clamp the range end, or `get`
    // would answer None and silently drop the chunk's projects (the
    // release-suite catch D-121 records).
    let end = (start + MINI_SNAPSHOT_PROJECTS).min(snapshot.projects().len());
    let projects = snapshot.projects().get(start..end)?;
    let mut scoped: Vec<Assignment> = Vec::new();
    for project in projects {
        if let Some(indices) = grouped.get(project.id.as_str()) {
            scoped.extend(
                indices
                    .iter()
                    .map(|&index| snapshot.assignments()[index].clone()),
            );
        }
    }
    let mini = BuildSnapshot::new(projects.to_vec(), snapshot.items().to_vec(), scoped).expect(
        "the full snapshot passed the same validation: a subset of its \
         assignments and projects over the same items cannot fail it",
    );
    Some((projects, mini))
}

/// Builds the projects of chunk `start..` — `sync_phase0`'s
/// `chunk_projects` mirror: the §9.2 body over the scoped
/// mini-snapshot, then [`prepare_project`], in chunk (=`sort_order`)
/// order.
fn chunk_projects(
    snapshot: &BuildSnapshot,
    grouped: &AssignmentsByProject,
    start: usize,
) -> Vec<BuiltProject> {
    let Some((projects, mini)) = scope_chunk(snapshot, grouped, start) else {
        return Vec::new();
    };
    projects
        .iter()
        .map(|project| {
            let body = build_body(&mini, &project.id)
                .expect("the mini-snapshot holds every project of its chunk");
            prepare_project(project, body)
        })
        .collect()
}

/// Computes the verdicts of the projects of chunk `start..` —
/// `dirty_recalc`'s `chunk_verdicts` mirror: the same build slice,
/// then DB-020's comparison — `is_dirty` over the stored hash, the
/// current one, the managed text and the assignments fact (DB-021).
fn chunk_verdicts(
    snapshot: &BuildSnapshot,
    grouped: &AssignmentsByProject,
    start: usize,
    stored_hashes: &StoredHashes,
) -> Vec<(String, bool)> {
    let Some((projects, mini)) = scope_chunk(snapshot, grouped, start) else {
        return Vec::new();
    };
    projects
        .iter()
        .map(|project| {
            let body = build_body(&mini, &project.id)
                .expect("the mini-snapshot holds every project of its chunk");
            let built = prepare_project(project, body);
            let stored = stored_hashes
                .get(project.id.as_str())
                .and_then(|hash| hash.as_deref());
            // The snapshot holds active projects only (ALG-001), so
            // the state fact is `Active` by construction.
            let dirty = is_dirty(
                ProjectState::Active,
                stored,
                &built.build_hash,
                &built.managed,
                grouped.contains_key(project.id.as_str()),
            );
            (built.project_id, dirty)
        })
        .collect()
}

/// The T-112 build over a whole snapshot, sequential chunk spelling:
/// every active project's [`BuiltProject`] in `sort_order`. The
/// `project_build` benchmark measures this (one read transaction +
/// this call) on the [`ONE_PROJECT_1000_ITEMS`] fixture — one chunk
/// wide, the §12.2 single-project row's whole production slice.
#[must_use]
pub fn build_projects_sequential(snapshot: &BuildSnapshot) -> Vec<BuiltProject> {
    let grouped = group_assignments(snapshot);
    let mut built = Vec::with_capacity(snapshot.projects().len());
    for start in (0..snapshot.projects().len()).step_by(MINI_SNAPSHOT_PROJECTS) {
        built.extend(chunk_projects(snapshot, &grouped, start));
    }
    built
}

/// The dirty verdicts over a whole snapshot, sequential chunk spelling
/// — D-121's `compute_dirty_verdicts` mirror and the reference the
/// tests pin [`compute_verdicts_parallel`] against.
#[must_use]
pub fn compute_verdicts_sequential(
    snapshot: &BuildSnapshot,
    stored_hashes: &StoredHashes,
) -> Vec<(String, bool)> {
    let grouped = group_assignments(snapshot);
    let mut verdicts = Vec::with_capacity(snapshot.projects().len());
    for start in (0..snapshot.projects().len()).step_by(MINI_SNAPSHOT_PROJECTS) {
        verdicts.extend(chunk_verdicts(snapshot, &grouped, start, stored_hashes));
    }
    verdicts
}

/// The dirty verdicts over a whole snapshot, **parallel** chunk
/// spelling — the production pass's shape (`recalc_and_emit` spreads
/// the same chunks over `spawn_blocking`): one scoped task per chunk
/// under `std::thread::scope`, collected in chunk order so the
/// verdicts stay in `sort_order` however the tasks interleave. This is
/// what the `dirty_recalc` benchmark measures as §12.2's set-L pass.
///
/// A panicked chunk surfaces as the honest join failure (the
/// blocking-join error production maps); the fixture data cannot panic
/// the chunks — the reasoned `expect`s name composition bugs, loudly.
#[must_use]
pub fn compute_verdicts_parallel(
    snapshot: &BuildSnapshot,
    stored_hashes: &StoredHashes,
) -> Vec<(String, bool)> {
    let grouped = group_assignments(snapshot);
    // Shared-ref copies, so every `move`d chunk task captures the
    // references (Copy), not the maps behind them.
    let grouped = &grouped;
    let starts = (0..snapshot.projects().len()).step_by(MINI_SNAPSHOT_PROJECTS);
    std::thread::scope(|scope| {
        let tasks: Vec<_> = starts
            .map(|start| {
                scope.spawn(move || chunk_verdicts(snapshot, grouped, start, stored_hashes))
            })
            .collect();
        let mut verdicts = Vec::with_capacity(snapshot.projects().len());
        for task in tasks {
            let chunk = task
                .join()
                .expect("a benchmark chunk panicked — a composition bug, not data");
            verdicts.extend(chunk);
        }
        verdicts
    })
}

/// Confirms the harness sees the shape the docs promise on the
/// single-project profile: one project, all 1000 instructions assigned
/// (the §12.2 row's premise), the library at L's size. The spec
/// constants' own pins live in `eunomia-store` (D-109); this pins the
/// *composition*, through the public interface only.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_project_profile_carries_the_whole_library() {
        let mut fixture = LoadFixture::create(&ONE_PROJECT_1000_ITEMS, "eunomia-load-profile")
            .expect("the single-project fixture builds");
        assert_eq!(fixture.generated().project_ids.len(), 1);
        assert_eq!(fixture.generated().instruction_ids.len(), 1000);
        assert_eq!(fixture.generated().assignment_count, 1000);
        let snapshot =
            read_snapshot(fixture.connection()).expect("the snapshot read succeeds on the fixture");
        assert_eq!(snapshot.projects().len(), 1);
        assert_eq!(snapshot.assignments().len(), 1000);
        let built = build_projects_sequential(&snapshot);
        assert_eq!(built.len(), 1);
        assert!(
            !built[0].managed.is_empty(),
            "1000 assigned instructions build a non-empty managed text"
        );
    }
}
