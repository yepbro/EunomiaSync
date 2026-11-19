//! The shared instruction library's domain types (spec §2).
//!
//! The library is one ordered structure shared by all projects
//! (§2 "Library item"): its items are instructions or section headings,
//! and every instruction carries its variants (§2 "Instruction",
//! "Variant"). The items arrive in `position` order (DB-005); the
//! position number itself is not repeated inside these types — whoever
//! hands items over (the build snapshot, a future library view)
//! expresses the order as collection order, so the same fact is not
//! stored twice where it could drift.
//!
//! These types carry data only. The §6.9 field rules themselves live
//! next door, in [`crate::library_validators`] (name lengths, the
//! reserved marker lines, the 262,144-character text limit — spec
//! §6.9, DB-006, DB-016); the store applies them at the write
//! boundary, and the cross-row rules (duplicate names) stay the
//! store's own job, so core receives rows that already hold. Per
//! spec SEC-008, variant text never goes into logs; that is
//! a logging-side discipline, and this type simply carries the data.

use std::convert::TryFrom;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The identifier of a library item (an instruction or a heading;
/// spec DB-002).
///
/// An opaque TEXT identifier, like [`super::project::ProjectId`]: the
/// UUIDv7 form is generated and owned by the store layer, never
/// validated here. Assignments reference instructions by this id
/// (headings cannot be assigned).
///
/// # Examples
///
/// ```
/// use eunomia_core::library::ItemId;
///
/// let id = ItemId::new("0193f1aa-6d6d-7cce-a3c0-3d1f2a4b5c6e".to_owned());
/// assert_eq!(id.as_str(), "0193f1aa-6d6d-7cce-a3c0-3d1f2a4b5c6e");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ItemId(String);

impl ItemId {
    /// Wraps an identifier string.
    #[must_use]
    pub fn new(id: String) -> Self {
        Self(id)
    }

    /// The identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ItemId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

/// The identifier of a variant (spec DB-002).
///
/// An opaque TEXT identifier; see [`ItemId`] for the boundary policy.
///
/// # Examples
///
/// ```
/// use eunomia_core::library::VariantId;
///
/// let id = VariantId::new("0193f1ab-6d6d-7cce-a3c0-3d1f2a4b5c6f".to_owned());
/// assert_eq!(id.as_str(), "0193f1ab-6d6d-7cce-a3c0-3d1f2a4b5c6f");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VariantId(String);

impl VariantId {
    /// Wraps an identifier string.
    #[must_use]
    pub fn new(id: String) -> Self {
        Self(id)
    }

    /// The identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VariantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for VariantId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

/// The error returned when a number is not a valid heading level.
///
/// # Examples
///
/// ```
/// use eunomia_core::library::InvalidHeadingLevel;
///
/// let error = InvalidHeadingLevel { value: 7 };
/// assert_eq!(error.to_string(), "invalid heading level 7: headings are levels 2 to 6");
/// assert_eq!(error.value, 7);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid heading level {value}: headings are levels 2 to 6")]
pub struct InvalidHeadingLevel {
    /// The rejected value.
    pub value: u8,
}

/// A section heading's level, `##` to `######` (spec §2 "Section
/// heading").
///
/// Levels 2 to 6 are the whole closed set — schema §6.2
/// `heading_level BETWEEN 2 AND 6` — so 1 and 7 are unrepresentable
/// here and [`TryFrom<u8>`] is the boundary conversion for the store's
/// integer column. The serde form is that canonical integer.
///
/// # Examples
///
/// ```
/// use std::convert::TryFrom;
///
/// use eunomia_core::library::HeadingLevel;
///
/// assert_eq!(u8::from(HeadingLevel::H2), 2);
/// assert_eq!(HeadingLevel::try_from(6), Ok(HeadingLevel::H6));
/// assert!(HeadingLevel::try_from(1).is_err());
/// assert!(HeadingLevel::try_from(7).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "u8", try_from = "u8")]
pub enum HeadingLevel {
    /// `##`
    H2,
    /// `###`
    H3,
    /// `####`
    H4,
    /// `#####`
    H5,
    /// `######`
    H6,
}

impl HeadingLevel {
    /// The canonical schema integer.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::H2 => 2,
            Self::H3 => 3,
            Self::H4 => 4,
            Self::H5 => 5,
            Self::H6 => 6,
        }
    }
}

impl From<HeadingLevel> for u8 {
    fn from(level: HeadingLevel) -> Self {
        level.as_u8()
    }
}

impl TryFrom<u8> for HeadingLevel {
    type Error = InvalidHeadingLevel;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            2 => Ok(Self::H2),
            3 => Ok(Self::H3),
            4 => Ok(Self::H4),
            5 => Ok(Self::H5),
            6 => Ok(Self::H6),
            value => Err(InvalidHeadingLevel { value }),
        }
    }
}

