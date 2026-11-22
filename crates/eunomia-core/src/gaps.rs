//! The LIB-032 heading-level skips: where the next heading arrives
//! more than one level deeper than the heading before it (spec §2,
//! §7.3; the §9.7 ALG-060 geometry the pairs hang on).
//!
//! LIB-032 (§7.3, the row it shares with LIB-034 and MAIN-018, 019)
//! states the rule and its status in one sentence:
//!
//! > Skipping a level (for example, `##` then `####`) is allowed and
//! > marked with a warning icon.
//!
//! The task card names "ALG-063" for this; §9.7 ends at ALG-062, and
//! no ALG-063 exists — the canon is the LIB-032 row above (recorded
//! in DECISIONS.md D-073). The dropped D-072 entry (removed from
//! DECISIONS.md by a later docs commit — D-151) settled the split this
//! module inherits: the skip is LEGAL (level skips are not
//! validation — nothing here rejects anything) and the icon is the
//! UI's marker; core's job is to supply the facts the icon needs.
//!
//! # Adjacent headings, not branch interiors
//!
//! The example is two heading markers — `##` then `####` — and a
//! heading marker is a thing only headings render (ALG-020 writes
//! heading lines, instructions write their variant texts), so the
//! pair the rule reads is **two consecutive headings**: the earlier
//! heading, then the next heading after it, with everything between
//! them — instructions of either state, hidden included — carrying
//! no level and so unable to interrupt the pair. An H2 followed by
//! instructions followed by an H4 is the same skip as an H2 directly
//! above an H4.
//!
//! The branch reading agrees on every pair the rule reports — and
//! only there. By ALG-060 ("The branch range of a level-L heading
//! goes from the heading itself to the item before the next heading
//! of level L or higher") a deeper heading never closes the branch
//! above it, so the earlier heading of an offending pair is the
//! later one's parent in the tree, and the skip is the parent edge
//! arriving more than one level down: the H4 hangs directly under
//! the H2 with the H3 level unused. The converse does NOT hold:
//! a heading that closes a deeper sub-branch and then hangs under
//! an ancestor across a level gap — the H4 in `H2, H5, H4` — is
//! NOT reported, because its immediate predecessor is deeper than
//! it (a close), while the parent-edge reading alone would flag
//! that second edge too. LIB-032's example is two adjacent markers,
//! so the adjacent-heading reading is the implemented one: the icon
//! marks the heading that ARRIVES too deep, not every descendant
//! of a skipped level. A report fires ONCE per offending edge,
//! however many levels it skips — H2 → H5 (two levels unused) is
//! one skip, not two.
//!
//! What is NOT a skip: a close or a sibling (`####` after `##`'s
//! subtree, `##` after `##` — level L or higher closes, ALG-060);
//! the first heading of the list (no heading before it, no pair);
//! the end of the list (nothing follows, no pair); a list of
//! instructions only, or an empty list (no headings at all).
//!
//! # The report
//!
//! [`heading_level_gaps`] walks the items once, purely, and returns
//! one [`HeadingLevelGap`] per offending pair: the two collection
//! indices and the two levels — the minimal fact a warning icon
//! needs (which row carries it, and which levels the tooltip
//! names). The icon's row is the LATER heading — the one that
//! arrives too deep; the earlier index is included so the parent
//! can be named. [`HeadingLevelGap::jump`] is the observed depth of
//! the arrival and [`HeadingLevelGap::expected_level`] the shallowest
//! legal next step (`##` then `####` skipped the `###` that
//! `expected_level` returns); anything further about the icon is
//! the UI's to decide. Following [`HeadingBranch`]'s precedent, the
//! report derives nothing but `Debug`/`Clone`/`PartialEq`/`Eq` —
//! no serde: mapping it over IPC is the command layer's task.
//!
//! [`HeadingBranch`]: crate::heading_range::HeadingBranch
//!
//! # Examples
//!
//! ```
//! use std::convert::TryFrom;
//!
//! use eunomia_core::gaps::heading_level_gaps;
//! use eunomia_core::library::{
//!     Heading, HeadingLevel, Instruction, InstructionState, ItemId, LibraryItem, Variant,
//!     VariantId,
//! };
//!
//! fn heading(id: &str, level: u8, title: &str) -> LibraryItem {
//!     LibraryItem::Heading(Heading {
//!         id: ItemId::new(id.to_owned()),
//!         level: HeadingLevel::try_from(level).unwrap(),
//!         title: title.to_owned(),
//!     })
//! }
//!
//! fn instruction(id: &str) -> LibraryItem {
//!     LibraryItem::Instruction(Instruction {
//!         id: ItemId::new(id.to_owned()),
//!         title: format!("Instruction {id}"),
//!         state: InstructionState::Active,
//!         variants: vec![Variant {
//!             id: VariantId::new(format!("v-{id}")),
//!             name: "Main".to_owned(),
//!             content: "Be brief.".to_owned(),
//!         }],
//!     })
//! }
//!
//! // The LIB-032 example itself: `##` then `####`, with instructions
//! // between that cannot interrupt the pair.
//! let items = vec![
//!     heading("style", 2, "Style"), // 0: `##`…
//!     instruction("concise"),       // 1: …no level of its own…
//!     heading("tone", 4, "Tone"),   // 2: …then `####`
//!     instruction("friendly"),      // 3
//! ];
//! let gaps = heading_level_gaps(&items);
//! assert_eq!(gaps.len(), 1);
//! assert_eq!(gaps[0].from_index, 0);
//! assert_eq!(gaps[0].to_index, 2);
//! assert_eq!(gaps[0].from_level, HeadingLevel::H2);
//! assert_eq!(gaps[0].to_level, HeadingLevel::H4);
//! assert_eq!(gaps[0].jump(), 2); // one level skipped…
//! assert_eq!(gaps[0].expected_level(), HeadingLevel::H3); // …this one
//!
//! // A close is not a skip, and neither is a sibling.
//! let dense = vec![
//!     heading("style", 2, "Style"), // 0
//!     heading("tone", 3, "Tone"),   // 1: one step deeper…
//!     heading("done", 2, "Done"),   // 2: …a close…
//!     heading("more", 2, "More"),   // 3: …and a sibling
//! ];
//! assert!(heading_level_gaps(&dense).is_empty());
//! ```

