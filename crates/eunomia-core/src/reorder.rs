//! The ALG-061 move: cut the range, insert it at the target position,
//! renumber (spec §6.1, §7.3, §7.4, §9.7).
//!
//! ALG-061 (verbatim): "A move runs in one transaction: cut the
//! range, insert it at the target position, renumber `position`
//! from 1. Moving one instruction is a special case of a one-item
//! range."
//! The guard comes from ALG-060 ("Moving a branch inside its own
//! range is not allowed") and LIB-034/MAIN-018, 019 (§7.3: "A branch
//! cannot be moved inside itself"); MAIN-015…023 (§7.4) name the
//! operations this serves — "moving headings and instructions between
//! sections". The acceptance criterion — "Moving a branch keeps the
//! order of nested items" — is the property the tests pin first.
//!
//! # What the target names
//!
//! `target` is an index into the list as it stands BEFORE the move,
//! `0..=items.len()`: the moved range lands so that every item
//! outside the range that currently sits before `target` stays before
//! it, and every item outside the range at or after `target` follows
//! it. A cut-and-insert admits two target conventions and this module
//! fixes one: the target is NOT an index into the remainder left
//! after the cut. Naming the target in the original list is what
//! makes "inside its own range" expressible — a target strictly
//! between `range.start` and `range.end` names a gap between the
//! range's own items — so the check D-070 foresaw ("moving a branch
//! inside its own range becomes an index check on `range`") is a
//! plain comparison, with no translation through the cut. Under
//! remainder indexing the cut range's items would not exist to gap
//! apart, and the ALG-060 guard would have no input to reject.
//!
//! The range's own boundaries are its current place, not its inside:
//! `target == range.start` ("stay put") and `target == range.end`
//! ("sit back down right after itself") are valid moves that change
//! nothing — the same order comes back, renumbered trivially. Only a
//! STRICTLY interior target is "inside itself" and rejected with
//! [`ReorderError::TargetInsideMovedRange`].
//!
//! # Renumbering (DB-005), and where the numbers live
//!
//! DB-005 (verbatim): "Item order is set by an integer `position`
//! starting from 1, with no gaps. Reordering renumbers the affected
//! range in one transaction." Core types carry no `position` number
//! (D-042): collection order is the canon, and copying the number
//! into the items is where the two could drift. The new numbering is
//! therefore the returned collection itself — index `i` is position
//! `i + 1`, `1..=len` with no gaps and no duplicates by construction,
//! over every item and not only the affected stretch. The one
//! transaction DB-005 demands is a database fact: persisting the
//! numbers is the store layer's move command, which hands this
//! function a slice in library order and writes the returned order
//! back as positions in a single transaction (writing only the rows
//! whose position changed is that command's latitude — the no-gaps
//! invariant holds over the whole list either way).
//!
//! # The E_VALIDATION boundary
//!
//! The task wording "moving inside itself is rejected with
//! `E_VALIDATION`" names the §7.10 error registry, which belongs to
//! the Tauri command layer (`AppError`/`ErrorCode`, D-034) — core
//! knows neither. Following the D-062 precedent
//! (`E_MARKERS_INVALID`), this module rejects with its own
//! [`ReorderError`]; `TargetInsideMovedRange` is the core twin of the
//! future command-layer `E_VALIDATION` mapping, and the mapping
//! itself is a task for whoever lands the command.
//!
//! # Purity
//!
//! Pure functions over `&[LibraryItem]`, in the style the crate
//! settled on for [`heading_range`] (D-070): the input is never
//! touched — [`move_range`] returns a NEW `Vec` holding the new
//! order. Callers moving a heading's branch hand over the heading's
//! index (or id); the ALG-060 range is computed inside via
//! [`heading_range`], so a caller cannot pass a range that disagrees
//! with the tree.
//!
//! # Examples
//!
//! ```
//! use eunomia_core::heading_range::heading_branch;
//! use eunomia_core::library::{
//!     Heading, HeadingLevel, Instruction, InstructionState, ItemId, LibraryItem, Variant,
//!     VariantId,
//! };
//! use eunomia_core::reorder::move_branch;
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
//!     heading("style", 2, "Style"),     // 0: the branch…
//!     instruction("concise"),           // 1: …with a nested H3…
//!     heading("tone", 3, "Tone"),       // 2: …and its item…
//!     instruction("friendly"),          // 3
//!     heading("release", 2, "Release"), // 4: …closing at the next H2
//!     instruction("notes"),             // 5
//! ];
//!
//! // Move the Style branch after the whole Release branch: the
//! // target 6 is the end of the list.
//! let moved = move_branch(&items, 0, 6).unwrap();
//! let ids: Vec<&str> = moved.iter().map(|i| i.item_id().as_str()).collect();
//! assert_eq!(ids, ["release", "notes", "style", "concise", "tone", "friendly"]);
//!
//! // A target strictly inside the branch's own range is rejected.
//! assert!(move_branch(&items, 0, 2).is_err());
//!
//! // The boundaries are the branch's own place, not its inside.
//! assert_eq!(move_branch(&items, 0, 0).unwrap(), items);
//! assert_eq!(move_branch(&items, 0, 4).unwrap(), items);
//! ```

