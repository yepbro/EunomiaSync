//! Project field validators: the §6.9 rules of the five editable
//! project fields — name, label, description, color and file name —
//! that the "Project settings" modal saves in one transaction (spec
//! §7.1, the PRJ-008, 009, 011, 021 row), that a new project's
//! fielded copy carries (PRJ-012), and the DB-042 default label a
//! new project's label field starts from ([`default_label`]).
//!
//! Each rule is transcribed next to the schema CHECK (§6.2) that
//! stores the same verdict, so a value that passes here never fails
//! the database:
//!
//! | Field     | §6.9 rule                                        | schema CHECK (§6.2)                            | this module returns                       |
//! | --------- | ------------------------------------------------ | ---------------------------------------------- | ----------------------------------------- |
//! | Name      | 1–100 characters after trimming spaces           | `length(trim(name)) BETWEEN 1 AND 100`         | the TRIMMED name                          |
//! | Label     | 1–6 visible characters, case is kept             | `length(label) BETWEEN 1 AND 6`                | the label, unchanged                      |
//! | Description | up to 1000 characters, NUL silently stripped   | `length(description) <= 1000`                  | the description, NUL stripped             |
//! | Color     | a value from the palette                         | none — `color TEXT NOT NULL` only              | the [`Color`]                             |
//! | File name | a value from the list                            | `base_name IN ('AGENTS.md','CLAUDE.md')`       | the [`BaseName`]                          |
//!
//! The counting unit everywhere is the Unicode scalar value
//! (`str::chars().count()` — code points), the same unit SQLite's
//! `length()` uses on TEXT: not bytes (`"é"` is one, not two) and
//! not graphemes (`"e\u{301}"` is two, not one).
//!
//! Two §6.9 rows are deliberately NOT here: the project PATH row is
//! file-system access (a `.git` check, readability — the fs crate's
//! job; core does no I/O, D-006), and the name's "duplicates are
//! allowed with a warning" clause is a cross-row query with a warning
//! severity, not a field rule — the store layer's job.
//!
//! # The name: trim, then count
//!
//! "1–100 characters after trimming spaces" reads in that order:
//! trim first, then the length. The trim is Rust's `str::trim` (all
//! Unicode whitespace), and [`validate_name`] returns the trimmed
//! string — the value to persist. That contract is what keeps the app
//! rule and the schema CHECK in exact agreement: SQLite's `trim()`
//! strips only U+0020, a SUBSET of what Rust strips, so a name
//! validated here and stored as returned satisfies
//! `length(trim(name)) BETWEEN 1 AND 100` with the same length the
//! validator measured. A name made entirely of non-U+0020 whitespace
//! (a tab, an NBSP) is rejected here as empty — stricter than the
//! bare CHECK, and the point of the rule: a name is not stored as
//! nothing-but-whitespace. (The row reserves no other
//! control-character rule for names, but `NUL` is rejected anywhere
//! it sits — the D-126 validators-wide ruling on D-076's open item,
//! the same one the library name fields carry: SQLite's `length()`
//! counts only up to the first `U+0000`, so a NUL-first name would
//! pass the length check while the store's own CHECK measures it as
//! 0. The label needs no NUL rule of its own: D-075's
//! control-character rejection already refuses `U+0000`.)
//!
//! # The label: what "visible" means
//!
//! §6.9 says "1–6 visible characters, case is kept" without defining
//! "visible". The ruling implemented here (DECISIONS.md D-075):
//!
//! - a character is VISIBLE when it is neither Unicode whitespace
//!   (`char::is_whitespace`) nor a control character
//!   (`char::is_control`);
//! - inner spaces stay legal — DB-042 derives the default label from
//!   the first up to 6 characters of the name, in upper case, and
//!   names contain spaces ("My Project" → "MY PRO"), while the label
//!   always sits next to the color as the real sign, never alone
//!   (DB-041);
//! - at least one character must be visible (DB-042: "An empty label
//!   is not allowed" — an empty or whitespace-only label renders
//!   nothing next to the color);
//! - a control character is rejected wherever it sits: it renders
//!   nothing and breaks the one-line tag;
//! - the TOTAL length — spaces included — stays within 6, exactly the
//!   schema's `length(label) BETWEEN 1 AND 6`, so whatever passes
//!   here passes the database unchanged.
//!
//! Case is kept: [`validate_label`] returns its input slice on
//! success — validation, not normalization. No trimming (the row
//! does not ask for one) and no upper-casing (DB-042's derivation is
//! the UI's default for a NEW label, not a rule every label must
//! follow).
//!
//! std's predicates stop at the Cc and White_Space tables, so three
//! near-invisible families ride along as legal input, each exactly
//! as it passes the schema's bare length CHECK: format characters
//! (Unicode Cf — a zero-width space, a ZWJ) are neither control nor
//! whitespace and so count as VISIBLE; the line and paragraph
//! separators (U+2028/U+2029, Zl/Zp) are whitespace and so ride like
//! a space; and a lone combining mark counts as visible. Separating
//! them needs a Unicode data crate outside the dependency allow-list
//! (D-017); that refinement waits for a consumer that needs it.
//!
//! # The default label: DB-042's derivation
//!
//! "The default label is the first up to 6 characters of the name, in
//! upper case. An empty label is not allowed." The sentence splits in
//! two here: the derivation is [`default_label`], the prohibition is
//! [`validate_label`]'s `LabelEmpty` — the derivation produces, the
//! validator gates.
//!
//! "Up to 6 characters" describes the RESULT — the default label is
//! a thing of at most 6 characters, upper case — so the bound must
//! hold AFTER the upper-casing, and the order is upper first, slice
//! second. The order only shows in Unicode, because upper-casing
//! EXPANDS: `"ß".to_uppercase()` is `"SS"`, one character becoming
//! two, and `"ΐ".to_uppercase()` is three code points. Slicing
//! first would let six name characters surface as up to EIGHTEEN
//! label code points (`"ΐΐΐΐΐΐ"` → 18, `"ßßßßßß"` → 12,
//! `"SSSSSSSSSSSS"`) — a "default" the §6.9 rule and the schema's
//! `length(label) BETWEEN 1 AND 6` both reject, an unstorable
//! default the spec cannot mean. Upper-then-slice keeps every
//! result within 6 and loses nothing of the letter: an upper-cased
//! name is its characters' upper-casings concatenated, so the first
//! 6 code points of that stream are exactly slice-then-upper
//! clipped back to 6. The `straße` test pins the order
//! (`"straße"` → `"STRASS"`, six — not the seven of `"STRASSE"`).
//!
//! The unit is the code point (D-075): six `chars()`, not bytes, not
//! graphemes — `ПРОЕКТ` fits, six astral emoji fit (24 bytes, 6 code
//! points), and a combining mark rides its base letter. The input is
//! the NAME as stored — the trimmed string [`validate_name`]
//! returns — and [`default_label`] trims its input itself, so a raw
//! caller and the validated pipeline derive the same label.
//!
//! The function is total: the empty string (or pure whitespace) in,
//! the empty string out, no panic. For a name [`validate_name`]
//! accepted the derived STRING is never empty — at least one code
//! point always survives — and upper-casing never deletes a visible
//! character or turns it invisible (a full scan over every scalar
//! value pins the lemma). The result has a VISIBLE character unless
//! the consumed prefix has none: after the trim the first character
//! is never whitespace, but it can be a control, and the rest of
//! the prefix can be controls or inner spaces — all invisible
//! (names reserve no control rule beyond `NUL` — D-075/D-076's
//! posture, extended by D-126 — and trim removes only the edges).
//! So `"\u{7}"` and `"\u{7} \u{8}"` are legal names whose
//! derived defaults render nothing and are rejected by
//! [`validate_label`] as `LabelEmpty`, while `"\u{7}abc"` derives
//! `"\u{7}ABC"` and fails the control check instead. The derivation
//! stays literal —
//! filtering would invent a rule DB-042 does not state — and the
//! modal shows the field error for the user to edit; both
//! counterexamples are pinned as the contract's recorded edge.
//!
//! # The description: NUL stripped, then measured
//!
//! "Up to 1000 characters" with no trim mentioned and no trim in the
//! CHECK: [`validate_description`] keeps the edges and trims nothing.
//! The one transformation is the SILENT NUL strip — the owner's
//! ruling that closed D-151 item 7 (2026-11-22): every `U+0000` is
//! removed from the value to persist, and the length is measured
//! AFTER the strip, so the ≤1000 limit governs the STORED
//! description (1001 characters including one NUL save as 1000).
//! The strip never rejects: a description is inert prose for the
//! stored row, and an invisible byte is not user intent — the exact
//! contrast with the names above, where the same byte is an outright
//! rejection (D-126: a name is file-name and file-content material).
//! Empty after the strip is the same legal empty description as
//! before (the column's default is `''`, §6.2), and newlines are
//! prose here, not layout damage.
//!
//! # The color and the file name: closed lists
//!
//! The palette is the fixed 12 colors of DB-040, and the §6.2 column
//! carries NO CHECK on them — [`validate_color`] is the only gate a
//! stored color ever passes. The file name is the §2 base-name list
//! (`AGENTS.md` or `CLAUDE.md`; the §6.9 note "path and extension
//! are not typed in" describes the picker — validation is plain list
//! membership, case-sensitive like the schema literals).
//!
//! # Errors
//!
//! Every failure is a typed [`ProjectFieldError`] variant naming the
//! field and the broken rule — never a string — so the command layer
//! can map any of them to `E_VALIDATION` (§8; the D-062/D-071/D-072
//! precedent of typed errors at core boundaries).
//!
//! # Examples
//!
//! The five fields of a "Project settings" save (PRJ-008), each at a
//! legal value:
//!
//! ```
//! use eunomia_core::project::{BaseName, Color};
//! use eunomia_core::project_validators::{
//!     default_label, validate_color, validate_description, validate_file_name, validate_label,
//!     validate_name,
//! };
//!
//! let name = validate_name("  EunomiaSync  ").unwrap(); // trimmed…
//! assert_eq!(name, "EunomiaSync");
//! assert_eq!(default_label(&name), "EUNOMI"); // …DB-042's derivation
//! assert_eq!(validate_label("MY PRO").unwrap(), "MY PRO"); // …case kept
//! assert_eq!(validate_description("").unwrap(), ""); // empty is legal
//! assert_eq!(validate_color("teal").unwrap(), Color::Teal);
//! assert_eq!(validate_file_name("CLAUDE.md").unwrap(), BaseName::ClaudeMd);
//!
//! // And the failures are typed, field by field:
//! use eunomia_core::project_validators::ProjectFieldError;
//!
//! assert_eq!(validate_name("   ").unwrap_err(), ProjectFieldError::NameEmpty);
//! assert_eq!(
//!     validate_label("ABCDEFG").unwrap_err(),
//!     ProjectFieldError::LabelTooLong { length: 7 }
//! );
//! ```

