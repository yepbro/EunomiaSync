//! Deterministic benchmark data sets M and L (spec §12.2, task
//! T-136).
//!
//! §12.2 checks its performance targets against two named data sets:
//! **M** — 50 projects and 200 instructions — and **L**, the
//! upper-limit scenario — 500 projects, 1000 instructions, 100
//! headings, 3 variants per instruction, and 100,000 assignments.
//! This module is the generator that fills a database with exactly
//! those shapes for the T-137+ benchmark tasks, plus every other
//! caller that needs a reproducibly sized database (load tests,
//! manual profiling).
//!
//! # Filled through the repositories, never around them
//!
//! [`generate`] writes every row through the repositories' own
//! create methods — [`crate::projects::create`],
//! [`crate::library::create_item`], [`crate::library::create_variant`]
//! and [`crate::library::assign`] — inside one transaction it **owns**
//! (`conn.transaction()`, which is why it takes `&mut Connection`:
//! this is the standalone-connection seam for benchmarks and test
//! fixtures, not the D-037 executor composition — an app-layer
//! operation that must join `Db::run`'s transaction composes the
//! repositories directly, the way the command tasks do).
//! The generator therefore cannot produce a row the production code
//! paths could not have written: DB-010's constructive main-variant
//! rules, DB-011's dense `position` sequence, DB-005's table-wide
//! `sort_order` and the D-02 one-assignment-per-pair rule all hold
//! because the repositories enforce them, not because the generator
//! re-implements them. Dense positions are a creation-time append
//! (`MAX + 1`), so items arrive dense `1..=N` and stay that way.
//!
//! # Determinism, defined
//!
//! The generator draws **no random numbers at all** — there is no
//! seed because there is no RNG. Every name, color, scheme, variant
//! text and assignment pair is a pure function of the profile counts
//! and the row's ordinal (the formulas are below, pinned by tests
//! that recompute them independently of this module). What it cannot
//! control are the columns the repositories own: row ids are fresh
//! UUIDv7 ([`crate::values::new_id`], DB-002/D-031) and timestamps
//! are wall-clock ([`crate::values::now_utc`]) — both deliberately
//! non-deterministic.
//!
//! So the determinism contract is defined over a **stabilized view**:
//! the ALG-001 collections read back through the repositories with
//! ids replaced by creation ordinals and timestamps dropped —
//!
//! * projects in `sort_order` order (creation order here): path,
//!   `path_key`, name, label, color, description, state, `added_via`,
//!   scheme, base name, both extra-file options;
//! * library items in `position` order: kind, heading level, title,
//!   state — the instruction ordinal is the item's index among the
//!   instructions of that list;
//! * every active variant in creation order (`created_at, id`) within
//!   its instruction: instruction ordinal, name, content, main flag;
//! * the assignments of the active projects as
//!   `(project ordinal, instruction ordinal, variant ordinal)`
//!   triples, sorted lexicographically — an unordered-set
//!   comparison.
//!
//! [`stabilized`] is that definition as an executable function:
//! **two runs of [`generate`] with the same profile and the same
//! roots produce equal stabilized views** (pinned by a test; the raw
//! ids of the two runs differ, which is exactly why the definition
//! stabilizes them). The base-prefix stripping is **by components**,
//! not by text: `stabilized`'s `base` names the canonical base
//! directory as text, and as many leading path components as that
//! text holds are dropped from both `path` and `path_key` (split on
//! `/` and `\`, empty components ignored, suffixes rejoined with
//! `/`). Component counting — not text-prefix matching — is what
//! makes the definition platform-true: DB-030's keying changes
//! spelling (separator runs, NFC, case folding on Windows/macOS) but
//! never component structure, so one count strips both columns on
//! every platform, and two runs under different temporary
//! directories compare equal — the suffix (`repo-0007`) identifies a
//! root, not the machine's scratch path.
//!
//! # The generated shapes
//!
//! All rows are **active** — §12.2 measures the plain upper-limit
//! scenario, not lifecycle variety: every project active, every
//! instruction active and (DB-010) carrying exactly one active main
//! variant, no hidden items, no archived projects.
//!
//! * *Projects* (`i` = ordinal): name `Benchmark Project 0007`, label
//!   `P0007` (≤ 6 visible characters up to `i = 99_999`), color from
//!   the DB-040 palette by `i % 12`, scheme `A/B/C` by `i % 3`, base
//!   name by `i % 2`, the extra-file options cycling with
//!   `i % 2`/`i % 3`, `added_via = scan` (the roots are scan-found
//!   repositories in every intended caller).
//! * *Library layout*: the headings partition the instructions into
//!   consecutive, near-equal groups (`instructions / headings` each,
//!   the first `instructions % headings` one larger); heading `h`
//!   gets level `2 + h % 5` and title `Section 0007`, its
//!   instructions titles `Instruction 0007`. `headings = 0` (set M)
//!   lays the instructions out flat, before any heading — legal per
//!   ALG-021.
//! * *Variants*: `variants_per_instruction` per instruction, the
//!   first one the main (DB-010's constructive rule), named from
//!   `Main`, `Compact`, `Detailed` and, beyond the third, numbered
//!   `Alternative 03`… — distinct NOCASE names for any count; the
//!   texts are short fixed LF-only prose templates parameterized by
//!   the instruction ordinal — no reserved marker lines (DB-016),
//!   sized so a fully assigned set-L project builds a file of about
//!   §12.2's "up to 10 KB" sync premise (the three templates average
//!   ≈52 characters; 200 assignments ≈ 10 KB).
//! * *Assignments*: project `p` receives exactly
//!   `assignments_per_project` distinct instructions — number `k` is
//!   instruction `(ASSIGNMENT_STRIDE·p + k) mod instructions` with
//!   [`ASSIGNMENT_STRIDE`] = 37 as the spread constant.
//!   Distinctness needs no arithmetic beyond the
//!   `assignments_per_project <= instructions` check: two `k` below
//!   the instruction count never collide modulo it. The stride
//!   decides where each project's window lands — 37 hops the whole
//!   library from a per-project offset instead of clustering in one
//!   section, so the matrix mixes dense and sparse instructions
//!   rather than repeating a handful of bands. The pinned totals:
//!   set M carries 50·40 = 2,000 assignments and set L
//!   500·200 = 100,000, every instruction included (≈100
//!   assignments each in set L). The variant is `(p + k)` modulo
//!   the instruction's variant count, spreading the three variants
//!   evenly.
//!
//! # Calling contract
//!
//! * The database must hold **no rows in the two tables the generator
//!   writes** (`projects` and `library_items`; `instruction_variants`
//!   and `project_instructions` are foreign-keyed below them and
//!   empty with their parents — `settings`, `action_log`,
//!   `project_exclusions` and `sync_runs` carry no generated data and
//!   are never touched). A non-empty database is rejected with
//!   [`DataSetError::NotEmpty`] before the first write: generation is
//!   a fresh-fixture operation, not a merge. The retry story is the
//!   transaction — a failed run rolls back completely, so re-running
//!   after a failure converges on one full data set (the
//!   make-operations-idempotent rule: derive from the state that
//!   exists, and the only states are "empty" or "rejected").
//! * `roots` supplies the canonical path and identity key of every
//!   project root (the [`ProjectRoot`] pair the repositories store
//!   verbatim — this crate computes no keys; DB-030 is
//!   `eunomia_fs::paths`' job). Intended source:
//!   `eunomia_fs::temp_repos::TempRepos`, which lays out real
//!   `<root>/.git` repositories under a self-cleaning temp directory
//!   for the sync benchmarks; at least `spec.projects` roots are
//!   required, surplus roots are ignored.

