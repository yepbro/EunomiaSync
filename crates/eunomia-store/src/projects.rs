//! Projects repository (spec §6.2 `projects` and `project_exclusions`,
//! §6.4 archiving, DB-015 uniqueness; task T-071).
//!
//! One module serves both path-bearing tables because they share one
//! feature boundary — the project lifecycle composes exclusions (§6.4:
//! archiving a scan-found project adds its path to `project_exclusions`,
//! restoring removes it again, PRJ-013…018) — and one error shape: the
//! [`ProjectsError::DuplicatePathKey`] conflict is the *same* runtime
//! outcome on both tables (a caller-facing "this path is already
//! registered" situation, PRJ-001/PRJ-006). The tables stay separate
//! namespaces: DB-015's uniqueness runs inside `projects` only, so a
//! project row and an exclusion row may legally carry the same
//! `path_key` — that coexistence is what the archive flow produces
//! (PRJ-017) and it is pinned by a test.
//!
//! # The uniqueness scope is the schema's, not a parameter
//!
//! The migration (`migrations/0001_init.sql`) gives both tables a full
//! `path_key TEXT NOT NULL UNIQUE` — uniqueness over active **and**
//! archived rows, exactly DB-015's wording ("unique among active and
//! archived projects"). An archived project therefore keeps blocking
//! its path forever until it is restored; the DB-015 restore flow
//! ("the app offers to restore it") reads the archived row through
//! [`find_by_path_key`], which returns rows of either state, and the
//! caller branches on [`Project::state`]. There is deliberately no
//! active-only lookup: the schema has no active-only constraint, and
//! an API that implied one would promise what the schema does not
//! enforce.
//!
//! # What this module does not do
//!
//! * It never computes or canonicalizes a `path_key` (spec DB-030/DB-031,
//!   `eunomia_fs::paths`): both tables store the TEXT the caller hands
//!   over, verbatim.
//! * It does not journal. LOG-001…004 puts project add/change/archive/
//!   restore in the `action_log` vocabulary (`Action::ProjectAdded` and
//!   siblings exist since D-051), and D-051 assigns the entry to the
//!   *producer*: the command task calls [`crate::action_log::insert`]
//!   inside its own transaction at the `Db::run` boundary, exactly like
//!   [`crate::settings::set`] stays journal-free. Nothing here writes to
//!   `action_log`.
//! * It does not re-validate §6.9 field rules (name/label limits, the
//!   palette): that validation lives in `eunomia_core`
//!   (`project_validators`, D-075/D-079) and runs at the command
//!   boundary; the schema CHECKs are the floor here and surface as
//!   [`ProjectsError::Sql`] when a caller bypasses the validators. The
//!   one deliberate exception is the UNIQUE violation, which is a
//!   *runtime* outcome between two individually legal rows — not a
//!   caller-bug validation — and therefore typed
//!   ([`ProjectsError::DuplicatePathKey`], mapped from SQLite's
//!   extended code `SQLITE_CONSTRAINT_UNIQUE`, never by string
//!   matching).
//!
//! # Transactions and ordering
//!
//! Every function joins the caller's transaction (the
//! [`crate::settings`] shape): the app-layer executor (D-037) wraps one
//! operation in one `BEGIN DEFERRED`, and this module stays
//! executor-agnostic. [`create`] and [`create_exclusion`] are single
//! statements (the append's `sort_order` is computed by a scalar
//! subquery inside the `INSERT`, so even a bare call cannot interleave);
//! [`reorder`] is deliberately multi-statement and is only atomic
//! inside the caller's transaction — its doc says so, and the
//! integration suite pins the rollback behavior.
//!
//! `sort_order` follows DB-005's item-order rule — integer, starting
//! from 1, no gaps — over the **whole table**: creation appends
//! `MAX(sort_order) + 1`, archive/restore keep a row's position (§6.4
//! says nothing about `sort_order`, so keeping it is this repository's
//! least-destructive reading; v1 has no physical project delete, so
//! the table only grows), and [`reorder`] renumbers the complete id
//! set `1..=N` in one transaction. Reads
//! order by `sort_order` (ALG-001 reads "active projects in `sort_order`
//! order"; the active list is the `sort_order`-ordered subset).
//!
//! A stored enum cell outside its closed vocabulary (`state`,
//! `added_via`, `scheme`, `base_name`, the positions, `reason`) fails
//! the read with [`ProjectsError::Member`] instead of being guessed at
//! — every writer goes through this module, so such a cell means
//! corruption or a newer app version (the D-039 corrupt-cells policy).

use std::collections::HashSet;
use std::fmt;

use rusqlite::Connection;
use rusqlite::OptionalExtension;

use crate::settings::{BaseName, ExtraFile, Position, Scheme};
use crate::values::{new_id, now_utc};

// ---- Vocabularies -------------------------------------------------------------

/// The lifecycle state of a project (§6.2 `projects.state`).
///
/// v1 has no physical project delete (§6.4): a row leaves the active
/// set only by entering [`ProjectState::Archived`], and returns only by
/// restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectState {
    /// The project takes part in the matrix, syncs and the build
    /// snapshot (ALG-001).
    Active,
    /// The project is parked: assignments kept, no syncs, not counted
    /// as dirty (DB-014); it still holds its `path_key` (DB-015).
    Archived,
}

impl ProjectState {
    /// The stored member string (§6.2 CHECK vocabulary).
    pub const fn as_str(&self) -> &'static str {
        match self {
            ProjectState::Active => "active",
            ProjectState::Archived => "archived",
        }
    }

    /// Parses a stored member string, rejecting anything outside the
    /// CHECK vocabulary (case-sensitive, like the schema).
    fn from_member(member: &str) -> Result<Self, ProjectsError> {
        match member {
            "active" => Ok(ProjectState::Active),
            "archived" => Ok(ProjectState::Archived),
            _ => Err(ProjectsError::Member {
                column: "state",
                found: member.to_owned(),
            }),
        }
    }
}

/// How a project was registered (§6.2 `projects.added_via`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddedVia {
    /// Picked by hand through the folder picker (PRJ-001).
    Manual,
    /// Found by the repository scan (PRJ-002…007).
    Scan,
}