use thiserror::Error;

use crate::project::{BaseName, Color};

/// A project field that failed its §6.9 rule: which field, and what
/// is wrong with it.
///
/// One variant per field and failure — the command layer maps any of
/// them to `E_VALIDATION` (§8) with the field-scoped message; the
/// variants carry the measured length (or the offending character,
/// or the unknown value) so the UI can say exactly that.
///
/// # Examples
///
/// ```
/// use eunomia_core::project_validators::{validate_description, ProjectFieldError};
///
/// let error = validate_description(&"x".repeat(1001)).unwrap_err();
/// assert_eq!(
///     error,
///     ProjectFieldError::DescriptionTooLong { length: 1001 }
/// );
/// assert_eq!(
///     error.to_string(),
///     "project description is 1001 characters; the limit is 1000"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectFieldError {
    /// The name is empty — the empty string, or nothing but
    /// whitespace, after trimming (§6.9: 1–100 after trimming).
    #[error("project name is empty after trimming")]
    NameEmpty,
    /// The name is longer than 100 characters after trimming; the
    /// length is the trimmed, code-point count.
    #[error("project name is {length} characters after trimming; the limit is 100")]
    NameTooLong {
        /// The trimmed length that exceeded the limit.
        length: usize,
    },
    /// The name contains `NUL` (`U+0000`) at `index`. Not a §6.9
    /// clause — the D-126 validators-wide ruling on D-076's open
    /// item (the library name fields carry it too): SQLite's
    /// `length()` counts only up to the first `U+0000`, so a
    /// NUL-first name would slip past the length checks while the
    /// store's own `length(trim(name))` CHECK measures it as 0.
    #[error("project name contains NUL at index {index}")]
    NameNul {
        /// The code-point index in the TRIMMED value the validator
        /// measured (NUL is not whitespace, so the trim never
        /// removes one).
        index: usize,
    },
    /// The label renders nothing: it is empty, or nothing but
    /// whitespace (DB-042: "An empty label is not allowed").
    #[error("project label has no visible characters")]
    LabelEmpty,
    /// The label is longer than 6 characters in total, spaces
    /// included — the same bound the schema CHECK measures.
    #[error("project label is {length} characters; the limit is 6")]
    LabelTooLong {
        /// The total length that exceeded the limit.
        length: usize,
    },
    /// The label contains a control character (Unicode Cc) at
    /// `index` — it renders nothing and breaks the one-line tag.
    #[error("project label contains the control character {character:?} at index {index}")]
    LabelControlCharacter {
        /// The code-point index the character sits at.
        index: usize,
        /// The offending character.
        character: char,
    },
    /// The description is longer than 1000 characters after the
    /// silent NUL strip — the measured value is the one that would be
    /// stored.
    #[error("project description is {length} characters; the limit is 1000")]
    DescriptionTooLong {
        /// The post-strip length that exceeded the limit — the stored
        /// truth, not the typed one.
        length: usize,
    },
    /// The color is not one of the 12 palette colors (DB-040) — the
    /// §6.2 column has no CHECK, so this validator is the only gate.
    #[error("{value:?} is not one of the 12 palette colors (DB-040)")]
    UnknownColor {
        /// The value that matched no palette literal.
        value: String,
    },
    /// The file name is not `AGENTS.md` or `CLAUDE.md` (§2 base-name
    /// list; §6.9 "a value from the list").
    #[error("{value:?} is not one of the file names AGENTS.md, CLAUDE.md")]
    UnknownFileName {
        /// The value that matched no list literal.
        value: String,
    },
}