use std::collections::HashMap;
use std::fmt;

use rusqlite::Connection;

use crate::library::{
    self, ItemKind, ItemState, LibraryError, NewAssignment, NewLibraryItem, NewVariant,
};
use crate::projects::{self, AddedVia, NewProject, ProjectState, ProjectsError};
use crate::settings::{BaseName, ExtraFile, Position, Scheme};

// ---- Profiles -------------------------------------------------------------------

/// The shape of a generated data set: §12.2's counts, parameterized so
/// the two named sets are constants and a benchmark may still ask for
/// a smaller smoke shape.
///
/// The invariants [`generate`] needs hold for any spec that passes its
/// typed checks: `assignments_per_project <= instructions` (one
/// assignment per project-instruction pair, D-02 — distinctness
/// follows from `k < assignments_per_project <= instructions` alone,
/// the stride only places the window) and
/// `variants_per_instruction >= 1` (DB-010: an active instruction
/// needs its main variant).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataSetSpec {
    /// How many project rows to create (§12.2 "projects").
    pub projects: usize,
    /// How many section headings interleave the instructions (§12.2
    /// "headings").
    pub headings: usize,
    /// How many instruction items to create (§12.2 "instructions").
    pub instructions: usize,
    /// Variants per instruction, the first one main (§12.2 "3 variants
    /// per instruction" for L).
    pub variants_per_instruction: usize,
    /// Assignments each project receives; the total is
    /// `projects × assignments_per_project` (§12.2 "100,000
    /// assignments" for L).
    pub assignments_per_project: usize,
}

/// Data set **M** (§12.2): 50 projects, 200 instructions — 2,000
/// assignments, one main variant per instruction, no headings.
pub const M: DataSetSpec = DataSetSpec {
    projects: 50,
    headings: 0,
    instructions: 200,
    variants_per_instruction: 1,
    assignments_per_project: 40,
};

