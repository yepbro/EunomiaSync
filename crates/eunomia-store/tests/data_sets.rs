//! Behavior tests of the §12.2 data set generator (task T-136).
//!
//! The tests run against real databases opened through `open_db` and
//! migrated with `run_migrations` — the production configuration —
//! and drive the generator only through its public API:
//!
//! * the **pinned sizes**: set M is exactly 50 projects and 200
//!   instructions (2,000 assignments, one main variant each), set L
//!   exactly 500 projects, 1,000 instructions, 100 headings, 3
//!   variants per instruction and 100,000 assignments (§12.2's own
//!   numbers, recomputed in these assertions by hand, not through the
//!   module's constants);
//! * the **invariants** the repositories own arrive for free because
//!   the generator writes through them: dense `position` and
//!   `sort_order` sequences, DB-010's exactly-one-active-main rule,
//!   and — through `build_snapshot::read` — a set L the whole build
//!   pipeline accepts as a valid ALG-001 snapshot;
//! * the **determinism contract**: two runs with the same profile
//!   over equally-named roots under different base directories
//!   produce equal stabilized views (ids differ, which is why the
//!   definition stabilizes them);
//! * set L's **assignment distribution** matches the documented
//!   stride formula, pair by pair;
//! * the **rejections**: a used database, too few roots, and the
//!   three unsupported-spec shapes all fail typed, before the first
//!   write.
//!
//! Project roots here are plain path strings, not folders on disk:
//! the repositories store `path`/`path_key` verbatim and touch no
//! file system, so nothing in this suite depends on the folders
//! existing. The real `.git` folders behind such roots are
//! `eunomia_fs::temp_repos`' own suite's subject.

#![allow(clippy::print_stderr)] // test skip-notices and benchmark timings print deliberately

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use eunomia_store::build_snapshot;
use eunomia_store::data_sets::{self, DataSetError, DataSetSpec, M, ProjectRoot, StabilizeError};
use eunomia_store::storage::{open_db, run_migrations};

/// A temporary database directory removed when the guard is dropped —
/// the sibling suites' teardown pattern.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates a unique temporary directory holding a migrated database
/// (opened through `open_db`, so the DB-001 pragma set applies).
fn data_set_db(label: &str) -> (Connection, TempDir) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-data-sets-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    let mut conn =
        open_db(&dir.join("db").join("eunomiasync.sqlite3")).expect("open_db should succeed");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to a fresh database");
    (conn, TempDir(dir))
}

/// Synthetic roots named `repo-0000…` under `base` — the strings the
/// repositories store verbatim (see the module docs for why no
/// folders are created). The key is the path's **case-folded**
/// spelling, mimicking DB-030's Windows/macOS key rule: `path` and
/// `path_key` then differ textually (the exact condition a
/// text-prefix strip cannot handle and a component-count strip must,
/// pinned by the determinism test), while on the Linux key rule they
/// coincide.
fn synthetic_roots(base: &str, count: usize) -> Vec<(String, String)> {
    (0..count)
        .map(|index| {
            let path = format!("{base}/repo-{index:04}");
            let path_key = path.to_lowercase();
            (path, path_key)
        })
        .collect()
}

/// Borrows the owned root pairs as the generator's input slice.
fn borrowed_roots(owned: &[(String, String)]) -> Vec<ProjectRoot<'_>> {
    owned
        .iter()
        .map(|(path, path_key)| ProjectRoot { path, path_key })
        .collect()
}

/// One scalar `COUNT(*)` over `table` with an optional `WHERE`
/// predicate — the SQL is assembled here, in one place, from the two
/// literal arguments.
fn count_where(conn: &Connection, table: &str, predicate: &str) -> usize {
    let sql = if predicate.is_empty() {
        format!("SELECT COUNT(*) FROM {table}")
    } else {
        format!("SELECT COUNT(*) FROM {table} WHERE {predicate}")
    };
    conn.query_row(&sql, [], |row| row.get::<_, i64>(0))
        .expect("the count query should succeed")
        .try_into()
        .expect("a row count fits usize")
}

/// `count_where` over the whole table.
fn count(conn: &Connection, table: &str) -> usize {
    count_where(conn, table, "")
}

// ---- Set M ------------------------------------------------------------------------

