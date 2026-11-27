//! The load-benchmark harness of spec §12.2 (task T-137; the
//! assignment-toggle row added by T-138; the sync and scan rows by
//! T-139): the fixtures, the standalone-connection reads, the
//! parallel substrate, the assignment-toggle save state, the composed
//! sync pass and the scan tree the five criterion benchmarks measure.
//!
//! §15.1's Load row names this crate's tooling — "Section 12.2
//! metrics on sets M and L — `criterion` benchmarks and e2e scripts"
//! — and the five budget rows measured here are "Building one project
//! with up to 1000 items: no more than 20 ms" (the `project_build`
//! bench), "Dirty recalculation for all projects (set L): no more
//! than 500 ms, in the background" (the `dirty_recalc` bench),
//! "Saving an assignment toggle (p95, set L): no more than 50 ms"
//! (the `assignment_toggle` bench), "Sync of 100 projects, files up
//! to 10 KB, SSD: no more than 10 s" (the `sync_pipeline` bench), and
//! "Scan of 100,000 folders on an SSD: no more than 30 s" (the
//! `scan_walk` bench — its walk half; the row's "UI input response no
//! more than 100 ms" half is about the UI thread and stays with the
//! e2e scripts). The
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
//! The fourth row composes the sync pipeline's measurable core: the
//! src-tauri phases orchestrate store transactions and app state
//! around per-step work that lives in `eunomia-fs`/`eunomia-core`,
//! and this harness cannot depend on the app crate (D-145's recorded
//! fact) — so the `sync_pipeline` bench walks §9.4's phase order
//! (phase 0's build half, phase 1's seven preflight steps, phase 2's
//! verified copies, phase 3's §9.5 atomic writes) through the REAL
//! seam functions the phases call, with the app-shell orchestration
//! cut. The fifth row is the scan engine itself:
//! `eunomia_fs::scan::scan_repositories` over a generated
//! 100k-folder tree, the ALG-050..054 walker with its own cancel
//! checks — a direct call, nothing mirrored.
//!
//! The seams that stay cut, each part of the row's definition:
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
//! * **No run bookkeeping for the sync pass.** The `sync_runs` /
//!   `sync_run_projects` rows, the `sync_running` flag, phase 3's
//!   per-project `last_synced` commit (a store UPDATE at D-157's
//!   measured ~65 µs — hundredths of a percent of the row's 10 s),
//!   the `backup_records` rows and the BKP-010 dedup lookup, and
//!   phase 4 whole (retention, journal, the run verdict) are the
//!   app-shell orchestration around the per-step work; the pass
//!   measures the per-step work itself. §9.4 phase 1 step 6's D-25
//!   confirmation list is not built either: the fixture's builds are
//!   never empty, so the branch is unreachable and costless. The
//!   per-iteration backup sweep (unmeasured setup) stands in for
//!   phase 4's retention, so copies do not accumulate across
//!   iterations.
//! * **No scan command layer.** The `scan://progress` throttle (its
//!   per-event mutex and clock check), the cancel slot, the SEC-003
//!   root gate, and the ALG-053 classification of found repositories
//!   (`scan_classify::classify_entry` — T-075's module with its own
//!   tests) are `src-tauri/src/scan.rs`'s surroundings (D-153,
//!   measured there by its own tests); the walk bench measures the
//!   engine under the §6.5 default settings.
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
//! The sync-pipeline bench (`tests/sync_pipeline_pins.rs`) composes
//! the REAL per-step seams of the src-tauri phases (every mechanic is
//! the production function itself: the fs probes, the ALG-033
//! comparison, the backup copy, the §9.5 atomic write, the ALG-031
//! splice), so there is no mirrored mechanics to tripwire — what the
//! pins guard are the bench's own premises: the per-project
//! toggle pairs exist, the built files carry the row's "up to 10 KB"
//! size, the alternation really flips every project's build, a
//! no-flip second pass reads all-unchanged (D-24's acceptance through
//! the composed pass), a flipped pass backs up and rewrites every
//! project with the ALG-030/ALG-031 byte shapes, and a missing
//! scheme-B target is skipped, not created (SYS-009). TWO ALG-030
//! spellings the pass does mirror over an already-built managed
//! text (production's own companions in `sync_phase3`/`sync_phase1`):
//! the whole-file tail (`whole_file`: `managed`, a blank line, the
//! marker — pinned against the real `build_content` builder's
//! output) and the markerless comparison basis (`markerless_whole`,
//! pinned behaviourally by the all-unchanged second pass).
//!
//! The scan bench (`tests/scan_walk_pins.rs`) calls the walk engine
//! itself; its pins guard the tree: the builder's arithmetic equals
//! an independent `std::fs` directory census, the real scan of a
//! small tree finds exactly the planted repositories with exactly the
//! arithmetic's event count, and the big tree's folder count meets
//! the row's 100,000 floor.
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
use std::fs;
use std::io;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::Connection;
use tempfile::TempDir;

