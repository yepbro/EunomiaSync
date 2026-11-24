//! The ALG-062 level operations: change one heading's level, shift a
//! whole branch's levels (spec §6.9, §7.3, §9.7).
//!
//! ALG-062 (verbatim):
//!
//! > "Shift branch level ±1" changes the levels of all headings in
//! > the branch if all new levels stay within 2–6; otherwise the
//! > operation is rejected with `E_VALIDATION`.
//!
//! LIB-032 (§7.3, the row it shares with LIB-034 and MAIN-018, 019)
//! names both commands:
//!
//! > Levels are not recalculated automatically: to change them,
//! > there are the "Change level" and "Shift branch level ±1"
//! > commands (allowed only if levels stay within 2–6). Skipping a
//! > level (for example, `##` then `####`) is allowed and marked
//! > with a warning icon.
//!
//! # What "Change level" changes — and what it deliberately does not
//!
//! LIB-032's sentence answers the question this module had to settle
//! before anything else: levels are NOT recalculated automatically,
//! so the command exists to change a level, and it changes exactly
//! that — ONE heading's level, in place, point-wise. No accompanying
//! effects fire: the item keeps its id, title and position in the
//! collection (the order is the canon, DB-005/D-042; nothing here
//! renumbers), no branch is rebuilt, no instructions move. The
//! section geometry changes implicitly, the way it always does:
//! ALG-060's ranges derive from the levels, so the next
//! [`heading_branch`](crate::heading_range::heading_branch) call —
//! this sync, this render, this next operation — computes the new
//! shape on demand (T-045's function, never re-implemented here).
//! The dropped-level warning icon of LIB-032 is a UI marker on a
//! legal state ("allowed and marked with a warning icon"), not a
//! validation: a skip such as H2 → H4 passes through unchecked, and
//! core is not where the icon would live anyway.
//!
//! # Shift: the branch, the ±1, and all-or-nothing
//!
//! "Shift branch level ±1" is one command with one operand: the
//! branch ALG-060 computes for the heading (the heading itself, all
//! its items, every nested subsection — hidden instructions ride
//! along untouched, D-11). EVERY heading in that range moves one
//! level; the range's instructions are items of the structure, not
//! headings, and keep everything they have. The delta is the closed
//! set the spec names — exactly `-1` or `+1` — because that is the
//! command the UI offers; any other delta is a caller's arithmetic
//! error and is rejected as such, not silently clamped. The
//! acceptance rule is ALG-062's: if ALL new levels stay within 2–6
//! the shift runs, if ANY would leave the range the operation is
//! rejected — and the §10 error registry's row for `E_VALIDATION`
//! fixes what rejection means: "The whole operation rejected". The
//! check therefore runs over every heading of the branch BEFORE any
//! level is written, and a rejected shift leaves nothing behind —
//! the function is pure, so "nothing changed" is a returned `Err`
//! and an input slice that cannot have moved (§6.9's table is about
//! field validation; this is its spirit applied to a structure
//! operation).
//!
//! The geometry hands the boundaries their shape: a heading strictly
//! inside a branch is always DEEPER than the root (ALG-060 closes at
//! the first heading of the root's level or higher), so the floor
//! case of a `-1` shift — an H2 that cannot go deeper — can only be
//! the ROOT itself, while the ceiling case of a `+1` shift — an H6 —
//! can sit anywhere in the branch. Both are the same rejection.
//!
//! # The E_VALIDATION boundary
//!
//! `E_VALIDATION` is a code of the §10 error registry (§7.10's IPC
//! contract fixes only the `AppError` carrier), which belongs to
//! the Tauri command layer (`AppError`/`ErrorCode`, D-034) — core
//! knows neither. Following the D-062 (`E_MARKERS_INVALID`) and
//! D-071 (`TargetInsideMovedRange`) precedents, this module rejects
//! with its own [`LevelError`]; `LevelOutOfRange` and
//! `BranchLevelOutOfRange` are the core twins of the future
//! command-layer `E_VALIDATION` mapping, and that mapping is a task
//! for whoever lands the command.
//!
//! # Purity
//!
//! Pure functions over `&[LibraryItem]`, in the style the crate
//! settled on for [`heading_range`](crate::heading_range) and
//! [`reorder`](crate::reorder) (D-070, D-071): the input is never
//! touched — both operations return a NEW `Vec` holding the same
//! items with new levels on the affected headings. The shift
//! resolves the ALG-060 branch inside via
//! [`heading_branch`](crate::heading_range::heading_branch), so a
//! caller cannot shift a set of headings that disagrees with the
//! tree.
//!
//! # Examples
//!
//! ```
//! use std::convert::TryFrom;
//!
//! use eunomia_core::heading_range::heading_branch;
//! use eunomia_core::levels::{change_heading_level, shift_branch_level, LevelError};
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
//!     heading("style", 2, "Style"),     // 0: the H2 branch…
//!     instruction("concise"),           // 1: …with a nested H3…
//!     heading("tone", 3, "Tone"),       // 2: …and its item…
//!     instruction("friendly"),          // 3
//!     heading("release", 2, "Release"), // 4: …closing at the next H2
//!     instruction("notes"),             // 5
//! ];
//!
//! // Change level is point-wise: the H3 becomes an H4, and nothing
//! // else moves — the new geometry is computed on demand.
//! let changed = change_heading_level(&items, 2, 4).unwrap();
//! assert_eq!(heading_branch(&changed, 2).unwrap().range, 2..4);
//! assert_eq!(changed[0], items[0]); // untouched: the H2 branch root
//! assert_eq!(changed[1], items[1]); // untouched: the instruction
//!
//! // Levels stay within 2–6, or the operation is rejected whole.
//! assert_eq!(
//!     change_heading_level(&items, 0, 1),
//!     Err(LevelError::LevelOutOfRange { index: 0, value: 1 })
//! );
//!
//! // Shift moves every heading of the branch one level.
//! let shifted = shift_branch_level(&items, 0, 1).unwrap();
//! let level_at = |items: &[LibraryItem], index: usize| match &items[index] {
//!     LibraryItem::Heading(heading) => heading.level.as_u8(),
//!     LibraryItem::Instruction(_) => 0,
//! };
//! assert_eq!(level_at(&shifted, 0), 3); // H2 → H3…
//! assert_eq!(level_at(&shifted, 2), 4); // …and the nested H3 → H4.
//! assert_eq!(shifted[1], items[1]); // instructions ride along untouched
//!
//! // An H6 in the branch stops a +1 shift before anything changes.
//! let deep = vec![
//!     heading("root", 5, "Root"), // 0: the branch…
//!     heading("leaf", 6, "Leaf"), // 1: …holds an H6
//!     heading("next", 2, "Next"), // 2: a neighbor branch
//! ];
//! assert!(matches!(
//!     shift_branch_level(&deep, 0, 1),
//!     Err(LevelError::BranchLevelOutOfRange { .. })
//! ));
//! ```

