//! The ALG-060 branch range of a section heading (spec §2, §9.7).
//!
//! ALG-060 (verbatim): "The branch range of a level-L heading goes
//! from the heading itself to the item before the next heading of
//! level L or higher (hidden instructions included). Moving a branch
//! inside its own range is not allowed." The range is expressed in
//! **collection indices** — `start..end` over the item slice the
//! caller hands over — and not as a copy of the items: the reordering
//! operations that ALG-060 serves (ALG-061's cut-and-insert move,
//! ALG-062's level shift) work by index on the ordered structure, so
//! an index range is the form they can consume directly.
//!
//! "Level L or higher" follows the spec's numbering (§2 "Section
//! heading"): `##` is the highest level, so a heading **closes** the
//! branch when its level number is **less than or equal** to L — an
//! H2 heading closes any branch, an H6 heading closes only an H6
//! branch. Skipped levels
//! need no special case: an H4 that follows an H2 directly (no H3
//! between) is still closed by the next heading of level 2, 3 or 4,
//! and it never closes the surrounding H2 branch.
//!
//! Hidden instructions are items of the structure like any other
//! (they keep their position and their assignments, D-11), so they
//! land inside the range naturally — the walk below never looks at an
//! instruction's state. A heading at the end of the list has no
//! closing heading: its branch runs to the item count.
//!
//! The §2 term "heading section" is the same geometry minus the
//! heading itself — the items after the heading up to the close.
//! [`HeadingBranch::section`] hands that subrange out so callers do
//! not re-derive it with off-by-one arithmetic. The body builder
//! keeps its own private copy of the closing walk (`body::section_end`,
//! D-053): that module predates this one and is out of this task's
//! zone; the two share the §9.2/ALG-060 closing rule, not code.
//!
//! # Examples
//!
//! ```
//! use eunomia_core::heading_range::heading_branch;
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
//! let items = vec![
//!     heading("style", 2, "Style"),      // 0: the H2 branch…
//!     instruction("concise"),            // 1: …holds the H3 subsection…
//!     heading("tone", 3, "Tone"),        // 2: …with everything under it…
//!     instruction("friendly"),           // 3: (hidden or active — alike)
//!     heading("release", 2, "Release"),  // 4: …and closes at the next H2.
//!     instruction("notes"),              // 5
//! ];
//!
//! let branch = heading_branch(&items, 0).unwrap();
//! assert_eq!(branch.range, 0..4); // the heading itself through item 3
//! assert_eq!(branch.section(), 1..4); // §2: the items after the heading
//!
//! // The last heading has no closing heading: it runs to the end.
//! assert_eq!(heading_branch(&items, 4).unwrap().range, 4..6);
//!
//! // Only headings start a branch.
//! assert!(heading_branch(&items, 1).is_err());
//! ```

use std::ops::Range;

use crate::library::{HeadingLevel, ItemId, LibraryItem};

/// The error [`heading_branch`] and [`heading_branch_by_id`] return
/// when the requested branch cannot exist.
///
/// # Examples
///
/// ```
/// use eunomia_core::heading_range::{heading_branch, HeadingBranchError};
/// use eunomia_core::library::{Instruction, InstructionState, ItemId, LibraryItem};
///
/// let items = vec![LibraryItem::Instruction(Instruction {
///     id: ItemId::new("i1".to_owned()),
///     title: "Be concise".to_owned(),
///     state: InstructionState::Active,
///     variants: vec![],
/// })];
///
/// assert_eq!(
///     heading_branch(&items, 0),
///     Err(HeadingBranchError::NotAHeading { index: 0 })
/// );
/// assert_eq!(
///     heading_branch(&items, 7),
///     Err(HeadingBranchError::IndexOutOfBounds { index: 7, len: 1 })
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeadingBranchError {
    /// The index does not address an item: it is at least the item
    /// count.
    #[error("heading index {index} is out of bounds: {len} items")]
    IndexOutOfBounds {
        /// The rejected index.
        index: usize,
        /// The number of items that were handed over.
        len: usize,
    },
    /// The index addresses an instruction, and only a heading starts
    /// a branch.
    #[error("item at index {index} is not a heading")]
    NotAHeading {
        /// The index of the instruction.
        index: usize,
    },
    /// No heading with the requested id is among the items (an
    /// instruction id is rejected here too: headings cannot be
    /// assigned, but they are the only branch roots).
    #[error("no heading with id {id} among the items")]
    HeadingNotFound {
        /// The requested identifier.
        id: ItemId,
    },
}