use eunomia_core::assembly_b::splice;
use eunomia_core::backup_name::{BackupTime, target_file_name};
use eunomia_core::compare::contents_match;
use eunomia_core::content_build::{
    AssignmentsByProject, BuiltProject, StoredHashes, build_projects, chunk_verdicts,
};
use eunomia_core::marker::{MarkerTime, creation_marker_line};
use eunomia_core::markers;
use eunomia_core::project::{BaseName, ProjectId, Scheme};
use eunomia_core::snapshot::BuildSnapshot;
use eunomia_fs::atomic::{AtomicWriteError, write_target_file};
use eunomia_fs::backup::{BackupError, BackupTarget, backup_target_file};
use eunomia_fs::free_space::{FreeSpaceError, check_backup_space, free_bytes};
use eunomia_fs::paths::{
    ProjectUnavailableError, TargetPathError, check_project_available, resolve_target_path,
};
use eunomia_fs::scan::{CancelToken, ScanEvent, ScanOptions, ScanOutcome, scan_repositories};
use eunomia_fs::temp_files::remove_stale_tmp_files;
use eunomia_fs::temp_repos::TempRepos;
use eunomia_fs::write_access::{WriteAccessError, check_write_access};
use eunomia_store::build_snapshot::{self, BuildSnapshotError};
use eunomia_store::data_sets::{self, DataSetError, DataSetSpec, GeneratedDataSet, ProjectRoot};
use eunomia_store::library::{self, Assignment, AssignmentTarget, LibraryError};
use eunomia_store::projects::{self, ProjectsError};
use eunomia_store::settings::{SettingKey, SettingValue};
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