use crate::heading_range::{HeadingBranch, HeadingBranchError, heading_branch};
use crate::library::{HeadingLevel, LibraryItem};

/// The error [`change_heading_level`] and [`shift_branch_level`]
/// return when the requested operation cannot run.
///
/// # Examples
///
/// ```
/// use eunomia_core::levels::{change_heading_level, shift_branch_level, LevelError};
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
///     change_heading_level(&items, 1, 3),
///     Err(LevelError::IndexOutOfBounds { index: 1, len: 1 })
/// );
/// assert_eq!(
///     change_heading_level(&items, 0, 3),
///     Err(LevelError::NotAHeading { index: 0 })
/// );
/// assert_eq!(
///     shift_branch_level(&items, 0, 0),
///     Err(LevelError::Branch(
///         eunomia_core::heading_range::HeadingBranchError::NotAHeading { index: 0 }
///     ))
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LevelError {
    /// The index does not address an item: it is at least the item
    /// count ([`change_heading_level`] checks its own index; the
    /// shift wraps the same rejection from the branch resolution).
    #[error("heading index {index} is out of bounds: {len} items")]
    IndexOutOfBounds {
        /// The rejected index.
        index: usize,
        /// The number of items that were handed over.
        len: usize,
    },
    /// The index addresses an instruction, and only a heading has a
    /// level to change.
    #[error("item at index {index} is not a heading")]
    NotAHeading {
        /// The index of the instruction.
        index: usize,
    },
    /// The requested level is outside 2–6 (§6.9: "Heading name …
    /// Level 2–6"); the heading keeps its level. This is the core
    /// twin the command layer will map to `E_VALIDATION`.
    #[error(
        "cannot change the heading at index {index} to level {value}: headings are levels 2 to 6"
    )]
    LevelOutOfRange {
        /// The index of the heading whose level was to change.
        index: usize,
        /// The rejected level.
        value: u8,
    },
    /// The shift's delta is not the closed set the spec names:
    /// "Shift branch level ±1" shifts by exactly `-1` or `+1`.
    #[error("branch level shift must be -1 or +1, not {delta}")]
    InvalidDelta {
        /// The rejected delta.
        delta: i8,
    },
    /// At least one heading of the branch would leave 2–6 (ALG-062):
    /// the whole operation is rejected and nothing changes. The
    /// offender is the FIRST such heading in collection order, so
    /// the rejection is deterministic; its current level together
    /// with `delta` names the value that left the range. This is the
    /// core twin the command layer will map to `E_VALIDATION`.
    #[error(
        "shifting the branch of the heading at index {index} by {delta} would take the \
         level-{offender_level} heading at index {offender} outside 2 to 6"
    )]
    BranchLevelOutOfRange {
        /// The index of the heading whose branch was being shifted.
        index: usize,
        /// The requested shift (`-1` or `+1`).
        delta: i8,
        /// The index of the first heading whose new level would leave
        /// 2–6.
        offender: usize,
        /// That heading's current level (always within 2–6).
        offender_level: u8,
    },
    /// The shift could not resolve a branch: a bad index, an
    /// instruction index, or an unknown id — every error
    /// [`heading_branch`](crate::heading_range::heading_branch) can
    /// return. The two index forms of this module surface only
    /// `IndexOutOfBounds` and `NotAHeading` (via `heading_branch`);
    /// `HeadingNotFound` waits for a by-id form, should the command
    /// layer ever add one (D-070/D-071 set that precedent).
    #[error(transparent)]
    Branch(#[from] HeadingBranchError),
}

/// The LIB-032 "Change level" command: set ONE heading's level to
/// `new_level`, point-wise.
///
/// The change is deliberately the whole effect: the heading keeps its
/// id, title and place in the collection (nothing reorders,
/// DB-005/D-042), instructions keep everything they have, and the
/// section geometry is whatever the next
/// [`heading_branch`](crate::heading_range::heading_branch) call
/// computes from the new levels — core holds no cached shape to
/// rebuild. Level skips are legal input (LIB-032: "Skipping a level
/// … is allowed and marked with a warning icon" — the icon is the
/// UI's marker, not a validation). Setting the level the heading
/// already has is a valid no-op: the equal collection comes back.
///
/// The input slice is never modified; the answer is a NEW `Vec`
/// holding the same items, the one heading rebuilt with its new
/// level.
///
/// # Errors
///
/// In order of precedence: [`LevelError::IndexOutOfBounds`] when
/// `index` does not address an item; [`LevelError::NotAHeading`]
/// when it addresses an instruction; [`LevelError::LevelOutOfRange`]
/// when `new_level` is not within 2–6 (§6.9).
///
/// # Examples
///
/// ```
/// use eunomia_core::levels::{change_heading_level, LevelError};
/// use eunomia_core::library::{Heading, HeadingLevel, ItemId, LibraryItem};
///
/// let items = vec![LibraryItem::Heading(Heading {
///     id: ItemId::new("style".to_owned()),
///     level: HeadingLevel::H2,
///     title: "Style".to_owned(),
/// })];
///
/// // A skip straight to the deepest level is allowed.
/// let changed = change_heading_level(&items, 0, 6).unwrap();
/// assert_eq!(
///     changed,
///     vec![LibraryItem::Heading(Heading {
///         id: ItemId::new("style".to_owned()),
///         level: HeadingLevel::H6,
///         title: "Style".to_owned(),
///     })]
/// );
///
/// // The same level is a valid no-op.
/// assert_eq!(change_heading_level(&items, 0, 2).unwrap(), items);
///
/// // Outside 2–6 is rejected — the boundary values included.
/// assert_eq!(
///     change_heading_level(&items, 0, 1),
///     Err(LevelError::LevelOutOfRange { index: 0, value: 1 })
/// );
/// assert_eq!(
///     change_heading_level(&items, 0, 7),
///     Err(LevelError::LevelOutOfRange { index: 0, value: 7 })
/// );
/// ```
pub fn change_heading_level(
    items: &[LibraryItem],
    index: usize,
    new_level: u8,
) -> Result<Vec<LibraryItem>, LevelError> {
    let Some(item) = items.get(index) else {
        return Err(LevelError::IndexOutOfBounds {
            index,
            len: items.len(),
        });
    };
    if !matches!(item, LibraryItem::Heading(_)) {
        return Err(LevelError::NotAHeading { index });
    }
    let new_level = HeadingLevel::try_from(new_level).map_err(|_| LevelError::LevelOutOfRange {
        index,
        value: new_level,
    })?;

    let mut changed = items.to_vec();
    if let LibraryItem::Heading(heading) = &mut changed[index] {
        heading.level = new_level;
    }
    Ok(changed)
}