/// Data set **L** (§12.2), the confirmed upper-limit scenario: 500
/// projects, 1000 instructions, 100 headings, 3 variants per
/// instruction, 100,000 assignments.
pub const L: DataSetSpec = DataSetSpec {
    projects: 500,
    headings: 100,
    instructions: 1000,
    variants_per_instruction: 3,
    assignments_per_project: 200,
};

/// The assignment-spread constant (see the module docs): the base of
/// the per-project offset `(stride·p + k) mod instructions` — a hop
/// large enough that consecutive projects' windows interleave across
/// the whole library instead of stacking into bands.
pub const ASSIGNMENT_STRIDE: usize = 37;

/// The DB-040 palette literals, in DB-040's order — the generator
/// cycles them by project ordinal.
const PALETTE: [&str; 12] = [
    "red", "orange", "amber", "lime", "green", "teal", "cyan", "blue", "indigo", "violet", "pink",
    "slate",
];

/// Variant names: the first three by name (`Main` is always the main
/// variant), every further one numbered — distinct NOCASE names for
/// any variant count (§6.9's active-sibling uniqueness).
fn variant_name(variant: usize) -> String {
    match variant {
        0 => "Main".to_owned(),
        1 => "Compact".to_owned(),
        2 => "Detailed".to_owned(),
        further => format!("Alternative {further:02}"),
    }
}

/// The prose of variant `variant % 3` for instruction `instruction` —
/// LF-only, marker-free (DB-016), well under the DB-006 limit, and
/// varying with the instruction ordinal so builds over different
/// instructions produce different text. Sized for §12.2's sync
/// premise: the three templates average ≈52 characters, so a fully
/// assigned set-L project (200 assignments) builds ≈10 KB.
fn variant_content(instruction: usize, variant: usize) -> String {
    match variant % 3 {
        0 => format!(
            "Apply rule {instruction:04} to every module.\n\
             \n\
             Keep output reviewed."
        ),
        1 => format!("Follow rule {instruction:04}."),
        _ => format!(
            "Apply rule {instruction:04} to every module.\n\
             \n\
             Keep output reviewed.\n\
             \n\
             Re-check after updates."
        ),
    }
}

// ---- Input and output ------------------------------------------------------------

/// One project root for [`generate`]: the canonical path and the
/// identity key the repositories store verbatim (DB-030 — this crate
/// computes neither; `eunomia_fs::paths` owns the derivation and
/// `eunomia_fs::temp_repos` hands over ready-made pairs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectRoot<'a> {
    /// The canonical repository path (DB-030), stored verbatim.
    pub path: &'a str,
    /// The identity key (DB-030/DB-031), stored verbatim.
    pub path_key: &'a str,
}

/// What [`generate`] created: the row ids in creation order, so a
/// benchmark can address individual rows (toggle an assignment,
/// reassign a variant) without re-reading the whole database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedDataSet {
    /// Project ids in creation order — here also `sort_order` order.
    pub project_ids: Vec<String>,
    /// Instruction item ids in creation order — here also `position`
    /// order among the instructions.
    pub instruction_ids: Vec<String>,
    /// Variant ids grouped by instruction ordinal, in creation order
    /// within each instruction (the first is the main variant).
    pub variant_ids: Vec<Vec<String>>,
    /// How many assignments were created (`projects ×
    /// assignments_per_project`).
    pub assignment_count: usize,
}

// ---- Errors ----------------------------------------------------------------------

/// Why [`generate`] failed.
///
/// Hand-rolled impls, returned directly — the D-093/D-097 boundary:
/// no `StorageError` nesting, no `thiserror` in this crate.
#[derive(Debug)]
pub enum DataSetError {
    /// The database already holds rows — generation is a
    /// fresh-fixture operation. The counts name the offending tables;
    /// nothing was written.
    NotEmpty {
        /// Rows already in `projects`.
        projects: usize,
        /// Rows already in `library_items`.
        library_items: usize,
    },
    /// Fewer roots than the profile's project count. Nothing was
    /// written.
    NotEnoughRoots {
        /// The project count the profile asks for.
        needed: usize,
        /// The roots the caller supplied.
        provided: usize,
    },
    /// The spec asks for more assignments per project than there are
    /// instructions — the extra ones would repeat a
    /// project-instruction pair, which D-02 forbids. Nothing was
    /// written.
    AssignmentsPerProjectTooHigh {
        /// The requested per-project assignment count.
        requested: usize,
        /// The instruction count that caps it.
        instructions: usize,
    },
    /// The spec asks for zero instructions — there is nothing to
    /// assign or carry variants. Nothing was written.
    InstructionsRequired,
    /// The spec asks for zero variants per instruction — DB-010
    /// requires every active instruction to carry an active main
    /// variant. Nothing was written.
    VariantsRequired {
        /// The requested (zero) variant count.
        requested: usize,
    },
    /// A projects-repository call failed.
    Projects(ProjectsError),
    /// A library-repository call failed.
    Library(LibraryError),
    /// The wrapping transaction itself failed to begin or commit.
    Transaction(rusqlite::Error),
}