/// The ALG-060 branch of one heading: its level and the index range
/// it spans in the item collection the caller handed over.
///
/// `range` starts at the heading itself and ends immediately before
/// the next heading of the heading's level L or higher — a level whose
/// schema number is less than or equal to L (§2: `##` is the highest
/// level). `end` is the index of that closing heading, or the item
/// count when the branch runs to the end of the list. The range is
/// never empty: it always holds at least the heading, and every value
/// these functions return satisfies
/// `range.start < range.end <= items.len()` — a caller may slice with
/// `&items[branch.range]` without a bounds re-check.
///
/// # Examples
///
/// ```
/// use std::convert::TryFrom;
///
/// use eunomia_core::heading_range::{heading_branch, HeadingBranch};
/// use eunomia_core::library::{Heading, HeadingLevel, ItemId, LibraryItem};
///
/// let items = vec![LibraryItem::Heading(Heading {
///     id: ItemId::new("h1".to_owned()),
///     level: HeadingLevel::try_from(6).unwrap(),
///     title: "Edge".to_owned(),
/// })];
///
/// let branch: HeadingBranch = heading_branch(&items, 0).unwrap();
/// assert_eq!(branch.level, HeadingLevel::H6);
/// assert_eq!(branch.range, 0..1); // a lone heading: itself, then the end
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingBranch {
    /// The branch root's level L (`##`…`######`); the branch closes
    /// at the next heading of this level or higher.
    pub level: HeadingLevel,
    /// The branch's index range: the heading itself up to (excluding)
    /// the closing heading — or up to the item count at the end of
    /// the list.
    pub range: Range<usize>,
}

impl HeadingBranch {
    /// The §2 "heading section": the branch's items after the heading
    /// itself, as an index range.
    ///
    /// The heading is the branch's root, not a member of its own
    /// section; callers that need "everything under this heading"
    /// (rather than "the heading and everything under it") get it
    /// here without repeating the `start + 1` arithmetic.
    ///
    /// # Examples
    ///
    /// ```
    /// use eunomia_core::heading_range::{heading_branch, HeadingBranch};
    /// use eunomia_core::library::{
    ///     Heading, HeadingLevel, ItemId, LibraryItem,
    /// };
    ///
    /// let items = vec![
    ///     LibraryItem::Heading(Heading {
    ///         id: ItemId::new("h1".to_owned()),
    ///         level: HeadingLevel::H2,
    ///         title: "Style".to_owned(),
    ///     }),
    /// ];
    ///
    /// let branch: HeadingBranch = heading_branch(&items, 0).unwrap();
    /// assert_eq!(branch.range, 0..1);
    /// assert_eq!(branch.section(), 1..1); // nothing after the heading
    /// ```
    #[must_use]
    pub fn section(&self) -> Range<usize> {
        self.range.start + 1..self.range.end
    }
}