/// The ALG-062 "Shift branch level ±1": change the levels of ALL
/// headings in the branch of the heading at `index` by `delta`, if
/// every new level stays within 2–6; otherwise reject the whole
/// operation.
///
/// The branch is ALG-060's, resolved inside via
/// [`heading_branch`](crate::heading_range::heading_branch) (the
/// heading itself, its items, every nested subsection — hidden
/// instructions included as items, D-11): every heading in the range
/// shifts one level; every instruction in the range is carried
/// along untouched, and so is everything outside the range. The
/// order never changes — levels are values here, not positions.
///
/// All-or-nothing (ALG-062 + the `E_VALIDATION` row's "The whole
/// operation rejected"): the validation walks every heading of the
/// branch BEFORE any level is written, so a rejected shift mutates
/// nothing — there is no partial state to find. Because a heading
/// strictly inside a branch is always deeper than the root, the
/// floor rejection (a `-1` hitting an H2) can only name the root
/// itself, while the ceiling rejection (a `+1` hitting an H6) can
/// name any heading of the branch.
///
/// `delta` must be exactly `-1` or `+1` — the command the spec names
/// and the UI offers; anything else is rejected as caller arithmetic
/// error, whatever the branch's levels are.
///
/// The input slice is never modified; the answer is a NEW `Vec`
/// holding the same items with the shifted levels.
///
/// # Errors
///
/// In order of precedence: [`LevelError::Branch`] when the branch
/// cannot be resolved (bad index, instruction index, unknown id);
/// [`LevelError::InvalidDelta`] when `delta` is not `-1` or `+1`;
/// [`LevelError::BranchLevelOutOfRange`] when the first heading of
/// the branch (in collection order) whose shifted level would leave
/// 2–6 exists.
///
/// # Examples
///
/// ```
/// use std::convert::TryFrom;
///
/// use eunomia_core::levels::{shift_branch_level, LevelError};
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
/// let items = vec![
///     heading("top", 3, "Top"),   // 0: the branch — an H3…
///     heading("sub", 4, "Sub"),   // 1: …holding an H4…
///     heading("next", 3, "Next"), // 2: …closed by a sibling H3
/// ];
///
/// let shifted = shift_branch_level(&items, 0, 1).unwrap();
/// let levels: Vec<u8> = shifted
///     .iter()
///     .map(|item| match item {
///         LibraryItem::Heading(heading) => heading.level.as_u8(),
///         LibraryItem::Instruction(_) => 0,
///     })
///     .collect();
/// assert_eq!(levels, [4, 5, 3]); // the branch shifted; the sibling stayed
///
/// // Shifted back, the original collection returns.
/// assert_eq!(shift_branch_level(&shifted, 0, -1).unwrap(), items);
///
/// // An H2 root cannot shift up: the whole operation is rejected.
/// let rooted = vec![heading("only", 2, "Only")];
/// assert_eq!(
///     shift_branch_level(&rooted, 0, -1),
///     Err(LevelError::BranchLevelOutOfRange {
///         index: 0,
///         delta: -1,
///         offender: 0,
///         offender_level: 2,
///     })
/// );
/// ```
pub fn shift_branch_level(
    items: &[LibraryItem],
    index: usize,
    delta: i8,
) -> Result<Vec<LibraryItem>, LevelError> {
    let branch = heading_branch(items, index)?;
    if delta != -1 && delta != 1 {
        return Err(LevelError::InvalidDelta { delta });
    }
    shift_branch_validated(items, index, delta, &branch)
}

/// Aligns a whole heading branch so its root has `new_root_level`,
/// preserving every heading's relative depth (ALG-062's automatic
/// drop-on-heading/header case).
///
/// The branch is resolved by the same ALG-060 function as
/// [`shift_branch_level`]. The requested root level must be within 2–6,
/// and every descendant's aligned level must remain within 2–6. The
/// validation pass completes before any item is rebuilt, so a refusal
/// returns the original input untouched. Instructions and neighboring
/// branches keep their values and order.
///
/// # Errors
///
/// [`LevelError::Branch`] when the root cannot be resolved;
/// [`LevelError::LevelOutOfRange`] when `new_root_level` is outside 2–6;
/// [`LevelError::BranchLevelOutOfRange`] when a descendant would leave
/// 2–6. The reported offender is the first in collection order.
pub fn shift_branch_to_level(
    items: &[LibraryItem],
    index: usize,
    new_root_level: u8,
) -> Result<Vec<LibraryItem>, LevelError> {
    let branch = heading_branch(items, index)?;
    let new_root =
        HeadingLevel::try_from(new_root_level).map_err(|_| LevelError::LevelOutOfRange {
            index,
            value: new_root_level,
        })?;
    let difference = i16::from(new_root.as_u8()) - i16::from(branch.level.as_u8());
    let delta = i8::try_from(difference).expect("heading levels 2 to 6 differ by at most 4");
    shift_branch_validated(items, index, delta, &branch)
}

/// The shared all-or-nothing implementation for manual ±1 shifts and
/// automatic drop alignment. The caller has resolved the branch and
/// validated the operation's closed input set.
fn shift_branch_validated(
    items: &[LibraryItem],
    index: usize,
    delta: i8,
    branch: &HeadingBranch,
) -> Result<Vec<LibraryItem>, LevelError> {
    // All-or-nothing (ALG-062): validate EVERY heading of the branch
    // before writing any level, so a rejection leaves nothing behind.
    // The scan runs in collection order, which makes the reported
    // offender the first one — deterministic rejection.
    let mut new_levels: Vec<HeadingLevel> = Vec::new();
    for item_index in branch.range.clone() {
        if let LibraryItem::Heading(heading) = &items[item_index] {
            let Some(shifted_level) = shifted(heading.level, delta) else {
                return Err(LevelError::BranchLevelOutOfRange {
                    index,
                    delta,
                    offender: item_index,
                    offender_level: heading.level.as_u8(),
                });
            };
            new_levels.push(shifted_level);
        }
    }

    let mut shifted_items = items.to_vec();
    let mut new_levels = new_levels.into_iter();
    for item_index in branch.range.clone() {
        if let LibraryItem::Heading(heading) = &mut shifted_items[item_index] {
            heading.level = new_levels
                .next()
                .expect("the validation pass collected a level for every heading");
        }
    }
    Ok(shifted_items)
}