impl fmt::Display for DataSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataSetError::NotEmpty {
                projects,
                library_items,
            } => write!(
                f,
                "the database is not empty: {projects} project rows and {library_items} \
                 library item rows already exist"
            ),
            DataSetError::NotEnoughRoots { needed, provided } => {
                write!(f, "{needed} roots are required, {provided} were provided")
            }
            DataSetError::AssignmentsPerProjectTooHigh {
                requested,
                instructions,
            } => write!(
                f,
                "{requested} assignments per project exceed the {instructions} \
                 distinct instructions available"
            ),
            DataSetError::InstructionsRequired => {
                write!(f, "the profile asks for zero instructions")
            }
            DataSetError::VariantsRequired { requested } => write!(
                f,
                "{requested} variants per instruction cannot satisfy DB-010 (one \
                 active main variant is required)"
            ),
            DataSetError::Projects(err) => write!(f, "projects repository: {err}"),
            DataSetError::Library(err) => write!(f, "library repository: {err}"),
            DataSetError::Transaction(err) => write!(f, "transaction failed: {err}"),
        }
    }
}

impl std::error::Error for DataSetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DataSetError::Projects(err) => Some(err),
            DataSetError::Library(err) => Some(err),
            DataSetError::Transaction(err) => Some(err),
            _ => None,
        }
    }
}

/// Why [`stabilized`] failed: only the repositories' read errors —
/// the function writes nothing and validates nothing beyond what the
/// reads themselves do.
#[derive(Debug)]
pub enum StabilizeError {
    /// Reading the projects repository failed.
    Projects(ProjectsError),
    /// Reading the library repository failed.
    Library(LibraryError),
}

impl fmt::Display for StabilizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StabilizeError::Projects(err) => write!(f, "reading projects failed: {err}"),
            StabilizeError::Library(err) => write!(f, "reading the library failed: {err}"),
        }
    }
}

impl std::error::Error for StabilizeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StabilizeError::Projects(err) => Some(err),
            StabilizeError::Library(err) => Some(err),
        }
    }
}

// ---- Generation ------------------------------------------------------------------

/// Which table a [`row_count`] pre-check reads — the failure maps to
/// the repository family that owns the table, so the error never
/// lies about where it came from.
#[derive(Debug, Clone, Copy)]
enum CountedTable {
    /// The projects repository's `projects` table.
    Projects,
    /// The library repository's `library_items` table.
    LibraryItems,
}

impl CountedTable {
    /// The table's SQL name.
    const fn as_sql(self) -> &'static str {
        match self {
            CountedTable::Projects => "projects",
            CountedTable::LibraryItems => "library_items",
        }
    }
}

/// Counts the rows of `table` — the emptiness pre-check of
/// [`generate`].
fn row_count(conn: &Connection, table: CountedTable) -> Result<usize, DataSetError> {
    let count: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM {}", table.as_sql()),
            [],
            |row| row.get(0),
        )
        .map_err(|err| match table {
            CountedTable::Projects => DataSetError::Projects(ProjectsError::Sql(err)),
            CountedTable::LibraryItems => DataSetError::Library(LibraryError::Sql(err)),
        })?;
    Ok(usize::try_from(count).expect("a row count fits usize"))
}