/// Whether a character counts as VISIBLE in a label: neither Unicode
/// whitespace nor a control character (the D-075 ruling; see the
/// module docs for what that accepts and why).
const fn is_visible(character: char) -> bool {
    !character.is_control() && !character.is_whitespace()
}

/// Validate a project name (§6.9: "1–100 characters after trimming
/// spaces"; PRJ-008's editable name field).
///
/// The input is trimmed — all Unicode whitespace — and the TRIMMED
/// string is what gets measured and returned: callers persist the
/// returned value, which keeps the schema's
/// `length(trim(name)) BETWEEN 1 AND 100` and this rule in exact
/// agreement (SQLite's `trim()` strips only U+0020, a subset of what
/// Rust strips — see the module docs). Inner spaces stay; only the
/// edges go. A name that is empty after trimming — the empty string
/// or pure whitespace — fails with [`ProjectFieldError::NameEmpty`].
/// A `NUL` (`U+0000`) anywhere fails with
/// [`ProjectFieldError::NameNul`] — the name-wide NUL rejection
/// (D-076's open item, closed by D-126; module docs); NUL is not
/// whitespace, so the trim never hides one.
///
/// # Examples
///
/// ```
/// use eunomia_core::project_validators::validate_name;
///
/// assert_eq!(validate_name("  My Project  ").unwrap(), "My Project");
/// assert_eq!(validate_name("A").unwrap(), "A");
/// assert_eq!(validate_name(&"a".repeat(100)).unwrap(), "a".repeat(100));
/// assert!(validate_name(&"a".repeat(101)).is_err());
/// assert!(validate_name("a\0b").is_err()); // NUL (D-126)
/// ```
pub fn validate_name(name: &str) -> Result<String, ProjectFieldError> {
    let trimmed = name.trim();
    let length = trimmed.chars().count();
    if length == 0 {
        Err(ProjectFieldError::NameEmpty)
    } else if length > 100 {
        Err(ProjectFieldError::NameTooLong { length })
    } else if let Some(index) = trimmed.chars().position(|character| character == '\0') {
        Err(ProjectFieldError::NameNul { index })
    } else {
        Ok(trimmed.to_owned())
    }
}

/// Validate a project label (§6.9: "1–6 visible characters, case is
/// kept"; PRJ-008's editable label field, shown next to the color —
/// DB-041).
///
/// Validation, not normalization: on success the input slice is
/// returned unchanged — no trimming, no case folding (DB-042's
/// upper-case derivation is the UI's default for a new label, not a
/// rule stored labels follow). The D-075 ruling on "visible"
/// (module docs): at most 6 characters in total, spaces included —
/// the schema CHECK's own bound; at least one character that is
/// neither whitespace nor control; and no control character anywhere.
/// Inner spaces are legal ("MY PRO" is the shape DB-042 derives).
///
/// # Examples
///
/// ```
/// use eunomia_core::project_validators::validate_label;
///
/// assert_eq!(validate_label("MY PRO").unwrap(), "MY PRO");
/// assert_eq!(validate_label("aBc").unwrap(), "aBc"); // case kept
/// assert_eq!(validate_label("ABCDEF").unwrap(), "ABCDEF"); // 6, the edge
/// assert!(validate_label("ABCDEFG").is_err()); // 7
/// assert!(validate_label("   ").is_err()); // renders nothing
/// ```
pub fn validate_label(label: &str) -> Result<&str, ProjectFieldError> {
    let length = label.chars().count();
    if length > 6 {
        Err(ProjectFieldError::LabelTooLong { length })
    } else if !label.chars().any(is_visible) {
        Err(ProjectFieldError::LabelEmpty)
    } else if let Some((index, character)) = label
        .chars()
        .enumerate()
        .find(|&(_, character)| character.is_control())
    {
        Err(ProjectFieldError::LabelControlCharacter { index, character })
    } else {
        Ok(label)
    }
}

/// Derive a new project's default label from its name (DB-042: "The
/// default label is the first up to 6 characters of the name, in
/// upper case. An empty label is not allowed.").
///
/// Upper case FIRST, then the first up to 6 CODE POINTS of the
/// upper-cased stream — the unit of `validate_label` (D-075). The
/// order is not stylistic: upper-casing expands (`"ß"` → `"SS"`,
/// `"ΐ"` → three code points), so slicing first could surface up
/// to 18 code points — a default the §6.9 rule and the schema
/// CHECK both reject. The input is the name as stored — the
/// trimmed string [`validate_name`] returns — and this function
/// trims its input itself, so a raw caller and the validated
/// pipeline derive the same label. It is total: the empty string
/// in yields the empty string out (no panic), and for a name
/// [`validate_name`] accepted the derived string is never empty —
/// the "not allowed" half of DB-042 is [`validate_label`]'s to
/// enforce, and it also catches the name whose consumed prefix has
/// no visible character (module docs).
///
/// # Examples
///
/// ```
/// use eunomia_core::project_validators::{default_label, validate_label};
///
/// assert_eq!(default_label("My Project"), "MY PRO"); // DB-042's example
/// assert_eq!(default_label("straße"), "STRASS"); // upper first: 6, not 7
/// assert_eq!(default_label("  EunomiaSync  "), "EUNOMI"); // trimmed
/// assert_eq!(default_label(""), ""); // total — validate_label gates this
/// assert_eq!(validate_label(&default_label("My Project")).unwrap(), "MY PRO");
/// ```
pub fn default_label(name: &str) -> String {
    name.trim()
        .chars()
        .flat_map(char::to_uppercase)
        .take(6)
        .collect()
}

/// Validate a project description (§6.9: "up to 1000 characters";
/// PRJ-008's editable description field).
///
/// The one transformation is the SILENT NUL strip — the owner's
/// ruling that closed D-151 item 7 (2026-11-22): every `U+0000` is
/// removed before anything is measured or returned. A description
/// never fails validation for a NUL, and a description stored
/// through this validator never contains one; the contrast with
/// [`validate_name`], where the same
/// byte is the outright [`ProjectFieldError::NameNul`] rejection
/// (D-126), is deliberate — a name is file-name and file-content
/// material, a description is inert prose for the stored row.
///
/// The length is measured AFTER the strip, so the ≤1000 limit
/// governs the value to persist: 1001 characters including one NUL
/// validate as 1000, and [`ProjectFieldError::DescriptionTooLong`]'s
/// length is the post-strip count. Everything else is verbatim — no
/// trim (the row names none), edges and newlines kept; empty is
/// legal (the column defaults to `''`, §6.2), including empty after
/// the strip.
///
/// Returns the sanitized string — the value to persist
/// ([`validate_name`]'s trim precedent); a NUL-free input comes back
/// byte-for-byte unchanged.
///
/// # Examples
///
/// ```
/// use eunomia_core::project_validators::validate_description;
///
/// assert_eq!(validate_description("").unwrap(), "");
/// assert_eq!(validate_description("a\0b").unwrap(), "ab"); // silent strip
/// assert_eq!(validate_description("\0\0").unwrap(), ""); // empty is legal
/// assert_eq!(
///     validate_description(&"x".repeat(1000)).unwrap(),
///     "x".repeat(1000)
/// );
/// assert!(validate_description(&"x".repeat(1001)).is_err());
/// // 1001 characters including one NUL store as 1000 — legal.
/// assert_eq!(
///     validate_description(&format!("{}\0", "x".repeat(1000))).unwrap(),
///     "x".repeat(1000)
/// );
/// ```
pub fn validate_description(description: &str) -> Result<String, ProjectFieldError> {
    // The silent-sanitization ruling that closed D-151 item 7: strip,
    // then measure — the limit governs the stored truth. `replace`
    // over a char pattern is one linear pass, the only transformation
    // this field gets (no trim, no case work); the owned return is
    // the value to persist, so a NUL-free input pays one copy and
    // comes back byte-for-byte.
    let sanitized = description.replace('\0', "");
    let length = sanitized.chars().count();
    if length > 1000 {
        Err(ProjectFieldError::DescriptionTooLong { length })
    } else {
        Ok(sanitized)
    }
}

