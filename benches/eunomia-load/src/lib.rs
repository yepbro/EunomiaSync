//! The load-benchmark harness of spec §12.2 (task T-137, the
//! assignment-toggle row added by T-138): the fixtures, the
//! standalone-connection reads, the parallel substrate and the
//! assignment-toggle save state the three criterion benchmarks
//! measure.
//!
//! §15.1's Load row names this crate's tooling — "Section 12.2
//! metrics on sets M and L — `criterion` benchmarks and e2e scripts"
//! — and the three budget rows measured here are "Building one project
//! with up to 1000 items: no more than 20 ms" (the `project_build`
//! bench), "Dirty recalculation for all projects (set L): no more
//! than 500 ms, in the background" (the `dirty_recalc` bench), and
//! "Saving an assignment toggle (p95, set L): no more than 50 ms"
//! (the `assignment_toggle` bench). The
//! benchmarks are a **local tool**: CI compiles the bench targets and
//! runs this crate's pin tests like any member's (no `cargo bench`
//! step exists in `.github/workflows/ci.yml`), but the benchmarks
//! themselves run only where `cargo bench -p eunomia-load` is
//! invoked, and §12.2's "reference machine" is therefore honestly
//! read as *the machine running `cargo bench`* — the numbers travel
//! with the DECISIONS.md entry of this task, they are not a CI gate.
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
//! The two read-side benchmarks mirror the two production passes
//! that own their §12.2 rows — `src-tauri/src/dirty_recalc.rs`
//! (T-101/D-121, the dirty recalculation) and
//! `src-tauri/src/sync_phase0.rs` (T-112/D-130, the pure per-project
//! content build) — over the **shared** build mechanics of
//! `eunomia_core::content_build` (T-265/D-145: this harness's own
//! third spelling of the managed-text/chunking mirrors dissolved
//! into that crate with the two src-tauri ones; what it measured
//! before through its mirrored copy it measures now through the same
//! code production runs). The third row is a **write**: the
//! `assignment_toggle` bench measures the store's own
//! `library::reassign` save directly (no production pass exists to
//! mirror — the `assignment_set` command, T-097, is not landed).
//! Four seams remain cut, and each is part of the card's definition
//! of the measurement:
//!
//! * **No event emit.** `state://dirty` delivery, the diff cache and
//!   the debounce are UI plumbing around the computation; the budget
//!   row is about the pass's cost (read + build + hash + compare).
//! * **No Tauri runtime.** Production spreads the build chunks over
//!   `tauri::async_runtime::spawn_blocking`; the parallel spelling
//!   here ([`compute_verdicts_parallel`]) spreads the same chunks —
//!   `eunomia_core::content_build::chunk_verdicts`, the very
//!   per-chunk call the production pass makes — over
//!   `std::thread::scope`: the same shape (one OS task per chunk,
//!   collected in chunk order), without an app handle. The
//!   sequential spelling lives in core (`compute_dirty_verdicts`)
//!   and remains the reference the tests pin the parallel one
//!   against (D-121's own cross-check pattern).
//! * **A plain `rusqlite::Connection`.** Production reads through the
//!   D-037 executor; here the caller's transaction is opened directly
//!   on the fixture's connection — the standalone-connection seam
//!   D-109 built the generator for. The reads themselves are the same
//!   repository calls in the same one-transaction shape (SYS-005).
//! * **No journal write for the toggle.** The `assignment_set`
//!   command's `action_log` entry and event emit are the IPC
//!   layer's; the store's `reassign` writes no journal row, so the
//!   toggle bench measures the store half alone (the D-135 seam
//!   discipline, applied to the write row).
//!
//! # How the substrate cannot drift
//!
//! `tests/mirror_pins.rs` keeps the harness honest against oracles
//! that do not run through its plumbing: the chunk scoping against an
//! independent full-snapshot build on a boundary-crossing fixture
//! (every project's chunked bytes equal what `build_body` produces
//! over the whole snapshot), the parallel pass against the sequential
//! one id-for-id, and DB-021's semantics in both directions. The
//! tripwire pins the shared chunk size by value
//! (`content_build::MINI_SNAPSHOT_PROJECTS == 25`): the §12.2 budgets
//! were measured at 25, and a core change turns this harness's suite
//! red until the budgets are re-measured.
//!
//! The assignment-toggle bench (`tests/assignment_toggle_pins.rs`)
//! needs no mirror tripwire: its iteration is the store's own
//! `library::reassign` call, not a harness copy of production code
//! (the D-145 direct-to-core posture, applied to the store write).
//! What its pins guard instead is the bench's own premise: the
//! toggled pair exists with the variant the generator's closed form
//! promises, every measured save is a real write (the alternation),
//! and the no-op diagnostic truly targets the held variant.
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