use crate::library::{HeadingLevel, LibraryItem};

/// One heading-level skip: the fact a LIB-032 warning icon marks.
///
/// The pair is two consecutive headings — no heading sits strictly
/// between [`from_index`](Self::from_index) and
/// [`to_index`](Self::to_index), because instructions carry no level
/// and cannot interrupt it. `to_level` is more than one level deeper
/// than `from_level`; everything else (a close, a sibling, a single
/// step down) is not a skip and produces no report.
///
/// The icon's row is the later heading (`to_index` — the one that
/// arrives too deep); `from_index` names its parent under the ALG-060
/// geometry so a tooltip can cite both. What the icon looks like is
/// the UI's to decide (the dropped D-072 ruling, D-151: the icon is
/// a marker, not a
/// validation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingLevelGap {
    /// The earlier heading of the pair — the parent (a deeper
    /// heading never closes the branch above it, ALG-060).
    pub from_index: usize,
    /// The later heading of the pair: the row the warning icon
    /// marks, the heading that arrives more than one level deeper.
    pub to_index: usize,
    /// The earlier heading's level.
    pub from_level: HeadingLevel,
    /// The later heading's level.
    pub to_level: HeadingLevel,
}

impl HeadingLevelGap {
    /// The observed jump: how many levels the later heading arrives
    /// below the earlier one. A skip means at least `2` (one level
    /// skipped); H2 → H5 jumps `3` with two levels skipped. The count
    /// of skipped levels is therefore `jump() - 1`.
    #[must_use]
    pub fn jump(&self) -> u8 {
        // saturating_sub keeps the helper total for a hand-built
        // upside-down pair; real reports always have to > from.
        self.to_level
            .as_u8()
            .saturating_sub(self.from_level.as_u8())
    }

    /// The shallowest legal next step after the earlier heading: the
    /// first level the pair skipped. `##` then `####` skipped the
    /// `###` this returns. For a real report `from_level` is at most
    /// an H4 (the arrival is deeper still and within 2–6), so the
    /// answer is always an H3…H5; a hand-built pair rooted at an H6
    /// saturates at H6, keeping the helper total the way [`jump`]
    /// is.
    ///
    /// [`jump`]: Self::jump
    #[must_use]
    pub fn expected_level(&self) -> HeadingLevel {
        let value = self.from_level.as_u8() + 1;
        // +1 leaves the closed 2–6 set only for a hand-built pair
        // rooted at an H6; saturating there keeps the helper total.
        // A real report's from_level is at most an H4.
        HeadingLevel::try_from(value).unwrap_or(HeadingLevel::H6)
    }
}