/// The ALG-060 branch range of the heading at `heading_index`: from
/// the heading itself to the item before the next heading of the same
/// or a higher level, hidden instructions included.
///
/// The indices are positions in `items` — the snapshot's item slice,
/// a store result, or any collection in library order (the order is
/// the canon, DB-005/D-042; a `position` number is never read here).
/// A heading at the end of the list closes at `items.len()`.
///
/// # Errors
///
/// [`HeadingBranchError::IndexOutOfBounds`] when `heading_index` does
/// not address an item, and [`HeadingBranchError::NotAHeading`] when
/// it addresses an instruction.
pub fn heading_branch(
    items: &[LibraryItem],
    heading_index: usize,
) -> Result<HeadingBranch, HeadingBranchError> {
    let level = match items.get(heading_index) {
        Some(LibraryItem::Heading(heading)) => heading.level,
        Some(LibraryItem::Instruction(_)) => {
            return Err(HeadingBranchError::NotAHeading {
                index: heading_index,
            });
        }
        None => {
            return Err(HeadingBranchError::IndexOutOfBounds {
                index: heading_index,
                len: items.len(),
            });
        }
    };
    let end = closing_index(items, heading_index, level);
    Ok(HeadingBranch {
        level,
        range: heading_index..end,
    })
}

/// The same branch, addressed by the heading's id instead of its
/// index — the form a reorder command naturally holds (the UI knows
/// which heading the user grabbed, not where it sits in the slice).
/// Should ids ever repeat (the database's primary key forbids it,
/// §6.2), the first occurrence in collection order wins.
///
/// # Errors
///
/// [`HeadingBranchError::HeadingNotFound`] when no heading with the
/// id is among the items, plus every error [`heading_branch`] can
/// return for the index it resolves to.
///
/// # Examples
///
/// ```
/// use eunomia_core::heading_range::heading_branch_by_id;
/// use eunomia_core::library::{Heading, HeadingLevel, ItemId, LibraryItem};
///
/// let items = vec![
///     LibraryItem::Heading(Heading {
///         id: ItemId::new("style".to_owned()),
///         level: HeadingLevel::H2,
///         title: "Style".to_owned(),
///     }),
///     LibraryItem::Heading(Heading {
///         id: ItemId::new("release".to_owned()),
///         level: HeadingLevel::H2,
///         title: "Release".to_owned(),
///     }),
/// ];
///
/// let branch = heading_branch_by_id(&items, &ItemId::new("style".to_owned())).unwrap();
/// assert_eq!(branch.range, 0..1); // Release closes Style immediately
/// ```
pub fn heading_branch_by_id(
    items: &[LibraryItem],
    heading_id: &ItemId,
) -> Result<HeadingBranch, HeadingBranchError> {
    let heading_index = items
        .iter()
        .position(|item| match item {
            LibraryItem::Heading(heading) => heading.id == *heading_id,
            LibraryItem::Instruction(_) => false,
        })
        .ok_or_else(|| HeadingBranchError::HeadingNotFound {
            id: heading_id.clone(),
        })?;
    heading_branch(items, heading_index)
}