/// Fills an **empty** database with the profile's data set through
/// the repositories' create methods, in one transaction, and returns
/// the created row ids in creation order.
///
/// # Errors
///
/// Returns [`DataSetError::NotEmpty`] when `projects` or
/// `library_items` already hold rows, [`DataSetError::NotEnoughRoots`]
/// when `roots` is shorter than the profile's project count, and
/// [`DataSetError::AssignmentsPerProjectTooHigh`] /
/// [`DataSetError::InstructionsRequired`] /
/// [`DataSetError::VariantsRequired`] for a spec whose assignment
/// counts or item counts cannot satisfy D-02 / DB-010 — all five
/// checked before the first write. A repository failure or a
/// transaction failure surfaces as [`DataSetError::Projects`] /
/// [`DataSetError::Library`] / [`DataSetError::Transaction`] with the
/// whole run rolled back, leaving the database empty for a clean
/// retry.
pub fn generate(
    conn: &mut Connection,
    spec: &DataSetSpec,
    roots: &[ProjectRoot<'_>],
) -> Result<GeneratedDataSet, DataSetError> {
    if roots.len() < spec.projects {
        return Err(DataSetError::NotEnoughRoots {
            needed: spec.projects,
            provided: roots.len(),
        });
    }
    if spec.instructions == 0 {
        return Err(DataSetError::InstructionsRequired);
    }
    if spec.variants_per_instruction == 0 {
        return Err(DataSetError::VariantsRequired { requested: 0 });
    }
    if spec.assignments_per_project > spec.instructions {
        return Err(DataSetError::AssignmentsPerProjectTooHigh {
            requested: spec.assignments_per_project,
            instructions: spec.instructions,
        });
    }
    let projects_present = row_count(conn, CountedTable::Projects)?;
    let items_present = row_count(conn, CountedTable::LibraryItems)?;
    if projects_present > 0 || items_present > 0 {
        return Err(DataSetError::NotEmpty {
            projects: projects_present,
            library_items: items_present,
        });
    }

    let tx = conn.transaction().map_err(DataSetError::Transaction)?;
    let generated = fill(&tx, spec, roots)?;
    tx.commit().map_err(DataSetError::Transaction)?;
    Ok(generated)
}

/// The fill proper, already inside [`generate`]'s transaction — every
/// statement is a repository create call.
fn fill(
    conn: &Connection,
    spec: &DataSetSpec,
    roots: &[ProjectRoot<'_>],
) -> Result<GeneratedDataSet, DataSetError> {
    // ---- Projects: the write-settings vocabulary cycles by ordinal.
    let mut project_ids = Vec::with_capacity(spec.projects);
    for (ordinal, root) in roots.iter().take(spec.projects).enumerate() {
        let name = format!("Benchmark Project {ordinal:04}");
        let label = format!("P{ordinal:04}");
        let description = format!("Seed project {ordinal:04} of the generated data set.");
        let project = projects::create(
            conn,
            &NewProject {
                path: root.path,
                path_key: root.path_key,
                name: &name,
                label: &label,
                color: PALETTE[ordinal % PALETTE.len()],
                description: &description,
                added_via: AddedVia::Scan,
                scheme: [Scheme::A, Scheme::B, Scheme::C][ordinal % 3],
                base_name: [BaseName::AgentsMd, BaseName::ClaudeMd][ordinal % 2],
                extra_local: ExtraFile {
                    enabled: ordinal % 2 == 0,
                    position: Position::Start,
                },
                extra_project: ExtraFile {
                    enabled: ordinal % 3 == 0,
                    position: Position::End,
                },
            },
        )
        .map_err(DataSetError::Projects)?;
        project_ids.push(project.id);
    }

    // ---- Library items: with headings, heading `h` precedes its
    // near-equal group of instructions (base per heading, the first
    // `remainder` headings one larger — the groups cover every
    // instruction exactly); without headings the flat loop below
    // creates them all.
    let mut instruction_ids = Vec::with_capacity(spec.instructions);
    let mut variant_ids: Vec<Vec<String>> = Vec::with_capacity(spec.instructions);
    let base_group = spec.instructions / spec.headings.max(1);
    let remainder = spec.instructions % spec.headings.max(1);
    let mut instruction = 0usize;
    for heading in 0..spec.headings {
        library::create_item(
            conn,
            &NewLibraryItem::Heading {
                title: &format!("Section {heading:04}"),
                level: u8::try_from(2 + heading % 5).expect("2 + heading % 5 stays in 2..=6"),
            },
        )
        .map_err(DataSetError::Library)?;
        let group = base_group + usize::from(heading < remainder);
        for _ in 0..group {
            create_instruction(
                conn,
                spec,
                instruction,
                &mut instruction_ids,
                &mut variant_ids,
            )?;
            instruction += 1;
        }
    }
    while instruction < spec.instructions {
        create_instruction(
            conn,
            spec,
            instruction,
            &mut instruction_ids,
            &mut variant_ids,
        )?;
        instruction += 1;
    }

    // ---- Assignments: the stride spread of the module docs.
    let mut assignment_count = 0usize;
    for (project, project_id) in project_ids.iter().enumerate() {
        for k in 0..spec.assignments_per_project {
            let instruction_ordinal = (ASSIGNMENT_STRIDE * project + k) % spec.instructions;
            let variant_ordinal = (project + k) % spec.variants_per_instruction;
            library::assign(
                conn,
                &NewAssignment {
                    project_id,
                    instruction_id: &instruction_ids[instruction_ordinal],
                    variant_id: &variant_ids[instruction_ordinal][variant_ordinal],
                },
            )
            .map_err(DataSetError::Library)?;
            assignment_count += 1;
        }
    }

    Ok(GeneratedDataSet {
        project_ids,
        instruction_ids,
        variant_ids,
        assignment_count,
    })
}

/// Creates instruction number `ordinal` with its variants (the first
/// one main — DB-010's constructive rule), appending the item id to
/// `instruction_ids` and the variant ids to `variant_ids`.
fn create_instruction(
    conn: &Connection,
    spec: &DataSetSpec,
    ordinal: usize,
    instruction_ids: &mut Vec<String>,
    variant_ids: &mut Vec<Vec<String>>,
) -> Result<(), DataSetError> {
    let title = format!("Instruction {ordinal:04}");
    let item = library::create_item(conn, &NewLibraryItem::Instruction { title: &title })
        .map_err(DataSetError::Library)?;
    let mut variants_of_instruction = Vec::with_capacity(spec.variants_per_instruction);
    for variant in 0..spec.variants_per_instruction {
        let name = variant_name(variant);
        let content = variant_content(ordinal, variant);
        let created = library::create_variant(
            conn,
            &NewVariant {
                instruction_id: &item.id,
                name: &name,
                content: &content,
                is_main: variant == 0,
            },
        )
        .map_err(DataSetError::Library)?;
        variants_of_instruction.push(created.id);
    }
    instruction_ids.push(item.id);
    variant_ids.push(variants_of_instruction);
    Ok(())
}

// ---- The stabilized view ---------------------------------------------------------

/// The stabilized view of one stored project (see the module docs):
/// every caller-meaningful column, no id, no timestamps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StabilizedProject {
    /// The canonical path, `base`-stripped when a base was given.
    pub path: String,
    /// The identity key, `base`-stripped the same way.
    pub path_key: String,
    /// The display name.
    pub name: String,
    /// The color-badge label.
    pub label: String,
    /// The palette color.
    pub color: String,
    /// The free-text description.
    pub description: String,
    /// Active or archived.
    pub state: ProjectState,
    /// Manual or scan.
    pub added_via: AddedVia,
    /// The write scheme.
    pub scheme: Scheme,
    /// The target file.
    pub base_name: BaseName,
    /// The `AGENTS.local.md` option.
    pub extra_local: ExtraFile,
    /// The `AGENTS.project.md` option.
    pub extra_project: ExtraFile,
}

/// The stabilized view of one stored library item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StabilizedItem {
    /// Instruction or heading.
    pub kind: ItemKind,
    /// The heading's level (`None` for an instruction).
    pub heading_level: Option<u8>,
    /// The display title.
    pub title: String,
    /// Active or hidden.
    pub state: ItemState,
}