impl AddedVia {
    /// The stored member string (§6.2 CHECK vocabulary).
    pub const fn as_str(&self) -> &'static str {
        match self {
            AddedVia::Manual => "manual",
            AddedVia::Scan => "scan",
        }
    }

    /// Parses a stored member string (see [`ProjectState::from_member`]).
    fn from_member(member: &str) -> Result<Self, ProjectsError> {
        match member {
            "manual" => Ok(AddedVia::Manual),
            "scan" => Ok(AddedVia::Scan),
            _ => Err(ProjectsError::Member {
                column: "added_via",
                found: member.to_owned(),
            }),
        }
    }
}

/// Why a path is excluded from the scan (§6.2
/// `project_exclusions.reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExclusionReason {
    /// The user archived a project the scan had found (PRJ-017): its
    /// path must not resurface as a scan candidate.
    RemovedScanned,
    /// The user excluded the path by hand.
    Manual,
}

impl ExclusionReason {
    /// The stored member string (§6.2 CHECK vocabulary).
    pub const fn as_str(&self) -> &'static str {
        match self {
            ExclusionReason::RemovedScanned => "removed_scanned",
            ExclusionReason::Manual => "manual",
        }
    }

    /// Parses a stored member string (see [`ProjectState::from_member`]).
    fn from_member(member: &str) -> Result<Self, ProjectsError> {
        match member {
            "removed_scanned" => Ok(ExclusionReason::RemovedScanned),
            "manual" => Ok(ExclusionReason::Manual),
            _ => Err(ProjectsError::Member {
                column: "reason",
                found: member.to_owned(),
            }),
        }
    }
}

/// Which rows [`list`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListScope {
    /// Every row, active and archived (the table only grows, so this
    /// is the full set).
    All,
    /// Only `state = 'active'` rows. Their `sort_order` values are
    /// the active subset of a table-wide gapless sequence — **not**
    /// necessarily contiguous once a row is archived (archive keeps
    /// the position): read this as an ordered subset, never as
    /// `1..=K`.
    Active,
    /// Only `state = 'archived'` rows — the "Archived projects" screen
    /// (PRJ-013…018).
    Archived,
}

// ---- Row models ---------------------------------------------------------------

/// The columns of a `projects` row, in the fixed order every query of
/// this module selects them in — one list, so the read mapping and the
/// SQL cannot drift apart.
const PROJECT_COLUMNS: &str = "id, path, path_key, name, label, color, description, \
     state, added_via, sort_order, scheme, base_name, \
     extra_local_enabled, extra_local_position, \
     extra_project_enabled, extra_project_position, \
     last_synced_at, last_synced_hash, created_at, updated_at, archived_at";

/// A stored project row (§6.2 `projects`).
///
/// All TEXT columns are stored verbatim: `path` is the canonicalize
/// output the caller computed (DB-030), `path_key` is the caller's
/// identity key, `name`/`label`/`color`/`description` are whatever the
/// validators let through. The enum columns are decoded;
/// [`Project::extra_local`] and [`Project::extra_project`] bundle the
/// enabled flag with its position (the [`ExtraFile`] shape §6.5 uses
/// for the defaults).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Row id, UUIDv7 TEXT (DB-002), generated by [`create`].
    pub id: String,
    /// The canonical project path (DB-030), never changed after
    /// creation (PRJ-008: the path cannot be edited).
    pub path: String,
    /// The identity key (DB-015/DB-030), unique across the table.
    pub path_key: String,
    /// 1–100 characters after trimming (schema CHECK; §6.9).
    pub name: String,
    /// 1–6 visible characters (schema CHECK; DB-042).
    pub label: String,
    /// A palette member (DB-040); stored as TEXT, no CHECK.
    pub color: String,
    /// Free text, at most 1000 characters (schema CHECK).
    pub description: String,
    /// Active or archived (§6.4).
    pub state: ProjectState,
    /// Manual or scan (§6.2), fixed at creation.
    pub added_via: AddedVia,
    /// The DB-005 order position, gapless `1..=N` over the table;
    /// owned by [`create`] and [`reorder`], never by [`update`].
    pub sort_order: i64,
    /// The project's own scheme (PRJ-012), independent of the
    /// `default.*` settings.
    pub scheme: Scheme,
    /// The project's own target file (PRJ-012).
    pub base_name: BaseName,
    /// The `AGENTS.local.md` hint line: enabled + position (PRJ-012).
    pub extra_local: ExtraFile,
    /// The `AGENTS.project.md` hint line: enabled + position (PRJ-012).
    pub extra_project: ExtraFile,
    /// The last successful sync's time (OUT-020); `None` until the
    /// first sync writes it. Owned by the sync tasks, not by this
    /// module.
    pub last_synced_at: Option<String>,
    /// The last successful sync's build hash (DB-020/OUT-020).
    pub last_synced_hash: Option<String>,
    /// Creation time, DB-002 UTC ([`crate::values::now_utc`]).
    pub created_at: String,
    /// Last write of this row by this module, DB-002 UTC.
    pub updated_at: String,
    /// When the row was archived (§6.4); `None` while active. The
    /// invariant `archived_at IS NOT NULL ⟺ state = 'archived'` is
    /// maintained by [`archive`]/[`restore`].
    pub archived_at: Option<String>,
}

/// The raw cells of one `projects` row, as rusqlite read them — the
/// decode step is fallible on the vocabulary columns, and the
/// `query_map` closure must stay infallible (the [`crate::action_log`]
/// `LogRow` pattern).
struct ProjectRow {
    id: String,
    path: String,
    path_key: String,
    name: String,
    label: String,
    color: String,
    description: String,
    state: String,
    added_via: String,
    sort_order: i64,
    scheme: String,
    base_name: String,
    extra_local_enabled: bool,
    extra_local_position: String,
    extra_project_enabled: bool,
    extra_project_position: String,
    last_synced_at: Option<String>,
    last_synced_hash: Option<String>,
    created_at: String,
    updated_at: String,
    archived_at: Option<String>,
}

