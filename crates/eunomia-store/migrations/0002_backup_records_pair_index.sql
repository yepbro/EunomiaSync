-- EunomiaSync database schema, version 2 (task T-256).
--
-- A single index-only change: `ix_backup_records_pair_lookup` serves
-- the per-pair `last_checked` lookup of
-- `eunomia_store::backup_records` (BKP-010's dedup input, §9.4 phase
-- 2). Sync phase 2 issues that lookup once per eligible project, and
-- without an index every one of them scanned the whole
-- `backup_records` table — N full scans per sync run (the D-132
-- follow-up this migration closes).
--
-- The column set is the owner's decision (the task card): equality on
-- (project_id, source_path, status) — the lookup's WHERE clause and
-- the "project and file" pair it scopes by — followed by `created_at`,
-- the ordering the lookup's "newest `ok` copy" reads (`created_at
-- DESC, id DESC`, D-122/D-132), so the index both seeks the pair and
-- walks its rows newest-first. The retention window's per-pair
-- ranking (PARTITION BY project_id, source_path ORDER BY created_at
-- DESC, id DESC, `backup_retention::apply`) reads the same pair and
-- stamp columns.
--
-- No table data, constraint or v1 object changes; migrations go
-- forward only and are never edited after shipping (spec DB-004),
-- and this one runs in its own transaction with the
-- D-038 pre-migration `VACUUM INTO` copy, like every step. After
-- this file the schema version is 2.

CREATE INDEX ix_backup_records_pair_lookup
    ON backup_records (project_id, source_path, status, created_at);