/// The level one ±1 shift lands on: `None` when the result would
/// leave 2–6 (ALG-062's rejection cases — an H2 shifting past the
/// floor, an H6 shifting past the ceiling). Computed in `i16` so the
/// closed-set check reads as arithmetic; `delta` is already validated
/// by the caller (±1 for the manual command, at most ±4 for automatic
/// alignment), so the value is always -2..=10 here.
fn shifted(level: HeadingLevel, delta: i8) -> Option<HeadingLevel> {
    let value = i16::from(level.as_u8()) + i16::from(delta);
    if (2..=6).contains(&value) {
        Some(HeadingLevel::try_from(value as u8).expect("the value is within 2..=6 here"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use super::*;
    use crate::heading_range::{HeadingBranchError, heading_branch};
    use crate::library::{
        Heading, HeadingLevel, Instruction, InstructionState, ItemId, Variant, VariantId,
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

    /// The shape of a tree: one character per item — the level's
    /// digit for a heading, `.` for an instruction (hidden ones ride
    /// along as items; their state never enters the level logic).
    fn shape(items: &[LibraryItem]) -> String {
        items
            .iter()
            .map(|item| match item {
                LibraryItem::Heading(heading) => char::from(b'0' + heading.level.as_u8()),
                LibraryItem::Instruction(_) => '.',
            })
            .collect()
    }

    // The tree the shift tests share: three sibling H2 branches, the
    // first with a nested H3 subsection and a hidden instruction.
    // Shape: "2.3.2.2."
    fn tree() -> Vec<LibraryItem> {
        vec![
            heading("alpha", 2, "Alpha"),   // 0: branch 0..4
            instruction("a-one"),           // 1
            heading("alpha-sub", 3, "Sub"), // 2: nested branch 2..4
            hidden("a-two"),                // 3: hidden, still an item
            heading("beta", 2, "Beta"),     // 4: branch 4..6
            instruction("b-one"),           // 5
            heading("gamma", 2, "Gamma"),   // 6: branch 6..8
            instruction("c-one"),           // 7
        ]
    }

    // ---- change: the successful point-wise moves ------------------------

    #[test]
    fn change_moves_a_heading_to_every_other_level_within_2_to_6() {
        let items = tree();
        // Every target level is legal from every source level — the
        // full 5×5 grid of (source, target) pairs, skips included
        // (LIB-032: "Skipping a level … is allowed").
        for source in 2..=6u8 {
            let mut sourced = items.clone();
            if let LibraryItem::Heading(heading) = &mut sourced[2] {
                heading.level = HeadingLevel::try_from(source).unwrap();
            }
            for target in 2..=6u8 {
                let changed = change_heading_level(&sourced, 2, target)
                    .unwrap_or_else(|e| panic!("({source} → {target}) should change: {e}"));
                let mut expected = sourced.clone();
                if let LibraryItem::Heading(heading) = &mut expected[2] {
                    heading.level = HeadingLevel::try_from(target).unwrap();
                }
                assert_eq!(changed, expected, "({source} → {target})");
            }
        }
    }

    #[test]
    fn change_touches_only_the_one_heading() {
        let items = tree();
        // Beta's level moves one deeper…
        let changed = change_heading_level(&items, 4, 5).unwrap();
        assert_eq!(shape(&changed), "2.3.5.2.");
        // …and every OTHER item is byte-for-byte the item it was:
        // same ids, same titles, same instruction states and texts.
        for (index, (before, after)) in items.iter().zip(&changed).enumerate() {
            if index == 4 {
                assert_ne!(before, after);
            } else {
                assert_eq!(before, after, "item {index} must not change");
            }
        }
    }

    #[test]
    fn change_to_the_current_level_is_a_no_op() {
        let items = tree();
        for index in [0, 2, 4, 6] {
            let current = match &items[index] {
                LibraryItem::Heading(heading) => heading.level.as_u8(),
                LibraryItem::Instruction(_) => unreachable!("the fixture has headings there"),
            };
            assert_eq!(
                change_heading_level(&items, index, current).unwrap(),
                items,
                "index {index}"
            );
        }
    }

    #[test]
    fn change_recalculates_nothing_the_next_branch_call_sees_the_new_geometry() {
        // LIB-032: "Levels are not recalculated automatically" — the
        // command changes the level, and the geometry is derived on
        // demand. Lifting sub (H4 → H3) widens its branch: the H4
        // sibling no longer closes an H3 — only the next H2 does —
        // so the sibling and its instruction join sub's branch
        // without either item moving.
        let items = vec![
            heading("alpha", 2, "Alpha"),     // 0
            heading("sub", 4, "Sub"),         // 1: branch 1..2 as an H4…
            heading("sibling", 4, "Sibling"), // 2: …closed by a same-level H4
            instruction("one"),               // 3
            heading("beta", 2, "Beta"),       // 4
        ];
        assert_eq!(heading_branch(&items, 1).unwrap().range, 1..2);
        let changed = change_heading_level(&items, 1, 3).unwrap();
        // The order did not move — only the level did…
        assert_eq!(
            changed
                .iter()
                .map(|item| item.item_id().as_str())
                .collect::<Vec<_>>(),
            ["alpha", "sub", "sibling", "one", "beta"]
        );
        // …and the recomputed geometry follows the new level.
        assert_eq!(heading_branch(&changed, 1).unwrap().range, 1..4);
    }

    // ---- change: the 2–6 boundaries -------------------------------------

    #[test]
    fn change_rejects_the_levels_below_2() {
        let items = vec![heading("style", 2, "Style")];
        for value in [0u8, 1] {
            assert_eq!(
                change_heading_level(&items, 0, value),
                Err(LevelError::LevelOutOfRange { index: 0, value })
            );
        }
        // The floor itself is legal — the boundary is exclusive.
        assert_eq!(shape(&change_heading_level(&items, 0, 2).unwrap()), "2");
    }

    #[test]
    fn change_rejects_the_levels_above_6() {
        let items = vec![heading("style", 6, "Style")];
        for value in [7u8, 8, 100, 255] {
            assert_eq!(
                change_heading_level(&items, 0, value),
                Err(LevelError::LevelOutOfRange { index: 0, value })
            );
        }
        // The ceiling itself is legal.
        assert_eq!(shape(&change_heading_level(&items, 0, 6).unwrap()), "6");
    }

    #[test]
    fn change_rejects_an_h2_heading_moved_below_2_and_an_h6_above_6() {
        // The DoD's two boundary cases, on a tree where a rejection
        // must leave the WHOLE tree as it was (the heading keeps its
        // level; its branch is not its concern).
        let items = vec![
            heading("top", 2, "Top"),   // 0: the H2 floor…
            heading("deep", 6, "Deep"), // 1: …and the H6 ceiling
            instruction("one"),         // 2
        ];
        assert_eq!(
            change_heading_level(&items, 0, 1),
            Err(LevelError::LevelOutOfRange { index: 0, value: 1 })
        );
        assert_eq!(
            change_heading_level(&items, 1, 7),
            Err(LevelError::LevelOutOfRange { index: 1, value: 7 })
        );
    }

    // ---- change: operands and precedence --------------------------------

    #[test]
    fn change_rejects_a_missing_index_and_an_instruction_index() {
        let items = tree();
        assert_eq!(
            change_heading_level(&items, 8, 3),
            Err(LevelError::IndexOutOfBounds { index: 8, len: 8 })
        );
        assert_eq!(
            change_heading_level(&[], 0, 3),
            Err(LevelError::IndexOutOfBounds { index: 0, len: 0 })
        );
        for index in [1, 3, 5, 7] {
            assert_eq!(
                change_heading_level(&items, index, 3),
                Err(LevelError::NotAHeading { index })
            );
        }
    }

    #[test]
    fn change_checks_the_operand_before_the_level() {
        let items = tree();
        // A missing index wins over a bad level…
        assert_eq!(
            change_heading_level(&items, 99, 1),
            Err(LevelError::IndexOutOfBounds { index: 99, len: 8 })
        );
        // …and an instruction index wins over a bad level too: the
        // heading must exist before there is a level to reject.
        assert_eq!(
            change_heading_level(&items, 1, 1),
            Err(LevelError::NotAHeading { index: 1 })
        );
    }

    // ---- automatic drop alignment: a whole branch to a chosen root level -

    #[test]
    fn aligning_a_branch_preserves_relative_depth_and_every_other_item() {
        let items = tree();
        let aligned = shift_branch_to_level(&items, 0, 4)
            .expect("aligning the H2/H3 branch to H4/H5 stays within 2 to 6");
        assert_eq!(shape(&aligned), "4.5.2.2.");
        for index in [1, 3, 4, 5, 6, 7] {
            assert_eq!(aligned[index], items[index], "item {index} must not change");
        }
    }

    #[test]
    fn aligning_a_branch_rejects_an_out_of_range_target_before_writing() {
        let items = tree();
        for target in [1, 7] {
            assert_eq!(
                shift_branch_to_level(&items, 0, target),
                Err(LevelError::LevelOutOfRange {
                    index: 0,
                    value: target
                })
            );
        }
        assert_eq!(shape(&items), "2.3.2.2.");
    }

    #[test]
    fn aligning_a_branch_rejects_a_descendant_that_would_pass_h6() {
        let items = vec![
            heading("root", 5, "Root"),
            heading("leaf", 6, "Leaf"),
            instruction("one"),
            heading("next", 2, "Next"),
        ];
        assert_eq!(
            shift_branch_to_level(&items, 0, 6),
            Err(LevelError::BranchLevelOutOfRange {
                index: 0,
                delta: 1,
                offender: 1,
                offender_level: 6,
            })
        );
        assert_eq!(shape(&items), "56.2");
    }

    #[test]
    fn aligning_a_branch_to_its_current_level_is_a_no_op() {
        let items = tree();
        assert_eq!(shift_branch_to_level(&items, 0, 2).unwrap(), items);
    }

    // ---- shift: the successful ±1 moves ----------------------------------

    #[test]
    fn shift_moves_every_heading_of_the_branch_one_level() {
        let items = tree();
        // Alpha's branch (0..4) holds the H2 root and the nested H3:
        // both shift, the rest of the tree stays.
        let up = shift_branch_level(&items, 0, 1).unwrap();
        assert_eq!(shape(&up), "3.4.2.2.");
        // A sibling branch shifts on its own just the same.
        let beta_up = shift_branch_level(&items, 4, 1).unwrap();
        assert_eq!(shape(&beta_up), "2.3.3.2.");
    }

    #[test]
    fn shift_up_moves_a_branch_that_fits() {
        let items = tree();
        let up = shift_branch_level(&items, 0, 1).unwrap();
        assert_eq!(shape(&up), "3.4.2.2.");
        // The nested branch shifts with its parent: alpha-sub
        // (H3 → H4) roots a branch of its own now closed only by a
        // heading of level 4 or higher — still the next H2, so the
        // ranges hold.
        assert_eq!(heading_branch(&up, 0).unwrap().range, 0..4);
        assert_eq!(heading_branch(&up, 2).unwrap().range, 2..4);
    }

    #[test]
    fn shift_moves_a_multi_level_branch_whole() {
        // H3 + H4 + H5 under one root: every heading shifts, the
        // instructions among them ride along untouched.
        let items = vec![
            heading("root", 3, "Root"), // 0: branch 0..6
            instruction("one"),         // 1
            heading("mid", 4, "Mid"),   // 2
            hidden("two"),              // 3
            heading("deep", 5, "Deep"), // 4
            instruction("three"),       // 5
            heading("next", 3, "Next"), // 6: the sibling that stays
            instruction("four"),        // 7
        ];
        assert_eq!(heading_branch(&items, 0).unwrap().range, 0..6);

        let up = shift_branch_level(&items, 0, 1).unwrap();
        assert_eq!(shape(&up), "4.5.6.3."); // 3→4, 4→5, 5→6; Next stayed

        // And back down: the original collection returns whole.
        let back = shift_branch_level(&up, 0, -1).unwrap();
        assert_eq!(back, items);
    }

    #[test]
    fn shift_never_touches_instructions_or_neighbour_branches() {
        let items = tree();
        let up = shift_branch_level(&items, 0, 1).unwrap();
        // Only the two headings of alpha's branch differ; the hidden
        // instruction inside, the instructions elsewhere, and both
        // neighbour branches are the items they were.
        for (index, (before, after)) in items.iter().zip(&up).enumerate() {
            if matches!(before, LibraryItem::Heading(_)) && (index == 0 || index == 2) {
                assert_ne!(before, after, "heading {index} should have shifted");
            } else {
                assert_eq!(before, after, "item {index} must not change");
            }
        }
        assert_eq!(up[3], hidden("a-two")); // the hidden item keeps its state
        assert_eq!(up[4], items[4]); // beta, its heading and…
        assert_eq!(up[5], items[5]); // …its instruction stayed
    }

    #[test]
    fn shift_moves_a_lone_heading_branch_of_one() {
        let items = vec![heading("only", 4, "Only")];
        assert_eq!(shape(&shift_branch_level(&items, 0, 1).unwrap()), "5");
        assert_eq!(shape(&shift_branch_level(&items, 0, -1).unwrap()), "3");
    }

    #[test]
    fn shift_round_trips_back_to_the_original_collection() {
        let items = tree();
        for index in [0, 2, 4, 6] {
            let up = shift_branch_level(&items, index, 1).unwrap();
            let back = shift_branch_level(&up, index, -1).unwrap();
            assert_eq!(back, items, "branch at {index}");
        }
        // And the other way round where the floor allows it — the
        // tree's only heading deeper than H2 is alpha-sub, whose
        // H3 branch is closed by an H2 (a close that survives the
        // widening test below).
        let down = shift_branch_level(&items, 2, -1).unwrap();
        let back = shift_branch_level(&down, 2, 1).unwrap();
        assert_eq!(back, items, "branch at 2");
    }

    #[test]
    fn a_down_shift_can_widen_the_branch_so_the_up_shift_covers_more() {
        // The roundtrip asymmetry, pinned: shifting a root shallower
        // can WIDEN its branch — an H4 that closed the H4 root does
        // not close the H3 it becomes — so the +1 that follows
        // covers the widened range too, and the pair is not an
        // identity. The +1-then-−1 of the roundtrip test always is:
        // deepening never widens, because every heading inside the
        // branch deepens with the root and keeps not-closing.
        let items = vec![heading("a", 4, "A"), heading("b", 4, "B")];
        // As H4s, b closes a's branch…
        assert_eq!(heading_branch(&items, 0).unwrap().range, 0..1);
        let down = shift_branch_level(&items, 0, -1).unwrap();
        assert_eq!(shape(&down), "34");
        // …but as the H3 a became, b sits inside the widened branch.
        assert_eq!(heading_branch(&down, 0).unwrap().range, 0..2);
        let back = shift_branch_level(&down, 0, 1).unwrap();
        assert_eq!(shape(&back), "45"); // b shifted too — not the identity
        assert_ne!(back, items);
    }

    #[test]
    fn shift_after_a_change_still_sees_the_new_level() {
        // The levels are values on the items: change alpha to an H3,
        // and a shift of ITS branch now walks the geometry an H3
        // roots — closed by the nested H3, not by beta any more.
        let items = vec![
            heading("a", 2, "A"), // 0
            heading("b", 3, "B"), // 1: inside a's H2 branch…
            instruction("one"),   // 2
            heading("c", 2, "C"), // 3
        ];
        // As an H2, a's branch holds b and one.
        assert_eq!(heading_branch(&items, 0).unwrap().range, 0..3);
        let changed = change_heading_level(&items, 0, 3).unwrap();
        // As an H3, a's branch closes at b — the sibling H3.
        assert_eq!(heading_branch(&changed, 0).unwrap().range, 0..1);
        let shifted = shift_branch_level(&changed, 0, 1).unwrap();
        assert_eq!(shape(&shifted), "43.2"); // only a shifted
    }

    // ---- shift: the 2–6 boundaries, all-or-nothing ------------------------

    #[test]
    fn shift_down_rejects_a_branch_with_an_h2_root() {
        let items = tree();
        // Alpha's branch holds an H2 (the root itself): the whole
        // operation is rejected — nothing changes, nowhere.
        assert_eq!(
            shift_branch_level(&items, 0, -1),
            Err(LevelError::BranchLevelOutOfRange {
                index: 0,
                delta: -1,
                offender: 0,
                offender_level: 2,
            })
        );
    }

    #[test]
    fn shift_down_can_only_reject_on_the_root_the_floor_is_never_nested() {
        // ALG-060 closes a branch at the first heading of the root's
        // level or higher, so a heading strictly inside is always
        // DEEPER than its root — an H2 can never sit inside a branch
        // the way an H6 can. The floor offender is therefore always
        // the root itself; beta's branch proves the point by holding
        // no H2 but the root, while its -1 is rejected anyway.
        let items = vec![
            heading("mid", 3, "Mid"), // 0: a branch without any H2…
            instruction("one"),       // 1: …inside it at all…
            heading("beta", 2, "B"),  // 2: …and an H2 ROOT beside it
        ];
        // Mid's own branch (0..2, closed by beta) shifts down freely.
        assert_eq!(shape(&shift_branch_level(&items, 0, -1).unwrap()), "2.2");
        // Beta's does not — the root is at the floor.
        assert_eq!(
            shift_branch_level(&items, 2, -1),
            Err(LevelError::BranchLevelOutOfRange {
                index: 2,
                delta: -1,
                offender: 2,
                offender_level: 2,
            })
        );
    }

    #[test]
    fn shift_up_rejects_a_branch_with_an_h6_inside() {
        let items = vec![
            heading("root", 3, "Root"),   // 0: branch 0..3
            heading("deep", 5, "Deep"),   // 1: fits at +1…
            heading("ceiling", 6, "Top"), // 2: …but the H6 does not
            heading("next", 3, "Next"),   // 3: the sibling that stays
        ];
        assert_eq!(heading_branch(&items, 0).unwrap().range, 0..3);
        assert_eq!(
            shift_branch_level(&items, 0, 1),
            Err(LevelError::BranchLevelOutOfRange {
                index: 0,
                delta: 1,
                offender: 2,
                offender_level: 6,
            })
        );
        // The H6's own branch (2..3) cannot shift up either.
        assert_eq!(
            shift_branch_level(&items, 2, 1),
            Err(LevelError::BranchLevelOutOfRange {
                index: 2,
                delta: 1,
                offender: 2,
                offender_level: 6,
            })
        );
    }

    #[test]
    fn a_rejected_shift_is_all_or_nothing_over_a_multi_level_branch() {
        // The branch holds headings that COULD shift (H3, H4) and
        // one that cannot (the nested H6): ALG-062 rejects the whole
        // operation — no partial shift of the first ones survives.
        let items = vec![
            heading("root", 3, "Root"), // 0: branch 0..5
            instruction("one"),         // 1
            heading("mid", 4, "Mid"),   // 2
            heading("deep", 6, "Deep"), // 3: the ceiling breaker
            instruction("two"),         // 4
            heading("next", 3, "Next"), // 5
        ];
        let answer = shift_branch_level(&items, 0, 1);
        assert_eq!(
            answer,
            Err(LevelError::BranchLevelOutOfRange {
                index: 0,
                delta: 1,
                offender: 3,
                offender_level: 6,
            })
        );
        // Purity is the "nothing" of all-or-nothing: the input
        // cannot have moved. A fresh clone of the input — the very
        // same items, in the very same order with the very same
        // levels — answers the very same rejection, and the input
        // still sits at its original shape.
        assert_eq!(shift_branch_level(&items.clone(), 0, 1), answer);
        assert_eq!(shape(&items), "3.46.3");
    }

    #[test]
    fn the_reported_offender_is_the_first_in_collection_order() {
        // Two H6 headings sit inside the branch: the one nearer the
        // root is reported — deterministic rejection.
        let items = vec![
            heading("root", 3, "Root"), // 0: branch 0..3
            heading("deep-a", 6, "A"),  // 1: the FIRST offender
            heading("deep-b", 6, "B"),  // 2
            heading("next", 3, "Next"), // 3
        ];
        assert_eq!(
            shift_branch_level(&items, 0, 1),
            Err(LevelError::BranchLevelOutOfRange {
                index: 0,
                delta: 1,
                offender: 1,
                offender_level: 6,
            })
        );
    }

    #[test]
    fn the_boundaries_themselves_are_legal_shift_targets() {
        // H2 is one step from the floor: shifting DOWN is the
        // rejection; H6 is one from the ceiling: shifting UP is.
        // The other directions are the legal boundary moves.
        let at_floor = vec![heading("floor", 2, "Floor")];
        assert!(shift_branch_level(&at_floor, 0, -1).is_err());
        assert_eq!(shape(&shift_branch_level(&at_floor, 0, 1).unwrap()), "3");

        let at_ceiling = vec![heading("ceiling", 6, "Ceiling")];
        assert!(shift_branch_level(&at_ceiling, 0, 1).is_err());
        assert_eq!(shape(&shift_branch_level(&at_ceiling, 0, -1).unwrap()), "5");
    }

    // ---- shift: the closed ±1 set ----------------------------------------

    #[test]
    fn shift_rejects_every_delta_that_is_not_exactly_one_step() {
        let items = tree();
        for delta in [-128i8, -3, -2, 0, 2, 3, 42, 127] {
            assert_eq!(
                shift_branch_level(&items, 0, delta),
                Err(LevelError::InvalidDelta { delta })
            );
        }
    }

    #[test]
    fn shift_resolves_the_branch_before_it_looks_at_the_delta() {
        let items = tree();
        // No branch at a bad index: that rejection comes first —
        // without a branch there is nothing to shift.
        assert_eq!(
            shift_branch_level(&items, 9, 0),
            Err(LevelError::Branch(HeadingBranchError::IndexOutOfBounds {
                index: 9,
                len: 8
            }))
        );
        assert_eq!(
            shift_branch_level(&items, 1, 5),
            Err(LevelError::Branch(HeadingBranchError::NotAHeading {
                index: 1
            }))
        );
    }

    #[test]
    fn shift_checks_the_delta_before_the_levels() {
        let items = tree();
        // Delta 2 on a branch whose H2 would also break the 2–6 rule
        // at +2: the closed-set check wins — it is the caller's
        // error, not the data's.
        assert_eq!(
            shift_branch_level(&items, 0, 2),
            Err(LevelError::InvalidDelta { delta: 2 })
        );
        // Delta 0 on a branch that would happily take either ±1:
        // still rejected — 0 is not a shift.
        assert_eq!(
            shift_branch_level(&items, 2, 0),
            Err(LevelError::InvalidDelta { delta: 0 })
        );
    }

    // ---- errors: messages and the boundary note ---------------------------

    #[test]
    fn the_error_messages_name_the_rejected_input() {
        assert_eq!(
            LevelError::IndexOutOfBounds { index: 9, len: 4 }.to_string(),
            "heading index 9 is out of bounds: 4 items"
        );
        assert_eq!(
            LevelError::NotAHeading { index: 2 }.to_string(),
            "item at index 2 is not a heading"
        );
        assert_eq!(
            LevelError::LevelOutOfRange { index: 3, value: 7 }.to_string(),
            "cannot change the heading at index 3 to level 7: headings are levels 2 to 6"
        );
        assert_eq!(
            LevelError::InvalidDelta { delta: 0 }.to_string(),
            "branch level shift must be -1 or +1, not 0"
        );
        assert_eq!(
            LevelError::BranchLevelOutOfRange {
                index: 1,
                delta: -1,
                offender: 4,
                offender_level: 2
            }
            .to_string(),
            "shifting the branch of the heading at index 1 by -1 would take the level-2 \
             heading at index 4 outside 2 to 6"
        );
        // The Branch variant is transparent: the wrapped message
        // shows through.
        assert_eq!(
            LevelError::Branch(HeadingBranchError::NotAHeading { index: 1 }).to_string(),
            "item at index 1 is not a heading"
        );
    }

    // ---- the T-045 integration: a real range drives the shift -------------

    #[test]
    fn the_shift_walks_exactly_the_range_heading_branch_computes() {
        let items = tree();
        let branch = heading_branch(&items, 0).unwrap();
        assert_eq!(branch.range, 0..4); // a real range, not a guess

        let up = shift_branch_level(&items, 0, 1).unwrap();
        // Every heading inside the range shifted by exactly one…
        for item_index in branch.range.clone() {
            match (&items[item_index], &up[item_index]) {
                (LibraryItem::Heading(before), LibraryItem::Heading(after)) => {
                    assert_eq!(
                        after.level.as_u8(),
                        before.level.as_u8() + 1,
                        "heading at {item_index}"
                    );
                }
                (before, after) => assert_eq!(before, after, "item {item_index}"),
            }
        }
        // …and every item outside it is the very same item.
        for (index, (before, after)) in items
            .iter()
            .zip(&up)
            .enumerate()
            .filter(|(index, _)| !branch.range.contains(index))
        {
            assert_eq!(before, after, "item {index} is outside the branch");
        }

        // The branch of the SAME heading, recomputed on the answer,
        // is rooted one level deeper — and still holds the same
        // items, because levels are values, not positions.
        let recomputed = heading_branch(&up, 0).unwrap();
        assert_eq!(recomputed.level, HeadingLevel::H3);
        assert_eq!(recomputed.range, 0..4);
        assert_eq!(
            up[recomputed.range.clone()]
                .iter()
                .map(|item| item.item_id().as_str())
                .collect::<Vec<_>>(),
            items[branch.range]
                .iter()
                .map(|item| item.item_id().as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_nested_branch_shifts_alone_inside_its_parent() {
        let items = tree();
        // alpha-sub (2..4) shifts without its parent alpha (0..4).
        let shifted = shift_branch_level(&items, 2, 1).unwrap();
        assert_eq!(shape(&shifted), "2.4.2.2.");
        // The parent's branch is unchanged as a RANGE — its nested
        // heading just sits one level deeper inside it.
        assert_eq!(heading_branch(&shifted, 0).unwrap().range, 0..4);
        assert_eq!(heading_branch(&shifted, 2).unwrap().range, 2..4);
    }
}

/// # Determinism and replay
///
/// The properties below pin the case budget (256) and the RNG seed
/// (the task's number — only its fixedness matters), the run policy
/// `body`, `heading_range` and `reorder` property tests use (D-053,
/// D-070, D-071), so every run explores the same trees and a green
/// run cannot flake. Any failure is persisted by proptest into
/// `crates/eunomia-core/proptest-regressions/levels.txt` and replayed
/// on every later run; a committed regressions file, if one ever
/// appears, is part of the suite.
#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;
    use proptest::test_runner::{Config as ProptestConfig, RngSeed};

    use super::*;
    use crate::heading_range::heading_branch;
    use crate::library::{
        Heading, HeadingLevel, Instruction, InstructionState, ItemId, Variant, VariantId,
    };

    /// The deterministic run policy: 256 cases, one fixed seed.
    fn config() -> ProptestConfig {
        ProptestConfig {
            cases: 256,
            rng_seed: RngSeed::Fixed(47),
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

        /// Change level over every heading of a random tree, into
        /// every legal level and out to the illegal ones: a legal
        /// target rebuilds ONLY that heading (every other item the
        /// very same, the order untouched); an illegal target
        /// (everything the closed set excludes) is rejected — and a
        /// skip past a level is as legal as a single step (LIB-032).
        #[test]
        fn for_random_trees_change_is_pointwise_and_bounds_checked(
            tree in items(),
        ) {
            for (index, item) in tree.iter().enumerate() {
                let LibraryItem::Heading(heading) = item else { continue };
                for target in 2u8..=6 {
                    let changed = change_heading_level(&tree, index, target)
                        .unwrap_or_else(|e| panic!("heading {index} → {target} should change: {e}"));
                    prop_assert_eq!(changed.len(), tree.len());
                    for (position, (before, after)) in tree.iter().zip(&changed).enumerate() {
                        if position == index {
                            let LibraryItem::Heading(new) = after else {
                                panic!("the changed item stopped being a heading");
                            };
                            prop_assert_eq!(&new.id, &heading.id);
                            prop_assert_eq!(&new.title, &heading.title);
                            prop_assert_eq!(new.level.as_u8(), target);
                        } else {
                            prop_assert_eq!(before, after, "item {} must not change", position);
                        }
                    }
                }
                for value in [0u8, 1, 7, 8, 99, 255] {
                    prop_assert_eq!(
                        change_heading_level(&tree, index, value),
                        Err(LevelError::LevelOutOfRange { index, value })
                    );
                }
            }
        }

        /// ALG-062 over every heading of a random tree, both deltas:
        /// a branch whose headings all stay within 2–6 shifts every
        /// heading by one and touches nothing else; a branch holding
        /// an H2 (down) or an H6 (up) is rejected WHOLE — with the
        /// FIRST offender in collection order named; and every
        /// successful shift round-trips back to the original
        /// collection.
        #[test]
        fn for_random_trees_shift_is_whole_guarded_and_reversible(
            tree in items(),
        ) {
            for (index, item) in tree.iter().enumerate() {
                let LibraryItem::Heading(_) = item else { continue };
                let branch = heading_branch(&tree, index).unwrap();

                let boundary_level = |delta: i8| if delta == -1 { 2u8 } else { 6u8 };
                let first_at_boundary = |delta: i8| {
                    branch
                        .range
                        .clone()
                        .find(|i| matches!(
                            &tree[*i],
                            LibraryItem::Heading(heading)
                                if heading.level.as_u8() == boundary_level(delta)
                        ))
                };

                for delta in [-1i8, 1] {
                    let answer = shift_branch_level(&tree, index, delta);
                    if let Some(offender) = first_at_boundary(delta) {
                        // Rejected whole: the offender is the first
                        // heading of the branch at the boundary.
                        prop_assert_eq!(
                            answer,
                            Err(LevelError::BranchLevelOutOfRange {
                                index,
                                delta,
                                offender,
                                offender_level: boundary_level(delta),
                            })
                        );
                        continue;
                    }

                    let shifted = answer
                        .unwrap_or_else(|e| panic!("branch {index} by {delta} should shift: {e}"));
                    prop_assert_eq!(shifted.len(), tree.len());
                    for (position, (before, after)) in tree.iter().zip(&shifted).enumerate() {
                        match (before, after) {
                            (LibraryItem::Heading(before), LibraryItem::Heading(after)) => {
                                let expected =
                                    (i16::from(before.level.as_u8()) + i16::from(delta)) as u8;
                                if branch.range.contains(&position) {
                                    prop_assert_eq!(after.level.as_u8(), expected);
                                    prop_assert_eq!(&after.id, &before.id);
                                    prop_assert_eq!(&after.title, &before.title);
                                } else {
                                    prop_assert_eq!(
                                        before,
                                        after,
                                        "heading {} is outside",
                                        position
                                    );
                                }
                            }
                            (before, after) => prop_assert_eq!(before, after),
                        }
                    }

                    // Roundtrip (the DoD's +1-then-−1): deepening
                    // never widens the branch — every inside heading
                    // deepens with the root and keeps not-closing —
                    // so the −1 covers exactly the range the +1
                    // touched, and the original returns. The mirror
                    // pair (−1 then +1) is NOT an identity in
                    // general: a shallower root owns a wider branch
                    // (see the widening unit test below).
                    if delta == 1 {
                        let roundtrip = shift_branch_level(&shifted, index, -delta).unwrap();
                        prop_assert_eq!(roundtrip.as_slice(), tree.as_slice());
                    }
                }
            }
        }

        /// The closed ±1 set: every other delta is rejected for
        /// every heading of every random tree, whatever its levels.
        #[test]
        fn for_random_trees_every_other_delta_is_rejected(
            tree in items(),
        ) {
            for index in 0..tree.len() {
                if !matches!(tree[index], LibraryItem::Heading(_)) { continue; }
                for delta in [-128i8, -3, -2, 0, 2, 3, 127] {
                    prop_assert_eq!(
                        shift_branch_level(&tree, index, delta),
                        Err(LevelError::InvalidDelta { delta })
                    );
                }
            }
        }
    }
}