impl ProjectRow {
    /// Reads one row in the fixed [`PROJECT_COLUMNS`] order.
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(ProjectRow {
            id: row.get(0)?,
            path: row.get(1)?,
            path_key: row.get(2)?,
            name: row.get(3)?,
            label: row.get(4)?,
            color: row.get(5)?,
            description: row.get(6)?,
            state: row.get(7)?,
            added_via: row.get(8)?,
            sort_order: row.get(9)?,
            scheme: row.get(10)?,
            base_name: row.get(11)?,
            extra_local_enabled: row.get(12)?,
            extra_local_position: row.get(13)?,
            extra_project_enabled: row.get(14)?,
            extra_project_position: row.get(15)?,
            last_synced_at: row.get(16)?,
            last_synced_hash: row.get(17)?,
            created_at: row.get(18)?,
            updated_at: row.get(19)?,
            archived_at: row.get(20)?,
        })
    }

    /// Decodes the vocabulary columns into the typed model.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectsError::Member`] for a cell outside its closed
    /// vocabulary — corruption or a newer app version (D-039), never a
    /// value to guess at.
    fn into_project(self) -> Result<Project, ProjectsError> {
        let state = ProjectState::from_member(&self.state)?;
        let added_via = AddedVia::from_member(&self.added_via)?;
        let scheme = parse_scheme(&self.scheme)?;
        let base_name = parse_base_name(&self.base_name)?;
        let extra_local = ExtraFile {
            enabled: self.extra_local_enabled,
            position: parse_position("extra_local_position", &self.extra_local_position)?,
        };
        let extra_project = ExtraFile {
            enabled: self.extra_project_enabled,
            position: parse_position("extra_project_position", &self.extra_project_position)?,
        };
        Ok(Project {
            id: self.id,
            path: self.path,
            path_key: self.path_key,
            name: self.name,
            label: self.label,
            color: self.color,
            description: self.description,
            state,
            added_via,
            sort_order: self.sort_order,
            scheme,
            base_name,
            extra_local,
            extra_project,
            last_synced_at: self.last_synced_at,
            last_synced_hash: self.last_synced_hash,
            created_at: self.created_at,
            updated_at: self.updated_at,
            archived_at: self.archived_at,
        })
    }
}

/// The `settings` module keeps its member parsers private, so this
/// module carries three thin adapters over the same closed
/// vocabularies; every writer feeds them through the same `as_str`
/// the `settings` enums serialize with, and a drifted member fails
/// the read path loudly (pinned by a unit test below).
fn parse_scheme(member: &str) -> Result<Scheme, ProjectsError> {
    match member {
        "A" => Ok(Scheme::A),
        "B" => Ok(Scheme::B),
        "C" => Ok(Scheme::C),
        _ => Err(ProjectsError::Member {
            column: "scheme",
            found: member.to_owned(),
        }),
    }
}

fn parse_base_name(member: &str) -> Result<BaseName, ProjectsError> {
    match member {
        "AGENTS.md" => Ok(BaseName::AgentsMd),
        "CLAUDE.md" => Ok(BaseName::ClaudeMd),
        _ => Err(ProjectsError::Member {
            column: "base_name",
            found: member.to_owned(),
        }),
    }
}

fn parse_position(column: &'static str, member: &str) -> Result<Position, ProjectsError> {
    match member {
        "start" => Ok(Position::Start),
        "end" => Ok(Position::End),
        _ => Err(ProjectsError::Member {
            column,
            found: member.to_owned(),
        }),
    }
}

/// Why a projects/exclusions operation failed.
///
/// Returned **directly** by every function of this module — not nested
/// in [`crate::storage::StorageError`] like the `Settings`/`ActionLog`
/// pairs. `StorageError` is matched exhaustively by the app layer
/// (`src-tauri`'s error classifier), so adding a variant there is a
/// cross-crate change this repository deliberately avoided; the
/// future command task maps `ProjectsError` beside the
/// `StorageError` it already unwraps (the boundary is recorded in
/// DECISIONS.md).
#[derive(Debug)]
pub enum ProjectsError {
    /// A `path_key` UNIQUE constraint fired — the key already exists in
    /// the same table (projects: DB-015, active **and** archived rows;
    /// exclusions: its own table-wide uniqueness). Mapped from SQLite's
    /// extended code `SQLITE_CONSTRAINT_UNIQUE`, deliberately not by
    /// string matching. The statement that failed was rolled back by
    /// SQLite's ABORT policy; the surrounding transaction stays usable.
    DuplicatePathKey {
        /// The key that already exists.
        path_key: String,
    },
    /// No row with this id. Produced by the by-id writers
    /// ([`update`], [`archive`], [`restore`], [`delete_exclusion`])
    /// whose successful outcome is "the row now has this shape" — an
    /// absent row cannot have it.
    NotFound {
        /// The id that matched nothing.
        id: String,
    },
    /// [`reorder`] received an id set that does not describe the whole
    /// table: unknown ids, omitted projects, or duplicates. The
    /// table's positions are untouched.
    ReorderMismatch {
        /// Ids from the input that match no project row.
        unknown: Vec<String>,
        /// How many project rows the input omitted.
        missing: usize,
        /// How many input entries duplicate an earlier one.
        duplicates: usize,
    },
    /// A stored cell holds a value outside its closed vocabulary
    /// (`state`, `added_via`, `scheme`, `base_name`, the extra-file
    /// positions, `reason`). Every writer goes through this module, so
    /// this means corruption or a row written by a newer app version;
    /// the read surfaces it instead of guessing (the D-039
    /// corrupt-cells policy).
    Member {
        /// The column the value came from.
        column: &'static str,
        /// The offending value.
        found: String,
    },
    /// The statement or query itself failed — a schema CHECK rejection
    /// (a §6.9 value that bypassed the `eunomia_core` validators; see
    /// the module docs for the validation split), a closed connection,
    /// a tampered schema. Everything SQLite reports that is not one of
    /// the typed outcomes above.
    Sql(rusqlite::Error),
}

impl fmt::Display for ProjectsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectsError::DuplicatePathKey { path_key } => {
                write!(f, "a row with path_key {path_key:?} already exists")
            }
            ProjectsError::NotFound { id } => write!(f, "no row with id {id:?}"),
            ProjectsError::ReorderMismatch {
                unknown,
                missing,
                duplicates,
            } => write!(
                f,
                "the id set does not match the table: {unknown:?} unknown, \
                 {missing} missing, {duplicates} duplicated"
            ),
            ProjectsError::Member { column, found } => {
                write!(f, "{found:?} is not a valid {column} value")
            }
            ProjectsError::Sql(err) => write!(f, "database error: {err}"),
        }
    }
}

