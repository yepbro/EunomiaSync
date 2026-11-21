//! Library field validators: the §6.9 rules of the editable library
//! fields — the instruction name, the variant name, the variant text
//! and the heading name — that the library editor saves in one
//! transaction (spec §7.3, the LIB-002 create-dialog row: "Validation
//! per table 6.9"), and that SEC-009 re-applies to an import's field
//! lengths.
//!
//! Each rule is transcribed next to the schema CHECK (§6.2) that
//! stores the same verdict, so a value that passes here never fails
//! the database — including the names, whose `NUL` rejection below
//! closes the one exception D-076 once had to record (a NUL-first
//! name SQLite measures as empty):
//!
//! | Field            | §6.9 rule                                                      | schema CHECK (§6.2)                     | this module returns |
//! | ---------------- | -------------------------------------------------------------- | --------------------------------------- | ------------------- |
//! | Instruction name | 1–200 characters, one line                                     | `length(trim(title)) BETWEEN 1 AND 200` | the TRIMMED name    |
//! | Variant name     | 1–100 characters (one line — the D-076 ruling)                 | `length(trim(name)) BETWEEN 1 AND 100`  | the TRIMMED name    |
//! | Variant text     | up to 262,144 characters, empty is allowed; `NUL` and reserved markers are rejected | `length(content) <= 262144` | the text, unchanged |
//! | Heading name     | 1–200 characters, one line                                     | `length(trim(title)) BETWEEN 1 AND 200` | the TRIMMED name    |
//!
//! The counting unit everywhere is the Unicode scalar value
//! (`str::chars().count()` — code points), the same unit SQLite's
//! `length()` uses on TEXT: not bytes (`"é"` is one, not two) and not
//! graphemes (`"e\u{301}"` is two, not one).
//!
//! Four §6.9 concerns are deliberately NOT here:
//!
//! - the heading LEVEL rule (2–6) needs no validator: the closed
//!   [`HeadingLevel`](crate::library::HeadingLevel) type (D-042)
//!   already makes 1 and 7 unrepresentable, and its `TryFrom<u8>` is
//!   the store's boundary conversion for the integer column — there
//!   is no free integer left to validate;
//! - the two "unique, case-insensitive" clauses are cross-row queries
//!   (the schema's `ux_variant_name` NOCASE index backs one of them)
//!   — the store layer's job, exactly as T-049 left the project
//!   name's duplicates clause out of
//!   [`crate::project_validators`];
//! - "the main variant is named `Main` by default" is a UI default
//!   for a NEW main variant, not a rule stored names follow;
//! - the backup-limit row is a settings field, not a library field.
//!
//! # The names: trim, then count — then one character scan
//!
//! The three name rows state bare length ranges, but their schema
//! CHECKs measure `length(trim(…))`, so the validator works on the
//! trimmed value and returns it for the caller to persist — the same
//! agreement contract as D-075's project name: Rust's `str::trim`
//! strips all Unicode whitespace, a SUPERSET of what SQLite's
//! `trim()` strips (U+0020 only), so a name validated here and stored
//! as returned satisfies `length(trim(…)) BETWEEN 1 AND …` at the
//! same length the validator measured. A name made entirely of
//! whitespace is rejected here as empty — stricter than the bare
//! CHECK, and the point of the trim.
//!
//! "One line" (§6.9 writes it for the instruction and heading names;
//! see below for the variant name) means the STORED name carries no
//! line terminator: after trimming, an inner `\n`, the `\r` of a
//! `\r\n`, or a lone inner `\r` is rejected with its code-point index
//! and character. A break at the very edges is not stored at all —
//! `str::trim` strips it before the check runs ("\nName\n" stores as
//! "Name"). The Unicode line and paragraph separators (U+2028/U+2029)
//! are whitespace, not `\n`/`\r`: they ride along as ordinary
//! characters, the same std-tables boundary D-075 recorded for
//! labels.
//!
//! The same scan reports a `NUL` (`U+0000`) with its code-point
//! index — the name-wide NUL rejection, D-076's open item closed by
//! D-126 (the `NUL` section below):
//! the length bounds fire first, then ONE positional scan reports the
//! first offending character, whichever rule it breaks — a name with
//! a line break at 1 and a NUL at 3 says the line break, one with the
//! two swapped says the NUL.
//!
//! The variant-name row says only "1–100 characters" — the one name
//! row without "one line". D-076 reads the omission as editorial (the
//! row's Limits column already carries the uniqueness and Main notes)
//! and extends the same one-line rule to it: a variant name is the
//! same kind of thing as the other two, a list label (the LIB-002
//! variant list), and the rule is what keeps it one. That extension
//! and the name-wide NUL rejection (the `NUL` section below) are the
//! module's deliberate divergences STRICTER than the bare rows;
//! everything else is verbatim.
//!
//! # The variant text: validation versus normalization
//!
//! The §6.9 row mixes two kinds of clauses, and they split by owner
//! (the split is D-076's centerpiece):
//!
//! | §6.9 clause                     | kind                          | owner |
//! | ------------------------------- | ----------------------------- | ----- |
//! | up to 262,144 characters (DB-006) | validation — rejection      | this module |
//! | `NUL` and reserved markers rejected | validation — rejection    | this module |
//! | CRLF is changed to LF on save   | normalization — transformation | the store layer's save fold |
//! | (lone CR, BOM, block edges)     | normalization — transformation | [`normalize::variant_text`] (ALG-002, D-044) |
//!
//! [`validate_variant_text`] therefore returns its input UNCHANGED:
//! validation, not normalization. It does not fold CRLF, does not
//! strip a BOM, does not trim edges — ALG-002's unconditional
//! build-time pass is the single owner of the file-facing form
//! (D-044), and the save-time "CRLF is changed to LF" fold is the
//! store's own §6.9 clause. The store order that preserves spec
//! fidelity is fold FIRST, then validate — the length then measures
//! exactly the stored form, which is what the schema CHECK sees.
//! Validating the raw form instead is conservative in one direction
//! only: it never lets a database violation through (raw length ≥
//! folded length — the fold only removes characters), but it can
//! reject a text §6.9 would allow, by one code point per CRLF — a
//! raw `"x\r\n"` paste keeps two thirds of its character count once
//! folded, so a raw 262,145 may be a legal stored 174,764. Unlike
//! the name
//! fields, the text has NO trim: neither the row nor the CHECK has
//! one, "empty is allowed" means the empty string, and a
//! whitespace-only text is a legal stored text that simply builds
//! to an empty block (D-13).
//!
//! # Reserved marker lines (DB-016)
//!
//! DB-016: variant text cannot "contain a line equal to the … marker"
//! — saving is rejected with the app-level `E_CONTENT_RESERVED`. The
//! scan reuses the two T-040 building blocks instead of restating
//! them, and widens DB-016's wording in the one direction that makes
//! the rule do its job:
//!
//! - **Which lines.** [`classify_line`] (D-062) decides, so a line
//!   padded with ASCII spaces and tabs counts — a padded marker line
//!   inside the managed block WOULD parse as a second marker pair in
//!   a scheme-B file, which is exactly the corruption DB-016 exists
//!   to prevent. A marker inside a longer line ("text `<!-- … -->`
//!   more"), a case-mangled spelling, or an inner-space variant is
//!   prose and passes.
//! - **Which text.** The scan runs over the text AS THE BUILD WRITES
//!   IT — one [`normalize::variant_text`] call, then a
//!   [`classify_line`] pass over the result's lines — because the
//!   line structure a scheme-B parse will see is post-ALG-002, where
//!   a lone `\r` has become LF and the leading BOM/empty-line edge
//!   is gone. A marker hidden behind a leading BOM
//!   (`"\u{feff}\n<!-- … -->"`) or exposed by a lone `\r`
//!   (`"x\r<!-- … -->"`) is therefore rejected too, and the verdict
//!   never depends on whether the caller already folded CRLF. The
//!   error's 1-based line number counts this built form — identical
//!   to the stored text's own numbering except where ALG-002 dropped
//!   the leading edge.
//! - **Which markers.** Exactly the two D-19 literals,
//!   [`BEGIN_MARKER`] and [`END_MARKER`], is what DB-016 reserves.
//!   The creation-marker line (D-061) is NOT reserved: DB-016 does
//!   not name it, it never matches [`classify_line`], and its
//!   timestamp makes an exact-match reservation meaningless anyway —
//!   a pasted copy is inert prose.
//!
//! # `NUL`
//!
//! §6.9's `NUL` rejection names the variant text only, and
//! [`validate_variant_text`] fires it there. The three NAME fields
//! reject a `NUL` (`U+0000`) as well — the open item D-076 left to a
//! validators-wide task, closed here (T-252):
//!
//! - SQLite's `length()` counts code points only UP TO the first
//!   `U+0000` (D-076, verified against SQLite directly), and every
//!   name CHECK leans on that arithmetic — a name whose FIRST
//!   character is NUL used to pass the length check here while the
//!   store's own `length(trim(…))` CHECK measured it as 0, so the
//!   insert failed with a raw database error instead of a field
//!   error. The instruction and variant names never enter the file
//!   (ALG-022) but share this stored edge.
//! - the heading name is the one name that reaches the target file
//!   verbatim (`## <title>` via the body's heading block), so a NUL
//!   there would put a NUL byte into `AGENTS.md`/`CLAUDE.md` — git's
//!   binary heuristic (it reads a file's leading bytes, where the
//!   managed heading sits) then treats the file as binary. That, not
//!   the SQLite arithmetic, is the reason the variant-text row bans
//!   NUL, and it holds for the heading name a fortiori.
//!
//! The rejection rides the name scan (see above): the first of a
//! line break or a NUL is reported with its code-point index, in the
//! same shape as [`LibraryFieldError::VariantTextNul`]. T-049's
//! project NAME carries the same rule in
//! [`crate::project_validators`]; the project LABEL needs none of
//! its own — D-075's control-character rejection already refuses
//! `U+0000` — and the closed-list fields (color, file name) cannot
//! match a NUL to a literal at all.
//!
//! # Errors
//!
//! Every failure is a typed [`LibraryFieldError`] variant naming the
//! field and the broken rule — never a string (the D-062/D-071/D-072/
//! D-075 precedent of typed errors at core boundaries). The
//! command layer maps the length, line-break and NUL failures to
//! `E_VALIDATION` (§8) and [`LibraryFieldError::ReservedContent`] to
//! `E_CONTENT_RESERVED` (§7.10's registry, D-034) in a later task —
//! the mapping is deliberately absent here, exactly as
//! [`crate::markers::MarkersError`] relates to `E_MARKERS_INVALID`
//! (D-062).
//!
//! # Examples
//!
//! The four fields of a library-editor save (LIB-002), each at a
//! legal value:
//!
//! ```
//! use eunomia_core::library_validators::{
//!     validate_heading_title, validate_instruction_title, validate_variant_name,
//!     validate_variant_text,
//! };
//!
//! let name = validate_instruction_title("  Be concise  ").unwrap(); // trimmed…
//! assert_eq!(name, "Be concise");
//! assert_eq!(validate_variant_name("Main").unwrap(), "Main");
//! assert_eq!(validate_variant_text("Be brief.\r\n").unwrap(), "Be brief.\r\n"); // …unchanged
//! assert_eq!(validate_heading_title("Style").unwrap(), "Style");
//!
//! // And the failures are typed, field by field:
//! use eunomia_core::library_validators::LibraryFieldError;
//!
//! assert_eq!(
//!     validate_instruction_title(&"a".repeat(201)).unwrap_err(),
//!     LibraryFieldError::InstructionTitleTooLong { length: 201 }
//! );
//! assert_eq!(
//!     validate_heading_title("a\0b").unwrap_err(),
//!     LibraryFieldError::HeadingTitleNul { index: 1 } // NUL (D-126)
//! );
//! assert_eq!(
//!     validate_variant_text("x\n<!-- EUNOMIASYNC:BEGIN -->").unwrap_err(),
//!     LibraryFieldError::ReservedContent {
//!         line: 2,
//!         marker: eunomia_core::markers::MarkerLine::Begin
//!     }
//! );
//! ```