/// The stabilized view of one stored active variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StabilizedVariant {
    /// Which instruction (by creation ordinal) this variant belongs
    /// to.
    pub instruction_ordinal: usize,
    /// The variant name.
    pub name: String,
    /// The variant text exactly as stored.
    pub content: String,
    /// The main-variant flag.
    pub is_main: bool,
}

/// The stabilized view of one stored assignment, as creation ordinals
/// of its project, instruction and variant (the variant ordinal is
/// within the instruction's active variants, creation order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StabilizedAssignment {
    /// The project's creation ordinal.
    pub project_ordinal: usize,
    /// The instruction's creation ordinal.
    pub instruction_ordinal: usize,
    /// The variant's ordinal within its instruction.
    pub variant_ordinal: usize,
}

/// The whole database under the determinism definition: the ALG-001
/// collections, ids as creation ordinals, no timestamps, assignments
/// as a sorted set. Equal values mean "the same data set" in exactly
/// the sense [`generate`] promises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StabilizedDataSet {
    /// Every project (both states) in `sort_order` order.
    pub projects: Vec<StabilizedProject>,
    /// Every library item in `position` order.
    pub items: Vec<StabilizedItem>,
    /// Every active variant, grouped per instruction ordinal, in
    /// creation order within the instruction.
    pub variants: Vec<StabilizedVariant>,
    /// The assignments of the active projects, sorted.
    pub assignments: Vec<StabilizedAssignment>,
}