use std::ops::Range;

use crate::heading_range::{HeadingBranchError, heading_branch, heading_branch_by_id};
use crate::library::{ItemId, LibraryItem};

/// The error [`move_range`], [`move_branch`] and [`move_branch_by_id`]
/// return when the requested move cannot run.
///
/// # Examples
///
/// ```
/// use std::ops::Range;
///
/// use eunomia_core::library::{Instruction, InstructionState, ItemId, LibraryItem};
/// use eunomia_core::reorder::{move_range, ReorderError};
///
/// let items: Vec<LibraryItem> = (0..3)
///     .map(|i| {
///         LibraryItem::Instruction(Instruction {
///             id: ItemId::new(format!("i{i}")),
///             title: format!("Instruction {i}"),
///             state: InstructionState::Active,
///             variants: vec![],
///         })
///     })
///     .collect();
///
/// // A target strictly inside the moved range is "inside itself".
/// assert_eq!(
///     move_range(&items, 0..2, 1),
///     Err(ReorderError::TargetInsideMovedRange {
///         range: Range { start: 0, end: 2 },
///         target: 1,
///     })
/// );
/// // Ranges are checked before targets.
/// assert_eq!(
///     move_range(&items, 0..0, 0),
///     Err(ReorderError::InvalidRange {
///         start: 0,
///         end: 0,
///         len: 3
///     })
/// );
/// assert_eq!(
///     move_range(&items, 0..1, 4),
///     Err(ReorderError::TargetOutOfBounds { target: 4, len: 3 })
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReorderError {
    /// The range is empty (`start >= end`) or runs past the item count
    /// (`end > len`).
    #[error("moved range {start}..{end} is not a non-empty range within {len} items")]
    InvalidRange {
        /// The rejected range's start.
        start: usize,
        /// The rejected range's end.
        end: usize,
        /// The number of items that were handed over.
        len: usize,
    },
    /// The target is not an insert position the list allows: it must
    /// be an index `0..=len` into the current list.
    #[error("insert target {target} is out of bounds: {len} items allow 0 to {len}")]
    TargetOutOfBounds {
        /// The rejected target.
        target: usize,
        /// The number of items that were handed over.
        len: usize,
    },
    /// The target names a gap strictly inside the moved range — the
    /// ALG-060/LIB-034 "a branch cannot be moved inside itself"
    /// rejection. The command layer will map this to `E_VALIDATION`
    /// (see the module docs' boundary note).
    #[error("cannot insert the range {range:?} inside itself at {target}")]
    TargetInsideMovedRange {
        /// The range that was being moved.
        range: Range<usize>,
        /// The target that fell strictly inside it.
        target: usize,
    },
    /// The heading form could not resolve a branch: a bad index, an
    /// instruction index, or an unknown id — every error
    /// [`heading_branch`](crate::heading_range::heading_branch) can
    /// return.
    #[error(transparent)]
    Branch(#[from] HeadingBranchError),
}

/// The ALG-061 cut-and-insert over any non-empty range of items:
/// cut `range` out of `items`, insert it at `target`, and return the
/// new order — which is also the new numbering (DB-005: index `i` is
/// position `i + 1`; see the module docs). Moving one instruction is
/// the one-item-range special case (`index..index + 1`, §7.3: "Single
/// instructions can be moved between sections").
///
/// `target` is an index into `items` as handed over (`0..=len`), not
/// into the remainder the cut leaves; the range's own boundaries
/// (`range.start`, `range.end`) are valid no-op targets, and every
/// index strictly between them is rejected as "inside itself"
/// (ALG-060, LIB-034). The input slice is never modified.
///
/// # Errors
///
/// In order of precedence: [`ReorderError::InvalidRange`] when
/// `range` is empty or runs past the items;
/// [`ReorderError::TargetOutOfBounds`] when `target` exceeds the item
/// count; [`ReorderError::TargetInsideMovedRange`] when `target`
/// falls strictly inside `range`.
///
/// # Examples
///
/// ```
/// use eunomia_core::library::{Instruction, InstructionState, ItemId, LibraryItem};
/// use eunomia_core::reorder::move_range;
///
/// fn instruction(id: &str) -> LibraryItem {
///     LibraryItem::Instruction(Instruction {
///         id: ItemId::new(id.to_owned()),
///         title: format!("Instruction {id}"),
///         state: InstructionState::Active,
///         variants: vec![],
///     })
/// }
///
/// let items = vec![instruction("a"), instruction("b"), instruction("c")];
///
/// // ALG-061's special case: one instruction, a one-item range.
/// let moved = move_range(&items, 1..2, 0).unwrap();
/// let ids: Vec<&str> = moved.iter().map(|i| i.item_id().as_str()).collect();
/// assert_eq!(ids, ["b", "a", "c"]);
///
/// // To the end: the target names the gap after the last item.
/// let moved = move_range(&items, 1..2, 3).unwrap();
/// let ids: Vec<&str> = moved.iter().map(|i| i.item_id().as_str()).collect();
/// assert_eq!(ids, ["a", "c", "b"]);
///
/// // The move never loses or duplicates an item: the answer is a
/// // renumbered permutation (DB-005).
/// let mut moved_ids: Vec<&str> = moved.iter().map(|i| i.item_id().as_str()).collect();
/// moved_ids.sort_unstable();
/// let mut item_ids: Vec<&str> = items.iter().map(|i| i.item_id().as_str()).collect();
/// item_ids.sort_unstable();
/// assert_eq!(moved_ids, item_ids);
/// ```
pub fn move_range(
    items: &[LibraryItem],
    range: Range<usize>,
    target: usize,
) -> Result<Vec<LibraryItem>, ReorderError> {
    let Range { start, end } = range;
    if start >= end || end > items.len() {
        return Err(ReorderError::InvalidRange {
            start,
            end,
            len: items.len(),
        });
    }
    if target > items.len() {
        return Err(ReorderError::TargetOutOfBounds {
            target,
            len: items.len(),
        });
    }
    if start < target && target < end {
        return Err(ReorderError::TargetInsideMovedRange { range, target });
    }

    // "Cut the range, insert it at the target position": the target
    // names a gap in the CURRENT list, so translate it to the gap's
    // neighbourhood in the cut-up remainder. A target at or before
    // `start` lands between the head items; a target at or past `end`
    // lands between the tail items — everything the cut did not take
    // keeps its relative order, and the range's own items keep theirs.
    let mut reordered = Vec::with_capacity(items.len());
    if target >= end {
        reordered.extend_from_slice(&items[..start]);
        reordered.extend_from_slice(&items[end..target]);
        reordered.extend_from_slice(&items[start..end]);
        reordered.extend_from_slice(&items[target..]);
    } else {
        // target <= start: the strictly interior targets never reach
        // here.
        reordered.extend_from_slice(&items[..target]);
        reordered.extend_from_slice(&items[start..end]);
        reordered.extend_from_slice(&items[target..start]);
        reordered.extend_from_slice(&items[end..]);
    }
    Ok(reordered)
}

/// The same move addressed to a heading's ALG-060 branch: the range
/// is [`heading_branch`]'s answer for `heading_index`, computed here
/// so a caller cannot hand in a range that disagrees with the tree.
/// Everything else — target convention, no-op boundaries, the
/// "inside itself" rejection, the renumbering — is
/// [`move_range`]'s.
///
/// # Errors
///
/// [`ReorderError::Branch`] first, for every error
/// [`heading_branch`] can return (the branch must be resolved before
/// there is a range to check the target against); then whatever
/// [`move_range`] rejects for the computed range.
///
/// # Examples
///
/// ```
/// use std::convert::TryFrom;
///
/// use eunomia_core::library::{Heading, HeadingLevel, ItemId, LibraryItem};
/// use eunomia_core::reorder::{move_branch, ReorderError};
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
/// // Style's branch is the heading alone (0..1); move it after
/// // Release (target 2).
/// let moved = move_branch(&items, 0, 2).unwrap();
/// let ids: Vec<&str> = moved.iter().map(|i| i.item_id().as_str()).collect();
/// assert_eq!(ids, ["release", "style"]);
///
/// // Only a heading roots a branch: an instruction index is
/// // rejected before the target is ever looked at.
/// let items = vec![
///     moved[1].clone(), // the Style heading
///     LibraryItem::Instruction(eunomia_core::library::Instruction {
///         id: ItemId::new("be-concise".to_owned()),
///         title: "Be concise".to_owned(),
///         state: eunomia_core::library::InstructionState::Active,
///         variants: vec![],
///     }),
/// ];
/// assert_eq!(
///     move_branch(&items, 1, 0),
///     Err(ReorderError::Branch(
///         eunomia_core::heading_range::HeadingBranchError::NotAHeading { index: 1 }
///     ))
/// );
/// ```
pub fn move_branch(
    items: &[LibraryItem],
    heading_index: usize,
    target: usize,
) -> Result<Vec<LibraryItem>, ReorderError> {
    let branch = heading_branch(items, heading_index)?;
    move_range(items, branch.range, target)
}

/// The same branch move, addressed by the heading's id instead of its
/// index — the form a reorder command naturally holds (the UI knows
/// which heading the user grabbed; D-070's by-id precedent).
///
/// # Errors
///
/// [`ReorderError::Branch`] when no heading with the id is among the
/// items (an instruction id is rejected too), then whatever
/// [`move_range`] rejects for the resolved range.
///
/// # Examples
///
/// ```
/// use eunomia_core::library::{Heading, HeadingLevel, ItemId, LibraryItem};
/// use eunomia_core::reorder::move_branch_by_id;
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
/// let moved = move_branch_by_id(&items, &ItemId::new("style".to_owned()), 2).unwrap();
/// let ids: Vec<&str> = moved.iter().map(|i| i.item_id().as_str()).collect();
/// assert_eq!(ids, ["release", "style"]);
///
/// // An unknown id resolves to no branch at all.
/// assert!(move_branch_by_id(&items, &ItemId::new("nope".to_owned()), 0).is_err());
/// ```
pub fn move_branch_by_id(
    items: &[LibraryItem],
    heading_id: &ItemId,
    target: usize,
) -> Result<Vec<LibraryItem>, ReorderError> {
    let branch = heading_branch_by_id(items, heading_id)?;
    move_range(items, branch.range, target)
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use super::*;
    use crate::heading_range::{HeadingBranchError, heading_branch, heading_branch_by_id};
    use crate::library::{
        Heading, HeadingLevel, Instruction, InstructionState, Variant, VariantId,
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

    fn ids(items: &[LibraryItem]) -> Vec<&str> {
        items.iter().map(|item| item.item_id().as_str()).collect()
    }

    /// DB-005, in the core reading (D-042/D-070: the order is the
    /// canon, the position number is index + 1): a move must answer
    /// with EVERY item exactly once — same count, same multiset — or
    /// the implied `1..=len` numbering has a gap or a duplicate.
    fn assert_full_renumbering(original: &[LibraryItem], moved: &[LibraryItem]) {
        assert_eq!(
            moved.len(),
            original.len(),
            "a move must not change the item count"
        );
        let mut remaining: std::collections::HashMap<&LibraryItem, usize> = Default::default();
        for item in original {
            *remaining.entry(item).or_default() += 1;
        }
        for item in moved {
            let count = remaining.get_mut(item).unwrap_or_else(|| {
                panic!("item {} is not among the original items", item.item_id())
            });
            assert!(
                *count > 0,
                "item {} appears more than once after the move",
                item.item_id()
            );
            *count -= 1;
            assert!(
                *count == 0,
                "item {} appears more than once after the move",
                item.item_id()
            );
        }
        let extra: Vec<&str> = remaining
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(item, _)| item.item_id().as_str())
            .collect();
        assert!(extra.is_empty(), "items lost by the move: {extra:?}");
    }

    // A three-branch tree with a nested subsection and a hidden
    // instruction inside the first branch (D-11: hidden items are
    // ordinary items of the ordered structure — they move along).
    fn three_branch_tree() -> Vec<LibraryItem> {
        vec![
            heading("alpha", 2, "Alpha"),   // 0: branch 0..4
            instruction("a-one"),           // 1
            heading("alpha-sub", 3, "Sub"), // 2: nested branch 2..4
            hidden("a-two"),                // 3
            heading("beta", 2, "Beta"),     // 4: branch 4..6
            instruction("b-one"),           // 5
            heading("gamma", 2, "Gamma"),   // 6: branch 6..8
            instruction("c-one"),           // 7
        ]
    }

    // ---- LIB-034's acceptance criterion: nested order survives -------

    #[test]
    fn moving_a_branch_down_keeps_every_nested_item_in_order() {
        let items = three_branch_tree();
        // Alpha (0..4, with its nested H3 and hidden instruction)
        // moves below Beta: target 6 names the gap before Gamma.
        let moved = move_branch(&items, 0, 6).unwrap();
        assert_eq!(
            ids(&moved),
            [
                "beta",
                "b-one",
                "alpha",
                "a-one",
                "alpha-sub",
                "a-two",
                "gamma",
                "c-one"
            ]
        );
        assert_full_renumbering(&items, &moved);
    }

    #[test]
    fn moving_a_branch_up_keeps_every_nested_item_in_order() {
        let items = three_branch_tree();
        // Gamma (6..8) moves above everything: target 0.
        let moved = move_branch(&items, 6, 0).unwrap();
        assert_eq!(
            ids(&moved),
            [
                "gamma",
                "c-one",
                "alpha",
                "a-one",
                "alpha-sub",
                "a-two",
                "beta",
                "b-one"
            ]
        );
        assert_full_renumbering(&items, &moved);
    }

    #[test]
    fn moving_a_branch_across_several_branches_keeps_it_whole() {
        let items = three_branch_tree();
        // Alpha across both other branches, to the very end.
        let moved = move_branch(&items, 0, 8).unwrap();
        assert_eq!(
            ids(&moved),
            [
                "beta",
                "b-one",
                "gamma",
                "c-one",
                "alpha",
                "a-one",
                "alpha-sub",
                "a-two"
            ]
        );
        assert_full_renumbering(&items, &moved);

        // And Beta across both, to the very top.
        let moved = move_branch(&items, 4, 0).unwrap();
        assert_eq!(
            ids(&moved),
            [
                "beta",
                "b-one",
                "alpha",
                "a-one",
                "alpha-sub",
                "a-two",
                "gamma",
                "c-one"
            ]
        );
        assert_full_renumbering(&items, &moved);
    }

    #[test]
    fn moving_a_nested_branch_into_another_section_keeps_it_whole() {
        // A branch may land inside ANOTHER branch (that is how
        // sections nest); only its own inside is off limits.
        let items = vec![
            heading("x", 2, "X"),      // 0: branch 0..3
            instruction("x1"),         // 1
            instruction("x2"),         // 2
            heading("y", 2, "Y"),      // 3: branch 3..6
            heading("y-sub", 3, "Ys"), // 4: nested branch 4..6
            instruction("ys1"),        // 5
        ];
        // y-sub (4..6) between x1 and x2: target 2 is inside X's
        // branch — legal, it is not inside y-sub's own range.
        let moved = move_branch(&items, 4, 2).unwrap();
        assert_eq!(ids(&moved), ["x", "x1", "y-sub", "ys1", "x2", "y"]);
        assert_full_renumbering(&items, &moved);
    }

    #[test]
    fn every_kind_of_move_answers_with_the_whole_order() {
        // DB-005's "one transaction" lives in the store layer; here
        // the contract is that each move answers with the complete
        // renumbered order — every kind of move over one tree.
        let items = three_branch_tree();
        for (heading_index, target) in [(0, 6), (6, 0), (0, 8), (4, 0), (2, 4), (0, 0), (6, 8)] {
            let moved = move_branch(&items, heading_index, target)
                .unwrap_or_else(|e| panic!("({heading_index}, {target}) should move: {e}"));
            assert_full_renumbering(&items, &moved);
        }
        // A one-item range over an instruction, downward and upward.
        for (start, target) in [(1, 0), (1, 6), (5, 2), (7, 0)] {
            let moved = move_range(&items, start..start + 1, target)
                .unwrap_or_else(|e| panic!("({start}, {target}) should move: {e}"));
            assert_full_renumbering(&items, &moved);
        }
    }

    // ---- ALG-060/LIB-034: inside itself is rejected ---------------------

    #[test]
    fn a_target_strictly_inside_the_range_is_rejected_at_every_position() {
        let items = three_branch_tree();
        // Alpha's branch is 0..4; the gaps at 1, 2 and 3 sit between
        // its own items — the beginning, the middle and the end of
        // the range's inside.
        for target in [1, 2, 3] {
            assert_eq!(
                move_branch(&items, 0, target),
                Err(ReorderError::TargetInsideMovedRange {
                    range: 0..4,
                    target,
                })
            );
        }
        // A nested branch rejects its own inside the same way.
        assert_eq!(
            move_branch(&items, 2, 3),
            Err(ReorderError::TargetInsideMovedRange {
                range: 2..4,
                target: 3,
            })
        );
        // And the primitive form, over an instruction range.
        assert_eq!(
            move_range(&items, 0..3, 2),
            Err(ReorderError::TargetInsideMovedRange {
                range: 0..3,
                target: 2,
            })
        );
    }

    #[test]
    fn a_fat_branch_rejects_every_interior_target() {
        let items = vec![
            heading("a", 2, "A"),      // 0: branch 0..6
            instruction("one"),        // 1
            heading("a-sub", 3, "S"),  // 2: nested 2..6
            instruction("two"),        // 3
            heading("a-deep", 4, "D"), // 4: nested 4..6
            instruction("three"),      // 5
            heading("b", 2, "B"),      // 6
        ];
        for target in 1..6 {
            assert_eq!(
                move_range(&items, 0..6, target),
                Err(ReorderError::TargetInsideMovedRange {
                    range: 0..6,
                    target,
                })
            );
        }
        // Every strictly interior target of the nested branch too.
        for target in 3..6 {
            assert_eq!(
                move_range(&items, 2..6, target),
                Err(ReorderError::TargetInsideMovedRange {
                    range: 2..6,
                    target,
                })
            );
        }
    }

    #[test]
    fn a_one_item_range_has_no_inside_and_moves_everywhere() {
        // ALG-061: "Moving one instruction is a special case of a
        // one-item range" — no integer target is strictly inside
        // i..i+1, so the self-insert rejection has nothing to reject.
        let items = three_branch_tree();
        for index in 0..items.len() {
            for target in 0..=items.len() {
                let moved = move_range(&items, index..index + 1, target)
                    .unwrap_or_else(|e| panic!("({index}, {target}) should move: {e}"));
                assert_full_renumbering(&items, &moved);
            }
        }
    }

    // ---- the boundaries are the branch's place, not its inside ---------

    #[test]
    fn a_ranges_own_boundaries_are_no_ops() {
        let items = three_branch_tree();
        for heading_index in [0, 2, 4, 6] {
            let branch = heading_branch(&items, heading_index).unwrap();
            // "Stay put" and "sit back down after itself".
            assert_eq!(
                move_branch(&items, heading_index, branch.range.start).unwrap(),
                items
            );
            assert_eq!(
                move_branch(&items, heading_index, branch.range.end).unwrap(),
                items
            );
        }
    }

    #[test]
    fn the_first_and_last_branch_move_to_their_own_place_unchanged() {
        let items = three_branch_tree();
        // Alpha (0..4) to the top of the list; Gamma (6..8) to the
        // end — each is already there.
        assert_eq!(move_branch(&items, 0, 0).unwrap(), items);
        assert_eq!(move_branch(&items, 6, 8).unwrap(), items);
    }

    // ---- the list's ends -------------------------------------------------

    #[test]
    fn an_empty_list_rejects_every_range_and_branch() {
        assert_eq!(
            move_range(&[], 0..0, 0),
            Err(ReorderError::InvalidRange {
                start: 0,
                end: 0,
                len: 0
            })
        );
        assert_eq!(
            move_range(&[], 0..1, 0),
            Err(ReorderError::InvalidRange {
                start: 0,
                end: 1,
                len: 0
            })
        );
        assert_eq!(
            move_branch(&[], 0, 0),
            Err(ReorderError::Branch(HeadingBranchError::IndexOutOfBounds {
                index: 0,
                len: 0,
            }))
        );
        assert_eq!(
            move_branch_by_id(&[], &ItemId::new("h".to_owned()), 0),
            Err(ReorderError::Branch(HeadingBranchError::HeadingNotFound {
                id: ItemId::new("h".to_owned()),
            }))
        );
    }

    #[test]
    fn a_single_item_moves_only_to_itself() {
        let items = vec![instruction("only")];
        assert_eq!(move_range(&items, 0..1, 0).unwrap(), items);
        assert_eq!(move_range(&items, 0..1, 1).unwrap(), items);
        assert_eq!(
            move_range(&items, 0..1, 2),
            Err(ReorderError::TargetOutOfBounds { target: 2, len: 1 })
        );
    }

    // ---- errors and precedence -------------------------------------------

    #[test]
    fn the_range_is_checked_before_the_target() {
        let items = vec![instruction("a"), instruction("b")];
        // An invalid range wins over an out-of-bounds target.
        assert_eq!(
            move_range(&items, 2..2, 99),
            Err(ReorderError::InvalidRange {
                start: 2,
                end: 2,
                len: 2
            })
        );
        assert_eq!(
            move_range(&items, 0..3, 0),
            Err(ReorderError::InvalidRange {
                start: 0,
                end: 3,
                len: 2
            })
        );
        // The target bounds win over nothing else: an interior target
        // is always in bounds, so these two never compete.
        assert_eq!(
            move_range(&items, 0..1, 5),
            Err(ReorderError::TargetOutOfBounds { target: 5, len: 2 })
        );
        // Through the branch form the same rejection waits for the
        // branch to resolve first: Beta is a heading, so the target
        // is the thing that is wrong.
        let tree = three_branch_tree();
        assert_eq!(
            move_branch(&tree, 4, 99),
            Err(ReorderError::TargetOutOfBounds { target: 99, len: 8 })
        );
    }

    #[test]
    fn the_branch_resolves_before_the_target_is_checked() {
        let items = vec![instruction("a"), heading("h", 2, "H")];
        // index 0 is an instruction, and the target is nonsense: the
        // branch error comes first — without a branch there is no
        // range to check the target against.
        assert_eq!(
            move_branch(&items, 0, 99),
            Err(ReorderError::Branch(HeadingBranchError::NotAHeading {
                index: 0,
            }))
        );
        assert_eq!(
            move_branch(&items, 9, 0),
            Err(ReorderError::Branch(HeadingBranchError::IndexOutOfBounds {
                index: 9,
                len: 2,
            }))
        );
    }

    #[test]
    fn the_error_messages_name_the_rejected_input() {
        assert_eq!(
            ReorderError::InvalidRange {
                start: 2,
                end: 2,
                len: 5
            }
            .to_string(),
            "moved range 2..2 is not a non-empty range within 5 items"
        );
        assert_eq!(
            ReorderError::TargetOutOfBounds { target: 6, len: 5 }.to_string(),
            "insert target 6 is out of bounds: 5 items allow 0 to 5"
        );
        assert_eq!(
            ReorderError::TargetInsideMovedRange {
                range: 1..4,
                target: 2
            }
            .to_string(),
            "cannot insert the range 1..4 inside itself at 2"
        );
        // The Branch variant is transparent: the wrapped message
        // shows through.
        assert_eq!(
            ReorderError::Branch(HeadingBranchError::NotAHeading { index: 1 }).to_string(),
            "item at index 1 is not a heading"
        );
    }

    // ---- the by-id form ---------------------------------------------------

    #[test]
    fn the_id_form_moves_the_same_branch_as_the_index_form() {
        let items = three_branch_tree();
        let by_index = move_branch(&items, 0, 6).unwrap();
        let by_id = move_branch_by_id(&items, &ItemId::new("alpha".to_owned()), 6).unwrap();
        assert_eq!(by_index, by_id);
    }

    #[test]
    fn the_id_form_rejects_unknown_and_instruction_ids() {
        let items = three_branch_tree();
        assert_eq!(
            move_branch_by_id(&items, &ItemId::new("missing".to_owned()), 0),
            Err(ReorderError::Branch(HeadingBranchError::HeadingNotFound {
                id: ItemId::new("missing".to_owned()),
            }))
        );
        assert_eq!(
            move_branch_by_id(&items, &ItemId::new("a-one".to_owned()), 0),
            Err(ReorderError::Branch(HeadingBranchError::HeadingNotFound {
                id: ItemId::new("a-one".to_owned()),
            }))
        );
    }

    // ---- the T-045 integration: real range, real move, roundtrip ---------

    #[test]
    fn a_move_round_trips_through_the_computed_branch_range() {
        let items = three_branch_tree();
        let alpha = ItemId::new("alpha".to_owned());
        let gamma = ItemId::new("gamma".to_owned());

        // Alpha (0..4, the FIRST branch) down to the end of the list…
        let original_branch = heading_branch(&items, 0).unwrap();
        let moved_down = move_range(&items, original_branch.range.clone(), items.len()).unwrap();
        let moved_branch = heading_branch_by_id(&moved_down, &alpha).unwrap();
        // …the branch arrives whole: same items, same internal order.
        assert_eq!(
            ids(&moved_down[moved_branch.range.clone()]),
            ids(&items[original_branch.range.clone()])
        );
        // …and moving it back to its original place restores the tree.
        assert_eq!(
            move_range(&moved_down, moved_branch.range, original_branch.range.start).unwrap(),
            items
        );

        // Gamma (6..8, the LAST branch) up to the top of the list…
        let original_branch = heading_branch(&items, 6).unwrap();
        let moved_up = move_range(&items, original_branch.range.clone(), 0).unwrap();
        let moved_branch = heading_branch_by_id(&moved_up, &gamma).unwrap();
        assert_eq!(moved_branch.range, 0..original_branch.range.len());
        // …and back: the inverse target sits after the items that
        // were before the branch — the branch's length plus its
        // original start.
        assert_eq!(
            move_range(
                &moved_up,
                moved_branch.range.clone(),
                moved_branch.range.len() + original_branch.range.start
            )
            .unwrap(),
            items
        );
    }
}

/// # Determinism and replay
///
/// The properties below pin the case budget (256) and the RNG seed
/// (the task's number — only its fixedness matters), the run policy
/// `body` and `heading_range` property tests use (D-053, D-070), so
/// every run explores the same trees and a green run cannot flake.
/// Any failure is persisted by proptest into
/// `crates/eunomia-core/proptest-regressions/reorder.txt` and
/// replayed on every later run; a committed regressions file, if one
/// ever appears, is part of the suite.
#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;
    use proptest::test_runner::{Config as ProptestConfig, RngSeed};

    use super::*;
    use crate::heading_range::heading_branch;
    use crate::library::{
        Heading, HeadingLevel, Instruction, InstructionState, Variant, VariantId,
    };

    /// The deterministic run policy: 256 cases, one fixed seed.
    fn config() -> ProptestConfig {
        ProptestConfig {
            cases: 256,
            rng_seed: RngSeed::Fixed(46),
            ..ProptestConfig::default()
        }
    }

    // ---- random trees ----------------------------------------------------

    /// One random row of the structure: a heading of any level, or an
    /// instruction — active or hidden, since hidden items take part
    /// in the geometry identically (D-11).
    fn row() -> impl Strategy<Value = u8> {
        prop_oneof![Just(0u8), 2u8..=6]
    }

    /// Items built from random rows with UNIQUE ids (the row's
    /// position): the properties below track items by id, and unique
    /// ids make every tracked sequence unambiguous.
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

    fn ids(items: &[LibraryItem]) -> Vec<&str> {
        items.iter().map(|item| item.item_id().as_str()).collect()
    }

    proptest! {
        #![proptest_config(config())]

        /// ALG-061 + ALG-060 over every heading, every branch, every
        /// target of a random tree: an interior target is rejected;
        /// every other target answers with the same items, the branch
        /// whole and in order, the untouched items in their relative
        /// order, and the branch sitting after exactly the items the
        /// target leaves before it. One-item ranges (any item, not
        /// only headings — ALG-061's special case) never have an
        /// inside to violate.
        #[test]
        fn for_random_trees_every_move_is_whole_ordered_and_guarded(
            tree in items(),
        ) {
            let len = tree.len();
            let tree_ids = ids(&tree);

            let check = |range: std::ops::Range<usize>, target: usize| {
                let answer = move_range(&tree, range.clone(), target);
                if range.start < target && target < range.end {
                    prop_assert_eq!(
                        answer,
                        Err(ReorderError::TargetInsideMovedRange { range, target })
                    );
                    return Ok(());
                }
                let moved = answer.unwrap_or_else(|e| {
                    panic!("range {range:?} target {target} should move: {e}")
                });
                let moved_ids = ids(&moved);

                // DB-005: every item exactly once — the implied
                // position numbering has no gaps and no duplicates.
                let mut original_sorted = tree_ids.clone();
                original_sorted.sort_unstable();
                let mut moved_sorted = moved_ids.clone();
                moved_sorted.sort_unstable();
                prop_assert_eq!(moved_sorted, original_sorted);

                // LIB-034: the moved range stays whole and in order.
                let branch_len = range.len();
                let branch = &tree_ids[range.clone()];
                let landed = moved_ids
                    .windows(branch_len)
                    .position(|window| window == branch);
                let landed = landed.expect("the moved range must appear whole");
                prop_assert_eq!(&moved_ids[landed..landed + branch_len], branch);

                // Everything the move did not take keeps its relative
                // order.
                let rest: Vec<&str> = moved_ids
                    .iter()
                    .copied()
                    .enumerate()
                    .filter(|(index, _)| *index < landed || *index >= landed + branch_len)
                    .map(|(_, id)| id)
                    .collect();
                let expected_rest: Vec<&str> = tree_ids
                    .iter()
                    .copied()
                    .enumerate()
                    .filter(|(index, _)| !range.contains(index))
                    .map(|(_, id)| id)
                    .collect();
                prop_assert_eq!(rest, expected_rest);

                // The target names the gap it names: exactly the
                // non-range items before it stay before the range.
                let before: Vec<&str> = if target >= range.end {
                    tree_ids[..range.start]
                        .iter()
                        .chain(tree_ids[range.end..target].iter())
                        .copied()
                        .collect()
                } else {
                    tree_ids[..target].to_vec()
                };
                prop_assert_eq!(&moved_ids[..before.len()], before.as_slice());
                Ok(())
            };

            // Every heading's ALG-060 branch, every target.
            for index in 0..len {
                if matches!(tree[index], LibraryItem::Heading(_)) {
                    let branch = heading_branch(&tree, index).unwrap();
                    for target in 0..=len {
                        check(branch.range.clone(), target)?;
                        // The by-id form resolves the same branch and
                        // moves it to the same place: ids are unique
                        // in this generator, so the resolution is
                        // unambiguous.
                        prop_assert_eq!(
                            move_branch_by_id(&tree, tree[index].item_id(), target),
                            move_range(&tree, branch.range.clone(), target)
                        );
                    }
                }
                // ALG-061's special case: one item, no inside.
                for target in 0..=len {
                    check(index..index + 1, target)?;
                }
            }
        }
    }
}
