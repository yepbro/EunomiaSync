-- EunomiaSync database schema, version 1 (spec §6.2; task T-016).
--
-- This migration replaces the former `meta` stub: application key/value
-- state lives in `settings` (spec §6.5). It is applied by
-- `eunomia_store::storage::run_migrations` through rusqlite_migration,
-- which records the applied version in `PRAGMA user_version` (spec DB-004)
-- and wraps each migration in its own transaction. After this file the
-- schema version is 1.
--
-- Conventions (spec §6.1):
--   DB-002  row identifiers are UUIDv7 TEXT, timestamps are UTC ISO-8601
--           strings with milliseconds (`eunomia_store::values`);
--   DB-003  booleans are INTEGER 0/1 with CHECK, enums are TEXT with
--           CHECK;
--   DB-005  item order is an integer `position` starting from 1; the
--           no-gaps half of the rule is enforced by the service layer in
--           one transaction (spec DB-011), the DB-level floor is the
--           `position >= 1` CHECK on `library_items`;
--   DB-006  variant text is at most 262144 characters.
--
-- The DDL follows the §6.2 reference schema: column names may be
-- adjusted per the spec preamble, but constraints and relations may not.
-- Deviations from the reference text: formatting; two strengthened
-- `library_items` constraints (`position >= 1` and the explicit
-- `heading_level IS NOT NULL`, both explained above and in DECISIONS.md
-- D-036); and the two extra `project_instructions` indexes required by
-- the §6.2 closing paragraph (index names are an implementation choice).

-- Application settings: `key` is a dotted path such as `sync.skip_identical`,
-- `value` holds the JSON-encoded setting (spec §6.5).
CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- A synchronized repository. `path_key` is the canonical path form used for
-- duplicate detection (spec DB-030/DB-015); it stays unique across active
-- and archived projects.
CREATE TABLE projects (
    id                     TEXT PRIMARY KEY,
    path                   TEXT NOT NULL,
    path_key               TEXT NOT NULL UNIQUE,
    name                   TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 100),
    label                  TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 6),
    color                  TEXT NOT NULL,
    description            TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 1000),
    state                  TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'archived')),
    added_via              TEXT NOT NULL CHECK (added_via IN ('manual', 'scan')),
    sort_order             INTEGER NOT NULL,
    scheme                 TEXT NOT NULL CHECK (scheme IN ('A', 'B', 'C')),
    base_name              TEXT NOT NULL CHECK (base_name IN ('AGENTS.md', 'CLAUDE.md')),
    extra_local_enabled    INTEGER NOT NULL DEFAULT 0 CHECK (extra_local_enabled IN (0, 1)),
    extra_local_position   TEXT NOT NULL DEFAULT 'end' CHECK (extra_local_position IN ('start', 'end')),
    extra_project_enabled  INTEGER NOT NULL DEFAULT 0 CHECK (extra_project_enabled IN (0, 1)),
    extra_project_position TEXT NOT NULL DEFAULT 'end' CHECK (extra_project_position IN ('start', 'end')),
    last_synced_at         TEXT,
    last_synced_hash       TEXT,
    created_at             TEXT NOT NULL,
    updated_at             TEXT NOT NULL,
    archived_at            TEXT
);

-- Paths the scan must skip: archived-by-scan projects and manual exclusions.
CREATE TABLE project_exclusions (
    id         TEXT PRIMARY KEY,
    path       TEXT NOT NULL,
    path_key   TEXT NOT NULL UNIQUE,
    reason     TEXT NOT NULL CHECK (reason IN ('removed_scanned', 'manual')),
    created_at TEXT NOT NULL
);

-- The shared instruction library: instructions and section headings in one
-- ordered list (spec §7.3). `position` is the DB-005 order; `heading_level`
-- applies to headings only (levels 2-6) and must be NULL for instructions;
-- headings cannot be hidden. The first table-level CHECK spells out
-- `heading_level IS NOT NULL` explicitly: under SQL three-valued logic the
-- §6.2 reference expression alone lets a heading with a NULL level through
-- (NULL BETWEEN 2 AND 6 is NULL, and a CHECK only rejects FALSE), which
-- defeats the constraint's intent.
CREATE TABLE library_items (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL CHECK (kind IN ('instruction', 'heading')),
    position      INTEGER NOT NULL CHECK (position >= 1),
    heading_level INTEGER,
    title         TEXT NOT NULL CHECK (length(trim(title)) BETWEEN 1 AND 200),
    state         TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'hidden')),
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    hidden_at     TEXT,
    CHECK ((kind = 'heading' AND heading_level IS NOT NULL AND heading_level BETWEEN 2 AND 6)
        OR (kind = 'instruction' AND heading_level IS NULL)),
    CHECK (kind = 'instruction' OR state = 'active')
);
CREATE INDEX ix_items_position ON library_items(position);