/// Reads the stabilized view (the module docs' determinism
/// definition) of the database behind `conn`, through the
/// repositories' own reads.
///
/// `base` — the canonical base directory the roots live under, as
/// text — is stripped **by component count**: as many leading path
/// components as the base text holds are dropped from both `path`
/// and `path_key` (components split on `/` and `\`, empties ignored,
/// suffixes rejoined with `/`; a count exceeding a value's components
/// leaves the empty string). Counting components rather than matching
/// a text prefix keeps the definition platform-true: DB-030's keying
/// re-spells separators, NFC and case but never changes component
/// structure, so one count strips both columns on Windows and macOS
/// too. `None` keeps the stored texts verbatim.
///
/// The scope matches ALG-001's collections plus the archived
/// projects as project entries: all items (any state), the **active**
/// variants (soft-deleted ones never reach a build), and the
/// assignments of the **active** projects only.
///
/// Calling contract: the four reads run in whatever transaction the
/// caller holds — the [`crate::build_snapshot`] shape. A comparison
/// of two views should read each inside one transaction (or on a
/// quiescent single connection, the fixture case), or a concurrent
/// writer could fabricate a mismatch between one view's collections.
///
/// # Errors
///
/// Returns [`StabilizeError::Projects`] or
/// [`StabilizeError::Library`] when a repository read fails — a
/// corrupt vocabulary cell surfaces as its `Member` error (D-039).
pub fn stabilized(
    conn: &Connection,
    base: Option<&str>,
) -> Result<StabilizedDataSet, StabilizeError> {
    let drop_count = base.map_or(0, |base| base_components(base).len());
    // `None` keeps the stored texts verbatim — not even the empty
    // components of a leading `/` are touched (that is the point of
    // the negative half of the determinism test).
    let strip = |value: &str| match base {
        Some(_) => strip_components(value, drop_count),
        None => value.to_owned(),
    };
    let projects =
        projects::list(conn, projects::ListScope::All).map_err(StabilizeError::Projects)?;
    let items =
        library::list_items(conn, library::ItemScope::All).map_err(StabilizeError::Library)?;
    let variants = library::list_active_variants(conn).map_err(StabilizeError::Library)?;
    let assignments =
        library::list_assignments_for_active_projects(conn).map_err(StabilizeError::Library)?;

    // Ordinal maps: creation order fixed by the reads' orders —
    // projects by sort_order, items by position (instruction ordinals
    // counted on the fly), variants by (created_at, id) within one
    // instruction. The variants arrive grouped by instruction id
    // (list_active_variants' order); regrouping onto the position
    // walk below yields the definition's per-instruction-ordinal
    // order.
    let mut variants_by_instruction: HashMap<&str, Vec<&library::Variant>> =
        HashMap::with_capacity(items.len());
    for variant in &variants {
        variants_by_instruction
            .entry(variant.instruction_id.as_str())
            .or_default()
            .push(variant);
    }

    let mut project_ordinal: HashMap<&str, usize> = HashMap::with_capacity(projects.len());
    let mut stabilized_projects = Vec::with_capacity(projects.len());
    for (ordinal, project) in projects.iter().enumerate() {
        project_ordinal.insert(project.id.as_str(), ordinal);
        stabilized_projects.push(StabilizedProject {
            path: strip(&project.path),
            path_key: strip(&project.path_key),
            name: project.name.clone(),
            label: project.label.clone(),
            color: project.color.clone(),
            description: project.description.clone(),
            state: project.state,
            added_via: project.added_via,
            scheme: project.scheme,
            base_name: project.base_name,
            extra_local: project.extra_local,
            extra_project: project.extra_project,
        });
    }
    let mut instruction_ordinal: HashMap<&str, usize> = HashMap::with_capacity(items.len());
    let mut variant_ordinal: HashMap<&str, (usize, usize)> = HashMap::with_capacity(variants.len());
    let mut stabilized_items = Vec::with_capacity(items.len());
    let mut stabilized_variants = Vec::with_capacity(variants.len());
    let mut next_instruction = 0usize;
    for item in &items {
        stabilized_items.push(StabilizedItem {
            kind: item.kind,
            heading_level: item.heading_level,
            title: item.title.clone(),
            state: item.state,
        });
        if item.kind != ItemKind::Instruction {
            continue;
        }
        if let Some(group) = variants_by_instruction.get(item.id.as_str()) {
            for (within, variant) in group.iter().enumerate() {
                variant_ordinal.insert(variant.id.as_str(), (next_instruction, within));
                stabilized_variants.push(StabilizedVariant {
                    instruction_ordinal: next_instruction,
                    name: variant.name.clone(),
                    content: variant.content.clone(),
                    is_main: variant.is_main,
                });
            }
        }
        instruction_ordinal.insert(item.id.as_str(), next_instruction);
        next_instruction += 1;
    }
    let mut stabilized_assignments = Vec::with_capacity(assignments.len());
    for assignment in &assignments {
        // A reference into an assignment the maps cannot resolve means
        // the row points outside the read collections (an assignment
        // of a soft-deleted-only variant, or of an instruction that
        // no longer exists) — the snapshot validation's territory
        // (DB-012); the stabilized view skips what it cannot name
        // instead of guessing, and the generator's own data always
        // resolves.
        if let (Some(&project), Some(&instruction)) = (
            project_ordinal.get(assignment.project_id.as_str()),
            instruction_ordinal.get(assignment.instruction_id.as_str()),
        ) {
            let variant_id = match &assignment.target {
                crate::library::AssignmentTarget::Variant(variant_id) => variant_id,
                // The deterministic generator creates variant-backed
                // assignments only; a custom row — replace text or an
                // extend pair — is outside its shape.
                crate::library::AssignmentTarget::Custom(_)
                | crate::library::AssignmentTarget::Extend { .. } => continue,
            };
            if let Some(&(_, variant)) = variant_ordinal.get(variant_id.as_str()) {
                stabilized_assignments.push(StabilizedAssignment {
                    project_ordinal: project,
                    instruction_ordinal: instruction,
                    variant_ordinal: variant,
                });
            }
        }
    }
    stabilized_assignments.sort_unstable();

    Ok(StabilizedDataSet {
        projects: stabilized_projects,
        items: stabilized_items,
        variants: stabilized_variants,
        assignments: stabilized_assignments,
    })
}