/// Find every heading-level skip (LIB-032): each place where the next
/// heading arrives more than one level deeper than the heading before
/// it — `##` then `####`, with or without instructions between them.
///
/// The walk reads consecutive headings only: instructions of either
/// state are transparent (they render no heading line, ALG-020, and
/// they never close a branch, ALG-060). A report fires once per
/// offending pair; closes, siblings and single steps down are not
/// skips, the first heading of the list has no pair, and nothing
/// follows the last one. Skips are legal input — this function
/// REJECTS nothing; it is the fact supplier for the LIB-032 warning
/// icon, which is UI, not validation (the dropped D-072 ruling,
/// D-151).
///
/// Pure: the input slice is never touched, and the answer is a fresh
/// `Vec` in collection order — gaps are reported where they sit, the
/// earlier pair first. The function cannot fail; an empty or
/// heading-less input simply has no pairs to read.
///
/// # Examples
///
/// ```
/// use std::convert::TryFrom;
///
/// use eunomia_core::gaps::heading_level_gaps;
/// use eunomia_core::library::{Heading, HeadingLevel, ItemId, LibraryItem};
///
/// fn heading(id: &str, level: u8, title: &str) -> LibraryItem {
///     LibraryItem::Heading(Heading {
///         id: ItemId::new(id.to_owned()),
///         level: HeadingLevel::try_from(level).unwrap(),
///         title: title.to_owned(),
///     })
/// }
///
/// // Several skips in one list, and the pairs that are not skips
/// // between them.
/// let items = vec![
///     heading("a", 2, "A"), // 0 — H2
///     heading("b", 4, "B"), // 1 — H4: 0 → 1 is a skip (one level)
///     heading("c", 5, "C"), // 2 — H5: 1 → 2 is one step deeper, not a skip
///     heading("d", 2, "D"), // 3 — H2: 2 → 3 is a close
///     heading("e", 6, "E"), // 4 — H6: 3 → 4 is a skip (three levels)
/// ];
/// let gaps = heading_level_gaps(&items);
/// assert_eq!(gaps.len(), 2);
/// assert_eq!((gaps[0].from_index, gaps[0].to_index), (0, 1));
/// assert_eq!(gaps[0].jump(), 2);
/// assert_eq!((gaps[1].from_index, gaps[1].to_index), (3, 4));
/// assert_eq!(gaps[1].jump(), 4); // H2 → H6: three levels skipped
///
/// // No headings, no pairs.
/// assert!(heading_level_gaps(&[]).is_empty());
/// ```
#[must_use]
pub fn heading_level_gaps(items: &[LibraryItem]) -> Vec<HeadingLevelGap> {
    let mut gaps = Vec::new();
    // The heading before the current one — the only state the rule
    // needs. Instructions never update it: they are not headings and
    // cannot be half of the pair.
    let mut previous: Option<(usize, HeadingLevel)> = None;
    for (index, item) in items.iter().enumerate() {
        if let LibraryItem::Heading(heading) = item {
            if let Some((from_index, from_level)) = previous
                && heading.level.as_u8() > from_level.as_u8() + 1
            {
                gaps.push(HeadingLevelGap {
                    from_index,
                    to_index: index,
                    from_level,
                    to_level: heading.level,
                });
            }
            previous = Some((index, heading.level));
        }
    }
    gaps
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use super::*;
    use crate::library::{
        Heading, HeadingLevel, Instruction, InstructionState, ItemId, LibraryItem, Variant,
        VariantId,
    };

    // ---- fixtures ------------------------------------------------------

    fn heading(id: &str, level: u8, title: &str) -> LibraryItem {
        let level = HeadingLevel::try_from(level)
            .unwrap_or_else(|error| panic!("invalid level {level}: {error}"));
        LibraryItem::Heading(Heading {
            id: ItemId::new(id.to_owned()),
            level,
            title: title.to_owned(),
        })
    }

    fn instruction_with_state(id: &str, state: InstructionState) -> LibraryItem {
        LibraryItem::Instruction(Instruction {
            id: ItemId::new(id.to_owned()),
            title: format!("Instruction title {id}"),
            state,
            variants: vec![Variant {
                id: VariantId::new(format!("v-{id}")),
                name: format!("Variant name {id}"),
                content: format!("Content of {id}"),
            }],
        })
    }

    fn instruction(id: &str) -> LibraryItem {
        instruction_with_state(id, InstructionState::Active)
    }

    fn hidden(id: &str) -> LibraryItem {
        instruction_with_state(id, InstructionState::Hidden)
    }

    // ---- dense trees: no skips ------------------------------------------

    #[test]
    fn a_dense_tree_has_no_gaps() {
        // Every step is one level down, a sibling or a close.
        let items = vec![
            heading("alpha", 2, "Alpha"),   // 0
            instruction("a-one"),           // 1
            heading("sub", 3, "Sub"),       // 2
            instruction("s-one"),           // 3
            heading("deep", 4, "Deep"),     // 4
            hidden("d-one"),                // 5
            heading("deeper", 5, "Deeper"), // 6
            heading("back", 3, "Back"),     // 7: a close…
            heading("sibling", 3, "Sib"),   // 8: …and a sibling
            heading("beta", 2, "Beta"),     // 9
            instruction("b-one"),           // 10
        ];
        assert!(heading_level_gaps(&items).is_empty());
    }

    #[test]
    fn every_single_step_down_is_dense() {
        // The full ladder H2→H3→H4→H5→H6, one level at a time.
        let items: Vec<LibraryItem> = (2..=6u8)
            .map(|level| heading(&format!("h{level}"), level, "Step"))
            .collect();
        assert!(heading_level_gaps(&items).is_empty());
    }

    // ---- the skips themselves -------------------------------------------

    #[test]
    fn a_single_skip_h2_to_h4_reports_the_pair_and_the_levels() {
        // The LIB-032 example: `##` then `####`.
        let items = vec![
            heading("style", 2, "Style"), // 0
            heading("tone", 4, "Tone"),   // 1
        ];
        let gaps = heading_level_gaps(&items);
        assert_eq!(gaps.len(), 1);
        assert_eq!(
            gaps[0],
            HeadingLevelGap {
                from_index: 0,
                to_index: 1,
                from_level: HeadingLevel::H2,
                to_level: HeadingLevel::H4,
            }
        );
    }

    #[test]
    fn a_two_level_skip_h2_to_h5_is_one_report_not_two() {
        let items = vec![
            heading("a", 2, "A"), // 0
            heading("b", 5, "B"), // 1: two levels skipped at once
        ];
        let gaps = heading_level_gaps(&items);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].from_level, HeadingLevel::H2);
        assert_eq!(gaps[0].to_level, HeadingLevel::H5);
        assert_eq!(gaps[0].jump(), 3);
        assert_eq!(gaps[0].expected_level(), HeadingLevel::H3);
    }

    #[test]
    fn h2_to_h6_after_an_empty_branch_is_still_a_skip() {
        // An empty branch — no items between the headings at all —
        // changes nothing: the pair is the two headings themselves.
        let items = vec![
            heading("alpha", 2, "Alpha"), // 0
            heading("omega", 6, "Omega"), // 1: the whole H3–H5 unused
        ];
        let gaps = heading_level_gaps(&items);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].from_index, 0);
        assert_eq!(gaps[0].to_index, 1);
        assert_eq!(gaps[0].jump(), 4);
        assert_eq!(gaps[0].expected_level(), HeadingLevel::H3);
    }

    #[test]
    fn several_skips_in_one_list_are_reported_in_collection_order() {
        let items = vec![
            heading("a", 2, "A"), // 0
            heading("b", 4, "B"), // 1: skip one level
            heading("c", 5, "C"), // 2: one step deeper — not a skip
            heading("d", 2, "D"), // 3: a close
            heading("e", 3, "E"), // 4: one step down — not a skip
            heading("f", 6, "F"), // 5: skip two levels
            heading("g", 4, "G"), // 6: a close
            heading("h", 6, "H"), // 7: skip one level
        ];
        let gaps = heading_level_gaps(&items);
        let reported: Vec<(usize, usize)> = gaps
            .iter()
            .map(|gap| (gap.from_index, gap.to_index))
            .collect();
        assert_eq!(reported, [(0, 1), (4, 5), (6, 7)]);
        assert_eq!(gaps[1].from_level, HeadingLevel::H3);
        assert_eq!(gaps[1].to_level, HeadingLevel::H6);
    }

    #[test]
    fn a_skip_deeper_in_the_tree_reports_its_real_parent() {
        // The skip hangs under an H3 inside an H2 branch: the pair is
        // the H3 and the H5, not the branch root.
        let items = vec![
            heading("alpha", 2, "Alpha"), // 0
            heading("sub", 3, "Sub"),     // 1
            instruction("one"),           // 2
            heading("deep", 5, "Deep"),   // 3: skips the H4
            heading("beta", 2, "Beta"),   // 4
        ];
        let gaps = heading_level_gaps(&items);
        assert_eq!(gaps.len(), 1);
        assert_eq!(
            gaps[0],
            HeadingLevelGap {
                from_index: 1,
                to_index: 3,
                from_level: HeadingLevel::H3,
                to_level: HeadingLevel::H5,
            }
        );
    }

    // ---- instructions are transparent ------------------------------------

    #[test]
    fn instructions_between_the_headings_do_not_interrupt_the_pair() {
        // Active and hidden instructions alike carry no level: the
        // pair is still the H2 and the H4, with the real indices.
        let items = vec![
            heading("style", 2, "Style"), // 0
            instruction("concise"),       // 1
            hidden("notes"),              // 2: hidden, still no level
            instruction("terse"),         // 3
            heading("tone", 4, "Tone"),   // 4
        ];
        let gaps = heading_level_gaps(&items);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].from_index, 0);
        assert_eq!(gaps[0].to_index, 4);
    }

    #[test]
    fn instructions_cannot_create_or_break_a_pair() {
        // The same heading ladder, bare and interleaved with hidden
        // and active instructions: the pairs — and so the reports —
        // are the same facts, only the indices shift.
        let bare = vec![
            heading("a", 2, "A"), // 0
            heading("b", 4, "B"), // 1
            heading("c", 2, "C"), // 2
        ];
        let interleaved = vec![
            instruction("lead"),  // 0: before any heading
            heading("a", 2, "A"), // 1
            hidden("between"),    // 2
            instruction("more"),  // 3
            heading("b", 4, "B"), // 4
            instruction("tail"),  // 5
            heading("c", 2, "C"), // 6
            instruction("last"),  // 7
        ];
        let bare_gaps = heading_level_gaps(&bare);
        let shifted_gaps = heading_level_gaps(&interleaved);
        assert_eq!(bare_gaps.len(), 1);
        assert_eq!(shifted_gaps.len(), 1);
        assert_eq!(shifted_gaps[0].from_index, bare_gaps[0].from_index + 1);
        assert_eq!(shifted_gaps[0].to_index, bare_gaps[0].to_index + 3);
        assert_eq!(shifted_gaps[0].from_level, bare_gaps[0].from_level);
        assert_eq!(shifted_gaps[0].to_level, bare_gaps[0].to_level);
    }

    // ---- what is not a skip ----------------------------------------------

    #[test]
    fn a_closing_jump_up_is_not_a_skip() {
        // H4 → H2 (and H6 → H3, and an equal-level sibling) close or
        // continue a branch — the rule reads arrivals deeper than one
        // step only.
        let items = vec![
            heading("a", 4, "A"), // 0
            heading("b", 2, "B"), // 1: a close by two levels
            heading("c", 6, "C"), // 2: a skip — reported…
            heading("d", 3, "D"), // 3: …then a close by three
            heading("e", 3, "E"), // 4: a sibling, level equal
            heading("f", 4, "F"), // 5: one step down
            heading("g", 4, "G"), // 6: a sibling again
            heading("h", 2, "H"), // 7: the last close
        ];
        let gaps = heading_level_gaps(&items);
        let reported: Vec<(usize, usize)> = gaps
            .iter()
            .map(|gap| (gap.from_index, gap.to_index))
            .collect();
        assert_eq!(reported, [(1, 2)]); // only the H2 → H6 arrival
    }

    #[test]
    fn the_end_of_the_list_creates_no_report() {
        // Nothing follows the last heading, so it cannot be half of a
        // pair — even when it is the deepest level there is.
        let items = vec![
            heading("a", 2, "A"), // 0
            heading("b", 3, "B"), // 1
            heading("c", 4, "C"), // 2: the last item, dense
        ];
        assert!(heading_level_gaps(&items).is_empty());

        // And a list ENDING in a skip reports that skip alone — no
        // extra report for the ending itself.
        let ending = vec![
            heading("a", 2, "A"), // 0
            instruction("one"),   // 1
            heading("b", 5, "B"), // 2: a skip, and the last item
        ];
        let gaps = heading_level_gaps(&ending);
        assert_eq!(gaps.len(), 1);
        assert_eq!((gaps[0].from_index, gaps[0].to_index), (0, 2));
    }

    #[test]
    fn the_first_heading_is_never_a_skip_even_when_deep() {
        // No heading before it, no pair — an H6 opening the list is a
        // legal shape (levels 2–6, §2), not a skip.
        let items = vec![
            heading("top", 6, "Top"),   // 0: first, and deepest
            heading("next", 2, "Next"), // 1: a close
        ];
        assert!(heading_level_gaps(&items).is_empty());
    }

    #[test]
    fn a_single_heading_has_no_pair() {
        let items = vec![heading("only", 5, "Only")];
        assert!(heading_level_gaps(&items).is_empty());
    }

    #[test]
    fn only_instructions_or_nothing_have_no_gaps() {
        let instructions = vec![instruction("one"), hidden("two")];
        assert!(heading_level_gaps(&instructions).is_empty());
        assert!(heading_level_gaps(&[]).is_empty());
    }

    #[test]
    fn a_heading_closing_a_deeper_branch_is_not_re_reported() {
        // The shape where the adjacent-heading reading and the bare
        // parent-edge reading diverge (review R1, P2-2): in
        // `H2, H5, H4` the closing H4 hangs under the H2 across a
        // level gap, but its immediate predecessor is the deeper H5
        // — a close, not an arrival. LIB-032's example is two
        // adjacent markers: only the H5, the heading that ARRIVES
        // too deep, carries the report.
        let items = vec![
            heading("a", 2, "A"), // 0
            heading("b", 5, "B"), // 1: the arrival — reported…
            heading("c", 4, "C"), // 2: …a close under a gap — not
        ];
        let gaps = heading_level_gaps(&items);
        let reported: Vec<(usize, usize)> = gaps
            .iter()
            .map(|gap| (gap.from_index, gap.to_index))
            .collect();
        assert_eq!(reported, [(0, 1)]);
        // The same pin with siblings: the second H4's predecessor is
        // the first H4 — equal level, a sibling — so it is not
        // reported either, even though it too hangs under the H2.
        let siblings = vec![
            heading("a", 2, "A"), // 0
            heading("b", 4, "B"), // 1: the arrival — reported…
            heading("c", 4, "C"), // 2: …a sibling under a gap — not
        ];
        let reported: Vec<(usize, usize)> = heading_level_gaps(&siblings)
            .iter()
            .map(|gap| (gap.from_index, gap.to_index))
            .collect();
        assert_eq!(reported, [(0, 1)]);
    }

    #[test]
    fn expected_level_is_total_for_a_hand_built_degenerate_pair() {
        // A real report's from_level is at most an H4 (so the answer
        // is an H3…H5), but the struct is public: a hand-built pair
        // rooted at an H6 saturates instead of panicking, the way
        // `jump` saturates for an upside-down pair.
        let degenerate = HeadingLevelGap {
            from_index: 0,
            to_index: 1,
            from_level: HeadingLevel::H6,
            to_level: HeadingLevel::H2,
        };
        assert_eq!(degenerate.jump(), 0);
        assert_eq!(degenerate.expected_level(), HeadingLevel::H6);
    }

    // ---- the DTO ----------------------------------------------------------

    #[test]
    fn the_reported_indices_address_the_reported_items() {
        let items = vec![
            instruction("lead"),  // 0
            heading("a", 2, "A"), // 1
            hidden("mid"),        // 2
            heading("b", 5, "B"), // 3
            heading("c", 2, "C"), // 4
            heading("d", 4, "D"), // 5
            instruction("tail"),  // 6
        ];
        for gap in heading_level_gaps(&items) {
            let LibraryItem::Heading(from) = &items[gap.from_index] else {
                panic!("from_index must address a heading");
            };
            let LibraryItem::Heading(to) = &items[gap.to_index] else {
                panic!("to_index must address a heading");
            };
            assert!(gap.to_index > gap.from_index);
            assert_eq!(from.level, gap.from_level);
            assert_eq!(to.level, gap.to_level);
            // Consecutive headings: no heading strictly between them.
            assert!(
                !items[gap.from_index + 1..gap.to_index]
                    .iter()
                    .any(|item| matches!(item, LibraryItem::Heading(_)))
            );
            // The skip itself: deeper than one step, expected level
            // the first unused one.
            assert!(gap.jump() >= 2);
            assert_eq!(u8::from(gap.expected_level()), u8::from(gap.from_level) + 1);
        }
        // Two skips at (1, 3) and (4, 5) — the walk above proves both.
        let reported: Vec<(usize, usize)> = heading_level_gaps(&items)
            .iter()
            .map(|gap| (gap.from_index, gap.to_index))
            .collect();
        assert_eq!(reported, [(1, 3), (4, 5)]);
    }

    #[test]
    fn the_derived_facts_read_off_the_levels() {
        let items = vec![heading("a", 3, "A"), heading("b", 6, "B")];
        let gap = &heading_level_gaps(&items)[0];
        assert_eq!(gap.jump(), 3); // H3 → H6…
        assert_eq!(gap.expected_level(), HeadingLevel::H4); // …skipped H4 and H5
    }

    #[test]
    fn the_walk_is_pure_and_deterministic() {
        let items = vec![
            heading("a", 2, "A"), // 0
            heading("b", 4, "B"), // 1
            instruction("one"),   // 2
        ];
        let first = heading_level_gaps(&items);
        let second = heading_level_gaps(&items);
        assert_eq!(first, second);
        // The input is untouched: the same items sit in the same
        // order with the same levels.
        assert!(matches!(&items[0], LibraryItem::Heading(h) if h.level == HeadingLevel::H2));
        assert!(matches!(&items[1], LibraryItem::Heading(h) if h.level == HeadingLevel::H4));
        assert_eq!(items.len(), 3);
    }

    // ---- the integration: skips track the level operations ----------------

    #[test]
    fn a_change_level_that_skips_creates_the_report() {
        // T-047's dense tree (levels.rs uses the same shape): change
        // the nested H3 to an H5 and the skip appears on that edge.
        let items = vec![
            heading("alpha", 2, "Alpha"), // 0
            instruction("a-one"),         // 1
            heading("sub", 3, "Sub"),     // 2
            hidden("a-two"),              // 3
            heading("beta", 2, "Beta"),   // 4
        ];
        assert!(heading_level_gaps(&items).is_empty());
        let changed = crate::levels::change_heading_level(&items, 2, 5).unwrap();
        let gaps = heading_level_gaps(&changed);
        assert_eq!(gaps.len(), 1);
        assert_eq!((gaps[0].from_index, gaps[0].to_index), (0, 2));
        assert_eq!(gaps[0].expected_level(), HeadingLevel::H3);
    }

    #[test]
    fn a_shift_that_closes_a_skip_removes_the_report() {
        // The H4 after the H2 skips; shifting the H4's branch down to
        // an H3 (T-047, ALG-062) makes the tree dense again.
        let items = vec![
            heading("alpha", 2, "Alpha"), // 0
            heading("deep", 4, "Deep"),   // 1: branch 1..2
            instruction("one"),           // 2
            heading("beta", 2, "Beta"),   // 3
        ];
        assert_eq!(heading_level_gaps(&items).len(), 1);
        let shifted = crate::levels::shift_branch_level(&items, 1, -1).unwrap();
        assert!(heading_level_gaps(&shifted).is_empty());
    }

    #[test]
    fn the_skip_pair_sits_inside_the_parents_branch() {
        // The ALG-060 cross-check: the later heading of the pair lies
        // inside the earlier one's branch (a deeper heading never
        // closes it), so the report's parent is a real parent.
        let items = vec![
            heading("alpha", 2, "Alpha"), // 0
            instruction("one"),           // 1
            heading("deep", 5, "Deep"),   // 2
            instruction("two"),           // 3
            heading("beta", 2, "Beta"),   // 4
        ];
        let gap = &heading_level_gaps(&items)[0];
        let branch = crate::heading_range::heading_branch(&items, gap.from_index).unwrap();
        assert!(branch.range.contains(&gap.to_index));
        assert_eq!(branch.range, 0..4);
    }
}

