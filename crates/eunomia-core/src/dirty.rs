//! The project dirty flag (spec §6.6 DB-021, with DB-014 and DB-023).
//!
//! Dirty means *there are unsynced changes*: the build the project
//! would write now is not the one its last successful sync wrote
//! (OUT-013 — the app warns the user about exactly this). The flag is
//! about the app's own writes and is independent of the
//! externally-modified state (SYS-008): a project can be dirty,
//! externally modified, or both.
//!
//! # The rule
//!
//! DB-021, transcribed over the input facts:
//!
//! - an archived project is never counted as dirty (DB-014: it keeps
//!   its assignments but takes part in no syncs, so unsynced changes
//!   cannot arise);
//! - an active project is dirty when its stored `last_synced_hash` is
//!   empty while the build text is not empty (never synced, and there
//!   is something to write), or when the stored hash differs from the
//!   current one;
//! - a project with no assignments that has never been synced is not
//!   counted as dirty — an absolute sentence, phrased exactly like
//!   DB-014's archived one, so it holds even when the build text is
//!   not empty.
//!
//! The one reading this module resolves: *an empty stored hash never
//! reaches the difference comparison.* "The hash differs from the
//! current one" presupposes a stored hash; with none stored, the
//! first clause alone decides. A bare `"" != current` inequality
//! would make every never-synced active project dirty regardless of
//! its build — the current hash is never empty, it is sixty-four hex
//! characters ([`build_hash`](crate::hash::build_hash)) — which would
//! render the first clause fully redundant and flag projects that
//! build nothing. The unit test
//! `an_empty_stored_hash_never_reaches_the_difference_comparison`
//! pins this; DECISIONS.md D-068 records the reasoning.
//!
//! # The facts, and where they come from
//!
//! [`is_dirty`] is pure: no clock, no I/O, no database — every fact
//! is a parameter, so DB-022's recalculation pipeline (the future
//! caller) computes them wherever it already does:
//!
//! - **state** — the `projects.state` column's two literals, as this
//!   module's own [`ProjectState`] input: the build snapshot carries
//!   no state at all (D-042 — it holds active projects only, and
//!   housekeeping columns stay out), while DB-014 still has to be
//!   encoded here;
//! - **last_synced_hash** — `Option<&str>`: the §6.2 column is
//!   nullable, and DB-024 fills it only after a `written` or
//!   `unchanged` status — and the only writer, `build_hash`, always
//!   emits sixty-four hex characters (D-064), so an empty string is
//!   never a stored value; `None` and `Some("")` therefore mean the
//!   same thing — *never synced*;
//! - **current_hash** — the hash of the current build, as computed by
//!   [`build_hash`](crate::hash::build_hash) (D-064); the predicate
//!   compares it opaquely and never inspects its shape;
//! - **build_text** — the markerless managed text, the same string
//!   the current hash was computed over (D-064's seam; today that is
//!   [`build_body`](crate::body::build_body)'s output). "Not empty"
//!   is `""` exactly: the build has already normalized its texts, no
//!   whitespace rounding happens here;
//! - **has_assignments** — whether the project currently has at least
//!   one assignment, to an active or a hidden instruction alike
//!   (DB-013 allows the latter; such an assignment is real for the
//!   exception sentence yet contributes nothing to the build text —
//!   the two facts are independent inputs).
//!
//! DB-023 needs no rule of its own in this module: changing the
//! scheme or the file name moves the *current hash* (both are
//! [`build_hash`](crate::hash::build_hash) inputs, D-064), and the
//! difference clause fires. The unit test
//! `changing_the_scheme_or_the_file_name_makes_the_project_dirty`
//! pins the composition through the real hash function.

/// The project's lifecycle state as the dirty rule reads it (§6.2
/// `projects.state`, the CHECK literals `'active'`/`'archived'`).
///
/// The predicate's own input, not a shared vocabulary type: the build
/// snapshot holds active projects only and carries no state column at
/// all (D-042), while DB-014's "an archived project … is not counted
/// as dirty" must still be encoded — so the two literals ride with
/// the rule that reads them. A broader project-state type, if a later
/// task needs one, subsumes this enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectState {
    /// `state = 'active'` — the project takes part in syncs.
    Active,
    /// `state = 'archived'` — keeps its assignments, takes part in no
    /// syncs, never counts as dirty (DB-014).
    Archived,
}