use std::fmt;

use rusqlite::Connection;
use tempfile::TempDir;

use eunomia_core::content_build::{AssignmentsByProject, StoredHashes, chunk_verdicts};
use eunomia_core::snapshot::BuildSnapshot;
use eunomia_fs::temp_repos::TempRepos;
use eunomia_store::build_snapshot::{self, BuildSnapshotError};
use eunomia_store::data_sets::{self, DataSetError, DataSetSpec, GeneratedDataSet, ProjectRoot};
use eunomia_store::library::{self, Assignment, LibraryError};
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

/// Which half of the assignment-toggle premise refused — the
/// discriminator [`LoadError::TogglePremise`] carries so a refusal
/// names its cause without free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PremiseHalf {
    /// The generated rows lack the toggle's shape: no project, no
    /// instruction, no variant, or no second variant to flip to (the
    /// id fields are empty — there is nothing to name).
    MissingShape,
    /// The first pair carries no assignment at all (`held` is empty;
    /// `expected` names the variant the closed form promised).
    NoAssignment,
    /// The pair's row holds a different variant than the closed form
    /// promises (both ids named).
    VariantMismatch,
}

impl fmt::Display for PremiseHalf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            PremiseHalf::MissingShape => "the fixture lacks the toggle's shape",
            PremiseHalf::NoAssignment => "the first pair carries no assignment",
            PremiseHalf::VariantMismatch => "the first pair holds an unexpected variant",
        };
        f.write_str(name)
    }
}

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
    /// An assignment write or read failed inside the toggle bench
    /// (`library::reassign` / `library::get_assignment`).
    Library(LibraryError),
    /// The assignment-toggle bench's fixture premise failed — a
    /// generator or profile change, not a benchmark condition. The
    /// [`PremiseHalf`] discriminator names which half refused; the
    /// id fields carry only ids, never free text.
    TogglePremise {
        /// Which half of the premise refused.
        half: PremiseHalf,
        /// What the row actually holds (an id; empty when no row
        /// exists to name).
        held: String,
        /// What the premise promised (the expected variant id; empty
        /// when the promise itself lacks the shape to name one).
        expected: String,
    },
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
            LoadError::Library(err) => write!(f, "the assignment toggle: {err}"),
            LoadError::TogglePremise {
                half,
                held,
                expected,
            } => write!(
                f,
                "assignment-toggle premise ({half}): held variant id {held:?}, promised {expected:?}"
            ),
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
            LoadError::Library(err) => Some(err),
            LoadError::TogglePremise { .. } => None,
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
        // A fresh database takes every embedded migration with no
        // upgrade backups to write (the store suite's own fixture
        // spelling — a bootstrap run writes no copies however many
        // steps it applies); the directory exists, so an upgrade run's
        // copies would land inside the guard too.
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