-- Variants of an instruction (spec §7.3). `UNIQUE (instruction_id, id)` is
-- required as the parent key of the composite foreign key from
-- `project_instructions` (SQLite demands a UNIQUE index on the referenced
-- columns). The partial unique indexes enforce "exactly one active main
-- variant per instruction" (DB-010) and case-insensitive variant-name
-- uniqueness among active variants (spec §6.9).
CREATE TABLE instruction_variants (
    id             TEXT PRIMARY KEY,
    instruction_id TEXT NOT NULL REFERENCES library_items(id) ON DELETE RESTRICT,
    name           TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 100),
    content        TEXT NOT NULL DEFAULT '' CHECK (length(content) <= 262144),
    is_main        INTEGER NOT NULL DEFAULT 0 CHECK (is_main IN (0, 1)),
    state          TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'deleted')),
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    deleted_at     TEXT,
    UNIQUE (instruction_id, id)
);
CREATE UNIQUE INDEX ux_variant_main ON instruction_variants(instruction_id)
    WHERE is_main = 1 AND state = 'active';
CREATE UNIQUE INDEX ux_variant_name ON instruction_variants(instruction_id, name COLLATE NOCASE)
    WHERE state = 'active';

-- Assignment matrix rows: one variant of one instruction per project
-- (spec §7.4). The composite foreign key ties the assignment to a variant
-- of exactly the referenced instruction, so the pair
-- (instruction_id, variant_id) cannot point at another instruction's
-- variant.
CREATE TABLE project_instructions (
    project_id     TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    instruction_id TEXT NOT NULL,
    variant_id     TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    PRIMARY KEY (project_id, instruction_id),
    FOREIGN KEY (instruction_id, variant_id)
        REFERENCES instruction_variants(instruction_id, id) ON DELETE RESTRICT
);
-- Extra lookups required by the closing paragraph of spec §6.2.
CREATE INDEX ix_project_instructions_instruction_id ON project_instructions(instruction_id);
CREATE INDEX ix_project_instructions_variant_id ON project_instructions(variant_id);

-- Revision history of instructions and variants (spec §9.8): `rev_no`
-- grows by one within an entity (ALG-071); `snapshot` is the JSON snapshot
-- and `snapshot_hash` its SHA-256 (ALG-070). Headings have no revisions.
CREATE TABLE revisions (
    id            TEXT PRIMARY KEY,
    entity_type   TEXT NOT NULL CHECK (entity_type IN ('instruction', 'variant')),
    entity_id     TEXT NOT NULL,
    rev_no        INTEGER NOT NULL,
    op            TEXT NOT NULL CHECK (op IN ('create', 'update', 'hide', 'restore', 'delete', 'revert')),
    snapshot      TEXT NOT NULL,
    snapshot_hash TEXT NOT NULL,
    reverted_from INTEGER,
    created_at    TEXT NOT NULL,
    UNIQUE (entity_type, entity_id, rev_no)
);

-- One row per sync run (spec §9.4); per-project outcomes live in
-- `sync_run_projects`.
CREATE TABLE sync_runs (
    id                  TEXT PRIMARY KEY,
    started_at          TEXT NOT NULL,
    finished_at         TEXT,
    status              TEXT NOT NULL CHECK (status IN
        ('running', 'success', 'partial', 'aborted', 'failed', 'interrupted')),
    projects_total      INTEGER NOT NULL DEFAULT 0,
    projects_written    INTEGER NOT NULL DEFAULT 0,
    projects_unchanged  INTEGER NOT NULL DEFAULT 0,
    projects_skipped    INTEGER NOT NULL DEFAULT 0,
    projects_failed     INTEGER NOT NULL DEFAULT 0,
    error_code          TEXT,
    details             TEXT
);

-- Per-project outcomes of a sync run (the m:n link between runs and
-- projects). Deleting a run removes its outcomes; a project with recorded
-- outcomes cannot be deleted while they exist.
CREATE TABLE sync_run_projects (
    sync_run_id    TEXT NOT NULL REFERENCES sync_runs(id) ON DELETE CASCADE,
    project_id     TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    status         TEXT NOT NULL CHECK (status IN
        ('written', 'unchanged', 'skipped', 'failed', 'not_started')),
    target_path    TEXT,
    build_hash     TEXT,
    error_code     TEXT,
    error_message  TEXT,
    PRIMARY KEY (sync_run_id, project_id)
);

-- Journal of file backups (spec §7.6): one row per copied file.
-- `backup_path` is unique; losing the run keeps the record
-- (`sync_run_id` becomes NULL).
CREATE TABLE backup_records (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    sync_run_id  TEXT REFERENCES sync_runs(id) ON DELETE SET NULL,
    source_path  TEXT NOT NULL,
    backup_path  TEXT NOT NULL UNIQUE,
    size_bytes   INTEGER NOT NULL,
    sha256       TEXT NOT NULL,
    status       TEXT NOT NULL CHECK (status IN ('ok', 'pruned', 'missing', 'discarded')),
    created_at   TEXT NOT NULL
);

-- Append-only activity log (spec §7.7). The id is a plain autoincrement
-- counter, not a UUIDv7: the log has no foreign references (an entry
-- outlives the entities it names), and `ts` is indexed for the date
-- filters of the Activity window.
CREATE TABLE action_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts          TEXT NOT NULL,
    level       TEXT NOT NULL CHECK (level IN ('info', 'warn', 'error')),
    visibility  TEXT NOT NULL CHECK (visibility IN ('user', 'diagnostic')),
    action      TEXT NOT NULL,
    entity_type TEXT,
    entity_id   TEXT,
    project_id  TEXT,
    sync_run_id TEXT,
    result      TEXT NOT NULL CHECK (result IN ('ok', 'error', 'skipped')),
    details     TEXT
);
CREATE INDEX ix_log_ts ON action_log(ts);