use thiserror::Error;

use crate::markers::{BEGIN_MARKER, END_MARKER, MarkerLine, classify_line};
use crate::normalize;

/// The name-length limit of an instruction and of a heading (§6.9:
/// "1–200 characters, one line"; the §6.2 `library_items.title` CHECK).
const TITLE_LIMIT: usize = 200;

/// The name-length limit of a variant (§6.9: "1–100 characters"; the
/// §6.2 `instruction_variants.name` CHECK).
const VARIANT_NAME_LIMIT: usize = 100;

/// The character limit of variant text (DB-006: "Variant text is up
/// to 262,144 characters"; the §6.2 `content` CHECK
/// `length(content) <= 262144`).
const VARIANT_TEXT_LIMIT: usize = 262_144;

/// A library field that failed its §6.9 rule: which field, and what
/// is wrong with it.
///
/// One variant per field and failure — the command layer maps the
/// length, line-break and NUL failures to `E_VALIDATION` (§8) and
/// [`ReservedContent`](Self::ReservedContent) to `E_CONTENT_RESERVED`
/// (§7.10, D-034) with the field-scoped message; the variants carry
/// the measured length (or the offending index and character, or the
/// line and marker) so the UI can say exactly that.
///
/// # Examples
///
/// ```
/// use eunomia_core::library_validators::{validate_variant_text, LibraryFieldError};
///
/// let error = validate_variant_text(&"x".repeat(262_145)).unwrap_err();
/// assert_eq!(
///     error,
///     LibraryFieldError::VariantTextTooLong { length: 262_145 }
/// );
/// assert_eq!(
///     error.to_string(),
///     "variant text is 262145 characters; the limit is 262144"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LibraryFieldError {
    /// The instruction name is empty — the empty string, or nothing
    /// but whitespace, after trimming (§6.9: 1–200).
    #[error("instruction name is empty after trimming")]
    InstructionTitleEmpty,
    /// The instruction name is longer than 200 characters after
    /// trimming; the length is the trimmed, code-point count.
    #[error("instruction name is {length} characters after trimming; the limit is 200")]
    InstructionTitleTooLong {
        /// The trimmed length that exceeded the limit.
        length: usize,
    },
    /// The instruction name contains a line break (`\n`, the `\r` of
    /// a `\r\n`, or a lone `\r`) at `index` — §6.9: one line.
    #[error("instruction name contains the line break {character:?} at index {index}")]
    InstructionTitleLineBreak {
        /// The code-point index the break sits at.
        index: usize,
        /// The offending character.
        character: char,
    },
    /// The instruction name contains `NUL` (`U+0000`) at `index`. No
    /// §6.9 row bans it in a name; the D-126 validators-wide ruling
    /// does (D-076's open item; module docs) — a NUL breaks the
    /// SQLite `length()` arithmetic every name CHECK leans on.
    #[error("instruction name contains NUL at index {index}")]
    InstructionTitleNul {
        /// The code-point index in the TRIMMED value the validator
        /// measured (NUL is not whitespace, so the trim never
        /// removes one).
        index: usize,
    },
    /// The variant name is empty after trimming (§6.9: 1–100).
    #[error("variant name is empty after trimming")]
    VariantNameEmpty,
    /// The variant name is longer than 100 characters after
    /// trimming; the length is the trimmed, code-point count.
    #[error("variant name is {length} characters after trimming; the limit is 100")]
    VariantNameTooLong {
        /// The trimmed length that exceeded the limit.
        length: usize,
    },
    /// The variant name contains a line break at `index`. §6.9 writes
    /// "one line" for the other two name rows only; the D-076 ruling
    /// extends it here (a variant name is a list label like they
    /// are).
    #[error("variant name contains the line break {character:?} at index {index}")]
    VariantNameLineBreak {
        /// The code-point index the break sits at.
        index: usize,
        /// The offending character.
        character: char,
    },
    /// The variant name contains `NUL` (`U+0000`) at `index` — the
    /// D-126 validators-wide ruling (D-076's open item; module
    /// docs), the same rejection the variant-name row's TEXT sibling
    /// has always had.
    #[error("variant name contains NUL at index {index}")]
    VariantNameNul {
        /// The code-point index in the TRIMMED value the validator
        /// measured (NUL is not whitespace, so the trim never
        /// removes one).
        index: usize,
    },
    /// The heading name is empty — the empty string, or nothing but
    /// whitespace, after trimming (§6.9: 1–200).
    #[error("heading name is empty after trimming")]
    HeadingTitleEmpty,
    /// The heading name is longer than 200 characters after
    /// trimming; the length is the trimmed, code-point count.
    #[error("heading name is {length} characters after trimming; the limit is 200")]
    HeadingTitleTooLong {
        /// The trimmed length that exceeded the limit.
        length: usize,
    },
    /// The heading name contains a line break at `index` — §6.9: one
    /// line.
    #[error("heading name contains the line break {character:?} at index {index}")]
    HeadingTitleLineBreak {
        /// The code-point index the break sits at.
        index: usize,
        /// The offending character.
        character: char,
    },
    /// The heading name contains `NUL` (`U+0000`) at `index` — the
    /// D-126 validators-wide ruling (D-076's open item; module
    /// docs). The heading name is the one name written into the
    /// target file verbatim (`## <title>`), where a NUL byte would
    /// make git's binary heuristic treat the file as binary.
    #[error("heading name contains NUL at index {index}")]
    HeadingTitleNul {
        /// The code-point index in the TRIMMED value the validator
        /// measured (NUL is not whitespace, so the trim never
        /// removes one).
        index: usize,
    },
    /// The variant text is longer than 262,144 characters (DB-006).
    #[error("variant text is {length} characters; the limit is 262144")]
    VariantTextTooLong {
        /// The code-point length that exceeded the limit.
        length: usize,
    },
    /// The variant text contains `NUL` (`U+0000`) at `index`
    /// (§6.9: "`NUL` … rejected").
    #[error("variant text contains NUL at index {index}")]
    VariantTextNul {
        /// The code-point index the character sits at.
        index: usize,
    },
    /// The variant text contains a reserved marker line (DB-016):
    /// saving is rejected with the app-level `E_CONTENT_RESERVED`.
    ///
    /// `line` is the 1-based line number in the text as the build
    /// writes it (the ALG-002 form — see the module docs), and
    /// `marker` is which of the two D-19 literals the line holds.
    #[error(
        "variant text contains the reserved {} marker line on line {line}",
        marker_literal(.marker)
    )]
    ReservedContent {
        /// The 1-based line number, over the built form.
        line: usize,
        /// The marker the line equals (modulo padding).
        marker: MarkerLine,
    },
}