/// §12.2's assignment-toggle row, made measurable: the fixed
/// (project, instruction) pair whose variant every iteration saves,
/// and the alternation that keeps every save a **real** write.
///
/// The row reads "Saving an assignment toggle (p95, set L): no more
/// than 50 ms" — one user save action, so one measured iteration is
/// one real `library::reassign` save: read the current row, verify
/// the target is an active sibling (DB-012), UPDATE the variant with
/// the `updated_at` restamp, read the stored row back. That store
/// function is §7.10's `assignment_set` store half (MAIN-005: the
/// change stays in this project); the command's journal write and
/// event emit are the IPC layer's and are cut here exactly as the
/// `project_build`/`dirty_recalc` benches cut their UI plumbing.
///
/// The pair is the generator's own first: project ordinal 0,
/// assignment `k = 0` → instruction ordinal `(37·0 + 0) mod I = 0`,
/// initial variant ordinal `(0 + 0) mod V = 0` (D-109's closed form —
/// scale-independent at the origin, so the pins verify the premise on
/// small fixtures while the bench runs set L). The toggle target is
/// the instruction's second variant, an active sibling by
/// construction. [`AssignmentToggle::save`] flips A→B→A… so no
/// iteration ever lands on the no-op path;
/// [`AssignmentToggle::noop_save`] is that path, isolated as the
/// diagnostic that attributes the save cost between its current-row
/// read + early return and everything the save adds on top (the
/// DB-012 verify read, the UPDATE, the commit, the read-back).
#[derive(Debug)]
pub struct AssignmentToggle {
    /// The toggled project (generator ordinal 0).
    project_id: String,
    /// The toggled instruction (the pair's `k = 0` assignment).
    instruction_id: String,
    /// The variant the generator's closed form starts the row on.
    variant_a: String,
    /// The sibling variant the first save flips to.
    variant_b: String,
    /// Which variant the row holds right now — flipped only by a
    /// **successful** save, so the state never desyncs from the row.
    /// Safe because today's `reassign` raises every error *before*
    /// its UPDATE (a failed save leaves the row untouched); if the
    /// store ever gains a fallible post-write step inside the
    /// measured call, derive the held variant from the returned row
    /// instead of flipping this bool (round-1 review's P3-5).
    holds_a: bool,
}

impl AssignmentToggle {
    /// Builds the toggle for the fixture's first pair and verifies
    /// the premise in the same breath: the pair's assignment exists
    /// and holds the generator's variant ordinal 0, and the
    /// instruction carries a second variant to flip to.
    ///
    /// # Errors
    ///
    /// [`LoadError::TogglePremise`] when the premise fails (a
    /// generator change), [`LoadError::Library`] when the read
    /// itself fails.
    pub fn new(fixture: &mut LoadFixture) -> Result<Self, LoadError> {
        let pair = Self::first_pair(fixture.generated())?;
        let current = library::get_assignment(fixture.connection(), &pair.0, &pair.1)
            .map_err(LoadError::Library)?
            .ok_or_else(|| LoadError::TogglePremise {
                half: PremiseHalf::NoAssignment,
                held: String::new(),
                expected: pair.2.clone(),
            })?;
        if current.variant_id != pair.2 {
            return Err(LoadError::TogglePremise {
                half: PremiseHalf::VariantMismatch,
                held: current.variant_id,
                expected: pair.2,
            });
        }
        Ok(Self {
            project_id: pair.0,
            instruction_id: pair.1,
            variant_a: pair.2,
            variant_b: pair.3,
            holds_a: true,
        })
    }

    /// The first pair's ids: `(project, instruction, variant 0,
    /// variant 1)` straight from the generated rows — the project,
    /// the instruction and the second variant's existence are all
    /// part of the premise, and every missing half refuses with
    /// [`PremiseHalf::MissingShape`] (nothing exists to name).
    fn first_pair(
        generated: &GeneratedDataSet,
    ) -> Result<(String, String, String, String), LoadError> {
        let refused = || LoadError::TogglePremise {
            half: PremiseHalf::MissingShape,
            held: String::new(),
            expected: String::new(),
        };
        let project = generated.project_ids.first().ok_or_else(refused)?;
        let instruction = generated.instruction_ids.first().ok_or_else(refused)?;
        let variants = generated.variant_ids.first().ok_or_else(refused)?;
        let variant_a = variants.first().ok_or_else(refused)?;
        let variant_b = variants.get(1).ok_or_else(refused)?;
        Ok((
            project.clone(),
            instruction.clone(),
            variant_a.clone(),
            variant_b.clone(),
        ))
    }