impl std::error::Error for ProjectsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ProjectsError::Sql(err) => Some(err),
            _ => None,
        }
    }
}

/// Lets `?` lift a rusqlite failure into the typed error.
impl From<rusqlite::Error> for ProjectsError {
    fn from(err: rusqlite::Error) -> Self {
        ProjectsError::Sql(err)
    }
}

/// Maps the UNIQUE-constraint outcome of an INSERT to the typed
/// duplicate error; everything else stays a plain SQL error.
///
/// The check is the extended result code
/// (`rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE`), not the message text:
/// SQLite guarantees the code, and a message may be reworded between
/// builds. The mapping relies on each table carrying **exactly one**
/// UNIQUE index — `path_key` (the id's PRIMARY KEY reports
/// `SQLITE_CONSTRAINT_PRIMARYKEY`, a different code): a future
/// migration that adds a UNIQUE index to either table must extend
/// this mapping, or its violation would be misreported as a duplicate
/// path.
fn duplicate_or_sql(err: rusqlite::Error, path_key: &str) -> ProjectsError {
    if let rusqlite::Error::SqliteFailure(ffi_error, _) = &err
        && ffi_error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    {
        return ProjectsError::DuplicatePathKey {
            path_key: path_key.to_owned(),
        };
    }
    ProjectsError::Sql(err)
}

/// Runs a single-row project query and decodes the row, mapping a
/// corrupt vocabulary cell to the typed error (D-039).
fn query_optional_project(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Option<Project>, ProjectsError> {
    let mut statement = conn.prepare(sql)?;
    statement
        .query_row(params, ProjectRow::from_row)
        .optional()?
        .map(ProjectRow::into_project)
        .transpose()
}

// ---- Projects: create and read ------------------------------------------------

/// The input of [`create`] — the caller-computed TEXT columns plus the
/// caller-chosen enum members. Ids and timestamps are generated inside
/// [`create`]; `sort_order` is appended by the repository; `state`
/// starts active (§6.2 default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProject<'a> {
    /// The canonical path (DB-030), stored verbatim.
    pub path: &'a str,
    /// The identity key (DB-030/DB-031), stored verbatim.
    pub path_key: &'a str,
    /// The display name (§6.9: 1–100 characters after trimming).
    pub name: &'a str,
    /// The color-badge label (DB-042: 1–6 visible characters).
    pub label: &'a str,
    /// The palette color (DB-040).
    pub color: &'a str,
    /// The free-text description (up to 1000 characters).
    pub description: &'a str,
    /// How the project was registered.
    pub added_via: AddedVia,
    /// The project's scheme (PRJ-012).
    pub scheme: Scheme,
    /// The project's target file (PRJ-012).
    pub base_name: BaseName,
    /// The `AGENTS.local.md` hint line: enabled + position (PRJ-012).
    pub extra_local: ExtraFile,
    /// The `AGENTS.project.md` hint line: enabled + position (PRJ-012).
    pub extra_project: ExtraFile,
}

/// Inserts a project and returns the stored row.
///
/// The row id is a fresh UUIDv7 TEXT and the timestamps are stamped
/// with the DB-002 UTC format ([`crate::values`]); `sort_order` is
/// `MAX(sort_order) + 1` over the whole table (1 for the first row) —
/// computed by a scalar subquery *inside* the `INSERT`, so the append
/// is one statement and cannot interleave even outside a transaction.
/// The row is then read back through the same mapping every read uses,
/// so the returned [`Project`] is exactly what a subsequent [`get`]
/// returns. `last_synced_at`/`last_synced_hash` start `NULL` (OUT-020:
/// only a successful sync writes them); `archived_at` starts `NULL`.
///
/// The exclusions table is a separate namespace, so creating a project
/// whose path is currently in `project_exclusions` succeeds and leaves
/// the exclusion in place. Removing it — the manual re-add of a
/// previously excluded path must not keep the scan skipping the now
/// registered project — is the *caller's* composition, symmetric with
/// [`archive`]/[`restore`]: the command task calls
/// [`find_exclusion_by_path_key`] and [`delete_exclusion`] in the same
/// transaction when the product flow calls for it.
///
/// # Errors
///
/// Returns [`ProjectsError::DuplicatePathKey`] when a row with the
/// same `path_key` already exists — an active or an archived one
/// alike (DB-015). The failed statement is rolled back by SQLite's
/// ABORT policy and the surrounding transaction stays usable. Other
/// failures surface as [`ProjectsError::Sql`], including the schema
/// CHECK rejections of §6.9 values a caller passed without going
/// through the validators.
pub fn create(conn: &Connection, new: &NewProject<'_>) -> Result<Project, ProjectsError> {
    let id = new_id();
    conn.execute(
        "INSERT INTO projects (
             id, path, path_key, name, label, color, description,
             state, added_via, sort_order, scheme, base_name,
             extra_local_enabled, extra_local_position,
             extra_project_enabled, extra_project_position,
             last_synced_at, last_synced_hash, created_at, updated_at, archived_at
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7,
             'active', ?8,
             (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM projects),
             ?9, ?10,
             ?11, ?12,
             ?13, ?14,
             NULL, NULL, ?15, ?15, NULL
         )",
        rusqlite::params![
            id,
            new.path,
            new.path_key,
            new.name,
            new.label,
            new.color,
            new.description,
            new.added_via.as_str(),
            new.scheme.as_str(),
            new.base_name.as_str(),
            i64::from(new.extra_local.enabled),
            new.extra_local.position.as_str(),
            i64::from(new.extra_project.enabled),
            new.extra_project.position.as_str(),
            now_utc(),
        ],
    )
    .map_err(|err| duplicate_or_sql(err, new.path_key))?;
    // Unreachable while the INSERT above succeeded and no concurrent
    // writer deletes rows (one connection, D-037); a reasoned panic
    // (the house precedent for broken invariants) instead of a
    // misleading NotFound.
    Ok(get(conn, &id)?.expect("the row just inserted must be readable"))
}