/// The exclusive end of the branch: the index of the first heading
/// after `heading_index` whose level is `level` or higher — that is,
/// whose schema integer is less than or equal to `level`'s (§2: `##`
/// is the highest level) — or the item count when no such heading
/// follows (ALG-060, and the same closing walk §9.2's heading
/// inclusion uses).
fn closing_index(items: &[LibraryItem], heading_index: usize, level: HeadingLevel) -> usize {
    items
        .iter()
        .enumerate()
        .skip(heading_index + 1)
        .find_map(|(index, item)| match item {
            LibraryItem::Heading(heading) if heading.level.as_u8() <= level.as_u8() => Some(index),
            _ => None,
        })
        .unwrap_or(items.len())
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use super::*;
    use crate::library::{Heading, Instruction, InstructionState, Variant, VariantId};

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

    // ---- ALG-060: what closes a branch ----------------------------------

    #[test]
    fn a_branch_spans_from_the_heading_to_the_next_heading_of_the_same_level() {
        let items = vec![
            instruction("first"),         // 0: before the branch
            heading("style", 2, "Style"), // 1
            instruction("concise"),       // 2
            heading("release", 2, "Rel"), // 3: same level closes
            instruction("notes"),         // 4
        ];
        let branch = heading_branch(&items, 1).unwrap();
        assert_eq!(branch.level, HeadingLevel::H2);
        assert_eq!(branch.range, 1..3);
        assert_eq!(branch.section(), 2..3);
    }

    #[test]
    fn a_branch_contains_its_nested_deeper_subsections_whole() {
        let items = vec![
            heading("style", 2, "Style"),     // 0: the H2 branch…
            instruction("concise"),           // 1
            heading("tone", 3, "Tone"),       // 2: …holds a whole H3 branch…
            instruction("friendly"),          // 3
            heading("voice", 3, "Voice"),     // 4: …and a second one…
            hidden("terse"),                  // 5: (hidden, still inside)
            heading("words", 4, "Words"),     // 6: …deeper still
            instruction("simple"),            // 7
            heading("release", 2, "Release"), // 8: …and closes at the next H2
            instruction("notes"),             // 9
        ];
        let h2 = heading_branch(&items, 0).unwrap();
        assert_eq!(h2.range, 0..8);

        // The nested H3 branch is a subrange of the H2 branch.
        let h3 = heading_branch(&items, 2).unwrap();
        assert_eq!(h3.level, HeadingLevel::H3);
        assert_eq!(h3.range, 2..4);
        // A deeper heading (H4) does not close an H3 branch.
        let second_h3 = heading_branch(&items, 4).unwrap();
        assert_eq!(second_h3.range, 4..8);
        let h4 = heading_branch(&items, 6).unwrap();
        assert_eq!(h4.level, HeadingLevel::H4);
        assert_eq!(h4.range, 6..8);
    }

    #[test]
    fn skipped_levels_do_not_close_the_outer_branch() {
        // H2 → H4 with no H3 between (a level skip the spec allows):
        // the H4 is a branch of its own inside the H2, and only a
        // heading of level 2 closes the H2.
        let items = vec![
            heading("style", 2, "Style"), // 0
            heading("words", 4, "Words"), // 1: a skip — deeper, not closing
            instruction("simple"),        // 2
            heading("release", 2, "Rel"), // 3
        ];
        let h2 = heading_branch(&items, 0).unwrap();
        assert_eq!(h2.range, 0..3);
        let h4 = heading_branch(&items, 1).unwrap();
        assert_eq!(h4.range, 1..3);
    }

    #[test]
    fn a_deeper_branch_closes_on_the_first_heading_of_level_l_or_higher() {
        // The H4 branch closes on the first heading whose level number
        // is <= 4 — here an H3, before any same-level H4 shows up.
        let items = vec![
            heading("style", 2, "Style"), // 0
            heading("words", 4, "Words"), // 1
            instruction("simple"),        // 2
            heading("tone", 3, "Tone"),   // 3: higher level (smaller number) closes
            instruction("friendly"),      // 4
            heading("plain", 4, "Plain"), // 5
            instruction("direct"),        // 6
        ];
        assert_eq!(heading_branch(&items, 1).unwrap().range, 1..3);
        // The H3 is the highest level left: the H4 at 5 does not
        // close it (4 > 3), so it runs to the end of the list.
        assert_eq!(heading_branch(&items, 3).unwrap().range, 3..7);
        assert_eq!(heading_branch(&items, 5).unwrap().range, 5..7);
    }

    #[test]
    fn an_h6_branch_closes_only_on_another_h6_or_a_higher_level() {
        let items = vec![
            heading("deep", 6, "Deep"),   // 0
            instruction("tiny"),          // 1
            heading("deeper", 6, "More"), // 2: same level closes
            instruction("mid"),           // 3
            heading("less", 5, "Less"),   // 4: H5 is higher than H6 — closes too
            instruction("last"),          // 5
        ];
        assert_eq!(heading_branch(&items, 0).unwrap().range, 0..2);
        assert_eq!(heading_branch(&items, 2).unwrap().range, 2..4);
        assert_eq!(heading_branch(&items, 4).unwrap().range, 4..6);
    }

    #[test]
    fn an_h2_branch_is_closed_by_nothing_but_another_h2() {
        // H2 is the highest level there is (no H1, §6.2), so every
        // deeper heading — hidden instructions too — stays inside.
        let items = vec![
            heading("top", 2, "Top"),   // 0
            heading("a", 3, "A"),       // 1
            heading("b", 4, "B"),       // 2
            heading("c", 5, "C"),       // 3
            heading("d", 6, "D"),       // 4
            hidden("e"),                // 5
            heading("next", 2, "Next"), // 6
        ];
        assert_eq!(heading_branch(&items, 0).unwrap().range, 0..6);
    }

    // ---- ALG-060: the list's ends ---------------------------------------

    #[test]
    fn a_heading_at_the_start_of_the_list_opens_its_branch() {
        let items = vec![
            heading("style", 2, "Style"), // 0: index 0 is a fine root
            instruction("concise"),       // 1
            heading("release", 2, "Rel"), // 2
        ];
        let branch = heading_branch(&items, 0).unwrap();
        assert_eq!(branch.range, 0..2);
    }

    #[test]
    fn a_branch_without_a_closing_heading_runs_to_the_end_of_the_list() {
        let items = vec![
            heading("style", 2, "Style"), // 0
            instruction("concise"),       // 1
            heading("tone", 3, "Tone"),   // 2: no H2/H3 ever follows
            instruction("friendly"),      // 3
        ];
        assert_eq!(heading_branch(&items, 0).unwrap().range, 0..4);
        assert_eq!(heading_branch(&items, 2).unwrap().range, 2..4);
    }

    #[test]
    fn an_empty_branch_ends_right_after_the_heading() {
        // Two H2 headings next to each other: the first one's branch
        // is just the heading itself.
        let items = vec![
            instruction("first"),         // 0
            heading("style", 2, "Style"), // 1: no items before…
            heading("release", 2, "Rel"), // 2: …the next same-level close
            instruction("notes"),         // 3
        ];
        let branch = heading_branch(&items, 1).unwrap();
        assert_eq!(branch.range, 1..2);
        assert_eq!(branch.section(), 2..2); // an empty section
    }

    #[test]
    fn a_heading_as_the_last_item_is_a_branch_of_itself() {
        let items = vec![
            heading("style", 2, "Style"), // 0
            instruction("concise"),       // 1
            heading("tone", 5, "Tone"),   // 2: the list's last item
        ];
        let branch = heading_branch(&items, 2).unwrap();
        assert_eq!(branch.level, HeadingLevel::H5);
        assert_eq!(branch.range, 2..3);
        assert_eq!(branch.section(), 3..3);
    }

    #[test]
    fn an_instruction_as_the_last_item_stays_inside_the_branch() {
        let items = vec![
            heading("style", 3, "Style"), // 0
            hidden("terse"),              // 1: hidden, and the last item
        ];
        assert_eq!(heading_branch(&items, 0).unwrap().range, 0..2);
    }

    #[test]
    fn hidden_instructions_are_included_like_any_other_item() {
        // D-11: a hidden instruction keeps its place in the structure;
        // its state never narrows or widens a branch.
        let items = vec![
            heading("style", 2, "Style"), // 0
            hidden("terse"),              // 1
            instruction("concise"),       // 2
            hidden("quiet"),              // 3
            heading("tone", 3, "Tone"),   // 4
            hidden("friendly"),           // 5
            heading("release", 2, "Rel"), // 6
        ];
        assert_eq!(heading_branch(&items, 0).unwrap().range, 0..6);
        assert_eq!(heading_branch(&items, 4).unwrap().range, 4..6);
    }

    // ---- errors ----------------------------------------------------------

    #[test]
    fn an_index_past_the_items_is_rejected() {
        let items = vec![heading("style", 2, "Style")];
        assert_eq!(
            heading_branch(&items, 1),
            Err(HeadingBranchError::IndexOutOfBounds { index: 1, len: 1 })
        );
        assert_eq!(
            heading_branch(&items, 99),
            Err(HeadingBranchError::IndexOutOfBounds { index: 99, len: 1 })
        );
        assert_eq!(
            heading_branch(&[], 0),
            Err(HeadingBranchError::IndexOutOfBounds { index: 0, len: 0 })
        );
    }

    #[test]
    fn an_instruction_index_is_not_a_branch_root() {
        let items = vec![
            instruction("concise"),       // 0
            heading("style", 2, "Style"), // 1
            instruction("notes"),         // 2
        ];
        assert_eq!(
            heading_branch(&items, 0),
            Err(HeadingBranchError::NotAHeading { index: 0 })
        );
        assert_eq!(
            heading_branch(&items, 2),
            Err(HeadingBranchError::NotAHeading { index: 2 })
        );
        // The heading between them still answers.
        assert_eq!(heading_branch(&items, 1).unwrap().range, 1..3);
    }

    #[test]
    fn the_error_messages_name_the_rejected_input() {
        assert_eq!(
            HeadingBranchError::IndexOutOfBounds { index: 9, len: 4 }.to_string(),
            "heading index 9 is out of bounds: 4 items"
        );
        assert_eq!(
            HeadingBranchError::NotAHeading { index: 2 }.to_string(),
            "item at index 2 is not a heading"
        );
        assert_eq!(
            HeadingBranchError::HeadingNotFound {
                id: ItemId::new("h9".to_owned())
            }
            .to_string(),
            "no heading with id h9 among the items"
        );
    }

    // ---- the id-addressed form -------------------------------------------

    #[test]
    fn the_id_form_resolves_to_the_same_branch_as_the_index_form() {
        let items = vec![
            heading("style", 2, "Style"), // 0
            instruction("concise"),       // 1
            heading("tone", 3, "Tone"),   // 2
            instruction("friendly"),      // 3
            heading("release", 2, "Rel"), // 4
        ];
        let by_id = heading_branch_by_id(&items, &ItemId::new("tone".to_owned())).unwrap();
        let by_index = heading_branch(&items, 2).unwrap();
        assert_eq!(by_id, by_index);
        assert_eq!(by_id.range, 2..4);
    }

    #[test]
    fn the_id_form_rejects_unknown_and_instruction_ids() {
        let items = vec![
            instruction("concise"),       // 0: an instruction id is not a root
            heading("style", 2, "Style"), // 1
        ];
        // The empty structure has nothing to resolve to.
        assert_eq!(
            heading_branch_by_id(&[], &ItemId::new("style".to_owned())),
            Err(HeadingBranchError::HeadingNotFound {
                id: ItemId::new("style".to_owned())
            })
        );
        assert_eq!(
            heading_branch_by_id(&items, &ItemId::new("missing".to_owned())),
            Err(HeadingBranchError::HeadingNotFound {
                id: ItemId::new("missing".to_owned())
            })
        );
        assert_eq!(
            heading_branch_by_id(&items, &ItemId::new("concise".to_owned())),
            Err(HeadingBranchError::HeadingNotFound {
                id: ItemId::new("concise".to_owned())
            })
        );
    }
}