/// The D-19 literal a reserved line carried, for the error message.
fn marker_literal(marker: &MarkerLine) -> &'static str {
    match marker {
        MarkerLine::Begin => BEGIN_MARKER,
        MarkerLine::End => END_MARKER,
    }
}

/// The shared name-field verdict: why a trimmed name failed its
/// range-or-character rule, before the field's own error says which
/// field it was (the three name fields share one shape; see the
/// module docs).
enum NameProblem {
    /// Empty after trimming.
    Empty,
    /// Longer than the field's limit; the trimmed, code-point count.
    TooLong(usize),
    /// A line break at the code-point index.
    LineBreak { index: usize, character: char },
    /// A `NUL` (`U+0000`) at the code-point index (the D-126
    /// validators-wide ruling, closing D-076's open item).
    Nul { index: usize },
}

/// The shared body of the three §6.9 name validators (module docs):
/// the input must already be TRIMMED — the caller measures and
/// returns that value — and the checks fire in a fixed order so the
/// message is deterministic: the length bounds first, then one
/// positional scan that reports the FIRST offending character — a
/// line break (`\n` or `\r`, §6.9's one-line rule) or a `NUL`
/// (`U+0000`, the name-wide NUL rejection — D-126, closing D-076's
/// open item) — with its code-point
/// index.
fn name_problem(trimmed: &str, limit: usize) -> Option<NameProblem> {
    let length = trimmed.chars().count();
    if length == 0 {
        Some(NameProblem::Empty)
    } else if length > limit {
        Some(NameProblem::TooLong(length))
    } else {
        trimmed.chars().enumerate().find_map(|(index, character)| {
            if character == '\n' || character == '\r' {
                Some(NameProblem::LineBreak { index, character })
            } else if character == '\0' {
                Some(NameProblem::Nul { index })
            } else {
                None
            }
        })
    }
}

/// Validate an instruction's main name (§6.9: "1–200 characters, one
/// line"; the LIB-002 create dialog's main name field).
///
/// The input is trimmed — all Unicode whitespace — and the TRIMMED
/// string is what gets measured and returned: callers persist the
/// returned value, which keeps the schema's
/// `length(trim(title)) BETWEEN 1 AND 200` and this rule in exact
/// agreement (the D-075 contract; see the module docs). A name that
/// is empty after trimming fails with
/// [`LibraryFieldError::InstructionTitleEmpty`]; one over 200
/// characters with [`InstructionTitleTooLong`](Self::InstructionTitleTooLong);
/// an inner line break (`\n`, `\r\n`'s `\r`, or a lone `\r`) with
/// [`InstructionTitleLineBreak`](Self::InstructionTitleLineBreak) —
/// edge breaks are trimmed away before the check, so "\nName\n"
/// stores as "Name"; and a `NUL` (`U+0000`) anywhere with
/// [`InstructionTitleNul`](Self::InstructionTitleNul) — the
/// name-wide NUL rejection (D-126, closing D-076's open item;
/// module docs).
///
/// The row's "unique among visible instructions, case-insensitive"
/// clause is a cross-row store query, not a field rule (module docs).
///
/// # Examples
///
/// ```
/// use eunomia_core::library_validators::validate_instruction_title;
///
/// assert_eq!(validate_instruction_title("  Be concise  ").unwrap(), "Be concise");
/// assert_eq!(validate_instruction_title(&"a".repeat(200)).unwrap(), "a".repeat(200));
/// assert!(validate_instruction_title(&"a".repeat(201)).is_err());
/// assert!(validate_instruction_title("a\nb").is_err()); // one line
/// assert!(validate_instruction_title("a\0b").is_err()); // NUL (D-126)
/// ```
pub fn validate_instruction_title(title: &str) -> Result<String, LibraryFieldError> {
    let trimmed = title.trim();
    match name_problem(trimmed, TITLE_LIMIT) {
        None => Ok(trimmed.to_owned()),
        Some(NameProblem::Empty) => Err(LibraryFieldError::InstructionTitleEmpty),
        Some(NameProblem::TooLong(length)) => {
            Err(LibraryFieldError::InstructionTitleTooLong { length })
        }
        Some(NameProblem::LineBreak { index, character }) => {
            Err(LibraryFieldError::InstructionTitleLineBreak { index, character })
        }
        Some(NameProblem::Nul { index }) => Err(LibraryFieldError::InstructionTitleNul { index }),
    }
}