/// Reads one project by id; `None` when no row matches.
///
/// # Errors
///
/// Returns [`ProjectsError::Member`] when a stored vocabulary cell is
/// corrupt (D-039), and [`ProjectsError::Sql`] when the query fails.
pub fn get(conn: &Connection, id: &str) -> Result<Option<Project>, ProjectsError> {
    query_optional_project(
        conn,
        &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1"),
        [id],
    )
}

/// Finds a project by its identity key, in **either** state.
///
/// DB-015 makes `path_key` unique among active and archived projects,
/// so a found row is the only row with that key; its
/// [`Project::state`] tells the caller which situation it is in: an
/// active row is the "already added" case of PRJ-001, an archived row
/// is the "offer to restore" case (PRJ-001/PRJ-013…018). `None` means
/// the path is free.
///
/// # Errors
///
/// Same contract as [`get`].
pub fn find_by_path_key(
    conn: &Connection,
    path_key: &str,
) -> Result<Option<Project>, ProjectsError> {
    query_optional_project(
        conn,
        &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE path_key = ?1"),
        [path_key],
    )
}

/// Lists projects in `sort_order` order (ALG-001's read order),
/// filtered by [`ListScope`].
///
/// # Errors
///
/// Returns [`ProjectsError::Member`] when any returned row has a
/// corrupt vocabulary cell (D-039), and [`ProjectsError::Sql`] when
/// the query fails.
pub fn list(conn: &Connection, scope: ListScope) -> Result<Vec<Project>, ProjectsError> {
    let sql = match scope {
        ListScope::All => {
            format!("SELECT {PROJECT_COLUMNS} FROM projects ORDER BY sort_order")
        }
        ListScope::Active => format!(
            "SELECT {PROJECT_COLUMNS} FROM projects \
             WHERE state = 'active' ORDER BY sort_order"
        ),
        ListScope::Archived => format!(
            "SELECT {PROJECT_COLUMNS} FROM projects \
             WHERE state = 'archived' ORDER BY sort_order"
        ),
    };
    let mut statement = conn.prepare(&sql)?;
    let rows = statement
        .query_map([], ProjectRow::from_row)?
        .map(|raw| {
            raw.map_err(ProjectsError::Sql)
                .and_then(|row| row.into_project())
        })
        .collect::<Result<Vec<_>, ProjectsError>>()?;
    Ok(rows)
}

// ---- Projects: update, archive, restore, reorder ------------------------------

/// The mutable fields of a project (PRJ-008/PRJ-012), as an update
/// instruction: `None` leaves the field unchanged.
///
/// Deliberately absent: `path` and `path_key` (PRJ-008: the path
/// cannot be changed — the identity is fixed at creation),
/// `added_via` (fixed at creation), `state` ([`archive`]/[`restore`]
/// own it), `sort_order` ([`reorder`] owns it), `last_synced_at`/
/// `last_synced_hash` (OUT-020: the sync tasks own them) and
/// `created_at`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectUpdate<'a> {
    /// The new display name, if it changes.
    pub name: Option<&'a str>,
    /// The new label, if it changes.
    pub label: Option<&'a str>,
    /// The new color, if it changes.
    pub color: Option<&'a str>,
    /// The new description, if it changes.
    pub description: Option<&'a str>,
    /// The new scheme, if it changes (DB-023: this makes the project
    /// dirty — the flag's recomputation is the app layer's).
    pub scheme: Option<Scheme>,
    /// The new target file, if it changes (DB-023 applies alike).
    pub base_name: Option<BaseName>,
    /// The new `AGENTS.local.md` hint line, if it changes.
    pub extra_local: Option<ExtraFile>,
    /// The new `AGENTS.project.md` hint line, if it changes.
    pub extra_project: Option<ExtraFile>,
}

/// Adds one SET assignment and its bound value to the dynamic UPDATE
/// of [`update`].
fn push_field(
    assignments: &mut Vec<&'static str>,
    values: &mut Vec<Box<dyn rusqlite::ToSql>>,
    assignment: &'static str,
    value: Box<dyn rusqlite::ToSql>,
) {
    assignments.push(assignment);
    values.push(value);
}