/// Decides whether a project is dirty (spec §6.6 DB-021, DB-014).
///
/// The five parameters are the rule's facts, in its reading order:
/// the lifecycle state (DB-014's gate), the stored `last_synced_hash`
/// (`None` and `Some("")` both mean *never synced* — DB-024 stores a
/// hash only after a successful write or an `unchanged` status — and
/// the only writer, `build_hash`, never stores an empty string), the
/// current build hash, the markerless build text the hash was computed
/// over, and whether the project has at least one assignment (the
/// exception sentence's fact). See the module docs for the rule and
/// each boundary decision.
///
/// The call is pure — no clock, no I/O — so the same facts always
/// yield the same verdict.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use eunomia_core::dirty::{ProjectState, is_dirty};
/// use eunomia_core::hash::build_hash;
/// use eunomia_core::project::{ExtraFile, ExtraFilePosition, Scheme};
///
/// let (local, project) = (
///     ExtraFile { enabled: true, position: ExtraFilePosition::Start },
///     ExtraFile { enabled: false, position: ExtraFilePosition::End },
/// );
/// let path = Path::new("/repos/demo/AGENTS.md");
/// let build = "## Style\n\nBe brief.";
/// let current = build_hash(Scheme::A, path, local, project, build);
///
/// // Active, has assignments, never synced, and something to write:
/// // dirty (DB-021, first clause).
/// assert!(is_dirty(ProjectState::Active, None, &current, build, true));
///
/// // After that sync stores the matching hash (DB-024): clean.
/// assert!(!is_dirty(ProjectState::Active, Some(&current), &current, build, true));
///
/// // Archived is never dirty, whatever the other facts say (DB-014).
/// assert!(!is_dirty(ProjectState::Archived, None, &current, build, true));
/// ```
#[must_use]
pub fn is_dirty(
    state: ProjectState,
    last_synced_hash: Option<&str>,
    current_hash: &str,
    build_text: &str,
    has_assignments: bool,
) -> bool {
    match state {
        // DB-014: an archived project takes part in no sync, so
        // unsynced changes cannot arise — never dirty, whatever the
        // stored hash, the build or the assignments say.
        ProjectState::Archived => false,
        ProjectState::Active => match last_synced_hash {
            // Never synced (an empty stored hash — `NULL` or `''` —
            // means no sync ever stored one, DB-024): the exception
            // sentence shields a project with no assignments, and the
            // first clause alone decides the rest — the empty stored
            // hash never reaches the difference comparison.
            None | Some("") => has_assignments && !build_text.is_empty(),
            // A stored hash: dirty exactly when it differs from the
            // current one (the second clause). The build text and the
            // assignments reach this comparison only through the hash
            // — the same fields fingerprint it (DB-020).
            Some(stored) => stored != current_hash,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::hash::build_hash;
    use crate::project::{ExtraFile, ExtraFilePosition, Scheme};

    /// A non-empty build text in the `hash` module's fixture shape.
    const BUILD: &str = "## Style\n\nBe brief.";

    /// Two hash values in the real spelling: sixty-four lowercase hex
    /// characters (D-064). The predicate compares them opaquely — any
    /// two distinct strings would do — but the fixtures keep the shape
    /// `build_hash` produces. `HASH_ONE` is, in fact, that function's
    /// oracle value for this very `BUILD` under its fixture inputs.
    const HASH_ONE: &str = "d2a2446b2437df4c64a4db2b3a49a45e519fddf98e8718ac0fb9f682f9b83d27";
    const HASH_TWO: &str = "6a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9";

    /// One row of the oracle matrix: the five facts, then the expected
    /// verdict.
    type Row = (
        ProjectState,
        Option<&'static str>,
        &'static str,
        &'static str,
        bool,
        bool,
    );

    use ProjectState::{Active, Archived};

    #[test]
    fn the_oracle_matrix_covers_every_branch_of_the_rule() {
        // state × stored hash (never synced as `None`, never synced as
        // `''`, stored and equal, stored and differing) × build text
        // (non-empty, empty) × assignments — every cell of the matrix,
        // each with the clause that fixes its verdict.
        let rows: &[Row] = &[
            // — Archived (DB-014): never dirty, and the rows below
            //   deliberately reuse fact combinations that the clauses
            //   would decide the other way were the project active.
            (Archived, None, HASH_ONE, BUILD, true, false), // clause 1 would fire
            (Archived, Some(""), HASH_ONE, BUILD, true, false), // clause 1 would fire
            (Archived, Some(HASH_ONE), HASH_ONE, BUILD, true, false), // clause 2: equal
            (Archived, Some(HASH_TWO), HASH_ONE, BUILD, true, false), // clause 2: differs
            (Archived, None, HASH_ONE, "", true, false),    // clause 1 not met (build empty)
            (Archived, Some(""), HASH_ONE, "", true, false),
            (Archived, Some(HASH_ONE), HASH_ONE, "", true, false),
            (Archived, Some(HASH_TWO), HASH_ONE, "", true, false),
            (Archived, None, HASH_ONE, BUILD, false, false), // exception irrelevant
            (Archived, Some(""), HASH_ONE, BUILD, false, false),
            (Archived, Some(HASH_ONE), HASH_ONE, BUILD, false, false),
            (Archived, Some(HASH_TWO), HASH_ONE, BUILD, false, false),
            (Archived, None, HASH_ONE, "", false, false),
            (Archived, Some(""), HASH_ONE, "", false, false),
            (Archived, Some(HASH_ONE), HASH_ONE, "", false, false),
            (Archived, Some(HASH_TWO), HASH_ONE, "", false, false),
            // — Active, never synced (DB-024: nothing stored yet).
            //   The first clause alone decides: empty stored hash plus
            //   a non-empty build is dirty…
            (Active, None, HASH_ONE, BUILD, true, true),
            (Active, Some(""), HASH_ONE, BUILD, true, true),
            // …the exception sentence shields a project with no
            //   assignments even then…
            (Active, None, HASH_ONE, BUILD, false, false),
            (Active, Some(""), HASH_ONE, BUILD, false, false),
            // …and an empty build keeps a never-synced project clean,
            //   assignments or not — the empty stored hash never
            //   reaches the difference comparison.
            (Active, None, HASH_ONE, "", true, false),
            (Active, Some(""), HASH_ONE, "", true, false),
            (Active, None, HASH_ONE, "", false, false),
            (Active, Some(""), HASH_ONE, "", false, false),
            // — Active, synced, stored hash equal to the current one
            //   (the second clause): clean whatever the build and the
            //   assignments look like — the hash already fingerprints
            //   them (DB-020).
            (Active, Some(HASH_ONE), HASH_ONE, BUILD, true, false),
            (Active, Some(HASH_ONE), HASH_ONE, BUILD, false, false),
            (Active, Some(HASH_ONE), HASH_ONE, "", true, false),
            (Active, Some(HASH_ONE), HASH_ONE, "", false, false),
            // — Active, synced, stored hash differing (the second
            //   clause): dirty — including the intersections. The
            //   exception cannot apply ("has never been synced"
            //   fails), and an emptied or changed build has already
            //   moved the hash.
            (Active, Some(HASH_TWO), HASH_ONE, BUILD, true, true),
            // Assignments removed after a sync — the file still
            // carries the old build:
            (Active, Some(HASH_TWO), HASH_ONE, BUILD, false, true),
            // Every assignment went to a hidden instruction (DB-013):
            // real assignments, emptied build:
            (Active, Some(HASH_TWO), HASH_ONE, "", true, true),
            // Build emptied after a sync, no assignments left:
            (Active, Some(HASH_TWO), HASH_ONE, "", false, true),
        ];

        for (index, (state, stored, current, build, has_assignments, expected)) in
            rows.iter().enumerate()
        {
            assert_eq!(
                is_dirty(*state, *stored, current, build, *has_assignments),
                *expected,
                "matrix row {index}: state={state:?} stored={stored:?} \
                 build_is_empty={} has_assignments={has_assignments}",
                build.is_empty(),
            );
        }
    }

    #[test]
    fn an_empty_stored_hash_never_reaches_the_difference_comparison() {
        // "The hash differs from the current one" presupposes a stored
        // hash; with none stored, the first clause alone decides
        // (DECISIONS.md D-068). Read as a bare inequality, "" != the
        // current hash would hold for ANY project — the current hash
        // is never empty, it is sixty-four hex characters (D-064) —
        // making the first clause redundant and this project (active,
        // assignments only to hidden instructions per DB-013, never
        // synced, an empty build) dirty with nothing to write:
        assert!(!is_dirty(Active, None, HASH_ONE, "", true));
        assert!(!is_dirty(Active, Some(""), HASH_ONE, "", true));
    }

    #[test]
    fn the_no_assignments_exception_shields_even_a_non_empty_build() {
        // DB-021's second sentence is absolute — "is not counted as
        // dirty", the same wording as DB-014's archived rule — so a
        // never-synced project with no assignments stays clean even
        // when the build text is not empty (extra-file reading lines
        // are the one way such a project builds anything: ALG-030
        // joins them without a single assignment). The first clause
        // would fire; the exception wins:
        assert!(!is_dirty(Active, None, HASH_ONE, BUILD, false));
        assert!(!is_dirty(Active, Some(""), HASH_ONE, BUILD, false));
        // Once a sync HAS stored a hash the exception no longer
        // applies — "has never been synced" fails — and the second
        // clause decides:
        assert!(!is_dirty(Active, Some(HASH_ONE), HASH_ONE, BUILD, false));
        assert!(is_dirty(Active, Some(HASH_TWO), HASH_ONE, BUILD, false));
    }

    #[test]
    fn changing_the_scheme_or_the_file_name_makes_the_project_dirty() {
        // DB-023 needs no rule of its own here: the scheme and the
        // target path are build-hash inputs (D-064), so either change
        // moves the current hash and the difference clause fires.
        // Pinned through the real `build_hash`, with the stored value
        // from scheme A over AGENTS.md:
        let (local, project) = (
            ExtraFile {
                enabled: true,
                position: ExtraFilePosition::Start,
            },
            ExtraFile {
                enabled: false,
                position: ExtraFilePosition::End,
            },
        );
        let stored = build_hash(
            Scheme::A,
            Path::new("/repos/demo/AGENTS.md"),
            local,
            project,
            BUILD,
        );
        // Same configuration: clean.
        assert!(!is_dirty(
            Active,
            Some(&stored),
            &build_hash(
                Scheme::A,
                Path::new("/repos/demo/AGENTS.md"),
                local,
                project,
                BUILD
            ),
            BUILD,
            true,
        ));
        // The scheme changed (the old file keeps its name — the rename
        // consequence is the sync pipeline's, not this predicate's):
        assert!(is_dirty(
            Active,
            Some(&stored),
            &build_hash(
                Scheme::B,
                Path::new("/repos/demo/AGENTS.md"),
                local,
                project,
                BUILD
            ),
            BUILD,
            true,
        ));
        // The file name changed:
        assert!(is_dirty(
            Active,
            Some(&stored),
            &build_hash(
                Scheme::A,
                Path::new("/repos/demo/CLAUDE.md"),
                local,
                project,
                BUILD
            ),
            BUILD,
            true,
        ));
    }

    #[test]
    fn the_flag_follows_the_lifecycle_edit_then_sync_then_edit() {
        // OUT-013's story over real hashes: synced and clean — an edit
        // affecting the build moves the current hash and the project
        // turns dirty — the successful sync stores the new hash
        // (DB-024) and it is clean again — the next edit turns it
        // dirty again.
        let (local, project) = (
            ExtraFile {
                enabled: false,
                position: ExtraFilePosition::End,
            },
            ExtraFile {
                enabled: false,
                position: ExtraFilePosition::End,
            },
        );
        let path = Path::new("/repos/demo/AGENTS.md");
        let hash_of = |build: &str| build_hash(Scheme::A, path, local, project, build);

        let build_one = "## Style\n\nBe brief.";
        let build_two = "## Style\n\nBe very brief.";
        assert_ne!(hash_of(build_one), hash_of(build_two));

        // Synced over build_one: clean.
        assert!(!is_dirty(
            Active,
            Some(&hash_of(build_one)),
            &hash_of(build_one),
            build_one,
            true
        ));
        // An edit affects the build: dirty.
        assert!(is_dirty(
            Active,
            Some(&hash_of(build_one)),
            &hash_of(build_two),
            build_two,
            true
        ));
        // The sync runs, DB-024 stores the new hash: clean.
        assert!(!is_dirty(
            Active,
            Some(&hash_of(build_two)),
            &hash_of(build_two),
            build_two,
            true
        ));
        // The next edit: dirty again.
        assert!(is_dirty(
            Active,
            Some(&hash_of(build_two)),
            &hash_of(build_one),
            build_one,
            true
        ));
    }
}