/// The visibility state of an instruction (spec §2, D-11).
///
/// The serde form is the schema §6.2 `library_items.state` CHECK
/// literal (the `hidden` member; headings are always active, so only
/// instructions carry a state).
///
/// # Examples
///
/// ```
/// use eunomia_core::library::InstructionState;
///
/// assert_eq!(InstructionState::Active.as_str(), "active");
/// assert_eq!(serde_json::to_string(&InstructionState::Hidden).unwrap(), "\"hidden\"");
/// assert!(serde_json::from_str::<InstructionState>("\"Deleted\"").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstructionState {
    /// Visible and part of the build.
    #[serde(rename = "active")]
    Active,
    /// Hidden: assignments are kept, but the instruction is not
    /// included in the build (D-11, DB-013).
    #[serde(rename = "hidden")]
    Hidden,
}

impl InstructionState {
    /// The canonical schema literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Hidden => "hidden",
        }
    }
}

/// One variant of an instruction: its own name and text (spec §2
/// "Variant").
///
/// `content` is the text as stored — the build pipeline normalizes it
/// per ALG-002 (CRLF/CR to LF, BOM and edge whitespace dropped) before
/// it reaches a file; the snapshot deliberately carries the raw stored
/// form. An empty content is valid: an empty variant produces no block
/// but still counts as assigned and keeps its heading in the file
/// (D-13). The save-time limits (NUL, reserved marker lines, 262,144
/// characters — §6.9, DB-006, DB-016) are enforced by the store and
/// IPC layers, not here.
///
/// The main-variant flag is deliberately absent: nothing in the build
/// reads it, and the "exactly one active main variant" invariant
/// (DB-010) is checked by the store.
///
/// # Examples
///
/// ```
/// use eunomia_core::library::{Variant, VariantId};
///
/// let variant = Variant {
///     id: VariantId::new("v1".to_owned()),
///     name: "Main".to_owned(),
///     content: "Be brief.".to_owned(),
/// };
/// assert_eq!(variant.name, "Main");
/// assert_eq!(variant.content, "Be brief.");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Variant {
    /// The variant's identifier.
    pub id: VariantId,
    /// The variant's name (unique within the instruction,
    /// case-insensitive — §6.9).
    pub name: String,
    /// The variant's text, as stored.
    pub content: String,
}

/// A section heading library item (spec §2 "Section heading").
///
/// Headings have no assignment cells and no variants; they structure
/// the file per ALG-020: a heading reaches the file when its section
/// contains at least one assigned instruction.
///
/// # Examples
///
/// ```
/// use std::convert::TryFrom;
///
/// use eunomia_core::library::{Heading, HeadingLevel, ItemId};
///
/// let heading = Heading {
///     id: ItemId::new("h1".to_owned()),
///     level: HeadingLevel::try_from(2).unwrap(),
///     title: "Style".to_owned(),
/// };
/// assert_eq!(heading.title, "Style");
/// assert_eq!(u8::from(heading.level), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Heading {
    /// The item's identifier.
    pub id: ItemId,
    /// The heading's level, `##`…`######`.
    #[serde(rename = "heading_level")]
    pub level: HeadingLevel,
    /// The heading's title (1–200 characters, one line — §6.9).
    pub title: String,
}

/// An instruction library item (spec §2 "Instruction").
///
/// Carries the active variants in the order the store returns them
/// (the schema defines no variant order). For an active instruction
/// there is at least one active variant (DB-010); the build snapshot
/// re-checks that when it assembles.
///
/// # Examples
///
/// ```
/// use eunomia_core::library::{Instruction, InstructionState, ItemId, Variant, VariantId};
///
/// let instruction = Instruction {
///     id: ItemId::new("i1".to_owned()),
///     title: "Be concise".to_owned(),
///     state: InstructionState::Active,
///     variants: vec![Variant {
///         id: VariantId::new("v1".to_owned()),
///         name: "Main".to_owned(),
///         content: "Be brief.".to_owned(),
///     }],
/// };
/// assert!(instruction.is_active());
/// assert_eq!(instruction.variants.len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Instruction {
    /// The item's identifier.
    pub id: ItemId,
    /// The instruction's main name (1–200 characters, one line,
    /// unique among visible instructions — §6.9). The name never goes
    /// into a built file (ALG-022).
    pub title: String,
    /// The visibility state; hidden instructions keep their
    /// assignments but are excluded from the build (D-11, DB-013).
    pub state: InstructionState,
    /// The instruction's active variants, in the order the store
    /// returns them (the schema defines no variant order; DB-005
    /// orders library items, not variants).
    pub variants: Vec<Variant>,
}

impl Instruction {
    /// Whether the instruction is active (part of the build).
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.state, InstructionState::Active)
    }
}

/// One entry of the shared ordered library structure (spec §2 "Library
/// item"): an instruction or a section heading.
///
/// The serde tag is the schema §6.2 `library_items.kind` CHECK literal
/// (`"instruction"` / `"heading"`).
///
/// # Examples
///
/// ```
/// use eunomia_core::library::{Heading, HeadingLevel, ItemId, LibraryItem};
///
/// let heading = LibraryItem::Heading(Heading {
///     id: ItemId::new("h1".to_owned()),
///     level: HeadingLevel::H2,
///     title: "Style".to_owned(),
/// });
/// assert!(matches!(heading, LibraryItem::Heading(_)));
/// assert_eq!(heading.item_id().as_str(), "h1");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LibraryItem {
    /// A section heading.
    #[serde(rename = "heading")]
    Heading(Heading),
    /// An instruction with its variants.
    #[serde(rename = "instruction")]
    Instruction(Instruction),
}