/// Applies an update to the project with this id and returns the
/// stored row.
///
/// Exactly the fields present in `update` are written; absent fields
/// keep their values (PRJ-008 saves the whole modal at once, but the
/// repository does not force a full overwrite). `updated_at` is
/// restamped with the DB-002 UTC format whenever at least one field is
/// written — the [`crate::settings::set`] convention; an update with
/// no field present writes nothing and returns the row unchanged.
/// The update is one statement inside the caller's transaction.
///
/// # Errors
///
/// Returns [`ProjectsError::NotFound`] when no row has this id, and
/// [`ProjectsError::Sql`] when the statement fails (schema CHECK
/// rejections included — see the module docs for the validation
/// split).
pub fn update(
    conn: &Connection,
    id: &str,
    update: &ProjectUpdate<'_>,
) -> Result<Project, ProjectsError> {
    // Each present field contributes one SET assignment and one bound
    // parameter; None fields contribute nothing, so the SQL never
    // mentions them. A free helper (not a closure) keeps the two
    // vectors readable between pushes.
    let mut assignments: Vec<&'static str> = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(name) = update.name {
        push_field(
            &mut assignments,
            &mut values,
            "name",
            Box::new(name.to_owned()),
        );
    }
    if let Some(label) = update.label {
        push_field(
            &mut assignments,
            &mut values,
            "label",
            Box::new(label.to_owned()),
        );
    }
    if let Some(color) = update.color {
        push_field(
            &mut assignments,
            &mut values,
            "color",
            Box::new(color.to_owned()),
        );
    }
    if let Some(description) = update.description {
        push_field(
            &mut assignments,
            &mut values,
            "description",
            Box::new(description.to_owned()),
        );
    }
    if let Some(scheme) = update.scheme {
        push_field(
            &mut assignments,
            &mut values,
            "scheme",
            Box::new(scheme.as_str().to_owned()),
        );
    }
    if let Some(base_name) = update.base_name {
        push_field(
            &mut assignments,
            &mut values,
            "base_name",
            Box::new(base_name.as_str().to_owned()),
        );
    }
    if let Some(extra) = update.extra_local {
        push_field(
            &mut assignments,
            &mut values,
            "extra_local_enabled",
            Box::new(i64::from(extra.enabled)),
        );
        push_field(
            &mut assignments,
            &mut values,
            "extra_local_position",
            Box::new(extra.position.as_str().to_owned()),
        );
    }
    if let Some(extra) = update.extra_project {
        push_field(
            &mut assignments,
            &mut values,
            "extra_project_enabled",
            Box::new(i64::from(extra.enabled)),
        );
        push_field(
            &mut assignments,
            &mut values,
            "extra_project_position",
            Box::new(extra.position.as_str().to_owned()),
        );
    }

    if assignments.is_empty() {
        // Nothing to write: a no-op must not churn `updated_at`.
        return get(conn, id)?.ok_or_else(|| ProjectsError::NotFound { id: id.to_owned() });
    }
    push_field(
        &mut assignments,
        &mut values,
        "updated_at",
        Box::new(now_utc()),
    );

    let sql = format!(
        "UPDATE projects SET {} WHERE id = ?",
        assignments
            .iter()
            .map(|assignment| format!("{assignment} = ?"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    // The last placeholder is the WHERE clause's id.
    values.push(Box::new(id.to_owned()));
    let param_refs: Vec<&dyn rusqlite::ToSql> = values.iter().map(|value| value.as_ref()).collect();
    let changed = conn.execute(sql.as_str(), param_refs.as_slice())?;
    if changed == 0 {
        return Err(ProjectsError::NotFound { id: id.to_owned() });
    }
    // Unreachable: the UPDATE above matched the row (one connection,
    // D-037 — no concurrent delete exists).
    Ok(get(conn, id)?.expect("the row just updated must be readable"))
}

/// Archives the project with this id (§6.4): `state` becomes
/// `archived`, `archived_at` is stamped with the DB-002 UTC time, and
/// `updated_at` follows.
///
/// The archived project keeps its assignments, its `sort_order`
/// position and — DB-015 — its `path_key`, which keeps blocking the
/// path until a restore. The exclusions entry the archive flow adds
/// for a scan-found project (PRJ-017) is the *caller's* composition:
/// this function writes the projects row only, so one archive can be
/// one transaction at the command boundary.
///
/// Idempotent: an already-archived project is returned unchanged —
/// the row is not restamped, so a repeated call converges instead of
/// drifting `archived_at` (the make-operations-idempotent rule).
///
/// # Errors
///
/// Returns [`ProjectsError::NotFound`] when no row has this id, and
/// [`ProjectsError::Sql`] when the statement fails.
pub fn archive(conn: &Connection, id: &str) -> Result<Project, ProjectsError> {
    let changed = conn.execute(
        "UPDATE projects
         SET state = 'archived', archived_at = ?1, updated_at = ?1
         WHERE id = ?2 AND state = 'active'",
        rusqlite::params![now_utc(), id],
    )?;
    if changed == 0 {
        // Either the id matches nothing, or the row is already
        // archived; the read tells the two apart.
        return existing_row(conn, id);
    }
    // Unreachable: the UPDATE above matched an active row (one
    // connection, D-037 — no concurrent delete exists).
    Ok(get(conn, id)?.expect("the row just archived must be readable"))
}

/// Restores the archived project with this id (PRJ-013…018): `state`
/// returns to `active`, `archived_at` is cleared (the invariant
/// `archived_at IS NOT NULL ⟺ state = 'archived'`), `updated_at` is
/// restamped.
///
/// The project returns with its assignments and its old `sort_order`
/// position (that position-keeping is this repository's reading: §6.4
/// does not mention `sort_order`, and the visible order is the
/// `sort_order`-ordered active subset). Removing the path's
/// exclusions entry (PRJ-013…018: "Restoring … removes its path from
/// the exclusions") is the *caller's* composition — see [`archive`].
///
/// Idempotent: an already-active project is returned unchanged.
///
/// # Errors
///
/// Returns [`ProjectsError::NotFound`] when no row has this id, and
/// [`ProjectsError::Sql`] when the statement fails.
pub fn restore(conn: &Connection, id: &str) -> Result<Project, ProjectsError> {
    let changed = conn.execute(
        "UPDATE projects
         SET state = 'active', archived_at = NULL, updated_at = ?1
         WHERE id = ?2 AND state = 'archived'",
        rusqlite::params![now_utc(), id],
    )?;
    if changed == 0 {
        return existing_row(conn, id);
    }
    // Unreachable: the UPDATE above matched an archived row (one
    // connection, D-037 — no concurrent delete exists).
    Ok(get(conn, id)?.expect("the row just restored must be readable"))
}

/// Records one project's last-sync facts (DB-024/OUT-020, §9.4 phase
/// 3 step 2): `last_synced_at` and `last_synced_hash` are set to the
/// caller's values, and NOTHING else moves — `updated_at` is
/// deliberately not restamped (the column names the project row's
/// last EDIT; a sync stamping a file is not an edit of the project,
/// and §9.4 phase 3 names exactly these two columns), `state`,
/// `sort_order` and every content column stay as they were.
///
/// `last_synced_at` is the **caller's** run-instant timestamp in the
/// DB-002 UTC shape (the [`create_running`]-family contract: the app
/// layer captures the moment — §9.4 phase 3's "right away" — the
/// column names it), and `last_synced_hash` is the run's DB-020 build
/// hash of the markerless managed text (the very fingerprint phase 0
/// froze into `sync_run_projects.build_hash`), NOT the digest of the
/// bytes on disk: the written file carries the run's creation marker,
/// and D-24/ALG-033 compare the markerless form, so the stored hash
/// must be the value the next comparison recomputes against.
///
/// Idempotent: re-recording overwrites both cells and returns the
/// fresh row (SQLite's `changes()` counts a matched row even when the
/// cells already held these values).
///
/// Joins the caller's transaction (D-037/D-051); the phase-3
/// production shape applies this in the SAME short transaction as the
/// row's `written` transition — one commit per project.
///
/// # Errors
///
/// Returns [`ProjectsError::NotFound`] when no row has this id, and
/// [`ProjectsError::Sql`] when the statement fails.
///
/// [`create_running`]: crate::sync_runs::create_running
pub fn record_last_synced(
    conn: &Connection,
    id: &str,
    last_synced_at: &str,
    last_synced_hash: &str,
) -> Result<Project, ProjectsError> {
    let changed = conn.execute(
        "UPDATE projects
         SET last_synced_at = ?2, last_synced_hash = ?3
         WHERE id = ?1",
        rusqlite::params![id, last_synced_at, last_synced_hash],
    )?;
    if changed == 0 {
        return get(conn, id)?.ok_or_else(|| ProjectsError::NotFound { id: id.to_owned() });
    }
    // Unreachable while the UPDATE above matched (one connection,
    // D-037 — no concurrent delete exists).
    Ok(get(conn, id)?.expect("the row just recorded must be readable"))
}

/// Reads the row for a conditional writer whose `UPDATE` matched
/// nothing: the existing row (idempotent re-run) or
/// [`ProjectsError::NotFound`] (no such id).
fn existing_row(conn: &Connection, id: &str) -> Result<Project, ProjectsError> {
    get(conn, id)?.ok_or_else(|| ProjectsError::NotFound { id: id.to_owned() })
}

/// Renumbers the whole table's `sort_order` to `1..=N` following
/// `ordered_ids` (DB-005: reordering renumbers in one transaction).
///
/// `ordered_ids` must be a permutation of **every** id in the table —
/// active and archived alike: positions are a table-wide gapless
/// invariant, and archived rows carry theirs through archive/restore.
/// A set that omits a project, names an unknown id, or repeats one is
/// rejected with [`ProjectsError::ReorderMismatch`] and the table is
/// left untouched (the validation runs before the first write).
///
/// Atomicity: this is a multi-statement operation and joins the
/// caller's transaction like every function here — but unlike the
/// single-statement writers it is **only** atomic inside one. The
/// app-layer executor (D-037) supplies the transaction; a bare call on
/// a connection without an open transaction would renumber
/// statement-by-statement. The integration suite pins the rollback
/// behavior through an explicit transaction.
///
/// Composing the order (for the PRJ-019/020 surfaces): positions are
/// table-wide, so after any archive the *active* rows' positions are
/// not contiguous — `list(ListScope::Active)` alone is therefore never
/// a valid permutation. The command task builds the new order from
/// `list(ListScope::All)` (or `list(Active)` plus the archived ids in
/// their relative order), splices the move in, and passes the full
/// permutation here.
///
/// # Errors
///
/// Returns [`ProjectsError::ReorderMismatch`] when the id set does not
/// describe the whole table, and [`ProjectsError::Sql`] when a
/// statement fails.
pub fn reorder(conn: &Connection, ordered_ids: &[&str]) -> Result<(), ProjectsError> {
    let stored: Vec<String> = {
        let mut statement = conn.prepare("SELECT id FROM projects")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let stored_set: HashSet<&str> = stored.iter().map(String::as_str).collect();
    let mut input_set: HashSet<&str> = HashSet::with_capacity(ordered_ids.len());
    let mut duplicates = 0usize;
    for id in ordered_ids {
        if !input_set.insert(id) {
            duplicates += 1;
        }
    }
    let unknown: Vec<String> = ordered_ids
        .iter()
        .filter(|id| !stored_set.contains(**id))
        .map(|id| (*id).to_owned())
        .collect();
    let missing = stored_set.difference(&input_set).count();
    if !unknown.is_empty() || missing > 0 || duplicates > 0 {
        return Err(ProjectsError::ReorderMismatch {
            unknown,
            missing,
            duplicates,
        });
    }

    for (index, id) in ordered_ids.iter().enumerate() {
        // SQLite's own row-count bound keeps the index inside i64.
        let position = i64::try_from(index + 1).expect("row count fits i64");
        conn.execute(
            "UPDATE projects SET sort_order = ?1 WHERE id = ?2",
            rusqlite::params![position, id],
        )?;
    }
    Ok(())
}

// ---- Exclusions ----------------------------------------------------------------

/// The columns of a `project_exclusions` row, in the fixed read order.
const EXCLUSION_COLUMNS: &str = "id, path, path_key, reason, created_at";

/// A stored path exclusion (§6.2 `project_exclusions`): a path the
/// scan must skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exclusion {
    /// Row id, UUIDv7 TEXT (DB-002), generated by [`create_exclusion`].
    pub id: String,
    /// The canonical path (DB-030), stored verbatim.
    pub path: String,
    /// The identity key, unique within this table.
    pub path_key: String,
    /// Why the path was excluded (§6.2).
    pub reason: ExclusionReason,
    /// Creation time, DB-002 UTC.
    pub created_at: String,
}

/// The raw cells of one `project_exclusions` row (see [`ProjectRow`]).
struct ExclusionRow {
    id: String,
    path: String,
    path_key: String,
    reason: String,
    created_at: String,
}

impl ExclusionRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(ExclusionRow {
            id: row.get(0)?,
            path: row.get(1)?,
            path_key: row.get(2)?,
            reason: row.get(3)?,
            created_at: row.get(4)?,
        })
    }

    fn into_exclusion(self) -> Result<Exclusion, ProjectsError> {
        Ok(Exclusion {
            id: self.id,
            path: self.path,
            path_key: self.path_key,
            reason: ExclusionReason::from_member(&self.reason)?,
            created_at: self.created_at,
        })
    }
}

