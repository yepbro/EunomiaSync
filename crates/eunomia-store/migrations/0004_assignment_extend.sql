-- EunomiaSync database schema, version 4 (task T-356).
--
-- Land MAIN-034's extend cell (D-203 reading 4a): an assignment may
-- keep a base variant reference BESIDE per-project custom text (the
-- supplement), §6.2's third assignment shape. The v3 CHECK's
-- exactly-one-target rule (D-192) is superseded for this arm only:
-- variant_id + custom_text is now the extend cell, and "neither"
-- (variant_id IS NULL AND custom_text IS NULL) stays the one illegal
-- shape.
--
-- SQLite cannot alter a table-level CHECK in place, so the child table
-- is rebuilt inside rusqlite_migration's per-step transaction — the
-- 0003 pattern. No table references project_instructions, and existing
-- v3 rows are all variant-only or custom-text-only, so the copy is
-- lossless. The old table (and its renamed indexes) is dropped before
-- the §6.2 index names are recreated. After this file the schema
-- version is 4.

ALTER TABLE project_instructions RENAME TO project_instructions_v3;

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
        OR (variant_id IS NULL AND custom_text IS NOT NULL)
        OR (variant_id IS NOT NULL AND custom_text IS NOT NULL))
);

INSERT INTO project_instructions (
    project_id, instruction_id, variant_id, custom_text, created_at, updated_at
)
SELECT project_id, instruction_id, variant_id, custom_text, created_at, updated_at
  FROM project_instructions_v3;

DROP TABLE project_instructions_v3;

CREATE INDEX ix_project_instructions_instruction_id ON project_instructions(instruction_id);
CREATE INDEX ix_project_instructions_variant_id ON project_instructions(variant_id);
