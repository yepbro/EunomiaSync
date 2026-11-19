//! New-project color assignment: the D-29 palette rule (spec DB-040,
//! DB-041, PRJ-010).
//!
//! D-29 (the spec's decision, quoted verbatim):
//!
//! > The least used palette color among active projects; on a tie,
//! > chosen by the hash of the project ID (deterministic)
//!
//! DB-041 routes PRJ-010 ("If no color is chosen, it is assigned by
//! rule D-29") through exactly that sentence, and PRJ-010's
//! acceptance criterion — "For 12 projects in a row, colors do not
//! repeat" — falls out of the rule's first half alone: twelve colors
//! exist (DB-040), a color with count 0 always beats a color with
//! count ≥ 1, so twelve consecutive adds from an empty library each
//! land on a still-unused color. The second half (the tie-break)
//! only starts deciding once every color is in use, where DB-041
//! already says a color is "not the only sign of a project" — the
//! label rides next to it and the name in a tooltip (NFR-020) — so
//! a repeat there is legal and expected.
//!
//! # The inputs, and whose job the active filter is
//!
//! [`pick_color`] is a pure function over two inputs: the colors
//! already held by ACTIVE projects (a slice the store layer fills
//! from `SELECT color FROM projects WHERE state = 'active'`, parsed
//! through the palette gate of D-075), and the new project's
//! identifier. "Among active projects" is D-29's own wording and
//! DB-014's spirit — an archived project "does not take part in
//! syncs" and counts for nothing here either — so the ACTIVE filter
//! lives in the query that builds the slice, one seam away: archive
//! a project and its color stops being passed in, which makes it
//! the unique count-0 candidate for the next add (a restore re-adds
//! the color to the slice). The `projects.color` column is
//! `NOT NULL` with no CHECK (§6.2), every value a project can hold
//! parses to a [`Color`] through `validate_color` (D-075's gate),
//! and a stored row that somehow does not is the store layer's parse
//! failure — it never reaches this function, whose input type is
//! already the closed twelve-member palette.
//!
//! # The tie-break: sha256, folded to an index over palette order
//!
//! D-29 names "the hash of the project ID" and nothing more — not
//! the function, not the fold, not the mapping onto the tied
//! candidates. The missing thirds are fixed here and recorded in
//! DECISIONS.md D-077:
//!
//! 1. _The hash._ SHA-256 over the identifier's UTF-8 bytes — the
//!    `sha2` crate already pinned in core's allow-list (D-017) and
//!    already used by the build hash (D-064), so no dependency
//!    moves, and a cryptographic hash makes the fold's low bits as
//!    unmotivated-by-project-name as a local app can cheaply get.
//! 2. _The fold._ The digest's FIRST 8 bytes, big-endian, as one
//!    `u64` scalar. Eight bytes are far more entropy than a choice
//!    among at most twelve candidates needs; big-endian is fixed
//!    for cross-build stability, the same "write it down once" spirit
//!    as D-064's little-endian length framing (either order works;
//!    only pinning one does).
//! 3. _The mapping._ The tied candidates are taken in DB-040's
//!    palette order — [`Color::ALL`]'s order, `red` … `slate` — and
//!    the scalar indexes them by remainder:
//!    `scalar % candidate_count`. Remainder over palette order is
//!    deterministic by construction, depends on every bit of the
//!    identifier, and reshuffles completely when the candidate set
//!    shrinks or grows (a remainder's meaning is set-size-relative —
//!    two identifiers that tie-break identically over eleven
//!    candidates need not over ten; D-29 asks for determinism per
//!    decision point, which is exactly what a pure function gives).
//!
//! # Examples
//!
//! The PRJ-010 criterion, twelve adds in a row from an empty
//! library (each pick joins the occupied slice for the next):
//!
//! ```
//! use eunomia_core::color_pick::pick_color;
//! use eunomia_core::project::{Color, ProjectId};
//!
//! let mut occupied: Vec<Color> = Vec::new();
//! for n in 0..12 {
//!     let id = ProjectId::new(format!("0193f1a0-0000-7000-8000-{n:012x}"));
//!     let color = pick_color(&occupied, &id);
//!     assert!(!occupied.contains(&color)); // still unused
//!     occupied.push(color);
//! }
//! assert_eq!(occupied.len(), 12); // twelve adds, twelve distinct colors
//! ```
//!
//! And once the palette is full, the tie-break decides — the same
//! identifier picks the same color every time:
//!
//! ```
//! use eunomia_core::color_pick::pick_color;
//! use eunomia_core::project::{Color, ProjectId};
//!
//! let id = ProjectId::new("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d".to_owned());
//! let full: Vec<Color> = Color::ALL.to_vec();
//! assert_eq!(pick_color(&full, &id), pick_color(&full, &id));
//! ```