/// Set M is §12.2's shape exactly — 50 projects, 200 instructions —
/// and the repository invariants hold: dense sequences, DB-010's main
/// rule, the documented name/label/color shapes. Every expected
/// number below is written out from the spec, not derived from the
/// module's constants.
#[test]
fn m_profile_sizes_and_invariants_are_pinned() {
    let (mut conn, _guard) = data_set_db("m");
    let roots = synthetic_roots("/bench/m", 50);
    let generated = data_sets::generate(&mut conn, &M, &borrowed_roots(&roots))
        .expect("generating set M should succeed");

    // The §12.2 sizes, by hand: 50 projects, 200 instructions, no
    // headings, one variant per instruction, 2,000 assignments.
    assert_eq!(count(&conn, "projects"), 50);
    assert_eq!(
        count(&conn, "library_items"),
        200,
        "set M has no headings, so items are exactly its instructions"
    );
    assert_eq!(
        count(&conn, "instruction_variants"),
        200,
        "set M carries one variant per instruction"
    );
    assert_eq!(count(&conn, "project_instructions"), 2_000);
    assert_eq!(generated.assignment_count, 2_000);
    assert_eq!(generated.project_ids.len(), 50);
    assert_eq!(generated.instruction_ids.len(), 200);
    assert!(generated.variant_ids.iter().all(|group| group.len() == 1));

    // DB-011: positions are dense 1..=200.
    let positions: Vec<i64> = {
        let mut statement = conn
            .prepare("SELECT position FROM library_items ORDER BY position")
            .expect("the positions query should prepare");
        statement
            .query_map([], |row| row.get(0))
            .expect("the positions query should run")
            .collect::<Result<Vec<_>, _>>()
            .expect("positions should decode")
    };
    assert_eq!(positions, (1..=200).collect::<Vec<_>>());

    // DB-005: the projects' sort_order is dense 1..=50.
    let sort_orders: Vec<i64> = {
        let mut statement = conn
            .prepare("SELECT sort_order FROM projects ORDER BY sort_order")
            .expect("the sort_order query should prepare");
        statement
            .query_map([], |row| row.get(0))
            .expect("the sort_order query should run")
            .collect::<Result<Vec<_>, _>>()
            .expect("sort_order values should decode")
    };
    assert_eq!(sort_orders, (1..=50).collect::<Vec<_>>());

    // DB-010: every active instruction has exactly one active variant
    // and exactly one active main variant.
    let broken: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM (
                 SELECT instruction_id
                 FROM instruction_variants
                 WHERE state = 'active'
                 GROUP BY instruction_id
                 HAVING COUNT(*) != 1 OR SUM(is_main) != 1
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("the DB-010 audit should run")
        .try_into()
        .expect("a row count fits usize");
    assert_eq!(
        broken, 0,
        "every instruction must carry exactly one main variant"
    );

    // The documented shapes of the first two projects: the palette,
    // scheme and base-name cycles pinned against the module's docs.
    let view =
        data_sets::stabilized(&conn, Some("/bench/m")).expect("stabilizing set M should succeed");
    assert_eq!(view.projects.len(), 50);
    let first = &view.projects[0];
    assert_eq!(first.path, "repo-0000");
    // The key keeps its folded spelling while the path keeps its
    // stored one — both strip to the same suffix under the
    // component-count rule.
    assert_eq!(first.path_key, "repo-0000");
    assert_eq!(first.name, "Benchmark Project 0000");
    assert_eq!(first.label, "P0000");
    assert_eq!(first.color, "red");
    let second = &view.projects[1];
    assert_eq!(second.color, "orange");
    assert_eq!(second.scheme.as_str(), "B");
    assert_eq!(second.base_name.as_str(), "CLAUDE.md");
    // Flat layout: the first item is an instruction, no heading
    // precedes it.
    assert_eq!(
        view.items.first().map(|item| item.title.as_str()),
        Some("Instruction 0000")
    );

    // Every stabilized collection is populated — the determinism
    // comparison below is only meaningful if the view really carries
    // the items, variants and assignments (a view that silently
    // dropped a collection would otherwise compare equal vacuously).
    assert_eq!(view.items.len(), 200);
    assert_eq!(view.variants.len(), 200);
    assert_eq!(view.assignments.len(), 2_000);
    let first_variant = &view.variants[0];
    assert_eq!(first_variant.instruction_ordinal, 0);
    assert_eq!(first_variant.name, "Main");
    assert!(first_variant.is_main);
    assert_eq!(
        first_variant.content,
        "Apply rule 0000 to every module.\n\nKeep output reviewed."
    );
    assert!(view.items.iter().all(|item| item.heading_level.is_none()));

    // M's own distribution through the stabilized set, recomputed
    // from the module docs: project p, assignment k is instruction
    // (37·p + k) mod 200, variant (p + k) mod 1 — always the main.
    let mut expected = Vec::with_capacity(2_000);
    for project in 0..50usize {
        for k in 0..40usize {
            expected.push(data_sets::StabilizedAssignment {
                project_ordinal: project,
                instruction_ordinal: (37 * project + k) % 200,
                variant_ordinal: 0,
            });
        }
    }
    expected.sort_unstable();
    assert_eq!(view.assignments, expected);
}

// ---- Set L ------------------------------------------------------------------------

/// Set L is §12.2's upper limit exactly — 500 projects, 1,000
/// instructions, 100 headings, 3 variants per instruction, 100,000
/// assignments — with dense sequences, DB-010 intact, the documented
/// stride distribution pair by pair, and a build snapshot the whole
/// ALG-001 pipeline accepts. The counts are checked with aggregate
/// queries; only the distribution maps all rows (once), because it
/// must see every pair.
#[test]
fn l_profile_sizes_distribution_and_snapshot_are_pinned() {
    let start = std::time::Instant::now();
    let (mut conn, _guard) = data_set_db("l");
    let roots = synthetic_roots("/bench/l", 500);
    let generated = data_sets::generate(&mut conn, &data_sets::L, &borrowed_roots(&roots))
        .expect("generating set L should succeed");
    eprintln!("set L generation took {:?}", start.elapsed());

    // The §12.2 sizes, by hand.
    assert_eq!(count(&conn, "projects"), 500);
    assert_eq!(count(&conn, "library_items"), 1_100);
    assert_eq!(
        count_where(&conn, "library_items", "kind = 'instruction'"),
        1_000
    );
    assert_eq!(count_where(&conn, "library_items", "kind = 'heading'"), 100);
    assert_eq!(
        count_where(&conn, "instruction_variants", "state = 'active'"),
        3_000
    );
    assert_eq!(count(&conn, "project_instructions"), 100_000);
    assert_eq!(generated.assignment_count, 100_000);
    assert_eq!(generated.project_ids.len(), 500);
    assert_eq!(generated.instruction_ids.len(), 1_000);
    assert!(generated.variant_ids.iter().all(|group| group.len() == 3));

    // Dense sequences: COUNT equals MAX on both order columns, and
    // every heading level stays inside the §6.2 matrix.
    for (table, column) in [("library_items", "position"), ("projects", "sort_order")] {
        let (rows, highest): (i64, i64) = conn
            .query_row(
                &format!("SELECT COUNT(*), MAX({column}) FROM {table}"),
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("the density query should run");
        assert_eq!(rows, highest, "{table}.{column} must be dense 1..=N");
    }
    let (lowest_level, highest_level): (i64, i64) = conn
        .query_row(
            "SELECT MIN(heading_level), MAX(heading_level) FROM library_items \
             WHERE kind = 'heading'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the heading level query should run");
    assert!((2..=6).contains(&lowest_level) && (2..=6).contains(&highest_level));

    // DB-010 at scale: every instruction carries exactly three active
    // variants, exactly one of them main.
    let broken: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM (
                 SELECT instruction_id
                 FROM instruction_variants
                 WHERE state = 'active'
                 GROUP BY instruction_id
                 HAVING COUNT(*) != 3 OR SUM(is_main) != 1
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("the DB-010 audit should run")
        .try_into()
        .expect("a row count fits usize");
    assert_eq!(broken, 0);

    // The documented layout: heading first, then its ten instructions
    // (1,000 over 100 near-equal groups of exactly ten here).
    let first_items: Vec<(String, String, Option<i64>)> = {
        let mut statement = conn
            .prepare(
                "SELECT kind, title, heading_level FROM library_items \
                 ORDER BY position LIMIT 3",
            )
            .expect("the layout query should prepare");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("the layout query should run")
            .collect::<Result<Vec<_>, _>>()
            .expect("the layout rows should decode")
    };
    assert_eq!(
        first_items,
        vec![
            ("heading".to_owned(), "Section 0000".to_owned(), Some(2)),
            (
                "instruction".to_owned(),
                "Instruction 0000".to_owned(),
                None
            ),
            (
                "instruction".to_owned(),
                "Instruction 0001".to_owned(),
                None
            ),
        ]
    );

    // The stride distribution, pair by pair: project p, assignment k
    // is instruction (37·p + k) mod 1000 with variant (p + k) mod 3 —
    // recomputed here from the module docs, never from the module.
    let mut project_ordinal: std::collections::HashMap<String, usize> =
        std::collections::HashMap::with_capacity(500);
    for (ordinal, id) in generated.project_ids.iter().enumerate() {
        project_ordinal.insert(id.clone(), ordinal);
    }
    let mut instruction_ordinal: std::collections::HashMap<String, usize> =
        std::collections::HashMap::with_capacity(1_000);
    for (ordinal, id) in generated.instruction_ids.iter().enumerate() {
        instruction_ordinal.insert(id.clone(), ordinal);
    }
    let mut stored: Vec<(usize, usize, usize)> = Vec::with_capacity(100_000);
    {
        let mut statement = conn
            .prepare("SELECT project_id, instruction_id, variant_id FROM project_instructions")
            .expect("the assignments query should prepare");
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("the assignments query should run");
        for row in rows {
            let (project_id, instruction_id, variant_id) =
                row.expect("an assignment row should decode");
            let project = *project_ordinal
                .get(&project_id)
                .expect("an assignment names a generated project");
            let instruction = *instruction_ordinal
                .get(&instruction_id)
                .expect("an assignment names a generated instruction");
            let variant = generated.variant_ids[instruction]
                .iter()
                .position(|candidate| candidate == &variant_id)
                .expect("an assignment names a variant of its instruction");
            stored.push((project, instruction, variant));
        }
    }
    stored.sort_unstable();
    let mut expected = Vec::with_capacity(100_000);
    for project in 0..500usize {
        for k in 0..200usize {
            let instruction = (37 * project + k) % 1000;
            let variant = (project + k) % 3;
            expected.push((project, instruction, variant));
        }
    }
    expected.sort_unstable();
    assert_eq!(
        stored, expected,
        "set L's assignments must match the documented stride distribution"
    );

    // The whole set is a valid ALG-001 build snapshot: the read the
    // sync pipeline performs succeeds and sees every assignment.
    let snapshot = build_snapshot::read(&conn).expect("set L must read as a build snapshot");
    assert_eq!(snapshot.assignments().len(), 100_000);
}

// ---- Determinism ------------------------------------------------------------------

/// The determinism contract, exactly as the module defines it: two
/// runs of the same profile over equally named roots under different
/// base directories produce equal stabilized views — while their raw
/// row ids differ, which is why the definition stabilizes them. The
/// roots' keys are the case-folded spellings of their paths (the
/// `synthetic_roots` rule), so the views' equality also pins that the
/// component-count strip handles a key whose text differs from its
/// path — DB-030's Windows/macOS shape (round-1 review's P1: a text
/// prefix cannot).
#[test]
fn two_m_runs_with_equal_inputs_have_equal_stabilized_views() {
    let (mut first_conn, _first_guard) = data_set_db("determinism-a");
    let (mut second_conn, _second_guard) = data_set_db("determinism-b");
    let first_roots = synthetic_roots("/bench/Run-A", 50);
    let second_roots = synthetic_roots("/bench/Run-B", 50);
    let first = data_sets::generate(&mut first_conn, &M, &borrowed_roots(&first_roots))
        .expect("the first M run should succeed");
    let second = data_sets::generate(&mut second_conn, &M, &borrowed_roots(&second_roots))
        .expect("the second M run should succeed");

    // The definition's positive half: equal stabilized views, base
    // prefixes stripped by component count so the different scratch
    // directories — and the folded key spellings — do not count as
    // content.
    let first_view = data_sets::stabilized(&first_conn, Some("/bench/Run-A"))
        .expect("stabilizing the first run should succeed");
    let second_view = data_sets::stabilized(&second_conn, Some("/bench/Run-B"))
        .expect("stabilizing the second run should succeed");
    assert_eq!(
        first_view, second_view,
        "two runs with the same inputs must stabilize to equal data sets"
    );
    assert_eq!(first_view.projects[0].path, "repo-0000");
    assert_eq!(first_view.projects[0].path_key, "repo-0000");

    // The honest negative half: the raw ids differ between the runs —
    // UUIDv7 is generated per row (DB-002) — so "bitwise equal
    // databases" is not what determinism can mean here; the
    // stabilized view is. The first differing id would make this
    // vacuous, so assert on all of them being distinct.
    assert_ne!(first.project_ids, second.project_ids);
    assert_ne!(first.instruction_ids, second.instruction_ids);

    // Without a base the views keep the stored paths and therefore
    // differ — the prefix stripping is part of the definition, not a
    // no-op (a view that ignored `base` would pass the first
    // assertion vacuously while lying about what is stored).
    let raw_first = data_sets::stabilized(&first_conn, None)
        .expect("stabilizing without a base should succeed");
    let raw_second = data_sets::stabilized(&second_conn, None)
        .expect("stabilizing without a base should succeed");
    assert_ne!(raw_first, raw_second);
    assert_eq!(raw_first.projects[0].path, "/bench/Run-A/repo-0000");
    assert_eq!(
        raw_first.projects[0].path_key, "/bench/run-a/repo-0000",
        "the folded key spelling is stored verbatim and differs from the path"
    );
}

// ---- Rejections -------------------------------------------------------------------

/// The typed rejections all fire before the first write: a used
/// database, too few roots, and the three unsupported specs — each
/// leaving a fresh database empty (or a used one untouched).
#[test]
fn typed_rejections_write_nothing() {
    // Too few roots: M wants 50, gets 49.
    let (mut conn, _guard) = data_set_db("reject-roots");
    let short = synthetic_roots("/bench/short", 49);
    let error = data_sets::generate(&mut conn, &M, &borrowed_roots(&short))
        .expect_err("49 roots must not satisfy M");
    assert!(
        matches!(
            error,
            DataSetError::NotEnoughRoots {
                needed: 50,
                provided: 49
            }
        ),
        "wrong rejection: {error:?}"
    );
    assert_eq!(count(&conn, "projects"), 0, "nothing was written");

    // Assignments per project beyond the instruction count.
    let oversized = DataSetSpec {
        projects: 2,
        headings: 0,
        instructions: 3,
        variants_per_instruction: 1,
        assignments_per_project: 4,
    };
    let roots = synthetic_roots("/bench/spec", 2);
    let error = data_sets::generate(&mut conn, &oversized, &borrowed_roots(&roots))
        .expect_err("four assignments over three instructions must be rejected");
    assert!(
        matches!(
            error,
            DataSetError::AssignmentsPerProjectTooHigh {
                requested: 4,
                instructions: 3
            }
        ),
        "wrong rejection: {error:?}"
    );

    // Zero variants per instruction cannot satisfy DB-010.
    let bare = DataSetSpec {
        projects: 2,
        headings: 0,
        instructions: 5,
        variants_per_instruction: 0,
        assignments_per_project: 2,
    };
    let error = data_sets::generate(&mut conn, &bare, &borrowed_roots(&roots))
        .expect_err("zero variants per instruction must be rejected");
    assert!(
        matches!(error, DataSetError::VariantsRequired { requested: 0 }),
        "wrong rejection: {error:?}"
    );
    assert_eq!(
        count(&conn, "library_items"),
        0,
        "still nothing was written"
    );

    // Zero instructions is its own refusal.
    let empty_library = DataSetSpec {
        projects: 2,
        headings: 0,
        instructions: 0,
        variants_per_instruction: 1,
        assignments_per_project: 0,
    };
    let error = data_sets::generate(&mut conn, &empty_library, &borrowed_roots(&roots))
        .expect_err("a profile without instructions must be rejected");
    assert!(
        matches!(error, DataSetError::InstructionsRequired),
        "wrong rejection: {error:?}"
    );

    // A used database is refused with the counts of what is there,
    // and its content is untouched.
    let seeded_roots = synthetic_roots("/bench/seeded", 2);
    let seeded = DataSetSpec {
        projects: 2,
        headings: 0,
        instructions: 5,
        variants_per_instruction: 1,
        assignments_per_project: 2,
    };
    data_sets::generate(&mut conn, &seeded, &borrowed_roots(&seeded_roots))
        .expect("the small spec should generate");
    let error = data_sets::generate(
        &mut conn,
        &M,
        &borrowed_roots(&synthetic_roots("/bench/m2", 50)),
    )
    .expect_err("a used database must be rejected");
    assert!(
        matches!(
            error,
            DataSetError::NotEmpty {
                projects: 2,
                library_items: 5
            }
        ),
        "wrong rejection: {error:?}"
    );
    assert_eq!(
        count(&conn, "projects"),
        2,
        "the existing rows are untouched"
    );
}

/// The heading layout's remainder arithmetic and the empty-heading
/// edge — the branches set M (flat) and set L (exact 10×100 groups)
/// never reach: near-equal groups 4/3/3 for 3 headings over 10
/// instructions, and 1/1/1/0/0 for 5 headings over 3 (empty headings
/// are legal, ALG-021). Also pins the variant names beyond the third
/// — numbered alternatives, distinct NOCASE — and that every such
/// shape still reads as a valid build snapshot.
#[test]
fn heading_layout_edges_and_variant_names_beyond_three() {
    // Near-equal groups: 10 instructions over 3 headings → 4/3/3.
    let (mut conn, _guard) = data_set_db("layout-remainder");
    let spec = DataSetSpec {
        projects: 1,
        headings: 3,
        instructions: 10,
        variants_per_instruction: 5,
        assignments_per_project: 4,
    };
    let roots = synthetic_roots("/bench/layout", 1);
    data_sets::generate(&mut conn, &spec, &borrowed_roots(&roots))
        .expect("the near-equal spec should generate");
    let layout: Vec<(String, String)> = {
        let mut statement = conn
            .prepare("SELECT kind, title FROM library_items ORDER BY position")
            .expect("the layout query should prepare");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("the layout query should run")
            .collect::<Result<Vec<_>, _>>()
            .expect("the layout rows should decode")
    };
    let expected: Vec<(String, String)> = [
        ("heading", "Section 0000"),
        ("instruction", "Instruction 0000"),
        ("instruction", "Instruction 0001"),
        ("instruction", "Instruction 0002"),
        ("instruction", "Instruction 0003"),
        ("heading", "Section 0001"),
        ("instruction", "Instruction 0004"),
        ("instruction", "Instruction 0005"),
        ("instruction", "Instruction 0006"),
        ("heading", "Section 0002"),
        ("instruction", "Instruction 0007"),
        ("instruction", "Instruction 0008"),
        ("instruction", "Instruction 0009"),
    ]
    .into_iter()
    .map(|(kind, title)| (kind.to_owned(), title.to_owned()))
    .collect();
    assert_eq!(layout, expected);

    // Variant names beyond the third are numbered alternatives, the
    // first one alone is main.
    let names: Vec<(String, bool)> = {
        let mut statement = conn
            .prepare(
                "SELECT name, is_main FROM instruction_variants \
                 WHERE instruction_id = (SELECT id FROM library_items WHERE kind = \
                 'instruction' ORDER BY position LIMIT 1) ORDER BY created_at, id",
            )
            .expect("the variant names query should prepare");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("the variant names query should run")
            .collect::<Result<Vec<_>, _>>()
            .expect("the variant name rows should decode")
    };
    assert_eq!(
        names,
        [
            ("Main".to_owned(), true),
            ("Compact".to_owned(), false),
            ("Detailed".to_owned(), false),
            ("Alternative 03".to_owned(), false),
            ("Alternative 04".to_owned(), false),
        ]
    );
    build_snapshot::read(&conn).expect("the near-equal shape must read as a snapshot");

    // Headings outnumbering the instructions: 1/1/1/0/0 — the last
    // two headings are empty and the snapshot still accepts them.
    let (mut conn, _guard) = data_set_db("layout-empty-headings");
    let spec = DataSetSpec {
        projects: 1,
        headings: 5,
        instructions: 3,
        variants_per_instruction: 1,
        assignments_per_project: 3,
    };
    data_sets::generate(&mut conn, &spec, &borrowed_roots(&roots))
        .expect("the empty-headings spec should generate");
    let layout: Vec<(String, String)> = {
        let mut statement = conn
            .prepare("SELECT kind, title FROM library_items ORDER BY position")
            .expect("the layout query should prepare");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("the layout query should run")
            .collect::<Result<Vec<_>, _>>()
            .expect("the layout rows should decode")
    };
    let expected: Vec<(String, String)> = [
        ("heading", "Section 0000"),
        ("instruction", "Instruction 0000"),
        ("heading", "Section 0001"),
        ("instruction", "Instruction 0001"),
        ("heading", "Section 0002"),
        ("instruction", "Instruction 0002"),
        ("heading", "Section 0003"),
        ("heading", "Section 0004"),
    ]
    .into_iter()
    .map(|(kind, title)| (kind.to_owned(), title.to_owned()))
    .collect();
    assert_eq!(layout, expected);
    build_snapshot::read(&conn).expect("empty headings are legal in a snapshot");
}

/// A repository failure **inside** the fill rolls the whole run back:
/// the second root reuses the first's identity key, so the second
/// `projects::create` fails after the first project was already
/// written — and nothing survives.
#[test]
fn a_mid_fill_failure_rolls_the_whole_run_back() {
    let (mut conn, _guard) = data_set_db("rollback");
    let mut roots = synthetic_roots("/bench/rollback", 3);
    // The second root carries the first's path_key (and a different
    // path): DB-015's uniqueness is about the key, not the folder.
    roots[1].1 = roots[0].1.clone();
    let spec = DataSetSpec {
        projects: 3,
        headings: 0,
        instructions: 4,
        variants_per_instruction: 1,
        assignments_per_project: 2,
    };
    let error = data_sets::generate(&mut conn, &spec, &borrowed_roots(&roots))
        .expect_err("a duplicate identity key must fail the fill");
    assert!(
        matches!(
            error,
            DataSetError::Projects(eunomia_store::projects::ProjectsError::DuplicatePathKey { .. })
        ),
        "wrong failure: {error:?}"
    );
    for table in [
        "projects",
        "library_items",
        "instruction_variants",
        "project_instructions",
    ] {
        assert_eq!(
            count(&conn, table),
            0,
            "the rolled-back run must leave {table} empty"
        );
    }
    // And the emptied database accepts a clean re-run — the retry
    // story of the calling contract.
    let fresh_roots = synthetic_roots("/bench/rollback-2", 3);
    data_sets::generate(&mut conn, &spec, &borrowed_roots(&fresh_roots))
        .expect("a retry after the rollback should succeed");
    assert_eq!(count(&conn, "projects"), 3);
}

/// The stabilized read surfaces the repositories' read errors
/// unchanged — the corrupt-cell policy (D-039) through this API too.
#[test]
fn stabilized_propagates_read_errors() {
    let (mut conn, _guard) = data_set_db("stabilize-error");
    let roots = synthetic_roots("/bench/corrupt", 2);
    let seeded = DataSetSpec {
        projects: 2,
        headings: 0,
        instructions: 3,
        variants_per_instruction: 1,
        assignments_per_project: 1,
    };
    data_sets::generate(&mut conn, &seeded, &borrowed_roots(&roots))
        .expect("the small spec should generate");
    // The corruption seam of the sibling suites: the §6.2 CHECKs are
    // the schema floor, so writing a cell outside the vocabulary
    // needs `ignore_check_constraints` for the duration of the
    // update (out-of-band data, the D-039 premise).
    conn.pragma_update(None, "ignore_check_constraints", true)
        .expect("the corruption seam should enable");
    conn.execute("UPDATE projects SET scheme = 'Z' WHERE sort_order = 1", [])
        .expect("corrupting the scheme cell should succeed");
    conn.pragma_update(None, "ignore_check_constraints", false)
        .expect("the corruption seam should disable");
    let error = data_sets::stabilized(&conn, None)
        .expect_err("a corrupt vocabulary cell must fail the read");
    assert!(
        matches!(
            error,
            StabilizeError::Projects(eunomia_store::projects::ProjectsError::Member {
                column: "scheme",
                ..
            })
        ),
        "the corrupt cell must surface as the typed Member error, got {error:?}"
    );
}