/// # Determinism and replay
///
/// The properties below pin the case budget (256) and the RNG seed
/// (the task's number — only its fixedness matters), the same run
/// policy `body`'s property tests use, so every run explores the same
/// trees and a green run cannot flake. Any failure is persisted by
/// proptest into
/// `crates/eunomia-core/proptest-regressions/heading_range.txt` and
/// replayed on every later run; a committed regressions file, if one
/// ever appears, is part of the suite.
#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;
    use proptest::test_runner::{Config as ProptestConfig, RngSeed};

    use super::*;
    use crate::library::{Heading, Instruction, InstructionState, Variant, VariantId};

    /// The deterministic run policy: 256 cases, one fixed seed.
    fn config() -> ProptestConfig {
        ProptestConfig {
            cases: 256,
            rng_seed: RngSeed::Fixed(45),
            ..ProptestConfig::default()
        }
    }

    // ---- random trees ----------------------------------------------------

    /// One random item: a heading of any level, or an instruction —
    /// active or hidden, since hidden items take part in the
    /// geometry identically (D-11).
    fn raw_item() -> impl Strategy<Value = LibraryItem> {
        prop_oneof![
            (2u8..=6, "[a-z]{1,8}").prop_map(|(level, id)| {
                LibraryItem::Heading(Heading {
                    id: ItemId::new(format!("h-{id}")),
                    level: HeadingLevel::try_from(level).expect("2..=6 is in range"),
                    title: format!("Title {id}"),
                })
            }),
            ("[a-z]{1,8}", 0usize..=1).prop_map(|(id, hidden)| {
                LibraryItem::Instruction(Instruction {
                    id: ItemId::new(format!("i-{id}-{hidden}")),
                    title: format!("Instruction {id}"),
                    state: if hidden == 1 {
                        InstructionState::Hidden
                    } else {
                        InstructionState::Active
                    },
                    variants: vec![Variant {
                        id: VariantId::new(format!("v-{id}")),
                        name: "Main".to_owned(),
                        content: "Be brief.".to_owned(),
                    }],
                })
            }),
        ]
    }

    fn raw_items() -> impl Strategy<Value = Vec<LibraryItem>> {
        // Item ids collide across positions on purpose (the generator
        // does not dedupe): the range walk only ever reads kinds and
        // levels, so uniqueness is not a precondition here. Ids drive
        // only the by-id form, which the properties below do not use.
        prop::collection::vec(raw_item(), 0..=24)
    }

    // ---- the properties ---------------------------------------------------

    proptest! {
        #![proptest_config(config())]

        /// ALG-060 restated as a check over every heading of a random
        /// tree: the branch starts at the heading, nothing of level L
        /// or higher sits strictly inside, and the item at `end` —
        /// when the list has one — is a heading of level L or higher.
        #[test]
        fn for_random_trees_every_branch_closes_exactly_at_the_first_level_l_or_higher(
            items in raw_items(),
        ) {
            for (index, item) in items.iter().enumerate() {
                let LibraryItem::Heading(heading) = item else { continue };
                let level = heading.level.as_u8();
                let branch = heading_branch(&items, index).unwrap();

                // The range starts at the heading itself and never
                // runs empty (the heading is always inside).
                prop_assert_eq!(branch.range.start, index);
                prop_assert!(branch.range.end > index);
                prop_assert_eq!(branch.section(), index + 1..branch.range.end);
                prop_assert_eq!(branch.level, heading.level);

                // Nothing of level L or higher strictly inside.
                for (inside, item) in items
                    .iter()
                    .enumerate()
                    .take(branch.range.end)
                    .skip(branch.range.start + 1)
                {
                    if let LibraryItem::Heading(inner) = item {
                        prop_assert!(
                            inner.level.as_u8() > level,
                            "heading of level {} at {} closes the level-{level} branch",
                            inner.level.as_u8(),
                            inside,
                        );
                    }
                }

                // The close, when it exists, is a heading of level L
                // or higher.
                match items.get(branch.range.end) {
                    Some(LibraryItem::Heading(closing)) => prop_assert!(
                        closing.level.as_u8() <= level,
                        "branch of level {level} must not extend past a level-{} heading",
                        closing.level.as_u8(),
                    ),
                    None => prop_assert_eq!(branch.range.end, items.len()),
                    Some(LibraryItem::Instruction(_)) => {
                        prop_assert!(false, "a branch must end on a heading or the list end");
                    }
                }
            }
        }
    }
}