use sha2::{Digest, Sha256};

use crate::project::{Color, ProjectId};

/// Picks the color for a new project by D-29 (DB-041, PRJ-010).
///
/// # Caller contract
///
/// `occupied` is the multiset of colors held by ACTIVE projects —
/// one entry per active project, in any order, repetitions
/// included; archived projects contribute nothing (DB-014, and see
/// the module docs for why the filter lives in the caller's
/// query). `new_project_id` identifies the project the color is
/// for; its UTF-8 bytes are the tie-break hash's only input.
///
/// The rule, verbatim from D-29: the returned color is a least-used
/// palette color among the entries of `occupied`, and when several
/// colors share that least count, the identifier's SHA-256 decides
/// which of them (first 8 digest bytes big-endian as a `u64`, taken
/// modulo the candidate count, indexing the candidates in DB-040
/// palette order — DECISIONS.md D-077). An empty `occupied` makes
/// all twelve colors candidates at count 0, which is how the
/// first-ever project gets its color.
///
/// Deterministic by construction: same slice (as a multiset), same
/// identifier — same color, on every machine and every run.
///
/// # Examples
///
/// ```
/// use eunomia_core::color_pick::pick_color;
/// use eunomia_core::project::{Color, ProjectId};
///
/// let id = ProjectId::new("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d".to_owned());
///
/// // A still-unused color always wins, whatever the hash would do:
/// let occupied = [Color::Red, Color::Red, Color::Blue];
/// assert!(!occupied.contains(&pick_color(&occupied, &id)));
///
/// // And with one color freed, it is the only count-0 candidate:
/// let mut full: Vec<Color> = Color::ALL.to_vec();
/// let freed = full.pop().unwrap();
/// assert_eq!(pick_color(&full, &id), freed);
/// ```
#[must_use]
pub fn pick_color(occupied: &[Color], new_project_id: &ProjectId) -> Color {
    // The palette is single-sourced from `Color::ALL` (T-049/D-075):
    // literals, order AND length — the count array is sized by the
    // palette itself, so a hypothetical thirteenth member flows
    // through instead of panicking an out-of-date local literal.
    let mut counts = [0usize; Color::ALL.len()];
    for &color in occupied {
        let index = Color::ALL
            .iter()
            .position(|&palette| palette == color)
            .expect("every Color is a palette member");
        counts[index] += 1;
    }

    let least = counts
        .iter()
        .copied()
        .min()
        .expect("the palette is never empty");

    let candidates: Vec<Color> = Color::ALL
        .iter()
        .zip(counts)
        .filter(|&(_, count)| count == least)
        .map(|(&color, _)| color)
        .collect();

    candidates[tie_break_index(new_project_id, candidates.len())]
}

