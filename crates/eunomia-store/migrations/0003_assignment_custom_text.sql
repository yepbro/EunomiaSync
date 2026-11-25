-- EunomiaSync database schema, version 3 (task T-331).
--
-- Land the current MAIN-007/D-125 assignment model: a cell points either
-- to an active same-instruction variant or to per-project custom text.
-- The active-variant half is a service rule (DB-012); the direct instruction
-- FK, same-instruction composite FK, text ceiling and exactly-one-target
-- invariant are database floors.
--
-- SQLite cannot add the direct FK/CHECK in place, so rebuild the child table
-- inside rusqlite_migration's per-step transaction. No table references
-- project_instructions, and existing rows all have a variant, so the copy is
-- lossless and sets custom_text to NULL. The old table (and its renamed
-- indexes) is dropped before the spec's index names are recreated.
-- After this file the schema version is 3.

ALTER TABLE project_instructions RENAME TO project_instructions_v2;

CREATE TABLE project_instructions (
    project_id     TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    instruction_id TEXT NOT NULL REFERENCES library_items(id) ON DELETE RESTRICT,
    variant_id     TEXT,
    custom_text    TEXT CHECK (custom_text IS NULL OR length(custom_text) <= 262144),
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    PRIMARY KEY (project_id, instruction_id),
    FOREIGN KEY (instruction_id, variant_id)
        REFERENCES instruction_variants(instruction_id, id) ON DELETE RESTRICT,
    CHECK ((variant_id IS NOT NULL AND custom_text IS NULL)
        OR (variant_id IS NULL AND custom_text IS NOT NULL))
);

INSERT INTO project_instructions (
    project_id, instruction_id, variant_id, custom_text, created_at, updated_at
)
SELECT project_id, instruction_id, variant_id, NULL, created_at, updated_at
  FROM project_instructions_v2;

DROP TABLE project_instructions_v2;

CREATE INDEX ix_project_instructions_instruction_id ON project_instructions(instruction_id);
CREATE INDEX ix_project_instructions_variant_id ON project_instructions(variant_id);