/// §12.2's sync row profile: "Sync of 100 projects, files up to
/// 10 KB, SSD — no more than 10 s". One hundred projects at data set
/// L's per-project density (200 assignments each over L's own library
/// shape: 1000 instructions under 100 headings, 3 variants per
/// instruction) — the generator's §12.2 variant-text sizing builds
/// each project's managed text at ≈10 KB (D-109: "a fully assigned
/// set-L project builds ≈10 KB"; 200 of the 1000 instructions is
/// exactly that density), which is the row's "files up to 10 KB"
/// premise.
///
/// The measured iteration is the composed §9.4 core over these
/// projects: see [`SyncFixture`] for the pass and its seams.
pub const SYNC_100_PROJECTS_10KB: DataSetSpec = DataSetSpec {
    projects: 100,
    headings: 100,
    instructions: 1000,
    variants_per_instruction: 3,
    assignments_per_project: 200,
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

/// Why a fixture, a read, or a composed sync-pass step failed.
/// Hand-rolled impls — the house boundary style (D-093/D-097): no
/// `thiserror`, sources carried by the std `Error` impl.
#[derive(Debug)]
pub enum LoadError {
    /// A temporary directory or repository root could not be created
    /// (`TempRepos`/`TempDir`/`ScanTree`).
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
    /// The sync pass's preflight availability probe refused the
    /// project root (`eunomia_fs::paths`).
    Unavailable(ProjectUnavailableError),
    /// The sync pass's target-path resolution refused the target
    /// (`eunomia_fs::paths`).
    Target(TargetPathError),
    /// The sync pass's write-access probe refused the target
    /// (`eunomia_fs::write_access`).
    Access(WriteAccessError),
    /// The sync pass's backup-space check refused the run
    /// (`eunomia_fs::free_space`).
    Space(FreeSpaceError),
    /// A backup copy or its verification failed inside the sync pass
    /// (`eunomia_fs::backup`).
    Backup(BackupError),
    /// An atomic write failed inside the sync pass
    /// (`eunomia_fs::atomic`, §9.5's typed verdicts).
    Write(AtomicWriteError),
    /// The assignment-toggle or sync-pass fixture premise failed — a
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
            LoadError::Unavailable(err) => write!(f, "the project root is unavailable: {err}"),
            LoadError::Target(err) => write!(f, "resolving the target path: {err}"),
            LoadError::Access(err) => write!(f, "the write-access probe: {err}"),
            LoadError::Space(err) => write!(f, "the backup-space check: {err}"),
            LoadError::Backup(err) => write!(f, "the backup copy: {err}"),
            LoadError::Write(err) => write!(f, "the atomic write: {err}"),
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
            LoadError::Unavailable(err) => Some(err),
            LoadError::Target(err) => Some(err),
            LoadError::Access(err) => Some(err),
            LoadError::Space(err) => Some(err),
            LoadError::Backup(err) => Some(err),
            LoadError::Write(err) => Some(err),
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
        let held = match current.target {
            AssignmentTarget::Variant(variant_id) => variant_id,
            // A cell with custom text — replace or extend (MAIN-034) —
            // holds no variant for the toggle's premise; the generator
            // never creates one, so this arm is a drifted world's
            // premise failure.
            AssignmentTarget::Custom(_) | AssignmentTarget::Extend { .. } => String::new(),
        };
        if held != pair.2 {
            return Err(LoadError::TogglePremise {
                half: PremiseHalf::VariantMismatch,
                held,
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

/// §12.2's sync row, made measurable: the fixture (a 100-project
/// generated world with temporary Git roots and a backups data
/// directory) plus the composed §9.4 pass and the per-project
/// alternation that keeps every measured pass a full write pass.
///
/// # What one iteration is
///
/// One measured iteration of the `full_pass_100_projects` bench is
/// the pipeline's measurable core in §9.4's phase order — one read
/// transaction for the ALG-001 snapshot and
/// [`content_build::build_projects`] (phase 0's pure half), then per
/// project in `sort_order`: the availability probe, the D-092 target
/// resolution, the regular-file gate, the D-094 write-access probe,
/// the existing file's read, the scheme branch (A/C: the ALG-033
/// comparison against the built managed text; B: the real ALG-031
/// splice of the NEW managed text into the file just read, compared
/// byte for byte — a decision identical to phase 1's markerless rule
/// on this fixture, whose marker line is a constant, and STRICTER in
/// general, so it cannot understate the row), and the D-088 stale-tmp
/// sweep (step 7) — then the free-space tail, phase 2's verified
/// copies for every ready project whose file exists, and phase 3's
/// §9.5 atomic writes. Every step is the production seam function the
/// src-tauri phase calls; the app-shell orchestration cut around them
/// is enumerated in the crate docs ("No run bookkeeping for the sync
/// pass"), and §9.4 phase 1 step 6's D-25 confirmation list is not
/// built — the fixture's builds are never empty, so the branch is
/// unreachable and costless.
///
/// The `fs_phases_100_projects` diagnostic runs the same pass over
/// content the unmeasured setup built, so the row's cost splits
/// between the store/build half and the fs half (the D-121/D-135
/// breakdown pattern).
///
/// # The alternation
///
/// A no-op path exists and would poison the measurement: after a
/// pass, every target file holds the built content, and §9.4 phase 1
/// step 5 (D-24/ALG-033 — the marker line never decides) would read
/// every project `unchanged`, measuring a cheap pass from the second
/// iteration on. The harness therefore flips ONE assignment per
/// project between iterations — project `p`'s `k = 0` pair
/// (D-109's closed form at the origin, scale-independent: instruction
/// ordinal `(37·p) mod I`, variant ordinal `p mod V`), moved to its
/// sibling variant and back — inside the **unmeasured** per-iteration
/// setup (`criterion`'s `iter_batched`), the D-157 alternation trick
/// scaled to the whole fixture: every measured pass sees every
/// project `ready`, every existing file backed up, every file
/// rewritten. The premise (each pair exists and holds the closed
/// form's variant) is verified at construction and pinned by
/// `tests/sync_pipeline_pins.rs`.
///
/// The scheme-B half of the fixture starts with hand-created marker
/// files (SYS-009: a missing scheme-B target is the no-markers skip,
/// never a creation), so the pass's B branch splices into a real
/// marked file exactly the way a live project would.
#[derive(Debug)]
pub struct SyncFixture {
    /// The generated world: connection, database home, Git roots.
    fixture: LoadFixture,
    /// One entry per active project in `sort_order`: the identity and
    /// write settings the fs phases need (captured once — the write
    /// settings cannot change while a benchmark holds the fixture).
    metas: Vec<ProjectMeta>,
    /// One toggle per project (the alternation state).
    toggles: Vec<ProjectToggle>,
    /// The backups data directory (§5.5's root; swept between
    /// iterations by the unmeasured setup).
    data_dir: TempDir,
    /// ALG-032's marker line, fixed for the fixture's lifetime (the
    /// comparison never looks at it — D-24).
    marker_line: String,
    /// The pass counter behind [`SyncFixture::next_backup_time`] —
    /// each pass's copies must land under fresh names (a `create_new`
    /// copy never overwrites), so the injected run time advances one
    /// second per pass (D-061: time is a parameter, never a clock).
    pass_no: u16,
}

/// One project's pass inputs that the ALG-001 snapshot owns but
/// [`BuiltProject`] does not carry.
#[derive(Debug, Clone)]
pub struct ProjectMeta {
    /// The project's typed id (the backup name builder's input).
    pub project_id: ProjectId,
    /// The display name (the backup slug's input).
    pub name: String,
    /// The canonical repository root (preflight steps 1–2's input).
    pub root: PathBuf,
    /// The target file's name (§5.7's enum output, step 2's input).
    pub file_name: String,
    /// The write scheme (the A/C and B branches).
    pub scheme: Scheme,
    /// The base name (the backup name builder's input).
    pub base_name: BaseName,
}

/// The alternation state of one project's `k = 0` assignment pair.
#[derive(Debug)]
struct ProjectToggle {
    /// The project whose build this toggle flips.
    project_id: String,
    /// The pair's instruction (`k = 0` of D-109's closed form).
    instruction_id: String,
    /// The variant the closed form starts the row on (`p mod V`).
    variant_a: String,
    /// The sibling the flip moves to (`(p + 1) mod V`).
    variant_b: String,
    /// Which variant the row holds right now — flipped only by a
    /// successful [`SyncFixture::flip_assignments`], so the state
    /// never desyncs from the row (the D-157 `holds_a` invariant).
    holds_a: bool,
}

/// What one composed pass decided and did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassStats {
    /// Projects phase 1 marked ready (existing or creation writes).
    pub ready: usize,
    /// Projects the ALG-033 comparison left unchanged (D-24).
    pub unchanged: usize,
    /// Projects refused by a preflight step (no backup, no write).
    pub skipped: usize,
    /// Verified backup copies phase 2 created.
    pub backed_up: usize,
    /// Files phase 3's atomic writes laid down.
    pub written: usize,
}

/// The marker line's fixed instant: any valid constant works (the
/// comparison drops the marker line; the write only carries it), and
/// a constant keeps the measured bytes identical across iterations.
const MARKER_TIME: MarkerTime = MarkerTime {
    year: 2026,
    month: 11,
    day: 22,
    hour: 0,
    minute: 0,
    second: 0,
};

/// The scheme-B placeholder a fresh fixture's marked files start
/// from: one valid marker pair in order (D-19) with nothing managed
/// between them and a line of outside prose — the shape a user's
/// first "paste the marker block" produces, and everything the
/// ALG-031 splice needs to preserve.
const SCHEME_B_SEED: &str = "Project notes live outside the managed block.\n\
                             <!-- EUNOMIASYNC:BEGIN -->\n\
                             <!-- EUNOMIASYNC:END -->\n";

impl SyncFixture {
    /// Builds the sync fixture for `spec`: the generated world, the
    /// per-project pass inputs, the verified toggles, and the backups
    /// data directory. Scheme-B targets are NOT created here — call
    /// [`SyncFixture::create_scheme_b_targets`] before the priming
    /// pass for the full mixed-scheme world (the bench does; the
    /// SYS-009 pin deliberately does not).
    ///
    /// # Errors
    ///
    /// [`LoadError::TogglePremise`] when the generator's closed form
    /// no longer holds at some project's `k = 0` pair (a generator
    /// change — the D-157 premise discipline, per project), the
    /// fixture/generator errors of [`LoadFixture::create`], and
    /// [`LoadError::Snapshot`] for the settings read.
    pub fn create(spec: &DataSetSpec, label: &str) -> Result<Self, LoadError> {
        let mut fixture = LoadFixture::create(spec, label)?;
        let snapshot = read_snapshot(fixture.connection())?;
        let metas: Vec<ProjectMeta> = snapshot
            .projects()
            .iter()
            .map(|project| {
                let settings = &project.settings;
                ProjectMeta {
                    project_id: project.id.clone(),
                    name: project.name.clone(),
                    root: project.root.clone(),
                    file_name: target_file_name(settings.scheme, settings.base_name).to_owned(),
                    scheme: settings.scheme,
                    base_name: settings.base_name,
                }
            })
            .collect();
        let toggles = Self::build_toggles(fixture.generated(), spec, &metas)?;
        let mut built = Self {
            fixture,
            metas,
            toggles,
            data_dir: TempDir::new().map_err(LoadError::Io)?,
            marker_line: creation_marker_line(MARKER_TIME, None, 0),
            pass_no: 0,
        };
        built.verify_toggles()?;
        Ok(built)
    }

    /// One toggle per project, from D-109's closed form at `k = 0`:
    /// instruction ordinal `(37·p) mod I`, variant ordinal
    /// `p mod V`, sibling `(p + 1) mod V`.
    fn build_toggles(
        generated: &GeneratedDataSet,
        spec: &DataSetSpec,
        metas: &[ProjectMeta],
    ) -> Result<Vec<ProjectToggle>, LoadError> {
        let mut toggles = Vec::with_capacity(metas.len());
        for (ordinal, meta) in metas.iter().enumerate() {
            let instruction = (37 * ordinal) % spec.instructions;
            let held_ordinal = ordinal % spec.variants_per_instruction;
            let flip_ordinal = (ordinal + 1) % spec.variants_per_instruction;
            let refused = |half: PremiseHalf| LoadError::TogglePremise {
                half,
                held: String::new(),
                expected: String::new(),
            };
            let variants = generated
                .variant_ids
                .get(instruction)
                .ok_or_else(|| refused(PremiseHalf::MissingShape))?;
            let variant_a = variants
                .get(held_ordinal)
                .ok_or_else(|| refused(PremiseHalf::MissingShape))?;
            let variant_b = variants
                .get(flip_ordinal)
                .ok_or_else(|| refused(PremiseHalf::MissingShape))?;
            toggles.push(ProjectToggle {
                project_id: meta.project_id.as_str().to_owned(),
                instruction_id: generated.instruction_ids[instruction].clone(),
                variant_a: variant_a.clone(),
                variant_b: variant_b.clone(),
                holds_a: true,
            });
        }
        Ok(toggles)
    }

    /// Verifies every toggle's premise through the store's own read
    /// (the constructor computed the pairs from the closed form; this
    /// checks them against the rows).
    fn verify_toggles(&mut self) -> Result<(), LoadError> {
        for toggle in &self.toggles {
            let current = library::get_assignment(
                self.fixture.connection(),
                &toggle.project_id,
                &toggle.instruction_id,
            )
            .map_err(LoadError::Library)?
            .ok_or_else(|| LoadError::TogglePremise {
                half: PremiseHalf::NoAssignment,
                held: String::new(),
                expected: toggle.variant_a.clone(),
            })?;
            let held = match current.target {
                AssignmentTarget::Variant(variant_id) => variant_id,
                AssignmentTarget::Custom(_) | AssignmentTarget::Extend { .. } => String::new(),
            };
            if held != toggle.variant_a {
                return Err(LoadError::TogglePremise {
                    half: PremiseHalf::VariantMismatch,
                    held,
                    expected: toggle.variant_a.clone(),
                });
            }
        }
        Ok(())
    }

    /// Creates the scheme-B targets as marked seed files (SYS-009:
    /// the pass refuses to CREATE a B target — a user pastes the
    /// marker block once, and this is that paste). Idempotent: a
    /// target that already exists is left alone.
    ///
    /// # Errors
    ///
    /// [`LoadError::Io`] when a file cannot be written.
    pub fn create_scheme_b_targets(&self) -> Result<(), LoadError> {
        for meta in &self.metas {
            if meta.scheme != Scheme::B {
                continue;
            }
            let target = meta.root.join(&meta.file_name);
            if !target.exists() {
                fs::write(target, SCHEME_B_SEED).map_err(LoadError::Io)?;
            }
        }
        Ok(())
    }

    /// Flips EVERY project's toggle assignment to its sibling (A→B,
    /// B→A, …) — the unmeasured between-iterations setup that keeps
    /// every measured pass a full write pass. The state flips only on
    /// successful writes (the D-157 invariant).
    ///
    /// # Errors
    ///
    /// [`LoadError::Library`] when a store write fails.
    pub fn flip_assignments(&mut self) -> Result<(), LoadError> {
        for toggle in &mut self.toggles {
            let target = if toggle.holds_a {
                toggle.variant_b.clone()
            } else {
                toggle.variant_a.clone()
            };
            library::reassign(
                self.fixture.connection(),
                &toggle.project_id,
                &toggle.instruction_id,
                &target,
            )
            .map_err(LoadError::Library)?;
            toggle.holds_a = !toggle.holds_a;
        }
        Ok(())
    }

    /// Reads a fresh snapshot and builds every project's content —
    /// the fs-only diagnostic's unmeasured setup half (the full pass
    /// does this inside its measured iteration instead).
    ///
    /// # Errors
    ///
    /// The [`read_snapshot`] errors.
    pub fn build_content(&mut self) -> Result<Vec<BuiltProject>, LoadError> {
        let snapshot = read_snapshot(self.fixture.connection())?;
        Ok(build_projects(&snapshot))
    }

    /// Removes the backups tree — the unmeasured between-iterations
    /// sweep that stands in for phase 4's retention (the pass's
    /// copies would otherwise accumulate one generation per
    /// iteration).
    ///
    /// # Errors
    ///
    /// [`LoadError::Io`] when the tree exists and cannot be removed.
    pub fn sweep_backups(&self) -> Result<(), LoadError> {
        let backups = self.data_dir.path().join("backups");
        match fs::remove_dir_all(&backups) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(LoadError::Io(source)),
        }
    }

    /// The composed §9.4 core over `built` (see the type docs for the
    /// step order and the cut seams). `built` must cover every
    /// project in `sort_order` — [`SyncFixture::build_content`] or
    /// the same pass's own build produces it.
    ///
    /// # Errors
    ///
    /// The typed seam errors of any refused step ([`LoadError`]'s fs
    /// arms). A refused seam ends the pass here — a generated fixture
    /// cannot produce one; production instead turns the same refusal
    /// into a per-project skip at that step and the run continues
    /// (phase 1's own rule, `sync_phase1.rs`).
    pub fn run_pass(&mut self, built: &[BuiltProject]) -> Result<PassStats, LoadError> {
        assert_eq!(
            built.len(),
            self.metas.len(),
            "the built content covers every project"
        );
        let mut stats = PassStats::default();
        let time = self.next_backup_time();
        let now = SystemTime::now();

        // One ready project's collected write recipe.
        struct Ready {
            resolved: PathBuf,
            final_bytes: Vec<u8>,
            /// The existing file's size at read time (step 4's own
            /// bytes), `None` for the creation case — production's
            /// `ReadyProject::existing_size` shape: the free-space
            /// tail's input and phase 2's eligibility, with no second
            /// stat of the target.
            existing_len: Option<u64>,
            project_id: ProjectId,
            name: String,
            scheme: Scheme,
            base_name: BaseName,
        }
        let mut readies: Vec<Ready> = Vec::with_capacity(built.len());

        // Phase 1 — the seven-step preflight, per project, in order.
        for (meta, project) in self.metas.iter().zip(built) {
            // Step 1 — availability (D-084).
            check_project_available(&meta.root).map_err(LoadError::Unavailable)?;
            // Step 2 — the resolved target path (D-092).
            let target =
                resolve_target_path(&meta.root, &meta.file_name).map_err(LoadError::Target)?;
            // The regular-file gate (the app layer's own ruling,
            // between steps 2 and 3: a non-file entry is a skip, not
            // a hang).
            match fs::metadata(target.path()) {
                Ok(metadata) if metadata.is_file() => {}
                Ok(_) => {
                    stats.skipped += 1;
                    continue;
                }
                Err(_) => {}
            }
            // Step 3 — write rights (D-094).
            check_write_access(target.path()).map_err(LoadError::Access)?;
            // Step 4 — the existing file (the creation case is None).
            let existing = match fs::read(target.path()) {
                Ok(bytes) => Some(bytes),
                Err(source) if source.kind() == io::ErrorKind::NotFound => None,
                Err(source) => return Err(LoadError::Io(source)),
            };
            // The size at read time — production's `existing_size`
            // shape, carried (not re-statted) into the phases below.
            let existing_len = existing.as_ref().map(|bytes| bytes.len() as u64);
            // Steps 4–6 — the scheme branch: build the final bytes and
            // decide `unchanged` (step 5) against them.
            let (final_bytes, unchanged) = match meta.scheme {
                Scheme::A | Scheme::C => {
                    let final_text = whole_file(&project.managed, &self.marker_line);
                    let unchanged = existing.is_some_and(|bytes| {
                        contents_match(&bytes, &markerless_whole(&project.managed))
                    });
                    (final_text.into_bytes(), unchanged)
                }
                Scheme::B => {
                    let Some(bytes) = existing else {
                        // SYS-009: a missing scheme-B target is the
                        // no-markers skip, never a creation.
                        stats.skipped += 1;
                        continue;
                    };
                    let Ok(text) = String::from_utf8(bytes) else {
                        // E_NOT_UTF8: a skip, the others continue.
                        stats.skipped += 1;
                        continue;
                    };
                    let Ok(span) = markers::parse(&text) else {
                        // E_MARKERS_INVALID: a skip, the others continue.
                        stats.skipped += 1;
                        continue;
                    };
                    // The real ALG-031 splice of the NEW managed text
                    // into the file just read — the write's own bytes
                    // and the comparison basis in one builder: the
                    // file is unchanged when splicing the new content
                    // reproduces its current bytes. Identical to
                    // phase 1's markerless ALG-033 decision for this
                    // fixture (the run's marker line is the file's —
                    // a constant); in general this formulation is
                    // STRICTER (a differently-stamped marker line
                    // would take the write path production would
                    // leave unchanged), so it cannot understate the
                    // row.
                    let final_text = splice(&text, &span, &project.managed, &self.marker_line);
                    let unchanged = final_text.as_bytes() == text.as_bytes();
                    (final_text.into_bytes(), unchanged)
                }
            };
            // Step 7 — the stale-tmp sweep (D-088), best-effort: a
            // refused listing never skips a project (production's own
            // rule — the outcome is logged there, ignored here).
            let folder = target.path().parent().unwrap_or(target.path()).to_owned();
            let _ = remove_stale_tmp_files(&folder, target.file_name(), now);
            if unchanged {
                stats.unchanged += 1;
                continue;
            }
            stats.ready += 1;
            readies.push(Ready {
                resolved: target.path().to_owned(),
                final_bytes,
                existing_len,
                project_id: meta.project_id.clone(),
                name: meta.name.clone(),
                scheme: meta.scheme,
                base_name: meta.base_name,
            });
        }

        // The free-space tail: the ready projects' existing files are
        // the copies phase 2 would make (§9.4's closing rule) — sized
        // by the bytes step 4 read (production's `existing_size`
        // shape, no second stat), and an EMPTY copy list skips the
        // probe outright: zero required bytes cannot come up short
        // (phase 1's own rule).
        let copies: Vec<u64> = readies
            .iter()
            .filter_map(|ready| ready.existing_len)
            .collect();
        if !copies.is_empty() {
            check_backup_space(&copies, || free_bytes(self.data_dir.path()))
                .map_err(LoadError::Space)?;
        }

        // Phase 2 — backups: for every ready project whose target
        // file exists (`existing_len.is_some()`, phase 2's own
        // eligibility rule), one verified copy (BKP-010 inside the fs
        // operation). The BKP-010 dedup lookup is the store-half cut;
        // `None` is the honest "no record consulted" input, and the
        // alternation makes every copy a fresh digest anyway.
        for ready in &readies {
            if ready.existing_len.is_none() {
                continue;
            }
            let outcome = backup_target_file(&BackupTarget {
                source: &ready.resolved,
                data_dir: self.data_dir.path(),
                project_id: &ready.project_id,
                project_name: &ready.name,
                scheme: ready.scheme,
                base_name: ready.base_name,
                time,
                last_backup_sha256_hex: None,
            })
            .map_err(LoadError::Backup)?;
            if matches!(outcome, eunomia_fs::backup::BackupOutcome::Created(_)) {
                stats.backed_up += 1;
            }
        }

        // Phase 3 — the writes: §9.5's atomic pipeline per project,
        // one by one, in order (the free-space closure is
        // production's own: the target's folder).
        for ready in &readies {
            let folder = ready
                .resolved
                .parent()
                .unwrap_or(&ready.resolved)
                .to_owned();
            write_target_file(&ready.resolved, &ready.final_bytes, || free_bytes(&folder))
                .map_err(LoadError::Write)?;
            stats.written += 1;
        }

        Ok(stats)
    }

    /// The injected run time of the next pass: one second later than
    /// the last, so every pass's `create_new` copies land under fresh
    /// names (a collision would be a `CopyFailed`, and the names must
    /// differ however many passes run). The counter is a `u16`: past
    /// 65 535 passes (hours of benching at this row's per-pass cost)
    /// the arithmetic would overflow — a bound no run approaches.
    fn next_backup_time(&mut self) -> BackupTime {
        self.pass_no += 1;
        let n = u32::from(self.pass_no);
        BackupTime {
            year: 2026,
            month: 11,
            day: 22,
            hour: ((n / 3600) % 24) as u8,
            minute: ((n / 60) % 60) as u8,
            second: (n % 60) as u8,
            millisecond: 0,
        }
    }

    /// The fixture's connection (the snapshot reads and the toggle
    /// writes go through it).
    pub fn connection(&mut self) -> &mut Connection {
        self.fixture.connection()
    }

    /// The created row ids in creation order (the pins recompute the
    /// closed form against them, independently of the toggle code).
    pub fn generated(&self) -> &GeneratedDataSet {
        self.fixture.generated()
    }

    /// ALG-032's marker line the pass stamps (a fixed constant — the
    /// pins rebuild the expected file bytes with it).
    #[must_use]
    pub fn marker_line(&self) -> &str {
        &self.marker_line
    }

    /// The backups data directory's path (the pin tests inspect the
    /// copies phase 2 created).
    pub fn data_dir(&self) -> &Path {
        self.data_dir.path()
    }

    /// The per-project metas, borrowed (the pin tests read the scheme
    /// distribution and the target paths).
    pub fn metas(&self) -> &[ProjectMeta] {
        &self.metas
    }
}

/// ALG-030's whole-file form over an ALREADY-BUILT managed text:
/// `managed`, one empty line, the marker, one line break — or, when
/// the build is empty, just the marker and its break. The same
/// three-line mirror production's `sync_phase3` spells (its own docs
/// call it `whole_file`, pinned there against the real builder); this
/// copy is pinned the same way — against `build_content`'s output —
/// by `tests/sync_pipeline_pins.rs`.
fn whole_file(managed: &str, marker_line: &str) -> String {
    if managed.is_empty() {
        format!("{marker_line}\n")
    } else {
        format!("{managed}\n\n{marker_line}\n")
    }
}

/// The A/C comparison basis: ALG-030's shape minus the marker line —
/// `managed` plus its blank line, or the empty string for an empty
/// build ([`contents_match`]'s documented example is the oracle; the
/// pins exercise it through the unchanged path).
fn markerless_whole(managed: &str) -> String {
    if managed.is_empty() {
        String::new()
    } else {
        format!("{managed}\n\n")
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

/// §12.2's scan row tree shape: "Scan of 100,000 folders on an SSD —
/// no more than 30 s". The tree is a uniform pyramid of plain
/// folders with the SECOND level's folders planted as repositories
/// (a `.git` directory each — ALG-052's probe form, the scan tests'
/// own fixture idiom): `groups` top-level folders, each with
/// `subgroups` repository folders, each of those with `leaves`
/// folders, each of those with `leaf_children` plain folders. Depth
/// 4, within the §6.5 default `scan.max_depth` of 6.
///
/// [`SCAN_TREE`] is the row's instance: 101 100 folders below the
/// root (≥ the row's 100 000 floor), 1 000 repositories among them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanTreeSpec {
    /// Top-level group folders.
    pub groups: usize,
    /// Repository folders per group (the second level).
    pub subgroups: usize,
    /// Plain folders per subgroup (the third level).
    pub leaves: usize,
    /// Plain folders per leaf (the fourth level).
    pub leaf_children: usize,
}

/// The scan row's tree: 100 × 10 × 10 × 9 — 101 100 folders below
/// the root, 1 000 repositories, depth 4 under the §6.5 default
/// depth of 6. Every folder the walk visits gets exactly one `.git`
/// probe (ALG-052), so the tree's cost profile is the row's: one
/// directory listing plus one metadata probe per folder.
pub const SCAN_TREE: ScanTreeSpec = ScanTreeSpec {
    groups: 100,
    subgroups: 10,
    leaves: 10,
    leaf_children: 9,
};

impl ScanTreeSpec {
    /// The NON-`.git` directory count, the root included: every
    /// folder the walk reports a found/plain event for. The walk's
    /// `checked` count is [`ScanTreeSpec::dir_count`] — this number
    /// PLUS the planted `.git` folders, each of which is itself one
    /// excluded-skip event.
    #[must_use]
    pub fn folder_count(&self) -> usize {
        let second = self.groups * self.subgroups;
        let third = second * self.leaves;
        1 + self.groups + second + third + third * self.leaf_children
    }

    /// The planted repositories (the second level, one per
    /// subgroup).
    #[must_use]
    pub fn repository_count(&self) -> usize {
        self.groups * self.subgroups
    }

    /// Every directory on disk, `.git` folders included — the census
    /// the pin test's independent `std::fs` walk counts.
    #[must_use]
    pub fn dir_count(&self) -> usize {
        self.folder_count() + self.repository_count()
    }

    /// The walk's event count: every directory produces exactly one
    /// event (found, plain, or excluded-skip for the `.git` folders).
    #[must_use]
    pub fn event_count(&self) -> usize {
        self.dir_count()
    }
}

/// A generated scan tree under a self-cleaning temporary directory.
#[derive(Debug)]
pub struct ScanTree {
    root: PathBuf,
    _home: TempDir,
}

impl ScanTree {
    /// Lays out the tree of `spec` under a fresh temporary directory:
    /// `group-<g>` / `sub-<s>` (each with a `.git` directory) /
    /// `leaf-<l>` / `child-<c>`. Nothing else is created — no files,
    /// no ignore files: ALG-054's premise (the scan reads only
    /// directory entry names) keeps the walk's cost a function of the
    /// folder count alone.
    ///
    /// # Errors
    ///
    /// [`LoadError::Io`] for any refused directory creation.
    pub fn create(spec: &ScanTreeSpec) -> Result<Self, LoadError> {
        let home = TempDir::new().map_err(LoadError::Io)?;
        let root = home.path().join("scan-tree");
        fs::create_dir(&root).map_err(LoadError::Io)?;
        for group in 0..spec.groups {
            let group_dir = root.join(format!("group-{group:02}"));
            fs::create_dir(&group_dir).map_err(LoadError::Io)?;
            for sub in 0..spec.subgroups {
                let repo = group_dir.join(format!("sub-{sub:02}"));
                // `create_dir_all`: the repository folder and its
                // `.git` arrive together (ALG-052's probe form).
                fs::create_dir_all(repo.join(".git")).map_err(LoadError::Io)?;
                for leaf in 0..spec.leaves {
                    let leaf_dir = repo.join(format!("leaf-{leaf:03}"));
                    fs::create_dir(&leaf_dir).map_err(LoadError::Io)?;
                    for child in 0..spec.leaf_children {
                        fs::create_dir(leaf_dir.join(format!("child-{child:02}")))
                            .map_err(LoadError::Io)?;
                    }
                }
            }
        }
        Ok(Self { root, _home: home })
    }

    /// The tree's root — the scan root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// The scan options of the §6.5 defaults — the honest configuration
/// a real scan runs with. Built from the SETTINGS REPOSITORY's own
/// default values (`SettingKey::default_value()`, pure — no
/// database), not re-spelled here: the store owns the §6.5 table's
/// defaults, and a settings change must reach the bench through the
/// same code production reads. `tests/scan_walk_pins.rs` pins the
/// wired values against the spec table's literals, so a store change
/// turns the harness red until the bench configuration is revisited.
#[must_use]
pub fn default_scan_options() -> ScanOptions {
    let depth = match SettingKey::ScanMaxDepth.default_value() {
        SettingValue::Int(depth) => usize::try_from(depth)
            .expect("the store's scan-depth default is a non-negative integer"),
        _ => panic!("the store's scan-depth default changed shape — a settings bug"),
    };
    let exclude_names = match SettingKey::ScanExcludeNames.default_value() {
        SettingValue::StrList(names) => names,
        _ => panic!("the store's exclusion-list default changed shape — a settings bug"),
    };
    ScanOptions {
        max_depth: NonZeroUsize::new(depth)
            .expect("the store's scan-depth default is non-zero by the spec's own table"),
        exclude_names,
    }
}

/// One complete scan of `root` under `options`, counting events the
/// way the command layer's tracker does (D-153's `checked`/`found`
/// arithmetic: every event is one folder the walk made a decision
/// about). The callback is the bench's whole consumer — the
/// surroundings the row's walk half does not include are the command
/// layer's: the progress throttle (its per-event mutex and clock
/// check), the cancel slot, the SEC-003 root gate, and the ALG-053
/// classification of found repositories
/// (`scan_classify::classify_entry`, T-075's module with its own
/// tests) — a per-repository `path_key` and registry lookups the
/// bench's two atomic increments do not perform.
///
/// The walk is read-only, so every iteration does identical full
/// work: there is no no-op path and the walk needs no alternation
/// (the honest difference from the sync pass, stated in the bench's
/// docs).
#[must_use]
pub fn scan_counting(root: &Path, options: &ScanOptions) -> (ScanOutcome, usize, usize) {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let checked = AtomicUsize::new(0);
    let found = AtomicUsize::new(0);
    let cancel = CancelToken::new();
    let outcome = scan_repositories(root, options, &cancel, |event| {
        checked.fetch_add(1, Ordering::Relaxed);
        if matches!(event, ScanEvent::Repository { .. }) {
            found.fetch_add(1, Ordering::Relaxed);
        }
    });
    (outcome, checked.into_inner(), found.into_inner())
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