/// The input of [`create_exclusion`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewExclusion<'a> {
    /// The canonical path (DB-030), stored verbatim.
    pub path: &'a str,
    /// The identity key (DB-030/DB-031), stored verbatim.
    pub path_key: &'a str,
    /// Why the path is excluded.
    pub reason: ExclusionReason,
}

/// Inserts a path exclusion and returns the stored row.
///
/// The row id and `created_at` are generated here (DB-002). The table
/// is a namespace of its own: a `path_key` may coexist with a
/// project's — that coexistence is what archiving a scan-found project
/// produces (PRJ-017).
///
/// # Errors
///
/// Returns [`ProjectsError::DuplicatePathKey`] when an exclusion with
/// the same `path_key` already exists (the same typed outcome as the
/// projects side), and [`ProjectsError::Sql`] otherwise.
pub fn create_exclusion(
    conn: &Connection,
    new: &NewExclusion<'_>,
) -> Result<Exclusion, ProjectsError> {
    let id = new_id();
    conn.execute(
        "INSERT INTO project_exclusions (id, path, path_key, reason, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, new.path, new.path_key, new.reason.as_str(), now_utc()],
    )
    .map_err(|err| duplicate_or_sql(err, new.path_key))?;
    get_exclusion(
        conn,
        &format!("SELECT {EXCLUSION_COLUMNS} FROM project_exclusions WHERE id = ?1"),
        [id.as_str()],
    )?
    .map(Ok)
    .expect("the exclusion just inserted must be readable")
}