/// Validate a variant's name (§6.9: "1–100 characters"; the LIB-002
/// variant list).
///
/// The same trim-then-count contract as
/// [`validate_instruction_title`], with the variant limit of 100. The
/// row does not repeat "one line", but a variant name is a list label
/// like the other two names, so the D-076 ruling extends the same
/// one-line rule here — an inner `\n` or `\r` fails with
/// [`LibraryFieldError::VariantNameLineBreak`] — and the name-wide
/// NUL rejection (D-126, closing D-076's open item) applies too: a
/// `U+0000` anywhere fails
/// with [`LibraryFieldError::VariantNameNul`]. The row's
/// "unique within the instruction, case-insensitive" clause and the
/// `Main` default are the store's and the UI's business (module
/// docs).
///
/// # Examples
///
/// ```
/// use eunomia_core::library_validators::validate_variant_name;
///
/// assert_eq!(validate_variant_name("Main").unwrap(), "Main");
/// assert_eq!(validate_variant_name(&"a".repeat(100)).unwrap(), "a".repeat(100));
/// assert!(validate_variant_name(&"a".repeat(101)).is_err());
/// assert!(validate_variant_name("a\nb").is_err()); // one line (D-076)
/// assert!(validate_variant_name("a\0b").is_err()); // NUL (D-126)
/// ```
pub fn validate_variant_name(name: &str) -> Result<String, LibraryFieldError> {
    let trimmed = name.trim();
    match name_problem(trimmed, VARIANT_NAME_LIMIT) {
        None => Ok(trimmed.to_owned()),
        Some(NameProblem::Empty) => Err(LibraryFieldError::VariantNameEmpty),
        Some(NameProblem::TooLong(length)) => Err(LibraryFieldError::VariantNameTooLong { length }),
        Some(NameProblem::LineBreak { index, character }) => {
            Err(LibraryFieldError::VariantNameLineBreak { index, character })
        }
        Some(NameProblem::Nul { index }) => Err(LibraryFieldError::VariantNameNul { index }),
    }
}

/// Validate a heading's name (§6.9: "1–200 characters, one line";
/// the LIB-002 create dialog's heading name field).
///
/// The same trim-then-count-and-one-line contract as
/// [`validate_instruction_title`] — the two fields share the
/// `library_items.title` column and its CHECK — plus the name-wide
/// NUL rejection (D-126, closing D-076's open item): a `U+0000`
/// anywhere fails with
/// [`LibraryFieldError::HeadingTitleNul`]. This is the sharpest of
/// the three names: a heading title is written into the target file
/// verbatim (`## <title>`), so a NUL here would put a NUL byte into
/// `AGENTS.md`/`CLAUDE.md` — git then treats the file as binary
/// (module docs). The row's "Level 2–6" needs no validator: the
/// closed [`HeadingLevel`](crate::library::HeadingLevel) type already
/// makes every other level unrepresentable (D-042; module docs).
///
/// # Examples
///
/// ```
/// use eunomia_core::library_validators::validate_heading_title;
///
/// assert_eq!(validate_heading_title("Style").unwrap(), "Style");
/// assert_eq!(validate_heading_title(&"a".repeat(200)).unwrap(), "a".repeat(200));
/// assert!(validate_heading_title(&"a".repeat(201)).is_err());
/// assert!(validate_heading_title("a\rb").is_err()); // one line
/// assert!(validate_heading_title("a\0b").is_err()); // NUL (D-126)
/// ```
pub fn validate_heading_title(title: &str) -> Result<String, LibraryFieldError> {
    let trimmed = title.trim();
    match name_problem(trimmed, TITLE_LIMIT) {
        None => Ok(trimmed.to_owned()),
        Some(NameProblem::Empty) => Err(LibraryFieldError::HeadingTitleEmpty),
        Some(NameProblem::TooLong(length)) => {
            Err(LibraryFieldError::HeadingTitleTooLong { length })
        }
        Some(NameProblem::LineBreak { index, character }) => {
            Err(LibraryFieldError::HeadingTitleLineBreak { index, character })
        }
        Some(NameProblem::Nul { index }) => Err(LibraryFieldError::HeadingTitleNul { index }),
    }
}