/// Folds the identifier into an index over `candidate_count`
/// candidates: SHA-256 of the ID's UTF-8 bytes, first 8 digest
/// bytes big-endian as a `u64`, modulo the candidate count (the
/// D-077 fixing of D-29's unnamed hash and mapping).
///
/// The count is never zero — the candidate list is the palette's
/// least-used slice and the palette has twelve members — and the
/// remainder of a `u64` by it always fits `usize` on every target
/// this app builds for (D-26).
fn tie_break_index(new_project_id: &ProjectId, candidate_count: usize) -> usize {
    let digest = Sha256::digest(new_project_id.as_str().as_bytes());
    let scalar = u64::from_be_bytes(digest[..8].try_into().expect("eight digest bytes"));
    (scalar % candidate_count as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::pick_color;

    use crate::project::{Color, ProjectId};

    /// Twelve plausible UUIDv7-shaped identifiers — the store's
    /// `new_id` shape — one per consecutive add in the PRJ-010
    /// scenario.
    fn consecutive_ids() -> Vec<ProjectId> {
        (0..12)
            .map(|n| ProjectId::new(format!("0193f1a0-6d6d-7000-8000-{n:012x}")))
            .collect()
    }

    /// The D-29 first half, asserted structurally: the pick's usage
    /// count in `occupied` is the palette's minimum.
    fn assert_least_used(occupied: &[Color], picked: Color) {
        let count = |probe: Color| occupied.iter().filter(|&&c| c == probe).count();
        let least = Color::ALL
            .iter()
            .map(|&c| count(c))
            .min()
            .expect("palette is never empty");
        assert_eq!(count(picked), least);
    }

    #[test]
    fn twelve_consecutive_adds_get_twelve_distinct_colors() {
        // PRJ-010's acceptance criterion, verbatim scenario: twelve
        // projects in a row from an empty library, colors never
        // repeat. Each pick joins the occupied slice for the next.
        let mut occupied: Vec<Color> = Vec::new();
        for id in consecutive_ids() {
            let color = pick_color(&occupied, &id);
            assert!(
                !occupied.contains(&color),
                "{color:?} repeated after {occupied:?}"
            );
            assert_least_used(&occupied, color);
            occupied.push(color);
        }
        let mut distinct = occupied.clone();
        distinct.sort_by_key(|color| Color::ALL.iter().position(|c| c == color));
        assert_eq!(distinct, Color::ALL.to_vec());
    }

    #[test]
    fn same_inputs_pick_the_same_color() {
        // Determinism, the D-29 parenthetical: one input pair, many
        // calls, one answer; and the slice is a multiset — order
        // must not matter.
        let ids = consecutive_ids();
        let occupied: Vec<Color> = Color::ALL.to_vec();
        for id in &ids {
            let first = pick_color(&occupied, id);
            assert_eq!(first, pick_color(&occupied, id));
            assert_eq!(first, pick_color(&occupied, id));
        }

        let mut reversed = occupied.clone();
        reversed.reverse();
        for id in &ids {
            assert_eq!(
                pick_color(&occupied, id),
                pick_color(&reversed, id),
                "order changed the multiset's answer for {id}"
            );
        }
    }

    #[test]
    fn a_still_unused_color_always_wins() {
        // However loaded the palette, count 0 beats count ≥ 1 — the
        // half of D-29 that alone carries PRJ-010. Eleven colors in
        // heavy use (red five times, the rest three), slate never
        // used: the unique count-0 candidate wins for every
        // identifier, whatever the hash would do.
        let ids = consecutive_ids();
        let heavy: Vec<Color> = Color::ALL
            .iter()
            .copied()
            .filter(|&c| c != Color::Slate)
            .flat_map(|c| [c, c, c])
            .chain([Color::Red, Color::Red])
            .collect();
        for id in &ids {
            assert_eq!(pick_color(&heavy, id), Color::Slate);
        }

        // Three colors unused, the rest used twice: the pick is
        // always one of the three — which one is the hash's
        // business, but a used color never wins.
        let unused = [Color::Amber, Color::Teal, Color::Slate];
        let used: Vec<Color> = Color::ALL
            .iter()
            .copied()
            .filter(|c| !unused.contains(c))
            .flat_map(|c| [c, c])
            .collect();
        assert_eq!(used.len(), 18); // nine colors, twice each
        for id in &ids {
            let picked = pick_color(&used, id);
            assert!(unused.contains(&picked), "used color {picked:?} picked");
            assert_eq!(picked, pick_color(&used, id));
        }
    }

    #[test]
    fn the_shared_minimum_outranks_every_higher_count() {
        // Counts: red 5, everything else 3 — the minimum (3) is
        // shared by eleven colors, and the pick must come from that
        // minimum even though red's count is unique at the top:
        // "least used" is a count comparison, not a uniqueness one.
        let occupied: Vec<Color> = Color::ALL
            .iter()
            .copied()
            .flat_map(|c| [c, c, c])
            .chain([Color::Red, Color::Red])
            .collect();
        assert_eq!(occupied.len(), 38); // 12×3 + 2

        let ids = consecutive_ids();
        for id in &ids {
            let picked = pick_color(&occupied, id);
            assert_least_used(&occupied, picked);
            assert_ne!(picked, Color::Red, "count-5 color picked");
        }
    }

    #[test]
    fn the_fips_vectors_pin_the_whole_tie_break_chain() {
        // External oracle, both vectors published by FIPS 180-4 (not
        // produced by this crate's code path):
        //
        //  sha256("abc")     = ba7816bf8f01cfea…f20015ad
        //  sha256("abcdbcde…nopnopq")            (Appendix B.2,
        //  the two-block message)
        //                   = 248d6a61d20638b8…19db06c1
        //
        // Each digest's first 8 bytes, big-endian, are one u64
        // (independent big-int arithmetic):
        //
        //  abc  → 0xba7816bf8f01cfea = 13_436_514_500_253_700_074;
        //  %12 = 6 (cyan), %11 = 10, %2 = 0
        //  b.2  → 0x248d6a61d20638b8 =  2_633_878_325_449_603_256;
        //  %12 = 8 (indigo), %11 = 2
        //
        // TWO vectors, because one vector's remainders are also
        // reproduced by degenerate folds that must fail: the FIRST
        // BYTE alone of the b.2 digest (0x24 = 36) gives 36 % 12 = 0
        // ≠ 8 (kills a one-byte fold), and any per-candidate-set
        // CONSTANT scalar cannot match both vectors at once (54, for
        // instance, reproduces every abc remainder but 54 % 12 = 6
        // ≠ 8 — kills an implementation that never reads the ID).
        // Together the two pin the hash function, the input (the ID
        // string), the byte order, the fold WIDTH, and the palette-
        // order indexing.
        const ABC_DIGEST_FIRST_8: u64 = 0xba78_16bf_8f01_cfea;
        const B2_DIGEST_FIRST_8: u64 = 0x248d_6a61_d206_38b8;
        assert_eq!(ABC_DIGEST_FIRST_8, 13_436_514_500_253_700_074);
        assert_eq!(B2_DIGEST_FIRST_8, 2_633_878_325_449_603_256);
        assert_eq!(ABC_DIGEST_FIRST_8 % 12, 6);
        assert_eq!(B2_DIGEST_FIRST_8 % 12, 8);

        let abc = ProjectId::new("abc".to_owned());
        let two_blocks =
            ProjectId::new("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq".to_owned());
        let empty: [Color; 0] = [];
        assert_eq!(pick_color(&empty, &abc), Color::ALL[6]);
        assert_eq!(pick_color(&empty, &two_blocks), Color::ALL[8]);

        // The state after "abc" itself took cyan: one project holds
        // cyan, so the eleven UNHELD colors are the count-0
        // candidates in palette order, and the same published scalar
        // re-folds over the smaller set (11 → 10 → slate). (The
        // inversion — passing the eleven held colors — would make
        // cyan the unique count-0 candidate and the hash idle: the
        // minimum rule dominates, which the freed-color tests pin.)
        assert_eq!(ABC_DIGEST_FIRST_8 % 11, 10);
        let after_abc_took_cyan = [Color::ALL[6]];
        let candidates: Vec<Color> = Color::ALL
            .iter()
            .copied()
            .filter(|&c| c != Color::ALL[6])
            .collect();
        assert_eq!(pick_color(&after_abc_took_cyan, &abc), candidates[10]);

        // And over exactly two candidates the same scalar folds to
        // the even side (2 → 0): red and blue held once, everything
        // else twice, so [red, blue] in palette order are the tied
        // minimum — "abc" takes red, stably.
        assert_eq!(ABC_DIGEST_FIRST_8 % 2, 0);
        let two_way: Vec<Color> = [Color::Red, Color::Blue]
            .into_iter()
            .chain(
                Color::ALL
                    .iter()
                    .copied()
                    .filter(|&c| c != Color::Red && c != Color::Blue)
                    .flat_map(|c| [c, c]),
            )
            .collect();
        assert_eq!(pick_color(&two_way, &abc), Color::Red);
    }

    #[test]
    fn different_identifiers_do_not_all_collapse_onto_one_color() {
        // The complement the two FIPS vectors cannot give alone:
        // across MANY identifiers over the same fully-tied palette,
        // the picks must spread — a constant scalar (an
        // implementation that ignores the ID) collapses them all
        // onto one color and cannot pass this. Spread is the claim,
        // not any particular distribution: twelve identifiers need
        // not hit twelve colors (ties at count 1 make repeats legal,
        // DB-041), only more than one.
        use std::collections::HashSet;

        let full: Vec<Color> = Color::ALL.to_vec();
        let picks: HashSet<Color> = consecutive_ids()
            .iter()
            .map(|id| pick_color(&full, id))
            .collect();
        assert!(picks.len() > 1, "every identifier picked {picks:?}");
    }

    #[test]
    fn a_full_palette_ties_at_every_count_and_the_hash_decides() {
        // DB-041: color is not the only sign of a project, so a
        // repeat is legal — with all twelve occupied once, every
        // color ties at 1 and D-29's second half picks among all
        // twelve; deterministically, and still least-used.
        let full: Vec<Color> = Color::ALL.to_vec();
        for id in consecutive_ids() {
            let picked = pick_color(&full, &id);
            assert_least_used(&full, picked);
            assert_eq!(picked, pick_color(&full, &id));
        }

        // A two-way tie is enough to engage the tie-break: red and
        // blue at 1, everything else at 2 — the pick is always one
        // of the two, stably per identifier.
        let two_way: Vec<Color> = [Color::Red, Color::Blue]
            .into_iter()
            .chain(
                Color::ALL
                    .iter()
                    .copied()
                    .filter(|&c| c != Color::Red && c != Color::Blue)
                    .flat_map(|c| [c, c]),
            )
            .collect();
        for id in consecutive_ids() {
            let picked = pick_color(&two_way, &id);
            assert!(
                picked == Color::Red || picked == Color::Blue,
                "non-tied color {picked:?} picked"
            );
            assert_eq!(picked, pick_color(&two_way, &id));
        }
    }

    #[test]
    fn archiving_frees_the_color_the_next_add_reuses() {
        // DB-014's half of the contract, through the input seam: the
        // caller stops passing an archived project's color, and a
        // twelve-times-used palette with one color freed makes that
        // color the unique count-0 candidate. Restoring (the color
        // returning to the slice) removes it from the candidates
        // again — the same query answers both directions.
        let mut occupied: Vec<Color> = Color::ALL.to_vec();
        for id in consecutive_ids() {
            let freed = occupied.pop().expect("one color archived");
            assert_eq!(pick_color(&occupied, &id), freed);
            occupied.push(freed); // restored before the next try
        }
    }

    #[test]
    fn every_pick_is_a_palette_member_and_counts_survive_noise() {
        // Repetitions beyond twelve entries and interleavings that
        // double counts must not break the invariants — the input is
        // a multiset, not a set, and D-29 counts uses.
        let ids = consecutive_ids();
        let noisy: Vec<Color> = Color::ALL
            .to_vec()
            .into_iter()
            .chain([Color::Slate, Color::Red, Color::Slate])
            .collect();
        assert_eq!(noisy.len(), 15);
        for id in &ids {
            let picked = pick_color(&noisy, id);
            assert_least_used(&noisy, picked);
            assert!(Color::ALL.contains(&picked));
        }
    }
}