/// Runs a single-row exclusions query and decodes the row (see
/// [`query_optional_project`]).
fn get_exclusion(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Option<Exclusion>, ProjectsError> {
    let mut statement = conn.prepare(sql)?;
    statement
        .query_row(params, ExclusionRow::from_row)
        .optional()?
        .map(ExclusionRow::into_exclusion)
        .transpose()
}

/// Lists all exclusions, oldest first (`created_at` is the DB-002
/// fixed-width UTC shape, so the text order is the chronological one;
/// `id` breaks same-millisecond ties in insertion order — UUIDv7 TEXT
/// sorts by creation time, D-031).
///
/// # Errors
///
/// Returns [`ProjectsError::Member`] for a corrupt `reason` cell
/// (D-039), and [`ProjectsError::Sql`] when the query fails.
pub fn list_exclusions(conn: &Connection) -> Result<Vec<Exclusion>, ProjectsError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {EXCLUSION_COLUMNS} FROM project_exclusions ORDER BY created_at, id"
    ))?;
    let rows = statement
        .query_map([], ExclusionRow::from_row)?
        .map(|raw| {
            raw.map_err(ProjectsError::Sql)
                .and_then(|row| row.into_exclusion())
        })
        .collect::<Result<Vec<_>, ProjectsError>>()?;
    Ok(rows)
}

/// Finds an exclusion by its identity key; `None` when the path is not
/// excluded. The scan flow consults this before offering a found
/// repository (PRJ-006's "Skipped … in exclusions").
///
/// # Errors
///
/// Same contract as [`list_exclusions`].
pub fn find_exclusion_by_path_key(
    conn: &Connection,
    path_key: &str,
) -> Result<Option<Exclusion>, ProjectsError> {
    get_exclusion(
        conn,
        &format!("SELECT {EXCLUSION_COLUMNS} FROM project_exclusions WHERE path_key = ?1"),
        [path_key],
    )
}

/// Physically deletes the exclusion with this id (§6.4: path
/// exclusions are the one entity with a physical delete — the path
/// becomes a scan candidate again).
///
/// # Errors
///
/// Returns [`ProjectsError::NotFound`] when no row has this id, and
/// [`ProjectsError::Sql`] when the statement fails.
pub fn delete_exclusion(conn: &Connection, id: &str) -> Result<(), ProjectsError> {
    let changed = conn.execute("DELETE FROM project_exclusions WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(ProjectsError::NotFound { id: id.to_owned() });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The re-serialization of every enum this module stores must
    /// produce exactly the member the schema CHECK accepts — the
    /// adapters above (`parse_scheme` and friends) and the writers
    /// share the vocabularies through `as_str`/`from_member`; a drifted
    /// member would make a written row unreadable, so the round trip is
    /// pinned here.
    #[test]
    fn vocabulary_members_round_trip_through_the_schema_literals() {
        for (state, literal) in [
            (ProjectState::Active, "active"),
            (ProjectState::Archived, "archived"),
        ] {
            assert_eq!(state.as_str(), literal);
            assert_eq!(ProjectState::from_member(literal).unwrap(), state);
            assert!(matches!(
                ProjectState::from_member(&format!("{literal}x")),
                Err(ProjectsError::Member {
                    column: "state",
                    ..
                })
            ));
        }
        for (added_via, literal) in [(AddedVia::Manual, "manual"), (AddedVia::Scan, "scan")] {
            assert_eq!(added_via.as_str(), literal);
            assert_eq!(AddedVia::from_member(literal).unwrap(), added_via);
            assert!(matches!(
                AddedVia::from_member(&format!("{literal}x")),
                Err(ProjectsError::Member {
                    column: "added_via",
                    ..
                })
            ));
        }
        for (reason, literal) in [
            (ExclusionReason::RemovedScanned, "removed_scanned"),
            (ExclusionReason::Manual, "manual"),
        ] {
            assert_eq!(reason.as_str(), literal);
            assert_eq!(ExclusionReason::from_member(literal).unwrap(), reason);
            assert!(matches!(
                ExclusionReason::from_member(&format!("{literal}x")),
                Err(ProjectsError::Member {
                    column: "reason",
                    ..
                })
            ));
        }
        // The shared §6.5 vocabularies, through this module's adapters:
        // every member `settings` produces must be decodable here.
        for scheme in [Scheme::A, Scheme::B, Scheme::C] {
            assert_eq!(parse_scheme(scheme.as_str()).unwrap(), scheme);
        }
        for base_name in [BaseName::AgentsMd, BaseName::ClaudeMd] {
            assert_eq!(parse_base_name(base_name.as_str()).unwrap(), base_name);
        }
        for position in [Position::Start, Position::End] {
            assert_eq!(
                parse_position("extra_local_position", position.as_str()).unwrap(),
                position
            );
        }
    }

    /// `Display` stays lowercase and sentence-shaped without trailing
    /// punctuation (the store's error-message convention).
    #[test]
    fn error_display_is_lowercase_and_specific() {
        let duplicate = ProjectsError::DuplicatePathKey {
            path_key: String::from("/repo"),
        };
        assert_eq!(
            duplicate.to_string(),
            "a row with path_key \"/repo\" already exists"
        );
        let not_found = ProjectsError::NotFound {
            id: String::from("x"),
        };
        assert_eq!(not_found.to_string(), "no row with id \"x\"");
        let reorder = ProjectsError::ReorderMismatch {
            unknown: vec![String::from("u")],
            missing: 2,
            duplicates: 1,
        };
        assert_eq!(
            reorder.to_string(),
            "the id set does not match the table: [\"u\"] unknown, 2 missing, 1 duplicated"
        );
        let member = ProjectsError::Member {
            column: "state",
            found: String::from("bogus"),
        };
        assert_eq!(member.to_string(), "\"bogus\" is not a valid state value");
    }
}