/// Validate a variant's text (§6.9: "up to 262,144 characters, empty
/// is allowed" with "`NUL` and reserved markers are rejected"; DB-006,
/// DB-016).
///
/// Validation, not normalization (the D-076 split; module docs): the
/// input is returned UNCHANGED on success — the save-time CRLF→LF
/// fold is the store layer's own §6.9 clause, and the build-time
/// lone-CR/BOM/edge form is [`normalize::variant_text`]'s (ALG-002,
/// D-044). The three rejection rules fire in a fixed order so the
/// message is deterministic:
///
/// 1. length over 262,144 code points —
///    [`VariantTextTooLong`](Self::VariantTextTooLong);
/// 2. the first `NUL`, by code-point index —
///    [`VariantTextNul`](Self::VariantTextNul);
/// 3. a reserved marker line (DB-016), found by scanning the text as
///    the build writes it — one [`normalize::variant_text`] pass,
///    then [`classify_line`] over the result's lines — so a marker
///    hidden behind a leading BOM or exposed by a lone `\r` is
///    caught too; the failure is
///    [`ReservedContent`](Self::ReservedContent) with the built
///    form's 1-based line number and which D-19 literal.
///
/// Empty is legal (an empty variant still counts as assigned, D-13),
/// and there is no trim: a whitespace-only text is a legal stored
/// text that builds to an empty block.
///
/// # Examples
///
/// ```
/// use eunomia_core::library_validators::validate_variant_text;
///
/// assert_eq!(validate_variant_text("").unwrap(), ""); // empty is allowed
/// // CRLF stays: the fold is the save layer's, not the validator's.
/// assert_eq!(validate_variant_text("Be brief.\r\n").unwrap(), "Be brief.\r\n");
/// assert!(validate_variant_text("a\0b").is_err()); // NUL
/// assert!(validate_variant_text("x\n<!-- EUNOMIASYNC:END -->").is_err()); // DB-016
/// ```
pub fn validate_variant_text(text: &str) -> Result<&str, LibraryFieldError> {
    let length = text.chars().count();
    if length > VARIANT_TEXT_LIMIT {
        return Err(LibraryFieldError::VariantTextTooLong { length });
    }
    if let Some(index) = text.chars().position(|character| character == '\0') {
        return Err(LibraryFieldError::VariantTextNul { index });
    }
    // DB-016 over the text as the build writes it: ALG-002's form is
    // the line structure a scheme-B parse will see, so the scan runs
    // there (module docs) — a marker behind a leading BOM or behind
    // a lone CR is as reserved as a plain one.
    let built = normalize::variant_text(text);
    let reserved = built
        .lines()
        .enumerate()
        .find_map(|(index, line)| classify_line(line).map(|marker| (index + 1, marker)));
    if let Some((line, marker)) = reserved {
        return Err(LibraryFieldError::ReservedContent { line, marker });
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- instruction name: 0 / 1 / 200 / 201 after trim ------------------

    #[test]
    fn instruction_name_boundaries_after_trim() {
        // 0 after trimming: the empty string, plain spaces, and a mix
        // of Unicode whitespace (tab, newline, NBSP, ideographic
        // space).
        for empty in ["", " ", "   ", "\t", "\n \u{00A0}\u{3000}"] {
            assert_eq!(
                validate_instruction_title(empty),
                Err(LibraryFieldError::InstructionTitleEmpty)
            );
        }
        // 1 and 200: legal.
        assert_eq!(validate_instruction_title("A").unwrap(), "A");
        let limit = "a".repeat(200);
        assert_eq!(validate_instruction_title(&limit).unwrap(), limit);
        // 201: one over — the schema CHECK's own bound.
        assert_eq!(
            validate_instruction_title(&"a".repeat(201)).unwrap_err(),
            LibraryFieldError::InstructionTitleTooLong { length: 201 }
        );
    }

    #[test]
    fn instruction_name_is_trimmed_at_the_edges_and_keeps_inner_spaces() {
        // Trim FIRST, then count: 200 characters with whitespace
        // around them still fit.
        let limit = "a".repeat(200);
        let padded = format!("  {limit}\t");
        assert_eq!(validate_instruction_title(&padded).unwrap(), limit);
        // Inner spaces are content, not edges.
        assert_eq!(
            validate_instruction_title("  Be concise  ").unwrap(),
            "Be concise"
        );
        // The trim is Unicode-wide: NBSP and ideographic space edges
        // go too.
        assert_eq!(
            validate_instruction_title("\u{00A0}Omega\u{3000}").unwrap(),
            "Omega"
        );
    }

    #[test]
    fn instruction_name_counts_code_points_not_bytes_or_graphemes() {
        // "é" is one code point however many bytes it takes: 200 fit,
        // 201 do not.
        assert_eq!(
            validate_instruction_title(&"é".repeat(200)).unwrap(),
            "é".repeat(200)
        );
        assert_eq!(
            validate_instruction_title(&"é".repeat(201)).unwrap_err(),
            LibraryFieldError::InstructionTitleTooLong { length: 201 }
        );
        // "e" + combining accent is TWO code points, one grapheme:
        // 101 of them measure 202 — over the limit at 101 graphemes.
        // The unit matches SQLite's length() on TEXT.
        assert_eq!(
            validate_instruction_title(&"e\u{301}".repeat(101)).unwrap_err(),
            LibraryFieldError::InstructionTitleTooLong { length: 202 }
        );
        // An astral-plane character is still one code point.
        assert_eq!(
            validate_instruction_title(&"\u{1F600}".repeat(200))
                .unwrap()
                .len(),
            800
        );
    }

    #[test]
    fn instruction_name_is_one_line() {
        // §6.9: one line. An inner \n fails with its index…
        assert_eq!(
            validate_instruction_title("a\nb").unwrap_err(),
            LibraryFieldError::InstructionTitleLineBreak {
                index: 1,
                character: '\n'
            }
        );
        // …a \r\n fails on the \r (the first break the scan meets)…
        assert_eq!(
            validate_instruction_title("a\r\nb").unwrap_err(),
            LibraryFieldError::InstructionTitleLineBreak {
                index: 1,
                character: '\r'
            }
        );
        // …and a lone \r fails just the same: all three are line
        // terminators in the file model (ALG-002 turns a lone CR
        // into LF too).
        assert_eq!(
            validate_instruction_title("a\rb").unwrap_err(),
            LibraryFieldError::InstructionTitleLineBreak {
                index: 1,
                character: '\r'
            }
        );
        // The index counts CODE POINTS, not bytes: after a two-byte
        // "é" the break is at index 1.
        assert_eq!(
            validate_instruction_title("é\nx").unwrap_err(),
            LibraryFieldError::InstructionTitleLineBreak {
                index: 1,
                character: '\n'
            }
        );
        assert_eq!(
            validate_instruction_title("\u{1F600}\rx").unwrap_err(),
            LibraryFieldError::InstructionTitleLineBreak {
                index: 1,
                character: '\r'
            }
        );
        // A break at the EDGES is not stored at all — trim strips it
        // before the one-line check runs.
        assert_eq!(validate_instruction_title("\nName\n").unwrap(), "Name");
        assert_eq!(validate_instruction_title("Name\r\n").unwrap(), "Name");
    }

    #[test]
    fn instruction_name_precedence_is_length_then_first_offending_character() {
        // A name can break several rules at once; the checks fire in
        // a fixed order so the message is deterministic.
        // 201 characters AND a trailing break: the trim eats the
        // break, and the length wins at 201.
        assert_eq!(
            validate_instruction_title(&format!("{}\n", "a".repeat(201))).unwrap_err(),
            LibraryFieldError::InstructionTitleTooLong { length: 201 }
        );
        // Whitespace-only: empty, not a line-break failure.
        assert_eq!(
            validate_instruction_title(" \n ").unwrap_err(),
            LibraryFieldError::InstructionTitleEmpty
        );
        // Too long beats the character scan, NUL included.
        let mut long = "a".repeat(201);
        long.push('\0');
        assert_eq!(
            validate_instruction_title(&long).unwrap_err(),
            LibraryFieldError::InstructionTitleTooLong { length: 202 }
        );
        // …and between a line break and a NUL, the FIRST one in the
        // string wins — one positional scan (module docs).
        assert_eq!(
            validate_instruction_title("a\nb\0c").unwrap_err(),
            LibraryFieldError::InstructionTitleLineBreak {
                index: 1,
                character: '\n'
            }
        );
        assert_eq!(
            validate_instruction_title("a\0b\nc").unwrap_err(),
            LibraryFieldError::InstructionTitleNul { index: 1 }
        );
    }

    #[test]
    fn instruction_name_std_boundary_line_separators_ride_along() {
        // U+2028/U+2029 are Unicode whitespace, not \n or \r: they
        // ride along as ordinary characters, the same std-tables
        // boundary D-075 recorded for labels (module docs).
        assert_eq!(
            validate_instruction_title("A\u{2028}B").unwrap(),
            "A\u{2028}B"
        );
        assert_eq!(
            validate_instruction_title("A\u{2029}B").unwrap(),
            "A\u{2029}B"
        );
    }

    #[test]
    fn instruction_name_nul_is_rejected_at_any_position() {
        // The D-076 open item, closed for every name field: a NUL in
        // a stored name breaks the SQLite length() arithmetic the
        // CHECK leans on (a NUL-first name measures 0), so the only
        // NUL, one in the middle, and one at the end all fail with
        // the code-point index.
        assert_eq!(
            validate_instruction_title("\0").unwrap_err(),
            LibraryFieldError::InstructionTitleNul { index: 0 }
        );
        assert_eq!(
            validate_instruction_title("a\0b").unwrap_err(),
            LibraryFieldError::InstructionTitleNul { index: 1 }
        );
        assert_eq!(
            validate_instruction_title("a\0").unwrap_err(),
            LibraryFieldError::InstructionTitleNul { index: 1 }
        );
        // The index counts CODE POINTS: after a two-byte "é" the NUL
        // sits at index 1.
        assert_eq!(
            validate_instruction_title("é\0x").unwrap_err(),
            LibraryFieldError::InstructionTitleNul { index: 1 }
        );
        // Only the FIRST NUL is reported.
        assert_eq!(
            validate_instruction_title("a\0b\0c").unwrap_err(),
            LibraryFieldError::InstructionTitleNul { index: 1 }
        );
        // NUL is not whitespace: the trim never hides it — a padded
        // NUL-only name trims to the bare NUL and still fails.
        assert_eq!(
            validate_instruction_title(" \0 ").unwrap_err(),
            LibraryFieldError::InstructionTitleNul { index: 0 }
        );
    }

    #[test]
    fn instruction_name_errors_say_what_is_wrong() {
        assert_eq!(
            validate_instruction_title("   ").unwrap_err().to_string(),
            "instruction name is empty after trimming"
        );
        assert_eq!(
            validate_instruction_title(&"a".repeat(201))
                .unwrap_err()
                .to_string(),
            "instruction name is 201 characters after trimming; the limit is 200"
        );
        assert_eq!(
            validate_instruction_title("a\nb").unwrap_err().to_string(),
            "instruction name contains the line break '\\n' at index 1"
        );
        assert_eq!(
            validate_instruction_title("a\0b").unwrap_err().to_string(),
            "instruction name contains NUL at index 1"
        );
    }

    // ---- variant name: 0 / 1 / 100 / 101 -----------------------------------

    #[test]
    fn variant_name_boundaries_after_trim() {
        for empty in ["", " ", "\t\u{00A0}"] {
            assert_eq!(
                validate_variant_name(empty),
                Err(LibraryFieldError::VariantNameEmpty)
            );
        }
        // 1 and 100: legal. The §6.9 default name validates like any
        // other, case kept.
        assert_eq!(validate_variant_name("M").unwrap(), "M");
        assert_eq!(validate_variant_name("Main").unwrap(), "Main");
        let limit = "a".repeat(100);
        assert_eq!(validate_variant_name(&limit).unwrap(), limit);
        // 101: one over — the schema CHECK's own bound.
        assert_eq!(
            validate_variant_name(&"a".repeat(101)).unwrap_err(),
            LibraryFieldError::VariantNameTooLong { length: 101 }
        );
        // Code points, not bytes: 101 two-byte characters measure
        // 101.
        assert_eq!(
            validate_variant_name(&"é".repeat(101)).unwrap_err(),
            LibraryFieldError::VariantNameTooLong { length: 101 }
        );
    }

    #[test]
    fn variant_name_is_trimmed_and_keeps_case() {
        assert_eq!(validate_variant_name("  Main  ").unwrap(), "Main");
        assert_eq!(validate_variant_name("main").unwrap(), "main");
        assert_eq!(validate_variant_name("SECOND").unwrap(), "SECOND");
        // Inner spaces stay; only the edges go.
        assert_eq!(validate_variant_name("Alt version").unwrap(), "Alt version");
    }

    #[test]
    fn variant_name_is_one_line_the_d076_ruling() {
        // §6.9 writes "one line" for the instruction and heading name
        // rows only; D-076 extends it to the variant name — a list
        // label like the others (LIB-002's variant list).
        // …and a lone \r fails just the same: all three are line
        // terminators in the file model (ALG-002 turns a lone CR
        // into LF too). Edge breaks ("Alt\n", "\nAlt") are trimmed
        // away instead — pinned two assertions down.
        for broken in ["a\nb", "a\r\nb", "a\rb"] {
            assert!(
                validate_variant_name(broken).is_err(),
                "{broken:?} must not validate"
            );
        }
        // The failure is the typed line-break variant, with the
        // code-point index.
        assert_eq!(
            validate_variant_name("a\nb").unwrap_err(),
            LibraryFieldError::VariantNameLineBreak {
                index: 1,
                character: '\n'
            }
        );
        assert_eq!(
            validate_variant_name("a\r\nb").unwrap_err(),
            LibraryFieldError::VariantNameLineBreak {
                index: 1,
                character: '\r'
            }
        );
        // Edge breaks are trimmed away, not rejected — the stored
        // form is one line.
        assert_eq!(validate_variant_name("\nMain\n").unwrap(), "Main");
    }

    #[test]
    fn variant_name_nul_is_rejected_at_any_position() {
        // The name-wide NUL rejection (D-076's open item, closed by
        // D-126; module docs): the variant NAME now refuses what its
        // text sibling always has.
        assert_eq!(
            validate_variant_name("\0").unwrap_err(),
            LibraryFieldError::VariantNameNul { index: 0 }
        );
        assert_eq!(
            validate_variant_name("a\0b").unwrap_err(),
            LibraryFieldError::VariantNameNul { index: 1 }
        );
        assert_eq!(
            validate_variant_name("a\0").unwrap_err(),
            LibraryFieldError::VariantNameNul { index: 1 }
        );
        // The index counts code points, only the FIRST NUL is
        // reported, and the trim never hides one.
        assert_eq!(
            validate_variant_name("é\0x\0").unwrap_err(),
            LibraryFieldError::VariantNameNul { index: 1 }
        );
        assert_eq!(
            validate_variant_name(" \0 ").unwrap_err(),
            LibraryFieldError::VariantNameNul { index: 0 }
        );
        // First offending character wins against a line break: the
        // NUL at 1 precedes the \n at 3.
        assert_eq!(
            validate_variant_name("a\0b\nc").unwrap_err(),
            LibraryFieldError::VariantNameNul { index: 1 }
        );
    }

    #[test]
    fn variant_name_errors_say_what_is_wrong() {
        assert_eq!(
            validate_variant_name("  ").unwrap_err().to_string(),
            "variant name is empty after trimming"
        );
        assert_eq!(
            validate_variant_name(&"a".repeat(101))
                .unwrap_err()
                .to_string(),
            "variant name is 101 characters after trimming; the limit is 100"
        );
        assert_eq!(
            validate_variant_name("a\rb").unwrap_err().to_string(),
            "variant name contains the line break '\\r' at index 1"
        );
        assert_eq!(
            validate_variant_name("a\0b").unwrap_err().to_string(),
            "variant name contains NUL at index 1"
        );
    }

    // ---- heading name: 0 / 1 / 200 / 201 ----------------------------------

    #[test]
    fn heading_name_boundaries_after_trim() {
        for empty in ["", " ", "\n\t "] {
            assert_eq!(
                validate_heading_title(empty),
                Err(LibraryFieldError::HeadingTitleEmpty)
            );
        }
        // 1 and 200: legal.
        assert_eq!(validate_heading_title("S").unwrap(), "S");
        let limit = "a".repeat(200);
        assert_eq!(validate_heading_title(&limit).unwrap(), limit);
        // 201: one over — the shared `library_items.title` CHECK.
        assert_eq!(
            validate_heading_title(&"a".repeat(201)).unwrap_err(),
            LibraryFieldError::HeadingTitleTooLong { length: 201 }
        );
        // Code points, not bytes.
        assert_eq!(
            validate_heading_title(&"é".repeat(201)).unwrap_err(),
            LibraryFieldError::HeadingTitleTooLong { length: 201 }
        );
    }

    #[test]
    fn heading_name_is_trimmed_and_one_line() {
        assert_eq!(validate_heading_title("  Style  ").unwrap(), "Style");
        // One line: \n, \r\n and a lone \r all fail, at the
        // code-point index.
        assert_eq!(
            validate_heading_title("a\nb").unwrap_err(),
            LibraryFieldError::HeadingTitleLineBreak {
                index: 1,
                character: '\n'
            }
        );
        assert_eq!(
            validate_heading_title("a\r\nb").unwrap_err(),
            LibraryFieldError::HeadingTitleLineBreak {
                index: 1,
                character: '\r'
            }
        );
        assert_eq!(
            validate_heading_title("a\rb").unwrap_err(),
            LibraryFieldError::HeadingTitleLineBreak {
                index: 1,
                character: '\r'
            }
        );
        // Edge breaks are trimmed away.
        assert_eq!(validate_heading_title("\nStyle\r\n").unwrap(), "Style");
        // The row's "Level 2–6" half is the closed HeadingLevel
        // type's business (D-042) — nothing here re-checks it.
    }

    #[test]
    fn heading_name_nul_is_rejected_at_any_position() {
        // The sharpest of the three names (module docs): a heading
        // title reaches the target file verbatim (`## <title>`), so a
        // NUL here would put a NUL byte into AGENTS.md and git would
        // treat the file as binary — the D-076 consequence.
        assert_eq!(
            validate_heading_title("\0").unwrap_err(),
            LibraryFieldError::HeadingTitleNul { index: 0 }
        );
        assert_eq!(
            validate_heading_title("a\0b").unwrap_err(),
            LibraryFieldError::HeadingTitleNul { index: 1 }
        );
        assert_eq!(
            validate_heading_title("a\0").unwrap_err(),
            LibraryFieldError::HeadingTitleNul { index: 1 }
        );
        assert_eq!(
            validate_heading_title("é\0x\0").unwrap_err(),
            LibraryFieldError::HeadingTitleNul { index: 1 }
        );
        assert_eq!(
            validate_heading_title(" \0 ").unwrap_err(),
            LibraryFieldError::HeadingTitleNul { index: 0 }
        );
        // First offending character wins against a line break.
        assert_eq!(
            validate_heading_title("a\0b\nc").unwrap_err(),
            LibraryFieldError::HeadingTitleNul { index: 1 }
        );
    }

    #[test]
    fn heading_name_errors_say_what_is_wrong() {
        assert_eq!(
            validate_heading_title(" ").unwrap_err().to_string(),
            "heading name is empty after trimming"
        );
        assert_eq!(
            validate_heading_title(&"a".repeat(201))
                .unwrap_err()
                .to_string(),
            "heading name is 201 characters after trimming; the limit is 200"
        );
        assert_eq!(
            validate_heading_title("a\nb").unwrap_err().to_string(),
            "heading name contains the line break '\\n' at index 1"
        );
        assert_eq!(
            validate_heading_title("a\0b").unwrap_err().to_string(),
            "heading name contains NUL at index 1"
        );
    }

    // ---- variant text: 0 / 1 / 262,144 / 262,145 --------------------------

    #[test]
    fn variant_text_boundaries_empty_through_the_limit() {
        // 0 is legal — an empty variant still counts as assigned
        // (D-13) — and so is 1.
        assert_eq!(validate_variant_text("").unwrap(), "");
        assert_eq!(validate_variant_text("x").unwrap(), "x");
        // 262,144 exactly: legal, DB-006's own bound.
        let limit = "x".repeat(262_144);
        assert_eq!(validate_variant_text(&limit).unwrap(), limit.as_str());
        // 262,145: one over.
        assert_eq!(
            validate_variant_text(&"x".repeat(262_145)).unwrap_err(),
            LibraryFieldError::VariantTextTooLong { length: 262_145 }
        );
        // Code points, not bytes or graphemes: 262,145 two-byte
        // characters measure 262,145…
        assert_eq!(
            validate_variant_text(&"é".repeat(262_145)).unwrap_err(),
            LibraryFieldError::VariantTextTooLong { length: 262_145 }
        );
        // …and "e" + combining accent counts two per grapheme.
        assert_eq!(
            validate_variant_text(&"e\u{301}".repeat(131_073)).unwrap_err(),
            LibraryFieldError::VariantTextTooLong { length: 262_146 }
        );
    }

    #[test]
    fn variant_text_is_verbatim_no_fold_no_trim() {
        // Validation, not normalization (the D-076 split): CRLF stays
        // — the save-time fold is the store's §6.9 clause, the
        // build-time form is normalize::variant_text's (D-044).
        assert_eq!(validate_variant_text("one\r\ntwo").unwrap(), "one\r\ntwo");
        // No trim either: a whitespace-only text is a legal stored
        // text that builds to an empty block — "empty is allowed"
        // means the empty string, and nothing else is emptied here.
        assert_eq!(validate_variant_text(" \n\t ").unwrap(), " \n\t ");
        assert_eq!(validate_variant_text("\r").unwrap(), "\r");
    }

    #[test]
    fn variant_text_raw_length_over_the_fold_edge_is_conservative() {
        // The raw-form contract (module docs): raw length ≥ folded
        // length, so a raw length over the limit that FOLDS under it
        // is still rejected — the conservative direction, and why the
        // store folds before validating. 87,382 × "x\r\n" is 262,146
        // raw characters but 174,764 folded: DB-006-legal once the
        // §6.9 save fold runs, rejected if handed over raw.
        let raw = "x\r\n".repeat(87_382);
        assert_eq!(
            validate_variant_text(&raw).unwrap_err(),
            LibraryFieldError::VariantTextTooLong { length: 262_146 }
        );
        // …and the folded form itself is legal, marker-scan included.
        let folded = normalize::variant_text(&raw);
        assert!(validate_variant_text(&folded).is_ok());
    }

    #[test]
    fn variant_text_nul_is_rejected_with_the_code_point_index() {
        // §6.9: NUL rejected, at any position, with the code-point
        // index of the first one.
        assert_eq!(
            validate_variant_text("\0").unwrap_err(),
            LibraryFieldError::VariantTextNul { index: 0 }
        );
        assert_eq!(
            validate_variant_text("a\0b").unwrap_err(),
            LibraryFieldError::VariantTextNul { index: 1 }
        );
        // The index counts code points: after a two-byte "é" the NUL
        // is at index 1, and after an astral-plane character at 1.
        assert_eq!(
            validate_variant_text("é\0").unwrap_err(),
            LibraryFieldError::VariantTextNul { index: 1 }
        );
        assert_eq!(
            validate_variant_text("\u{1F600}\0").unwrap_err(),
            LibraryFieldError::VariantTextNul { index: 1 }
        );
        // Between a CR and its LF the NUL still counts as content…
        assert_eq!(
            validate_variant_text("\r\0\n").unwrap_err(),
            LibraryFieldError::VariantTextNul { index: 1 }
        );
        // …and only the FIRST NUL is reported.
        assert_eq!(
            validate_variant_text("a\0b\0c").unwrap_err(),
            LibraryFieldError::VariantTextNul { index: 1 }
        );
    }

    #[test]
    fn variant_text_precedence_is_length_then_nul_then_reserved() {
        // A text can break all three rules at once; the checks fire
        // in a fixed order so the error code is deterministic.
        // Too long beats NUL…
        let mut long = "x".repeat(262_145);
        long.push('\0');
        assert_eq!(
            validate_variant_text(&long).unwrap_err(),
            LibraryFieldError::VariantTextTooLong { length: 262_146 }
        );
        // …and NUL beats the reserved marker.
        assert_eq!(
            validate_variant_text("a\0\n<!-- EUNOMIASYNC:BEGIN -->").unwrap_err(),
            LibraryFieldError::VariantTextNul { index: 1 }
        );
    }

    // ---- variant text: the reserved marker lines (DB-016) -----------------

    #[test]
    fn variant_text_rejects_each_marker_as_a_line() {
        // DB-016: a line equal to the marker — BEGIN…
        assert_eq!(
            validate_variant_text("x\n<!-- EUNOMIASYNC:BEGIN -->\ny").unwrap_err(),
            LibraryFieldError::ReservedContent {
                line: 2,
                marker: MarkerLine::Begin
            }
        );
        // …and END.
        assert_eq!(
            validate_variant_text("x\n<!-- EUNOMIASYNC:END -->\ny").unwrap_err(),
            LibraryFieldError::ReservedContent {
                line: 2,
                marker: MarkerLine::End
            }
        );
        // The very first line, and the last line without a trailing
        // break, are lines too.
        assert_eq!(
            validate_variant_text(BEGIN_MARKER).unwrap_err(),
            LibraryFieldError::ReservedContent {
                line: 1,
                marker: MarkerLine::Begin
            }
        );
        assert_eq!(
            validate_variant_text("x\n<!-- EUNOMIASYNC:END -->").unwrap_err(),
            LibraryFieldError::ReservedContent {
                line: 2,
                marker: MarkerLine::End
            }
        );
    }

    #[test]
    fn variant_text_rejects_padded_marker_lines() {
        // The scan reuses classify_line (D-062): ASCII space and tab
        // padding counts, because a padded marker line inside the
        // managed block would parse as a second marker pair in a
        // scheme-B file — the corruption DB-016 exists to prevent.
        for padded in [
            "  <!-- EUNOMIASYNC:BEGIN -->",
            "<!-- EUNOMIASYNC:END -->\t",
            "\t <!-- EUNOMIASYNC:BEGIN --> \t",
        ] {
            assert!(
                validate_variant_text(padded).is_err(),
                "{padded:?} must not validate"
            );
        }
        // A whole text that is nothing but a padded marker: the
        // trailing whitespace goes in the build, the marker line
        // stays.
        assert_eq!(
            validate_variant_text("  <!-- EUNOMIASYNC:BEGIN -->  ").unwrap_err(),
            LibraryFieldError::ReservedContent {
                line: 1,
                marker: MarkerLine::Begin
            }
        );
    }

    #[test]
    fn variant_text_finds_markers_through_normalization_edges() {
        // The scan runs over the text as the build writes it (one
        // normalize::variant_text pass — ALG-002/D-044), so the
        // places a marker can hide from a naive line split are
        // covered, and the verdict does not depend on whether the
        // caller already folded CRLF.

        // A CRLF line break does not mask the marker line…
        assert_eq!(
            validate_variant_text("x\r\n<!-- EUNOMIASYNC:BEGIN -->").unwrap_err(),
            LibraryFieldError::ReservedContent {
                line: 2,
                marker: MarkerLine::Begin
            }
        );
        // …and neither does a lone CR: ALG-002 turns it into LF, so
        // the marker becomes its own line in the built file.
        assert_eq!(
            validate_variant_text("x\r<!-- EUNOMIASYNC:END -->").unwrap_err(),
            LibraryFieldError::ReservedContent {
                line: 2,
                marker: MarkerLine::End
            }
        );
        // A form-feed-only leading line is a BLANK line to ALG-002
        // (D-044's ASCII class), so it cannot hide the marker — it
        // is dropped with the rest of the leading edge.
        assert_eq!(
            validate_variant_text("\u{c}\n<!-- EUNOMIASYNC:BEGIN -->").unwrap_err(),
            LibraryFieldError::ReservedContent {
                line: 1,
                marker: MarkerLine::Begin
            }
        );
        // A leading BOM hides a marker from a naive line-1 check…
        assert_eq!(
            validate_variant_text("\u{feff}<!-- EUNOMIASYNC:BEGIN -->").unwrap_err(),
            LibraryFieldError::ReservedContent {
                line: 1,
                marker: MarkerLine::Begin
            }
        );
        // …also behind leading empty lines, where the dropped edge
        // shifts the built form's numbering: the reported line is
        // the built one (module docs).
        assert_eq!(
            validate_variant_text("\n\n\u{feff}\n<!-- EUNOMIASYNC:END -->").unwrap_err(),
            LibraryFieldError::ReservedContent {
                line: 1,
                marker: MarkerLine::End
            }
        );
    }

    #[test]
    fn variant_text_passes_marker_prose_and_near_misses() {
        // A marker inside a longer line is prose, not a marker line
        // ("each on its own line", D-062)…
        assert!(validate_variant_text("text <!-- EUNOMIASYNC:BEGIN --> more").is_ok());
        assert!(validate_variant_text("<!-- EUNOMIASYNC:BEGIN --> tail").is_ok());
        // …and so are every other spelling difference: case…
        assert!(validate_variant_text("<!-- eunomiasync:begin -->").is_ok());
        // …missing inner spaces…
        assert!(validate_variant_text("<!--EUNOMIASYNC:BEGIN-->").is_ok());
        // …Unicode padding (NBSP is content to classify_line)…
        assert!(validate_variant_text("\u{00A0}<!-- EUNOMIASYNC:BEGIN -->").is_ok());
        // …and the other two ASCII near-whitespace characters, \f and
        // \v, which D-062's padding class (space and tab only) also
        // leaves as content — same verdict as markers.rs itself.
        assert!(validate_variant_text("\u{c}<!-- EUNOMIASYNC:BEGIN -->").is_ok());
        assert!(validate_variant_text("\u{b}<!-- EUNOMIASYNC:END -->").is_ok());
        // …and the creation-marker line (D-061): DB-016 reserves
        // exactly the two D-19 literals, and a pasted creation line
        // never matches classify_line — inert prose.
        assert!(
            validate_variant_text(
                "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->"
            )
            .is_ok()
        );
    }

    #[test]
    fn variant_text_errors_say_what_is_wrong() {
        assert_eq!(
            validate_variant_text(&"x".repeat(262_145))
                .unwrap_err()
                .to_string(),
            "variant text is 262145 characters; the limit is 262144"
        );
        assert_eq!(
            validate_variant_text("a\0b").unwrap_err().to_string(),
            "variant text contains NUL at index 1"
        );
        assert_eq!(
            validate_variant_text("x\n<!-- EUNOMIASYNC:BEGIN -->")
                .unwrap_err()
                .to_string(),
            "variant text contains the reserved <!-- EUNOMIASYNC:BEGIN --> marker line on line 2"
        );
    }

    // ---- the save pipeline over a realistic paste (D-044 integration) ------

    #[test]
    fn the_editor_save_validates_all_four_fields_together() {
        // One LIB-002 save: every field at a legal value, and the
        // returned values are what the store persists — the name
        // trimmed, the text exactly as handed (the store's own fold
        // comes first, the build's normalization later).
        let name = validate_instruction_title("  Be concise  ").unwrap();
        let variant = validate_variant_name("Main").unwrap();
        let text = validate_variant_text("Be brief.\r\nUse tabs:\tinner.\r\n").unwrap();
        let heading = validate_heading_title("Style").unwrap();
        assert_eq!(name, "Be concise");
        assert_eq!(variant, "Main");
        assert_eq!(text, "Be brief.\r\nUse tabs:\tinner.\r\n");
        assert_eq!(heading, "Style");
    }

    #[test]
    fn the_reserved_scan_reads_the_text_the_build_writes() {
        // The split, end to end: a Windows paste with a padded marker
        // is rejected by the VALIDATOR (the built form's lines)…
        let pasted = "note\r\n\r\n  <!-- EUNOMIASYNC:END -->\r\n";
        assert_eq!(
            validate_variant_text(pasted).unwrap_err(),
            LibraryFieldError::ReservedContent {
                line: 3,
                marker: MarkerLine::End
            }
        );
        // …while the same paste WITHOUT the marker validates verbatim
        // and still normalizes at the build — the two passes stay
        // separate owners (D-044), joined only inside the scan.
        let clean = "\u{feff}\r\n\r\nBe brief.\r\n  \r\n\r\n";
        assert_eq!(validate_variant_text(clean).unwrap(), clean);
        assert_eq!(normalize::variant_text(clean), "Be brief.");
    }
    #[test]
    fn every_failure_is_typed_never_a_string() {
        // The E_VALIDATION mapping (§8, §7.10) matches on variants;
        // every failure mode of every field has one — and each is ITS
        // field's own variant, so a cross-field mix-up (a variant-name
        // failure returning an instruction-title error, say) fails
        // here, not only in the per-field tests.
        type Failure = (fn() -> Result<(), LibraryFieldError>, LibraryFieldError);
        let failures: [Failure; 15] = [
            (
                || validate_instruction_title("").map(|_| ()),
                LibraryFieldError::InstructionTitleEmpty,
            ),
            (
                || validate_instruction_title(&"a".repeat(201)).map(|_| ()),
                LibraryFieldError::InstructionTitleTooLong { length: 201 },
            ),
            (
                || validate_instruction_title("a\nb").map(|_| ()),
                LibraryFieldError::InstructionTitleLineBreak {
                    index: 1,
                    character: '\n',
                },
            ),
            (
                || validate_instruction_title("a\0b").map(|_| ()),
                LibraryFieldError::InstructionTitleNul { index: 1 },
            ),
            (
                || validate_variant_name("").map(|_| ()),
                LibraryFieldError::VariantNameEmpty,
            ),
            (
                || validate_variant_name(&"a".repeat(101)).map(|_| ()),
                LibraryFieldError::VariantNameTooLong { length: 101 },
            ),
            (
                || validate_variant_name("a\nb").map(|_| ()),
                LibraryFieldError::VariantNameLineBreak {
                    index: 1,
                    character: '\n',
                },
            ),
            (
                || validate_variant_name("a\0b").map(|_| ()),
                LibraryFieldError::VariantNameNul { index: 1 },
            ),
            (
                || validate_heading_title("").map(|_| ()),
                LibraryFieldError::HeadingTitleEmpty,
            ),
            (
                || validate_heading_title(&"a".repeat(201)).map(|_| ()),
                LibraryFieldError::HeadingTitleTooLong { length: 201 },
            ),
            (
                || validate_heading_title("a\rb").map(|_| ()),
                LibraryFieldError::HeadingTitleLineBreak {
                    index: 1,
                    character: '\r',
                },
            ),
            (
                || validate_heading_title("a\0b").map(|_| ()),
                LibraryFieldError::HeadingTitleNul { index: 1 },
            ),
            (
                || validate_variant_text(&"x".repeat(262_145)).map(|_| ()),
                LibraryFieldError::VariantTextTooLong { length: 262_145 },
            ),
            (
                || validate_variant_text("a\0b").map(|_| ()),
                LibraryFieldError::VariantTextNul { index: 1 },
            ),
            (
                || validate_variant_text("x\n<!-- EUNOMIASYNC:BEGIN -->").map(|_| ()),
                LibraryFieldError::ReservedContent {
                    line: 2,
                    marker: MarkerLine::Begin,
                },
            ),
        ];
        for (produce, expected) in failures {
            assert_eq!(produce().unwrap_err(), expected);
        }
    }
}