/// # Determinism and replay
///
/// The properties below pin the case budget (256) and the RNG seed
/// (the task's number — only its fixedness matters), the run policy
/// `body`, `heading_range`, `reorder`, `levels` and `gaps` property
/// tests use (D-053, D-070, D-073; the dropped D-071/D-072 entries,
/// D-151), so every run
/// explores the
/// same trees and a green run cannot flake. Any failure is persisted
/// by proptest into
/// `crates/eunomia-core/proptest-regressions/gaps.txt` and replayed
/// on every later run; a committed regressions file, if one ever
/// appears, is part of the suite.
#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;
    use proptest::test_runner::{Config as ProptestConfig, RngSeed};

    use super::*;
    use crate::library::{
        Heading, HeadingLevel, Instruction, InstructionState, ItemId, LibraryItem, Variant,
        VariantId,
    };

    /// The deterministic run policy: 256 cases, one fixed seed.
    fn config() -> ProptestConfig {
        ProptestConfig {
            cases: 256,
            rng_seed: RngSeed::Fixed(48),
            ..ProptestConfig::default()
        }
    }

    // ---- random trees ----------------------------------------------------

    /// One random row of the structure: a heading of any level, or an
    /// instruction — active or hidden, since hidden items take part
    /// in the geometry identically (D-11) and carry no level either
    /// way.
    fn row() -> impl Strategy<Value = u8> {
        prop_oneof![Just(0u8), 2u8..=6]
    }

    /// Items built from random rows with UNIQUE ids (the row's
    /// position): the properties below track items by position, and
    /// unique ids keep every tracked answer unambiguous.
    fn items() -> impl Strategy<Value = Vec<LibraryItem>> {
        prop::collection::vec(row(), 0..=24).prop_map(|rows| {
            rows.into_iter()
                .enumerate()
                .map(|(index, row)| match row {
                    0 => LibraryItem::Instruction(Instruction {
                        id: ItemId::new(format!("i-{index}")),
                        title: format!("Instruction {index}"),
                        state: if index % 5 == 0 {
                            InstructionState::Hidden
                        } else {
                            InstructionState::Active
                        },
                        variants: vec![Variant {
                            id: VariantId::new(format!("v-{index}")),
                            name: "Main".to_owned(),
                            content: "Be brief.".to_owned(),
                        }],
                    }),
                    level => LibraryItem::Heading(Heading {
                        id: ItemId::new(format!("h-{index}")),
                        level: HeadingLevel::try_from(level).expect("2..=6 is in range"),
                        title: format!("Title {index}"),
                    }),
                })
                .collect()
        })
    }

    proptest! {
        #![proptest_config(config())]

        /// The reports are exactly the consecutive-heading pairs that
        /// arrive deeper than one step — recomputed by an independent
        /// window walk over the heading subsequence.
        #[test]
        fn reports_are_exactly_the_too_deep_adjacent_pairs(
            tree in items(),
        ) {
            let headings: Vec<(usize, HeadingLevel)> = tree
                .iter()
                .enumerate()
                .filter_map(|(index, item)| match item {
                    LibraryItem::Heading(heading) => Some((index, heading.level)),
                    LibraryItem::Instruction(_) => None,
                })
                .collect();
            let expected: Vec<(usize, usize)> = headings
                .windows(2)
                .filter_map(|pair| {
                    let (from, to) = (pair[0], pair[1]);
                    (to.1.as_u8() > from.1.as_u8() + 1)
                        .then_some((from.0, to.0))
                })
                .collect();
            let reported: Vec<(usize, usize)> = heading_level_gaps(&tree)
                .iter()
                .map(|gap| (gap.from_index, gap.to_index))
                .collect();
            prop_assert_eq!(reported, expected);
        }

        /// Every report is well-formed on its own terms: real items,
        /// real levels, consecutive headings, a jump of at least 2,
        /// the expected level one step down — and the later heading
        /// inside the earlier one's ALG-060 branch.
        #[test]
        fn every_report_is_a_real_parent_edge(
            tree in items(),
        ) {
            for gap in heading_level_gaps(&tree) {
                let (LibraryItem::Heading(from), LibraryItem::Heading(to)) =
                    (&tree[gap.from_index], &tree[gap.to_index])
                else {
                    prop_assert!(false, "indices must address headings");
                    // prop_assert! expands to a statement-level `if`,
                    // so the block still needs a diverging statement
                    // to satisfy let-else (E0308 without this).
                    continue;
                };
                prop_assert!(gap.to_index > gap.from_index);
                prop_assert_eq!(from.level, gap.from_level);
                prop_assert_eq!(to.level, gap.to_level);
                prop_assert!(gap.jump() >= 2);
                prop_assert_eq!(
                    u8::from(gap.expected_level()),
                    u8::from(gap.from_level) + 1
                );
                let between_headings = tree[gap.from_index + 1..gap.to_index]
                    .iter()
                    .filter(|item| matches!(item, LibraryItem::Heading(_)))
                    .count();
                prop_assert_eq!(between_headings, 0);
                let branch = crate::heading_range::heading_branch(
                    &tree,
                    gap.from_index,
                )
                .expect("from_index addresses a heading");
                prop_assert!(branch.range.contains(&gap.to_index));
            }
        }

        /// Instructions never take part: interleaving one between
        /// every consecutive pair of a tree leaves the same skips
        /// between the same headings, indices shifted only.
        #[test]
        fn interleaved_instructions_change_only_the_indices(
            tree in items(),
        ) {
            let gaps_of = |items: &[LibraryItem]| {
                heading_level_gaps(items)
                    .iter()
                    .map(|gap| {
                        (
                            items[gap.from_index].item_id().as_str().to_owned(),
                            items[gap.to_index].item_id().as_str().to_owned(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let bare = gaps_of(&tree);
            let mut interleaved: Vec<LibraryItem> = Vec::with_capacity(tree.len() * 2);
            for (index, item) in tree.iter().enumerate() {
                interleaved.push(instruction(&format!("x-{index}")));
                interleaved.push(item.clone());
            }
            interleaved.push(instruction("x-end"));
            prop_assert_eq!(gaps_of(&interleaved), bare);
        }

        /// The walk is deterministic: the same tree reports the same
        /// facts twice.
        #[test]
        fn the_same_tree_reports_the_same_facts_twice(
            tree in items(),
        ) {
            prop_assert_eq!(heading_level_gaps(&tree), heading_level_gaps(&tree));
        }
    }

    fn instruction(id: &str) -> LibraryItem {
        LibraryItem::Instruction(Instruction {
            id: ItemId::new(id.to_owned()),
            title: format!("Instruction title {id}"),
            state: InstructionState::Active,
            variants: vec![],
        })
    }
}