impl LibraryItem {
    /// The item's identifier, whatever its kind.
    #[must_use]
    pub fn item_id(&self) -> &ItemId {
        match self {
            Self::Heading(heading) => &heading.id,
            Self::Instruction(instruction) => &instruction.id,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use super::*;

    #[test]
    fn id_newtypes_wrap_and_display() {
        let item = ItemId::new("item-1".to_owned());
        let variant = VariantId::new("variant-1".to_owned());
        assert_eq!(item.as_str(), "item-1");
        assert_eq!(variant.as_str(), "variant-1");
        assert_eq!(item.to_string(), "item-1");
        assert_eq!(variant.to_string(), "variant-1");
    }

    #[test]
    fn heading_level_covers_exactly_levels_2_to_6() {
        for value in 2..=6 {
            let level = HeadingLevel::try_from(value)
                .unwrap_or_else(|e| panic!("level {value} should be valid: {e}"));
            assert_eq!(u8::from(level), value);
            assert_eq!(level.as_u8(), value);
        }
        for value in [0u8, 1, 7, 255] {
            assert_eq!(
                HeadingLevel::try_from(value),
                Err(InvalidHeadingLevel { value })
            );
        }
        assert_eq!(
            InvalidHeadingLevel { value: 7 }.to_string(),
            "invalid heading level 7: headings are levels 2 to 6"
        );
    }

    #[test]
    fn heading_level_serializes_as_the_schema_integer() {
        assert_eq!(serde_json::to_string(&HeadingLevel::H2).unwrap(), "2");
        assert_eq!(serde_json::to_string(&HeadingLevel::H6).unwrap(), "6");
        assert_eq!(
            serde_json::from_str::<HeadingLevel>("5").unwrap(),
            HeadingLevel::H5
        );
        assert!(serde_json::from_str::<HeadingLevel>("1").is_err());
        assert!(serde_json::from_str::<HeadingLevel>("7").is_err());
        // The integer form, not a string.
        assert!(serde_json::from_str::<HeadingLevel>("\"2\"").is_err());
    }

    #[test]
    fn instruction_state_literals_round_trip() {
        for (state, literal) in [
            (InstructionState::Active, "active"),
            (InstructionState::Hidden, "hidden"),
        ] {
            assert_eq!(state.as_str(), literal);
            assert_eq!(
                serde_json::to_string(&state).unwrap(),
                format!("\"{literal}\"")
            );
            assert_eq!(
                serde_json::from_str::<InstructionState>(&format!("\"{literal}\"")).unwrap(),
                state
            );
        }
        assert!(serde_json::from_str::<InstructionState>("\"Active\"").is_err());
        assert!(serde_json::from_str::<InstructionState>("\"deleted\"").is_err());
    }

    #[test]
    fn library_item_serializes_with_the_schema_kind_word() {
        let heading = LibraryItem::Heading(Heading {
            id: ItemId::new("h1".to_owned()),
            level: HeadingLevel::H3,
            title: "Tests".to_owned(),
        });
        let json = serde_json::to_string(&heading).unwrap();
        assert!(json.starts_with(r#"{"heading":"#));
        assert!(json.contains(r#""heading_level":3"#));
        assert_eq!(serde_json::from_str::<LibraryItem>(&json).unwrap(), heading);

        let instruction = LibraryItem::Instruction(Instruction {
            id: ItemId::new("i1".to_owned()),
            title: "Be concise".to_owned(),
            state: InstructionState::Hidden,
            variants: vec![],
        });
        let json = serde_json::to_string(&instruction).unwrap();
        assert!(json.starts_with(r#"{"instruction":"#));
        assert_eq!(
            serde_json::from_str::<LibraryItem>(&json).unwrap(),
            instruction
        );
        // No other kind word round-trips.
        assert!(serde_json::from_str::<LibraryItem>(r#"{"note":{}}"#).is_err());
    }

    #[test]
    fn item_id_reads_through_either_kind() {
        let heading = LibraryItem::Heading(Heading {
            id: ItemId::new("h1".to_owned()),
            level: HeadingLevel::H2,
            title: "Style".to_owned(),
        });
        let instruction = LibraryItem::Instruction(Instruction {
            id: ItemId::new("i1".to_owned()),
            title: "Be concise".to_owned(),
            state: InstructionState::Active,
            variants: vec![],
        });
        assert_eq!(heading.item_id().as_str(), "h1");
        assert_eq!(instruction.item_id().as_str(), "i1");
    }

    #[test]
    fn instruction_state_drives_is_active() {
        let mut instruction = Instruction {
            id: ItemId::new("i1".to_owned()),
            title: "Be concise".to_owned(),
            state: InstructionState::Active,
            variants: vec![],
        };
        assert!(instruction.is_active());
        instruction.state = InstructionState::Hidden;
        assert!(!instruction.is_active());
    }
}