/// The path components of a base text — split on both separators
/// (`/` and `\`, so a Windows-spelled base counts the same as its
/// stored form), empty components ignored (leading/trailing/doubled
/// separators carry no component).
fn base_components(base: &str) -> Vec<&str> {
    base.split(['/', '\\'])
        .filter(|component| !component.is_empty())
        .collect()
}

/// Drops the first `drop_count` components of a stored path or key
/// text and rejoins the rest with `/` — the component-stripping rule
/// of [`stabilized`] (see its docs for why counting, not matching).
fn strip_components(value: &str, drop_count: usize) -> String {
    base_components(value)
        .iter()
        .skip(drop_count)
        .copied()
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stride constant is what the module docs promise: the
    /// spread base of the assignment formula, a plain documented
    /// constant both profiles share.
    #[test]
    fn stride_is_the_documented_spread_constant() {
        assert_eq!(ASSIGNMENT_STRIDE, 37);
    }

    /// The named profiles carry §12.2's exact sizes — the constants
    /// are the oracle here, the integration suite pins them against
    /// the database.
    #[test]
    fn named_profiles_match_the_spec_sizes() {
        assert_eq!(M.projects, 50);
        assert_eq!(M.instructions, 200);
        assert_eq!(L.projects, 500);
        assert_eq!(L.instructions, 1000);
        assert_eq!(L.headings, 100);
        assert_eq!(L.variants_per_instruction, 3);
        assert_eq!(
            L.projects * L.assignments_per_project,
            100_000,
            "set L's assignment total is §12.2's 100,000"
        );
    }

    /// The prose templates stay LF-only and free of the DB-016
    /// reserved marker lines for every variant slot of both profiles.
    #[test]
    fn variant_content_is_lf_only_and_marker_free() {
        for instruction in [0usize, 1, 999, 1000] {
            for variant in 0..L.variants_per_instruction {
                let content = variant_content(instruction, variant);
                assert!(!content.contains('\r'), "CRLF found: {content:?}");
                assert!(!content.contains("<!--"), "a marker-like line: {content:?}");
            }
        }
    }

    /// Variant names stay NOCASE-distinct for any count a spec may
    /// ask for — the first three named, the rest numbered (§6.9's
    /// active-sibling uniqueness; a repeat would make `generate` fail
    /// mid-fill with `DuplicateVariantName`).
    #[test]
    fn variant_names_are_distinct_for_any_count() {
        let names: Vec<String> = (0..12).map(variant_name).collect();
        let mut seen = std::collections::HashSet::new();
        for name in &names {
            let lowered = name.to_lowercase();
            assert!(seen.insert(lowered), "the name {name:?} repeats");
        }
        assert_eq!(names[0], "Main");
        assert_eq!(names[2], "Detailed");
        assert_eq!(names[3], "Alternative 03");
        assert_eq!(names[11], "Alternative 11");
    }

    /// The component stripping of [`stabilized`]: one count strips a
    /// Windows-spelled path and its folded key alike, ignores empty
    /// components, and survives counts that overshoot — the property
    /// that makes the base-prefix rule platform-true (round-1
    /// review's P1: a text prefix cannot do this).
    #[test]
    fn component_stripping_is_spelling_agnostic() {
        // A base and two stored spellings of one root under it: the
        // path keeps case and backslashes, the key is folded and
        // forward-slashed — DB-030's Windows/macOS shapes. The drive
        // letter is a component like any other: five in total.
        let base = "C:\\Users\\Brooks\\Temp\\.tmpEunomia";
        assert_eq!(base_components(base).len(), 5);
        let path = "C:\\Users\\Brooks\\Temp\\.tmpEunomia\\repo-0007";
        let key = "c:/users/brooks/temp/.tmpeunomia/repo-0007";
        assert_eq!(strip_components(path, 5), "repo-0007");
        assert_eq!(strip_components(key, 5), "repo-0007");
        // Empty components (leading, trailing, doubled separators)
        // carry no component, so spellings with them count the same.
        assert_eq!(base_components("/tmp//x/"), ["tmp", "x"]);
        assert_eq!(strip_components("/tmp/x/repo-0001", 2), "repo-0001");
        // An overshooting count leaves the empty suffix, and a zero
        // count keeps everything.
        assert_eq!(strip_components("/a/b", 5), "");
        assert_eq!(strip_components("/a/b", 0), "a/b");
    }
}