    /// One real save: flips the pair to the variant it does not hold,
    /// through the store's `reassign` (the `assignment_set` store
    /// half), and returns the stored row.
    ///
    /// # Errors
    ///
    /// [`LoadError::Library`] when the write fails; the alternation
    /// state flips only on success, so a failed save leaves the pair
    /// and the next call consistent.
    pub fn save(&mut self, conn: &Connection) -> Result<Assignment, LoadError> {
        let target = self.other();
        let stored = library::reassign(conn, &self.project_id, &self.instruction_id, target)
            .map_err(LoadError::Library)?;
        self.holds_a = !self.holds_a;
        Ok(stored)
    }

    /// The no-op path, isolated: `reassign` naming the variant the
    /// row already holds — the verified no-write early return. The
    /// store checks DB-012 only *after* that return, so this
    /// diagnostic carries the current-row read alone, without the
    /// verify read, the UPDATE or the commit; the `save_set_L` minus
    /// this number is DB-012's verify + UPDATE + commit + read-back.
    /// What the `noop_same_variant_set_L` diagnostic measures.
    ///
    /// # Errors
    ///
    /// [`LoadError::Library`] when the call fails.
    pub fn noop_save(&self, conn: &Connection) -> Result<Assignment, LoadError> {
        library::reassign(conn, &self.project_id, &self.instruction_id, self.held())
            .map_err(LoadError::Library)
    }

    /// The variant id the row holds per the alternation state — the
    /// [`noop_save`] target. Pinned against the store's own read.
    fn held(&self) -> &str {
        if self.holds_a {
            &self.variant_a
        } else {
            &self.variant_b
        }
    }

    /// The variant id the next [`save`] will flip to.
    fn other(&self) -> &str {
        if self.holds_a {
            &self.variant_b
        } else {
            &self.variant_a
        }
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

/// The dirty verdicts over a whole snapshot, **parallel** chunk
/// spelling — the production pass's shape (`recalc_and_emit` spreads
/// the same chunks over `spawn_blocking`): one scoped task per chunk
/// under `std::thread::scope`, collected in chunk order so the
/// verdicts stay in `sort_order` however the tasks interleave. This is
/// what the `dirty_recalc` benchmark measures as §12.2's set-L pass.
///
/// The per-chunk half is core's own [`chunk_verdicts`] — the very
/// function the production pass's blocking tasks call (T-265/D-145);
/// only the substrate differs (scoped threads, no app handle). A
/// panicked chunk surfaces as the honest join failure (the
/// blocking-join error production maps); the fixture data cannot panic
/// the chunks — the reasoned `expect`s name composition bugs, loudly.
#[must_use]
pub fn compute_verdicts_parallel(
    snapshot: &BuildSnapshot,
    stored_hashes: &StoredHashes,
) -> Vec<(String, bool)> {
    let grouped: AssignmentsByProject = eunomia_core::content_build::group_assignments(snapshot);
    // Shared-ref copies, so every `move`d chunk task captures the
    // references (Copy), not the maps behind them.
    let grouped = &grouped;
    let starts = eunomia_core::content_build::chunk_starts(snapshot.projects().len());
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
        let built = eunomia_core::content_build::build_projects(&snapshot);
        assert_eq!(built.len(), 1);
        assert!(
            !built[0].managed.is_empty(),
            "1000 assigned instructions build a non-empty managed text"
        );
    }
}