/// Validate a project color (§6.9: "a value from the palette" — the
/// fixed 12 colors of DB-040; PRJ-008's editable color field).
///
/// The palette literal must match exactly, case-sensitive like every
/// schema literal. The §6.2 `color` column carries NO CHECK, so this
/// function is the only gate a stored color passes (D-075). Returns
/// the parsed [`Color`].
///
/// # Examples
///
/// ```
/// use eunomia_core::project::{Color};
/// use eunomia_core::project_validators::validate_color;
///
/// assert_eq!(validate_color("teal").unwrap(), Color::Teal);
/// assert_eq!(validate_color("slate").unwrap(), Color::Slate);
/// assert!(validate_color("purple").is_err()); // not in the palette
/// assert!(validate_color("Red").is_err()); // case-sensitive
/// ```
pub fn validate_color(color: &str) -> Result<Color, ProjectFieldError> {
    Color::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.as_str() == color)
        .ok_or_else(|| ProjectFieldError::UnknownColor {
            value: color.to_owned(),
        })
}

/// Validate a project's target file name (§6.9: "a value from the
/// list" — the §2 base-name list `AGENTS.md` / `CLAUDE.md`, D-03;
/// PRJ-012's per-project copy of the write settings).
///
/// Exact, case-sensitive list membership, like the schema CHECK. The
/// §6.9 note "path and extension are not typed in" describes the
/// picker the UI offers; validation itself is plain membership, so a
/// typed path (`docs/AGENTS.md`) or a case variant (`agents.md`)
/// simply matches no literal. Returns the parsed [`BaseName`].
///
/// # Examples
///
/// ```
/// use eunomia_core::project::{BaseName};
/// use eunomia_core::project_validators::validate_file_name;
///
/// assert_eq!(validate_file_name("AGENTS.md").unwrap(), BaseName::AgentsMd);
/// assert_eq!(validate_file_name("CLAUDE.md").unwrap(), BaseName::ClaudeMd);
/// assert!(validate_file_name("GEMINI.md").is_err());
/// assert!(validate_file_name("agents.md").is_err());
/// ```
pub fn validate_file_name(file_name: &str) -> Result<BaseName, ProjectFieldError> {
    [BaseName::AgentsMd, BaseName::ClaudeMd]
        .into_iter()
        .find(|candidate| candidate.as_str() == file_name)
        .ok_or_else(|| ProjectFieldError::UnknownFileName {
            value: file_name.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- name: 0 / 1 / 100 / 101 after trim ------------------------------

    #[test]
    fn name_boundaries_after_trim() {
        // 0 after trimming: the empty string, plain spaces, and a
        // mix of Unicode whitespace edges (tab, newline, NBSP) — a
        // name must render.
        for empty in ["", " ", "   ", "\t", "\n \u{00A0}\u{3000}"] {
            assert_eq!(validate_name(empty), Err(ProjectFieldError::NameEmpty));
        }
        // 1 and 100: legal.
        assert_eq!(validate_name("A").unwrap(), "A");
        let hundred = "a".repeat(100);
        assert_eq!(validate_name(&hundred).unwrap(), hundred);
        // 101: one over.
        assert_eq!(
            validate_name(&"a".repeat(101)).unwrap_err(),
            ProjectFieldError::NameTooLong { length: 101 }
        );
    }

    #[test]
    fn name_is_trimmed_at_the_edges_and_keeps_inner_spaces() {
        // Trim FIRST, then count: 100 characters with whitespace
        // around them still fit (§6.9 "after trimming spaces").
        let hundred = "a".repeat(100);
        let padded = format!("  {hundred}\t");
        assert_eq!(validate_name(&padded).unwrap(), hundred);
        // Inner spaces are content, not edges.
        assert_eq!(validate_name("  My Project  ").unwrap(), "My Project");
        // The trim is Unicode-wide: NBSP and ideographic space edges
        // go too.
        assert_eq!(validate_name("\u{00A0}Omega\u{3000}").unwrap(), "Omega");
    }

    #[test]
    fn name_counts_code_points_not_bytes_or_graphemes() {
        // "é" is one code point however many bytes it takes: 100 fit,
        // 101 do not.
        assert_eq!(validate_name(&"é".repeat(100)).unwrap(), "é".repeat(100));
        assert_eq!(
            validate_name(&"é".repeat(101)).unwrap_err(),
            ProjectFieldError::NameTooLong { length: 101 }
        );
        // "e" + combining accent is TWO code points, one grapheme:
        // 51 of them measure 102 — over the limit at 51 graphemes.
        // The unit matches SQLite's length() on TEXT.
        assert_eq!(
            validate_name(&"e\u{301}".repeat(51)).unwrap_err(),
            ProjectFieldError::NameTooLong { length: 102 }
        );
        // An astral-plane character is still one code point.
        assert_eq!(validate_name(&"\u{1F600}".repeat(100)).unwrap().len(), 400);
    }

    #[test]
    fn name_nul_is_rejected_at_any_position() {
        // The D-126 validators-wide ruling on D-076's open item
        // (module docs): SQLite's
        // length() counts only up to the first U+0000, so a NUL-first
        // name would pass the length checks while the store's own
        // CHECK measures it as 0 — a raw database error instead of a
        // field error. Only NUL, middle, and end all fail with the
        // code-point index.
        assert_eq!(
            validate_name("\0").unwrap_err(),
            ProjectFieldError::NameNul { index: 0 }
        );
        assert_eq!(
            validate_name("a\0b").unwrap_err(),
            ProjectFieldError::NameNul { index: 1 }
        );
        assert_eq!(
            validate_name("a\0").unwrap_err(),
            ProjectFieldError::NameNul { index: 1 }
        );
        // The index counts CODE POINTS: after a two-byte "é" the NUL
        // sits at index 1, and only the FIRST NUL is reported.
        assert_eq!(
            validate_name("é\0x\0").unwrap_err(),
            ProjectFieldError::NameNul { index: 1 }
        );
        // NUL is not whitespace: the trim never hides it.
        assert_eq!(
            validate_name(" \0 ").unwrap_err(),
            ProjectFieldError::NameNul { index: 0 }
        );
        // Too long beats the NUL scan — the fixed order keeps the
        // message deterministic.
        let mut long = "a".repeat(101);
        long.push('\0');
        assert_eq!(
            validate_name(&long).unwrap_err(),
            ProjectFieldError::NameTooLong { length: 102 }
        );
    }

    #[test]
    fn name_errors_say_what_is_wrong() {
        assert_eq!(
            validate_name("   ").unwrap_err().to_string(),
            "project name is empty after trimming"
        );
        assert_eq!(
            validate_name(&"a".repeat(101)).unwrap_err().to_string(),
            "project name is 101 characters after trimming; the limit is 100"
        );
        assert_eq!(
            validate_name("a\0b").unwrap_err().to_string(),
            "project name contains NUL at index 1"
        );
    }

    // ---- label: 0 / 1 / 6 / 7, visible characters, case kept --------------

    #[test]
    fn label_boundaries_total_and_visible() {
        // 0 visible — empty or pure whitespace (spaces, NBSP, tab):
        // renders nothing next to the color (DB-042).
        for empty in ["", " ", "      ", "\u{00A0}", "\t"] {
            assert_eq!(validate_label(empty), Err(ProjectFieldError::LabelEmpty));
        }
        // 1 and 6 total characters: legal.
        assert_eq!(validate_label("A").unwrap(), "A");
        assert_eq!(validate_label("ABCDEF").unwrap(), "ABCDEF");
        // 7: one over — the schema CHECK's own bound.
        assert_eq!(
            validate_label("ABCDEFG").unwrap_err(),
            ProjectFieldError::LabelTooLong { length: 7 }
        );
        // Six code points, not bytes: "Проект" fits, "Проект1" does
        // not.
        assert_eq!(validate_label("Проект").unwrap(), "Проект");
        assert_eq!(
            validate_label("Проект1").unwrap_err(),
            ProjectFieldError::LabelTooLong { length: 7 }
        );
    }

    #[test]
    fn label_keeps_case_and_text_verbatim() {
        // Case is kept (§6.9): both spellings validate, each returned
        // exactly as typed — no upper-casing, no folding.
        assert_eq!(validate_label("aBc").unwrap(), "aBc");
        assert_eq!(validate_label("abc").unwrap(), "abc");
        assert_eq!(validate_label("ABC").unwrap(), "ABC");
        assert_eq!(validate_label("ÄöÜ").unwrap(), "ÄöÜ");
        // And no trimming either: the edges are content here.
        assert_eq!(validate_label(" A").unwrap(), " A");
    }

    #[test]
    fn label_inner_space_is_legal_the_default_derivation_shape() {
        // DB-042 derives the default label from the first up to 6
        // characters of the name in upper case — "My Project" →
        // "MY PRO" — so an inner space must validate.
        assert_eq!(validate_label("MY PRO").unwrap(), "MY PRO");
        // A visible character plus padding up to the total of 6 is
        // still 1–6 visible characters.
        assert_eq!(validate_label("A     ").unwrap(), "A     ");
        assert_eq!(validate_label("     B").unwrap(), "     B");
        // But the TOTAL stays within 6: a space is a character too.
        assert_eq!(
            validate_label("AB CDE F").unwrap_err(),
            ProjectFieldError::LabelTooLong { length: 8 }
        );
        // The exact case that splits the readings: 6 visible
        // characters + 1 inner space = 7 total. The bound is the
        // schema CHECK's total, not "6 visible, spaces free".
        assert_eq!(
            validate_label("MY PROJ").unwrap_err(),
            ProjectFieldError::LabelTooLong { length: 7 }
        );
    }

    #[test]
    fn label_control_characters_are_rejected_wherever_they_sit() {
        // Cc renders nothing and breaks the one-line tag — rejected
        // with the position and the character, at any index. The
        // index counts CODE POINTS (the module's unit), not bytes:
        // after a two-byte "Ü" the BEL is at index 1, not 2.
        assert_eq!(
            validate_label("A\u{7}B").unwrap_err(),
            ProjectFieldError::LabelControlCharacter {
                index: 1,
                character: '\u{7}'
            }
        );
        assert_eq!(
            validate_label("Ü\u{7}").unwrap_err(),
            ProjectFieldError::LabelControlCharacter {
                index: 1,
                character: '\u{7}'
            }
        );
        assert_eq!(
            validate_label("\u{1F600}\u{7}").unwrap_err(),
            ProjectFieldError::LabelControlCharacter {
                index: 1,
                character: '\u{7}'
            }
        );
        assert_eq!(
            validate_label("AB\u{0}").unwrap_err(),
            ProjectFieldError::LabelControlCharacter {
                index: 2,
                character: '\u{0}'
            }
        );
        assert_eq!(
            validate_label("A\u{1B}").unwrap_err(),
            ProjectFieldError::LabelControlCharacter {
                index: 1,
                character: '\u{1B}'
            }
        );
        // A newline inside is a control character too — the label is
        // one line.
        assert_eq!(
            validate_label("A\nB").unwrap_err(),
            ProjectFieldError::LabelControlCharacter {
                index: 1,
                character: '\n'
            }
        );
    }

    #[test]
    fn label_nul_is_rejected_as_a_control_character() {
        // The label needs no NUL rule of its own: `U+0000` is a
        // control character, so D-075's rejection already refuses it
        // wherever it sits — the audit's confirmation, pinned here.
        // With a visible character present the control scan fires —
        // at the start here; the END-position NUL pin already lives
        // in the control test above ("AB\u{0}").
        assert_eq!(
            validate_label("\0A").unwrap_err(),
            ProjectFieldError::LabelControlCharacter {
                index: 0,
                character: '\0'
            }
        );
        // …and a NUL-only label renders nothing, so the visible-count
        // check fires first with the truer message (the documented
        // precedence: length, visible, control).
        assert_eq!(validate_label("\0"), Err(ProjectFieldError::LabelEmpty));
    }

    #[test]
    fn label_precedence_is_length_then_visible_then_control() {
        // A label can break several rules at once; the checks fire in
        // a fixed order so the message is deterministic.
        // 7 spaces: too long beats "renders nothing".
        assert_eq!(
            validate_label("       ").unwrap_err(),
            ProjectFieldError::LabelTooLong { length: 7 }
        );
        // A lone tab is whitespace AND control: no visible
        // characters is the truer message.
        assert_eq!(
            validate_label("\t").unwrap_err(),
            ProjectFieldError::LabelEmpty
        );
        // Whitespace around a control character still renders
        // nothing: the visible-count check fires first.
        assert_eq!(
            validate_label(" \u{7} ").unwrap_err(),
            ProjectFieldError::LabelEmpty
        );
    }

    #[test]
    fn label_format_characters_are_the_std_boundary() {
        // Unicode Cf (zero-width space, ZWJ) is neither whitespace
        // nor control in std's tables, so it counts as visible — the
        // same input the schema's bare length CHECK accepts. Pinned
        // here as the recorded limitation of the D-075 ruling, not an
        // accident.
        assert_eq!(validate_label("\u{200B}").unwrap(), "\u{200B}");
        assert_eq!(validate_label("A\u{200D}B").unwrap(), "A\u{200D}B");
    }

    #[test]
    fn label_errors_say_what_is_wrong() {
        assert_eq!(
            validate_label(" ").unwrap_err().to_string(),
            "project label has no visible characters"
        );
        assert_eq!(
            validate_label("ABCDEFG").unwrap_err().to_string(),
            "project label is 7 characters; the limit is 6"
        );
        assert_eq!(
            validate_label("A\u{7}B").unwrap_err().to_string(),
            "project label contains the control character '\\u{7}' at index 1"
        );
    }

    // ---- default label: the DB-042 derivation ------------------------------

    #[test]
    fn default_label_takes_the_first_up_to_six_characters_upper_cased() {
        // DB-042's own example plus the lengths around it: shorter
        // than 6 (1..=5), exactly 6, longer — the whole name when it
        // fits, the first 6 when it does not.
        assert_eq!(default_label("A"), "A");
        assert_eq!(default_label("aB"), "AB");
        assert_eq!(default_label("Abc"), "ABC");
        assert_eq!(default_label("Abcd"), "ABCD");
        assert_eq!(default_label("Short"), "SHORT");
        assert_eq!(default_label("Projec"), "PROJEC"); // 6 exactly
        assert_eq!(default_label("Проект"), "ПРОЕКТ"); // 6 code points
        assert_eq!(default_label("My Project"), "MY PRO"); // the example
        assert_eq!(default_label("EunomiaSync"), "EUNOMI");
        assert_eq!(default_label("ABCDEF"), "ABCDEF"); // already upper
        // The name ceiling derives the same six as any longer name.
        assert_eq!(default_label(&"a".repeat(100)), "AAAAAA");
    }

    #[test]
    fn default_label_upper_cases_before_the_slice_the_unicode_pin() {
        // The order only shows in Unicode: upper-casing EXPANDS
        // ("ß" → "SS"), so slice-first would return the 7-code-point
        // "STRASSE" — rejected by §6.9 and the schema CHECK — where
        // upper-first returns 6. "straße" is the discriminator.
        assert_eq!(default_label("straße"), "STRASS");
        // Six ß in the name: twelve code points if sliced first;
        // six once the upper-cased stream is clipped.
        assert_eq!(default_label("ßßßßßß"), "SSSSSS");
        // The worst case the ruling leans on: a character can map
        // to THREE code points ("ΐ" → "Ι\u{308}\u{301}"), so six of
        // them slice first to 18 — upper-first keeps six.
        assert_eq!(
            default_label("\u{0390}\u{0390}\u{0390}\u{0390}\u{0390}\u{0390}"),
            "\u{0399}\u{0308}\u{0301}\u{0399}\u{0308}\u{0301}"
        );
        assert_eq!(default_label("ß"), "SS"); // an expansion under 6 is legal
        // …and the pinned results really are labels the app may store.
        for name in [
            "straße",
            "ßßßßßß",
            "\u{0390}\u{0390}\u{0390}\u{0390}\u{0390}\u{0390}",
            "ß",
        ] {
            let label = default_label(name);
            assert!(label.chars().count() <= 6);
            assert!(validate_label(&label).is_ok(), "{label:?}");
        }
    }

    #[test]
    fn default_label_counts_code_points_not_bytes_or_graphemes() {
        // Six Cyrillic characters are 12 bytes — and a legal label.
        let label = default_label("проект синхронизации");
        assert_eq!(label, "ПРОЕКТ");
        assert_eq!(label.chars().count(), 6);
        assert_eq!(label.len(), 12);
        // Astral emoji: one code point and four bytes each — seven
        // in the name, six out, 24 bytes. A byte slice would keep
        // one and a half emoji — the assertion below this one is
        // the grapheme discriminator (each emoji here is one
        // grapheme per code point and agrees with them).
        let label =
            default_label("\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}");
        assert_eq!(label.chars().count(), 6);
        assert_eq!(label.len(), 24);
        assert!(validate_label(&label).is_ok());
        // A combining mark rides its base letter: "e\u{301}" stays
        // two code points through the upper-casing ("E\u{301}"),
        // and the slice cuts at code points — 8 in, 6 out. A
        // grapheme-count slice would keep all four accents (8 code
        // points) and fail this assertion.
        assert_eq!(
            default_label("e\u{301}e\u{301}e\u{301}e\u{301}"),
            "E\u{301}E\u{301}E\u{301}"
        );
    }

    #[test]
    fn default_label_reads_the_stored_name_so_it_trims_first() {
        // The name the app stores is the TRIMMED one
        // (validate_name's return contract, D-075): the derivation
        // trims the same way, so a raw caller and the validated
        // pipeline agree.
        assert_eq!(default_label("  My Project  "), "MY PRO");
        assert_eq!(default_label("\u{00A0}Omega\u{3000}"), "OMEGA");
        // Total: degenerate input yields the empty string — no
        // panic — and "an empty label is not allowed" stays
        // validate_label's rule to enforce.
        assert_eq!(default_label(""), "");
        assert_eq!(default_label(" "), "");
        assert_eq!(default_label("   "), "");
        assert_eq!(
            validate_label(&default_label("")),
            Err(ProjectFieldError::LabelEmpty)
        );
    }

    #[test]
    fn default_label_of_a_valid_name_is_a_valid_label() {
        // The property link: whatever validate_name accepts — whose
        // first six characters carry no control character, the one
        // escape recorded below — derives a default that
        // validate_label accepts: never empty, at most 6, at least
        // one visible character, no control of its own making.
        let names = [
            "A",
            "aB",
            "My Project",
            "EunomiaSync",
            "  padded  ",
            "Проект",
            "проект синхронизации",
            "straße",
            "ßßßßßß",
            "İstanbul",     // 'İ' is already upper; 'i' upper-cases to 'I'
            "e\u{301}tude", // a combining accent rides along
            "césar",
            "naïve résumé",
            "Órbita",
            "\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}",
        ];
        for name in names {
            let validated = validate_name(name).unwrap();
            let label = default_label(name);
            assert_eq!(label, default_label(&validated)); // trim-neutral
            let verdict = validate_label(&label);
            assert!(verdict.is_ok(), "{name:?} → {label:?} → {verdict:?}");
        }
        // The name's 100-character ceiling derives the same shape.
        let label = default_label(&"a".repeat(100));
        assert_eq!(label, "AAAAAA");
        assert_eq!(validate_label(&label).unwrap(), "AAAAAA");
    }

    #[test]
    fn default_label_counterexamples_controls_in_the_name() {
        // Names reserve NO control-character rule beyond `NUL`
        // (D-075, extended by D-126), and trim removes whitespace,
        // not controls — so these names are valid, and each derived
        // default is rejected by the label's own rules. The
        // derivation stays literal: filtering would
        // invent a rule DB-042 does not state. The settings modal
        // shows the field error and the user edits the label before
        // the save; pinned here as the contract's recorded edge.
        //
        // Edge one: visible content FOLLOWS the control — the label
        // fails the control check, with the position kept.
        let name = "\u{7}abc";
        let validated = validate_name(name).unwrap(); // a legal name
        let label = default_label(&validated);
        assert_eq!(label, "\u{7}ABC");
        assert_eq!(
            validate_label(&label),
            Err(ProjectFieldError::LabelControlCharacter {
                index: 0,
                character: '\u{7}'
            })
        );
        // Edge two: the consumed prefix has NO visible character —
        // controls only, or a control-and-space mix (the inner
        // space survives trim) — the label renders nothing, which
        // is `LabelEmpty`, the same verdict DB-042's "an empty
        // label is not allowed" names.
        for invisible_prefix in ["\u{7}", "\u{7}\u{8}", "\u{7}\u{7}\u{7}", "\u{7} \u{8}"] {
            assert!(validate_name(invisible_prefix).is_ok()); // legal names
            let label = default_label(invisible_prefix);
            assert_eq!(label, invisible_prefix); // ≥1 code point, never ""
            assert_eq!(validate_label(&label), Err(ProjectFieldError::LabelEmpty));
        }
    }

    #[test]
    fn upper_casing_never_deletes_or_hides_a_visible_character() {
        // The lemma the never-empty property rests on, checked over
        // EVERY scalar value: a character that is neither control
        // nor whitespace upper-cases to at least one character, and
        // every character it upper-cases to is itself neither
        // control nor whitespace. If a future Unicode table ever
        // broke it, this pin — not a user's project — would say so.
        for code in 0..=u32::from(char::MAX) {
            let Some(character) = char::from_u32(code) else {
                continue;
            };
            if is_visible(character) {
                let mut upper = character.to_uppercase();
                assert!(upper.next().is_some(), "{character:?} vanishes");
                for mapped in upper {
                    assert!(
                        is_visible(mapped),
                        "{character:?} upper-cases to invisible {mapped:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_new_project_composes_its_name_into_the_default_label() {
        // The PRJ-008 flow for a NEW project: validate the name,
        // derive the label default, validate the label — one round
        // trip through this module's own rules.
        let name = validate_name("  EunomiaSync  ").unwrap();
        let label = default_label(&name);
        assert_eq!(label, "EUNOMI");
        assert_eq!(validate_label(&label).unwrap(), "EUNOMI");
    }

    // ---- description: NUL stripped, 0 / 1000 / 1001 ----------------------

    #[test]
    fn description_boundaries_empty_through_1000() {
        // 0 is legal — the column defaults to '' (§6.2).
        assert_eq!(validate_description("").unwrap(), "");
        // 1000 exactly: legal, whitespace included.
        let thousand = "x".repeat(999) + "\n";
        assert_eq!(validate_description(&thousand).unwrap(), thousand);
        assert_eq!(
            validate_description(&"x".repeat(1000)).unwrap(),
            "x".repeat(1000)
        );
        // 1001: one over.
        assert_eq!(
            validate_description(&"x".repeat(1001)).unwrap_err(),
            ProjectFieldError::DescriptionTooLong { length: 1001 }
        );
        // Code points again: 1001 two-byte characters measure 1001.
        assert_eq!(
            validate_description(&"é".repeat(1001)).unwrap_err(),
            ProjectFieldError::DescriptionTooLong { length: 1001 }
        );
    }

    #[test]
    fn description_strips_nul_silently_at_any_position() {
        // The owner's ruling that closed D-151 item 7 (2026-11-22):
        // the description SANITIZES — every `U+0000` is silently
        // removed from the value to persist — it never rejects (the
        // contrast with the names, D-126, pinned below). NUL is an
        // invisible byte the user cannot see typing: the visible
        // intent survives it whole, wherever it sits and however
        // many there are.
        assert_eq!(validate_description("\0").unwrap(), "");
        assert_eq!(validate_description("\0\0\0").unwrap(), "");
        assert_eq!(validate_description("a\0b").unwrap(), "ab");
        assert_eq!(validate_description("\0lead").unwrap(), "lead");
        assert_eq!(validate_description("trail\0").unwrap(), "trail");
        assert_eq!(validate_description("a\0b\0c").unwrap(), "abc");
        // A NUL between multi-byte characters keeps both sides
        // intact — the cut is byte-safe inside UTF-8 by construction
        // (`replace` works on scalar values).
        assert_eq!(validate_description("проект\0sync").unwrap(), "проектsync");
        // The strip is NUL-ONLY: every other control character is
        // legal description content (newlines are prose, §6.9
        // reserves no control rule for descriptions; D-075's
        // visibility rules govern LABELS, not descriptions) and
        // passes through unchanged.
        assert_eq!(validate_description("a\u{7}b").unwrap(), "a\u{7}b");
        assert_eq!(validate_description("\u{1B}[0m").unwrap(), "\u{1B}[0m");
    }

    #[test]
    fn description_length_measures_the_stored_truth_after_the_strip() {
        // Strip FIRST, then count: the ≤1000 limit governs what lands
        // in the column, not what was typed — the schema CHECK sees
        // the STORED description, so the validator measures the same
        // value. (The PRJ-008 modal's `0 / 1000` counter must count
        // the sanitized value when it lands, T-279 — a plain
        // typed-length counter would show 1001/1000 for an input this
        // rule accepts.)
        // 1001 clean characters still do not fit.
        assert_eq!(
            validate_description(&"x".repeat(1001)).unwrap_err(),
            ProjectFieldError::DescriptionTooLong { length: 1001 }
        );
        // 1001 typed with one trailing NUL → 1000 stored: legal.
        let trailing = format!("{}\0", "x".repeat(1000));
        assert_eq!(validate_description(&trailing).unwrap(), "x".repeat(1000));
        // 1002 typed with one NUL → 1001 stored: rejected AS 1001 —
        // the error's length is the post-strip count, so the message
        // talks about the description that would be stored.
        let over = format!("{}\0y", "x".repeat(1000));
        assert_eq!(
            validate_description(&over).unwrap_err(),
            ProjectFieldError::DescriptionTooLong { length: 1001 }
        );
        // NUL-first — the SQLite trap D-126 recorded for names:
        // `length()` counts only up to the first `U+0000`, so the
        // store's own CHECK would measure "\0" + 1000 characters as
        // 0. The validator's count is the honest one: 1000 stored,
        // legal.
        let nul_first = format!("\0{}", "x".repeat(1000));
        assert_eq!(validate_description(&nul_first).unwrap(), "x".repeat(1000));
        // Several NULs shrink proportionally: 1001 typed, 998 stored.
        let several = format!("{}\0\0\0", "x".repeat(998));
        assert_eq!(validate_description(&several).unwrap(), "x".repeat(998));
    }

    #[test]
    fn description_clean_input_round_trips_byte_for_byte() {
        // The idempotence pin (sanitize-then-save rerun on clean
        // input is a no-op): a NUL-free description comes back
        // exactly as typed — multi-byte characters, newlines,
        // whitespace edges, a mid-string control character, and the
        // boundary length all byte-for-byte.
        let clean = "  Проект синхронизации — line 1\nline 2 é 🙂\t\u{7}";
        assert_eq!(validate_description(clean).unwrap(), clean);
        let thousand = "x".repeat(999) + "\n";
        assert_eq!(validate_description(&thousand).unwrap(), thousand);
        assert_eq!(validate_description("").unwrap(), "");
    }

    #[test]
    fn description_keeps_its_edges_and_whitespace_no_trim() {
        // The row names no trim and the CHECK has none: the edges
        // stay (the NUL strip is the ONLY transformation — the tests
        // above own that half).
        assert_eq!(validate_description("  padded  ").unwrap(), "  padded  ");
        assert_eq!(validate_description("\n\n").unwrap(), "\n\n");
    }

    #[test]
    fn names_reject_what_descriptions_strip() {
        // The two policies coexist by ruling: the SAME NUL-bearing
        // input is an outright rejection as a NAME (D-126 — a name is
        // file-name and file-content material) and a silent
        // sanitization as a DESCRIPTION (the D-151 item-7 closure —
        // inert prose for the stored row). One test, both verdicts,
        // so neither half can drift back into the other's policy.
        assert_eq!(
            validate_name("a\0b").unwrap_err(),
            ProjectFieldError::NameNul { index: 1 }
        );
        assert_eq!(validate_description("a\0b").unwrap(), "ab");
    }

    #[test]
    fn description_errors_say_what_is_wrong() {
        assert_eq!(
            validate_description(&"x".repeat(1001))
                .unwrap_err()
                .to_string(),
            "project description is 1001 characters; the limit is 1000"
        );
    }

    // ---- color: the 12 palette literals ------------------------------------

    #[test]
    fn color_accepts_exactly_the_twelve_palette_literals() {
        let palette = [
            (Color::Red, "red"),
            (Color::Orange, "orange"),
            (Color::Amber, "amber"),
            (Color::Lime, "lime"),
            (Color::Green, "green"),
            (Color::Teal, "teal"),
            (Color::Cyan, "cyan"),
            (Color::Blue, "blue"),
            (Color::Indigo, "indigo"),
            (Color::Violet, "violet"),
            (Color::Pink, "pink"),
            (Color::Slate, "slate"),
        ];
        // Every literal parses to its variant, and the variant says
        // the same literal back.
        for (color, literal) in palette {
            assert_eq!(validate_color(literal).unwrap(), color);
            assert_eq!(color.as_str(), literal);
        }
        // DB-040's order is the palette's order.
        let all: Vec<&str> = Color::ALL.iter().map(|color| color.as_str()).collect();
        assert_eq!(
            all,
            [
                "red", "orange", "amber", "lime", "green", "teal", "cyan", "blue", "indigo",
                "violet", "pink", "slate",
            ]
        );
    }

    #[test]
    fn color_rejects_everything_else_with_the_value_kept() {
        for unknown in [
            "purple", "Red", "RED", "", "red ", " red", "teal\n", "teal\0", "crimson", "grey",
            "rose",
        ] {
            assert_eq!(
                validate_color(unknown).unwrap_err(),
                ProjectFieldError::UnknownColor {
                    value: unknown.to_owned()
                },
                "the palette has no {unknown:?}"
            );
        }
        assert_eq!(
            validate_color("purple").unwrap_err().to_string(),
            "\"purple\" is not one of the 12 palette colors (DB-040)"
        );
    }

    // ---- file name: the two list literals -----------------------------------

    #[test]
    fn file_name_accepts_the_two_list_literals() {
        assert_eq!(validate_file_name("AGENTS.md").unwrap(), BaseName::AgentsMd);
        assert_eq!(validate_file_name("CLAUDE.md").unwrap(), BaseName::ClaudeMd);
    }

    #[test]
    fn file_name_rejects_paths_case_variants_and_other_names() {
        // §6.9: "path and extension are not typed in" — the picker's
        // job; validation is plain membership, so all of these match
        // no literal.
        for unknown in [
            "agents.md",
            "AGENTS.MD",
            "GEMINI.md",
            "AGENTS",
            "AGENTS.md ",
            "AGENTS\0.md",
            "docs/AGENTS.md",
            "",
        ] {
            assert_eq!(
                validate_file_name(unknown).unwrap_err(),
                ProjectFieldError::UnknownFileName {
                    value: unknown.to_owned()
                },
                "the list has no {unknown:?}"
            );
        }
        assert_eq!(
            validate_file_name("GEMINI.md").unwrap_err().to_string(),
            "\"GEMINI.md\" is not one of the file names AGENTS.md, CLAUDE.md"
        );
    }

    // ---- the five fields together -------------------------------------------

    #[test]
    fn the_settings_modal_fields_validate_together() {
        // One PRJ-008 save: every field at a legal value, and the
        // returned values are what the store persists.
        let name = validate_name("  EunomiaSync  ").unwrap();
        let label = validate_label("MY PRO").unwrap();
        let description = validate_description("").unwrap();
        let color = validate_color("teal").unwrap();
        let file_name = validate_file_name("AGENTS.md").unwrap();
        assert_eq!(name, "EunomiaSync");
        assert_eq!(label, "MY PRO");
        assert_eq!(description, "");
        assert_eq!(color, Color::Teal);
        assert_eq!(file_name, BaseName::AgentsMd);
    }

    #[test]
    fn every_failure_is_typed_never_a_string() {
        // The E_VALIDATION mapping (§8) matches on variants; each
        // field's every failure mode has one — and each is ITS
        // field's own variant, so a cross-field mix-up fails here,
        // not only in the per-field tests.
        type Failure = (fn() -> Result<(), ProjectFieldError>, ProjectFieldError);
        let failures: [Failure; 9] = [
            (
                || validate_name("").map(|_| ()),
                ProjectFieldError::NameEmpty,
            ),
            (
                || validate_name(&"a".repeat(101)).map(|_| ()),
                ProjectFieldError::NameTooLong { length: 101 },
            ),
            (
                || validate_name("a\0b").map(|_| ()),
                ProjectFieldError::NameNul { index: 1 },
            ),
            (
                || validate_label("").map(|_| ()),
                ProjectFieldError::LabelEmpty,
            ),
            (
                || validate_label("ABCDEFG").map(|_| ()),
                ProjectFieldError::LabelTooLong { length: 7 },
            ),
            (
                || validate_label("A\u{7}B").map(|_| ()),
                ProjectFieldError::LabelControlCharacter {
                    index: 1,
                    character: '\u{7}',
                },
            ),
            (
                || validate_description(&"x".repeat(1001)).map(|_| ()),
                ProjectFieldError::DescriptionTooLong { length: 1001 },
            ),
            (
                || validate_color("purple").map(|_| ()),
                ProjectFieldError::UnknownColor {
                    value: "purple".to_owned(),
                },
            ),
            (
                || validate_file_name("GEMINI.md").map(|_| ()),
                ProjectFieldError::UnknownFileName {
                    value: "GEMINI.md".to_owned(),
                },
            ),
        ];
        for (produce, expected) in failures {
            assert_eq!(produce().unwrap_err(), expected);
        }
    }
}
