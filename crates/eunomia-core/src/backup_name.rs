//! Backup name builders: the pure string half of spec §7.6's backup
//! row (BKP-001, 002, 008, 011) and §5.5's data-directory layout.
//!
//! Every backup the app writes lives at
//!
//! ```text
//! backups/<project_id>__<slug>/<file name>.<UTC YYYYMMDDTHHMMSSmmmZ>.<hash8>.bak
//! ```
//!
//! under the data directory (§5.5). This module builds those strings
//! — and nothing else: no file is read, written or even stat'ed here
//! (core does no I/O, DECISIONS.md D-006/D-017); `eunomia-fs`'s
//! backup module joins the returned relative path under the data
//! directory and does the BKP-005/BKP-010 copying and checking.
//!
//! # SEC-006: what a backup path is allowed to be built from
//!
//! SEC-006 (P0): "Backup paths are built from the ID and the cleaned
//! slug, not from arbitrary user strings. The target file name is
//! only a value from the enum." The sentence names the two blessed
//! sources — the project ID and the cleaned slug — so the slug IS a
//! permitted derivative: it is "the cleaned project name" by
//! BKP-001's own words, cleaned here to a charset that carries no
//! path syntax at all. What the rule bars is the RAW name (or any
//! other user text — a path, a description) reaching a path
//! component verbatim, and the file name being typed in: it arrives
//! only as a [`Scheme`] + [`BaseName`] pair, which this module maps
//! onto the closed list of target file names (§5.7 watches exactly
//! these files). No function here takes a free-form file-name
//! string.
//!
//! Path safety is proven, not asserted: the slug's charset is
//! exactly `[a-z0-9-]` (no `.`, so `..` is unrepresentable; no `/`,
//! `\`, NUL, space or `%`), the project ID is validated against
//! `[0-9A-Za-z_-]` (dots again impossible) before it can appear, and
//! the unit tests drive traversal payloads (`../`, `..\`, `..`,
//! percent spellings, NUL, Unicode) through the builders and check
//! the results as `std::path` components — single `Normal`
//! components every time, never `ParentDir`.
//!
//! # The slug: BKP-001's "cleaned project name, up to 40 characters"
//!
//! The spec fixes the bound and the purpose ("the main key is
//! `project_id`" — the slug is the human-readable half, the
//! acceptance criterion "the backup name shows the project, file,
//! and time"), but not the cleaning rules. The ruling implemented
//! here (DECISIONS.md D-080), in the classic slug shape:
//!
//! - KEPT: ASCII letters (lower-cased) and digits — `EunomiaSync`
//!   → `eunomiasync`;
//! - every MAXIMAL RUN of any other code points — spaces,
//!   punctuation (including `-`, `_`, `.`), controls, and ALL
//!   non-ASCII — collapses to ONE `-`: `My Project` →
//!   `my-project`, `a--b` → `a-b`;
//! - leading and trailing hyphens are trimmed;
//! - the first 40 CODE POINTS of the cleaned string survive
//!   ("up to 40 characters" describes the RESULT, D-079's reading
//!   of the same wording in DB-042 — clean first, cut second), and
//!   a cut that lands right after a separator run drops the
//!   trailing hyphen it would leave;
//! - an empty result — empty name, specials only, non-ASCII only —
//!   falls back to the literal `project`: BKP-001's "the main key
//!   is `project_id`" makes the slug display sugar, and the
//!   fallback is the spec's own common noun, carrying no user
//!   bytes.
//!
//! Non-ASCII is DROPPED, not transliterated: the spec owns no
//! transliteration table, and inventing one (`Проект` →
//! `proekt`?) would be a rule it never states. A Cyrillic or emoji
//! name cleans to the fallback slug while the project's backups
//! stay one directory away, keyed by the ID. The walk is by code
//! points (`str::chars()`), never bytes; after the cleaning the
//! slug is pure ASCII, where code points and bytes coincide — the
//! 40-unit ruling matters on the way in, where a multi-byte
//! character is one separator unit, not a byte pair.
//!
//! # The timestamp: `YYYYMMDDTHHMMSSmmmZ`
//!
//! BKP-001's template, uncompressed and unambiguous: 4-digit year,
//! 2-digit month, day, hour, minute, second, 3-digit millisecond,
//! the `T` date-time divider, the `Z` UTC marker — 18 digits-plus-T
//! characters before the `Z`, 19 in total, zero-padded, always UTC.
//! It is not the `YYYY-MM-DD HH:MM:SS` form of ALG-032's marker
//! (D-061) and not the store's ISO-8601 spelling: backups sort by
//! name in a plain directory listing, which the separator-free
//! form gives lexically.
//!
//! The time is a PARAMETER, never a clock: [`BackupTime`] is the
//! seven decomposed civil fields (year…millisecond) of D-061's
//! posture — core has no time library (jiff is the store layer's
//! dependency, D-035, and core's allow-list is serde/thiserror/
//! sha2, D-017), so the caller resolves the run's instant into UTC
//! civil fields once and passes them down; the field domains are
//! asserted with the crate's broken-invariant panic posture
//! (D-061: the fields arrive from a time library's civil
//! accessors, which never produce out-of-range values; month
//! lengths stay the calendar's truth, not the stamp's).
//!
//! # `hash8`: the first 8 of the SHA-256 hex
//!
//! The backup check of BKP-010 hashes the copied file with
//! SHA-256; the whole digest, rendered as 64 LOWERCASE hex
//! characters (the spelling of DB-020's build hash and of every
//! SHA-256 the backup rules compare), is what distinguishes two
//! backups made in the same millisecond. `hash8` is the FIRST 8
//! CHARACTERS of exactly that string — a readable short id in the
//! file name, unique enough for the name (the database row keeps
//! the full digest). The builders therefore take the full
//! 64-character hex digest and truncate it themselves: a caller
//! cannot hand-pick eight characters, and the one validation gate
//! (length 64, ASCII lowercase hex) sits next to the truncation it
//! guards.
//!
//! # Boundary
//!
//! No `AppError` mapping lives here (D-062's precedent: core's
//! typed errors stop at the crate boundary; the command layer maps
//! them to §8 codes in a later task). The returned path is the
//! §5.5 RELATIVE layout with `/` separators, a spec string — not an
//! OS path value; `eunomia-fs` joins it under the data directory
//! and owns the SEC-007 rights.
//!
//! # Examples
//!
//! ```
//! use eunomia_core::backup_name::{
//!     BackupTime, backup_relative_path, project_slug,
//! };
//! use eunomia_core::project::{BaseName, ProjectId, Scheme};
//!
//! // The slug: cleaned, kebab-cased, at most 40 characters.
//! assert_eq!(project_slug("My Project"), "my-project");
//! assert_eq!(project_slug("../etc/passwd"), "etc-passwd");
//! assert_eq!(project_slug("Проект"), "project"); // dropped, not transliterated
//!
//! // The §5.5 layout, whole:
//! let path = backup_relative_path(
//!     &ProjectId::new("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d".to_owned()),
//!     "My Project",
//!     Scheme::A,
//!     BaseName::AgentsMd,
//!     BackupTime {
//!         year: 2026, month: 11, day: 19,
//!         hour: 10, minute: 30, second: 0, millisecond: 123,
//!     },
//!     "d2a2446b2437df4c64a4db2b3a49a45e519fddf98e8718ac0fb9f682f9b83d27",
//! )
//! .unwrap();
//! assert_eq!(
//!     path,
//!     "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project/\
//!      AGENTS.md.20261119T103000123Z.d2a2446b.bak",
//! );
//! ```

use thiserror::Error;

use crate::project::{BaseName, ProjectId, Scheme};

/// The longest slug BKP-001 allows: "the cleaned project name, up
/// to 40 characters".
const SLUG_MAX: usize = 40;

/// The slug every name cleans to nothing returns (BKP-001: "the
/// main key is `project_id`" — the slug is display sugar; the
/// fallback is the spec's own common noun, no user bytes).
const SLUG_FALLBACK: &str = "project";

/// The UTC time a backup name stamps (spec §7.6 BKP-001:
/// `<UTC YYYYMMDDTHHMMSSmmmZ>`).
///
/// The seven civil fields of the stamp, decomposed — the shape a
/// time library's civil accessors hand out. `eunomia-core` has no
/// time library (D-017's allow-list; `jiff` is the store layer's
/// dependency, D-035), so the caller resolves the run's instant
/// into UTC civil fields once and passes them in; this crate never
/// touches a clock. The fields are UTC by construction: the `Z` the
/// stamp prints is a claim about them, and the caller makes it.
///
/// The field domains are checked when the stamp is formatted; month
/// lengths are not (D-061's posture: a day of 31 is accepted in any
/// month — the fields arrive from a time library's civil
/// accessors, which never produce impossible dates).
///
/// # Examples
///
/// ```
/// use eunomia_core::backup_name::BackupTime;
///
/// let time = BackupTime {
///     year: 2026, month: 11, day: 19,
///     hour: 10, minute: 30, second: 0, millisecond: 123,
/// };
/// assert_eq!((time.year, time.millisecond), (2026, 123));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackupTime {
    /// The year, `1..=9999`, printed as four digits (`YYYY`).
    pub year: i16,
    /// The month, `1..=12`, printed as two digits (`MM`).
    pub month: u8,
    /// The day of the month, `1..=31`, printed as two digits (`DD`).
    pub day: u8,
    /// The hour, `0..=23`, printed as two digits (`HH`).
    pub hour: u8,
    /// The minute, `0..=59`, printed as two digits (`MM`).
    pub minute: u8,
    /// The second, `0..=59`, printed as two digits (`SS`) — a time
    /// library's civil second never reaches 60.
    pub second: u8,
    /// The millisecond, `0..=999`, printed as three digits (`mmm`).
    pub millisecond: u16,
}

/// A backup name input this module refuses (spec §7.6/§13; the
/// rulings of DECISIONS.md D-080).
///
/// One variant per gate — the project ID that is not path-safe and
/// the SHA-256 that is not the 64-character lowercase hex form.
/// These are data conditions, not broken invariants: the ID crosses
/// from an opaque TEXT column (D-042: core never validates the
/// UUIDv7 form) and the digest from a hashing step, so this module
/// checks them at the only boundary they have. The command layer
/// maps them to §8 codes in a later task (D-062's precedent: no
/// `AppError` in core).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BackupNameError {
    /// The project ID is empty, or carries a character outside
    /// `[0-9A-Za-z_-]` — the charset that cannot spell a separator,
    /// a `..`, a NUL or a space. BKP-001's ids are UUIDv7 strings
    /// (the store's `new_id`), which always pass.
    #[error(
        "project id {value:?} is not path-safe (expected only digits, ASCII letters, '-' and '_')"
    )]
    UnsafeProjectId {
        /// The offending identifier.
        value: String,
    },
    /// The SHA-256 digest is not 64 ASCII lowercase hexadecimal
    /// characters — the BKP-010 check value's canonical spelling
    /// (the same rendering DB-020 fixes for the build hash).
    #[error("sha256 value {value:?} is not 64 lowercase hex characters")]
    InvalidSha256 {
        /// The offending digest string.
        value: String,
    },
    /// The stored path is not the §5.5 layout a backup copy's name
    /// must spell (`backups/<dir>/<file>.<stamp>.<hash8>.bak`) — the
    /// export-name derivation (BKP-012) refused it instead of guessing
    /// a suggestion out of a corrupt cell (the D-039 posture).
    #[error(
        "backup path {value:?} is not a backup copy name (backups/<dir>/<file>.<stamp>.<hash8>.bak)"
    )]
    NotABackupLayout {
        /// The offending path string.
        value: String,
    },
}

/// Cleans a project name into the backup slug (spec §7.6 BKP-001:
/// "the cleaned project name, up to 40 characters"; SEC-006's
/// cleaned half).
///
/// The rules, fixed by DECISIONS.md D-080 (the spec names the
/// bound, not the cleaning): ASCII letters lower-case and digits
/// are kept; every maximal run of any other code points — spaces,
/// punctuation, controls, all non-ASCII — collapses to one `-`;
/// edge hyphens are trimmed; the first 40 code points of the
/// cleaned string survive, a trailing hyphen the cut leaves behind
/// is dropped; a result that is empty falls back to the literal
/// `project`. The output charset is therefore exactly `[a-z0-9-]`:
/// no `.`, `/`, `\`, NUL, space or `%` can appear, and `.`/`..`
/// are unrepresentable.
///
/// Total: every `&str` in, a legal slug out — the fallback keeps
/// empty, specials-only and non-ASCII names addressable (BKP-001:
/// "the main key is `project_id`").
///
/// # Examples
///
/// ```
/// use eunomia_core::backup_name::project_slug;
///
/// assert_eq!(project_slug("My Project"), "my-project");
/// assert_eq!(project_slug("EunomiaSync"), "eunomiasync");
/// assert_eq!(project_slug("a--b__c"), "a-b-c");
/// assert_eq!(project_slug("../.."), "project"); // no dot survives
/// assert_eq!(project_slug(&"x".repeat(100)).len(), 40);
/// ```
#[must_use]
pub fn project_slug(name: &str) -> String {
    // One pass, code point by code point: kept characters append,
    // the FIRST character of a separator run appends the run's
    // single hyphen, the rest of the run appends nothing.
    let mut cleaned = String::with_capacity(name.len());
    let mut in_separator_run = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            cleaned.push(character.to_ascii_lowercase());
            in_separator_run = false;
        } else if !in_separator_run {
            cleaned.push('-');
            in_separator_run = true;
        }
    }

    // Clean first, cut second ("up to 40 characters" describes the
    // result, D-079's reading of the same wording); a cut landing
    // right after a separator run drops the hyphen it would leave.
    let slug: String = cleaned
        .trim_matches('-')
        .chars()
        .take(SLUG_MAX)
        .collect::<String>()
        .trim_end_matches('-')
        .to_owned();

    if slug.is_empty() {
        SLUG_FALLBACK.to_owned()
    } else {
        slug
    }
}

/// The target file name a project's backup carries (spec §5.5's
/// `<file>`; SEC-006: "The target file name is only a value from
/// the enum").
///
/// The closed list of §5.7's watched target files: schemes A and B
/// write the main file (`AGENTS.md` / `CLAUDE.md`, the [`BaseName`]
/// literal), scheme C writes `<STEM>.project.md` (D-20: the main
/// file is not edited). There is no free-form spelling — the name
/// arrives as two enum values and leaves as one of four literals,
/// which is what SEC-006's "only a value from the enum" asks the
/// backup path to contain.
///
/// # Examples
///
/// ```
/// use eunomia_core::backup_name::target_file_name;
/// use eunomia_core::project::{BaseName, Scheme};
///
/// assert_eq!(target_file_name(Scheme::A, BaseName::AgentsMd), "AGENTS.md");
/// assert_eq!(target_file_name(Scheme::B, BaseName::ClaudeMd), "CLAUDE.md");
/// assert_eq!(target_file_name(Scheme::C, BaseName::AgentsMd), "AGENTS.project.md");
/// assert_eq!(target_file_name(Scheme::C, BaseName::ClaudeMd), "CLAUDE.project.md");
/// ```
#[must_use]
pub fn target_file_name(scheme: Scheme, base_name: BaseName) -> &'static str {
    match scheme {
        Scheme::A | Scheme::B => base_name.as_str(),
        Scheme::C => match base_name {
            BaseName::AgentsMd => "AGENTS.project.md",
            BaseName::ClaudeMd => "CLAUDE.project.md",
        },
    }
}

/// The backup directory name of a project (spec §5.5/§7.6:
/// `<project_id>__<slug>`).
///
/// `project_name` is cleaned by [`project_slug`] here — the caller
/// cannot pass a stale or raw slug. The ID is validated against
/// `[0-9A-Za-z_-]` first: BKP-001's ids are the store's UUIDv7
/// strings, but the column is opaque TEXT to core (D-042), and
/// this module owns the path safety of the whole layout, so an ID
/// that could spell a separator, a `..`, a NUL or a space is
/// refused with [`BackupNameError::UnsafeProjectId`] rather than
/// trusted.
///
/// The joined name always contains the `__` divider and a non-empty
/// slug, so it can never equal a Windows reserved device name
/// (`CON`, `NUL`, … — none of which carries an underscore).
///
/// # Examples
///
/// ```
/// use eunomia_core::backup_name::backup_dir_name;
/// use eunomia_core::project::ProjectId;
///
/// let id = ProjectId::new("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d".to_owned());
/// assert_eq!(
///     backup_dir_name(&id, "My Project").unwrap(),
///     "0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project",
/// );
/// assert_eq!(
///     backup_dir_name(&id, "!!!").unwrap(),
///     "0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__project", // the fallback slug
/// );
/// assert!(backup_dir_name(&ProjectId::new("../etc".to_owned()), "x").is_err());
/// ```
pub fn backup_dir_name(
    project_id: &ProjectId,
    project_name: &str,
) -> Result<String, BackupNameError> {
    validate_project_id(project_id.as_str())?;
    Ok(format!(
        "{}__{}",
        project_id.as_str(),
        project_slug(project_name)
    ))
}

/// The backup file name of one copied file (spec §5.5/§7.6:
/// `<file name>.<UTC YYYYMMDDTHHMMSSmmmZ>.<hash8>.bak`).
///
/// The file arrives as its enum pair ([`target_file_name`]), the
/// time as decomposed UTC civil fields ([`BackupTime`] — no clock),
/// and the digest as the FULL 64-character lowercase-hex SHA-256 of
/// the file content (BKP-010's check value): `hash8` in the name is
/// this module's truncation of it to the first 8 characters, so a
/// caller cannot hand-pick the short id, and the one format gate
/// sits next to the truncation it guards.
///
/// # Errors
///
/// [`BackupNameError::InvalidSha256`] unless `sha256_hex` is
/// exactly 64 ASCII lowercase hexadecimal characters. (The time
/// fields' domains are asserts, not errors — D-061's
/// broken-invariant posture.)
///
/// # Examples
///
/// ```
/// use eunomia_core::backup_name::{BackupTime, backup_file_name};
/// use eunomia_core::project::{BaseName, Scheme};
///
/// let name = backup_file_name(
///     Scheme::A,
///     BaseName::AgentsMd,
///     BackupTime {
///         year: 2026, month: 11, day: 19,
///         hour: 10, minute: 30, second: 0, millisecond: 123,
///     },
///     "d2a2446b2437df4c64a4db2b3a49a45e519fddf98e8718ac0fb9f682f9b83d27",
/// )
/// .unwrap();
/// assert_eq!(name, "AGENTS.md.20261119T103000123Z.d2a2446b.bak");
/// ```
pub fn backup_file_name(
    scheme: Scheme,
    base_name: BaseName,
    time: BackupTime,
    sha256_hex: &str,
) -> Result<String, BackupNameError> {
    let hash8 = hash8_of(sha256_hex)?;
    Ok(format!(
        "{}.{}.{}.bak",
        target_file_name(scheme, base_name),
        utc_stamp(time),
        hash8,
    ))
}

/// The whole §5.5 backup path of one copied file, relative to the
/// data directory:
/// `backups/<project_id>__<slug>/<file>.<UTC>.<hash8>.bak`.
///
/// Pure composition of [`backup_dir_name`] and
/// [`backup_file_name`]: the two gates (ID charset, digest shape)
/// and the two cleanings (slug, file-name enum) apply unchanged.
/// The separators are the `/` of the §5.5 layout string — the
/// value is the spec's relative layout, not an OS path;
/// `eunomia-fs` joins it under the data directory and owns the
/// SEC-007 rights.
///
/// # Errors
///
/// [`BackupNameError::UnsafeProjectId`] or
/// [`BackupNameError::InvalidSha256`], as in the two halves.
///
/// # Examples
///
/// ```
/// use eunomia_core::backup_name::{BackupTime, backup_relative_path};
/// use eunomia_core::project::{BaseName, ProjectId, Scheme};
///
/// let path = backup_relative_path(
///     &ProjectId::new("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d".to_owned()),
///     "My Project",
///     Scheme::C,
///     BaseName::ClaudeMd,
///     BackupTime {
///         year: 2026, month: 5, day: 3,
///         hour: 7, minute: 8, second: 9, millisecond: 42,
///     },
///     "d2a2446b2437df4c64a4db2b3a49a45e519fddf98e8718ac0fb9f682f9b83d27",
/// )
/// .unwrap();
/// assert_eq!(
///     path,
///     "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project/\
///      CLAUDE.project.md.20260503T070809042Z.d2a2446b.bak",
/// );
/// ```
pub fn backup_relative_path(
    project_id: &ProjectId,
    project_name: &str,
    scheme: Scheme,
    base_name: BaseName,
    time: BackupTime,
    sha256_hex: &str,
) -> Result<String, BackupNameError> {
    Ok(format!(
        "{BACKUPS_LAYOUT_ROOT}/{}/{}",
        backup_dir_name(project_id, project_name)?,
        backup_file_name(scheme, base_name, time, sha256_hex)?,
    ))
}

/// The save dialog's suggested export name of one stored copy (spec
/// §7.10 `backup_save`, BKP-012, D-203 (2)):
/// `<stem>.backup-<stamp>.<ext>` — the prototype's layout, derived
/// from the copy's own §5.5 name.
///
/// The input is the record's stored `backup_path` cell (the
/// [`backup_relative_path`] spelling), and the derivation is this
/// module's own grammar inverted — never a second parser elsewhere:
/// the layout must be exactly `backups/<dir>/<file>`, and BOTH the
/// directory and the file component are held to the builders'
/// charsets (`[0-9A-Za-z_-]` and `[0-9A-Za-z._-]` — the directory
/// cannot spell a `.`, so a whole `..` component cannot ride in, and
/// neither component can spell a path separator on ANY platform,
/// `\` included: a `'/'`-split alone cannot see it but Windows'
/// `Path` would). The file name is the builder's composition
/// `<target>.<stamp>.<hash8>.bak` read from the right (the target
/// file name itself may contain dots). The suggestion then carries
/// the target's stem and extension with the copy's creation stamp:
/// `AGENTS.md.20261119T103000123Z.d2a2446b.bak`
/// → `AGENTS.backup-20261119T103000123Z.md`.
///
/// A cell that does not spell the layout is
/// [`BackupNameError::NotABackupLayout`] — a corrupt cell is refused,
/// never guessed around (D-039). The command layer maps the refusal;
/// no `AppError` lives here (D-062's boundary).
///
/// # Examples
///
/// ```
/// use eunomia_core::backup_name::export_suggested_name;
///
/// let name = export_suggested_name(
///     "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project/\
///      AGENTS.md.20261119T103000123Z.d2a2446b.bak",
/// )
/// .unwrap();
/// assert_eq!(name, "AGENTS.backup-20261119T103000123Z.md");
/// assert!(export_suggested_name("backups/../etc/passwd").is_err());
/// ```
pub fn export_suggested_name(backup_path: &str) -> Result<String, BackupNameError> {
    // The refusal is one shape (the variant carries the path); the
    // doc comments above name the arms this function checks.
    let refuse = || BackupNameError::NotABackupLayout {
        value: backup_path.to_owned(),
    };

    // The §5.5 layout: exactly three '/'-separated components, the
    // first the `backups` root. Fewer or more (an absolute path, a
    // nested escape, a bare file name) is not a stored copy's path.
    let components: Vec<&str> = backup_path.split('/').collect();
    if components.len() != 3 || components[0] != BACKUPS_LAYOUT_ROOT {
        return Err(refuse());
    }

    // The directory component: the builders' charset only — a dot is
    // unrepresentable, so a whole `..` component cannot ride in and
    // the layout cannot climb out of the backups tree it names.
    let directory = components[1];
    if directory.is_empty()
        || !directory
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(refuse());
    }

    // The file component: the builders' charset too —
    // `[0-9A-Za-z._-]`, the whole vocabulary the four target literals
    // and the grammar's stamp/hash8/bak segments can spell. No byte
    // that could parse as a path separator on ANY platform (`\`
    // included — a `'/'`-split cannot see it, Windows' `Path` would)
    // can ride in, so the joined path stays inside the backups tree
    // whatever the stored cell says (the platform-independent
    // containment rule; the round-1 P1-1 fix).
    let file_name = components[2];
    if file_name.is_empty()
        || !file_name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-' || byte == b'_'
        })
    {
        return Err(refuse());
    }

    // The file component, read from the right the way it was composed:
    // <target>.<stamp>.<hash8>.bak — the target may itself contain
    // dots, so the last three segments are claimed by the grammar and
    // the rest is the target file name.
    let segments: Vec<&str> = file_name.split('.').collect();
    if segments.len() < 4
        || segments[segments.len() - 1] != "bak"
        || !is_hash8(segments[segments.len() - 2])
        || !is_utc_stamp(segments[segments.len() - 3])
    {
        return Err(refuse());
    }
    let stamp = segments[segments.len() - 3];
    let target = segments[..segments.len() - 3].join(".");
    if target.is_empty() {
        return Err(refuse());
    }

    // The target's own stem and extension (the builders' four literals
    // all carry one); a dotless target is not one of them.
    let Some(dot) = target.rfind('.') else {
        return Err(refuse());
    };
    if dot == 0 || dot + 1 >= target.len() {
        return Err(refuse());
    }
    let (stem, extension) = target.split_at(dot);
    let extension = &extension[1..];

    Ok(format!("{stem}.backup-{stamp}.{extension}"))
}

/// The §5.5 layout's root component — the first piece of every path
/// the builders return, and the one spelling both [`backup_relative_
/// path`]'s composition and the decomposition ([`export_suggested_
/// name`]'s layout gate, BKP-012) read (the D-149 one-spelling law,
/// the round-1 P3-3 fix).
const BACKUPS_LAYOUT_ROOT: &str = "backups";

/// The `hash8` segment of a copy's file name: exactly 8 ASCII
/// lowercase hexadecimal characters — the truncation shape
/// [`hash8_of`] produces.
fn is_hash8(segment: &str) -> bool {
    segment.len() == 8
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// The `<UTC YYYYMMDDTHHMMSSmmmZ>` segment of a copy's file name: 19
/// characters — 8 digits, the `T` divider, 9 digits, the `Z` marker —
/// the exact shape [`utc_stamp`] formats.
fn is_utc_stamp(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    segment.len() == 19
        && bytes[8] == b'T'
        && bytes[18] == b'Z'
        && bytes[..8].iter().all(u8::is_ascii_digit)
        && bytes[9..18].iter().all(u8::is_ascii_digit)
}

/// Gates the project ID: non-empty, every byte in `[0-9A-Za-z_-]`.
///
/// Bytes, not chars, on purpose: any non-ASCII code point fails (a
/// UTF-8 continuation byte is none of the four), and the four
/// allowed ASCII classes cannot spell `/`, `\`, `.`, NUL or a
/// space — so `..` and every separator stay unrepresentable in a
/// joined path. BKP-001's ids (the store's UUIDv7 TEXT) always
/// pass; anything else is data this module refuses, not an
/// invariant it trusts.
fn validate_project_id(id: &str) -> Result<(), BackupNameError> {
    let safe = !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if safe {
        Ok(())
    } else {
        Err(BackupNameError::UnsafeProjectId {
            value: id.to_owned(),
        })
    }
}

/// The `<UTC YYYYMMDDTHHMMSSmmmZ>` stamp of BKP-001's template:
/// zero-padded civil fields, the `T` divider, the `Z` marker — 19
/// characters total, no separators between the fields (the
/// lexical-sort form; see the module docs).
///
/// # Panics
///
/// Panics if a [`BackupTime`] field left its documented domain —
/// the crate's broken-invariant posture (D-061): the fields arrive
/// from a time library's civil accessors, which never produce
/// out-of-range values.
fn utc_stamp(time: BackupTime) -> String {
    let BackupTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        millisecond,
    } = time;
    assert!(
        (1..=9999).contains(&year),
        "the year must be 1..=9999, printed as four digits"
    );
    assert!((1..=12).contains(&month), "the month must be 1..=12");
    assert!((1..=31).contains(&day), "the day must be 1..=31");
    assert!((0..=23).contains(&hour), "the hour must be 0..=23");
    assert!((0..=59).contains(&minute), "the minute must be 0..=59");
    assert!((0..=59).contains(&second), "the second must be 0..=59");
    assert!(
        (0..=999).contains(&millisecond),
        "the millisecond must be 0..=999"
    );

    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}{millisecond:03}Z")
}

/// The `hash8` of BKP-001's template: the first 8 characters of the
/// 64-character lowercase-hex SHA-256 (see the module docs), or the
/// typed refusal if the digest is not that shape.
fn hash8_of(sha256_hex: &str) -> Result<&str, BackupNameError> {
    let is_canonical = sha256_hex.len() == 64
        && sha256_hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if is_canonical {
        Ok(&sha256_hex[..8])
    } else {
        Err(BackupNameError::InvalidSha256 {
            value: sha256_hex.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Component, Path};

    use super::*;

    // A canonical UUIDv7-shaped id (BKP-001's ids), and the hash.rs
    // oracle digest — an independently computed SHA-256, reused as
    // an unrelated-to-this-module literal.
    const ID: &str = "0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d";
    const SHA256: &str = "d2a2446b2437df4c64a4db2b3a49a45e519fddf98e8718ac0fb9f682f9b83d27";

    fn time_2026_11_19() -> BackupTime {
        BackupTime {
            year: 2026,
            month: 11,
            day: 19,
            hour: 10,
            minute: 30,
            second: 0,
            millisecond: 123,
        }
    }

    fn project() -> ProjectId {
        ProjectId::new(ID.to_owned())
    }

    /// Every component of the path must be one `Normal` piece —
    /// never `ParentDir`, `RootDir` or a `.` — which is what
    /// SEC-006's "path-safe" means mechanically, on every OS std
    /// knows (both separators parse; the names carry none).
    fn assert_all_normal(path: &str) {
        let components: Vec<Component<'_>> = Path::new(path).components().collect();
        assert!(
            !components.is_empty(),
            "{path:?} parsed to no path components"
        );
        for component in &components {
            assert!(
                matches!(component, Component::Normal(_)),
                "{path:?} is not a flat relative path: {component:?} in {components:?}"
            );
        }
    }

    /// The traversal/injection payloads one raw name could carry;
    /// shared by the slug-level and the whole-path-level proofs.
    const HOSTILE: [&str; 8] = [
        "../",
        "..",
        "..\\..",
        "a../../b",
        "../../../etc/passwd",
        "..%2f..%2f..%2fetc%2fpasswd",
        "\u{0}/../",
        "a/b\\c..d%00e%2ef g",
    ];

    // ---- the slug: BKP-001's cleaning -------------------------------

    #[test]
    fn letters_lowercase_digits_keep_and_spaces_become_one_hyphen() {
        // The classic slug shape, D-080's ruling.
        assert_eq!(project_slug("My Project"), "my-project");
        assert_eq!(project_slug("EunomiaSync"), "eunomiasync");
        assert_eq!(project_slug("Project 7"), "project-7");
        assert_eq!(project_slug("EUNOMIA"), "eunomia"); // case folds
    }

    #[test]
    fn special_characters_and_separator_runs_collapse_to_single_hyphens() {
        // One maximal run of specials — no matter what it mixes —
        // contributes exactly one '-'. The NUL, the backslash, the
        // dots and the percent signs all ride the same rule.
        assert_eq!(project_slug("a/b\\c..d%00e%2ef g"), "a-b-c-d-00e-2ef-g");
        assert_eq!(project_slug("NUL\u{0}name"), "nul-name");
        assert_eq!(project_slug("a\u{7}\u{8}b"), "a-b"); // two controls, one run
        assert_eq!(project_slug("a--b__c"), "a-b-c"); // hyphen/underscore fold in
        assert_eq!(project_slug("my  -  project"), "my-project");
    }

    #[test]
    fn traversal_payloads_cannot_survive_the_cleaning() {
        // ../ and ..\ attacks, the .. dot itself: the charset has no
        // dot, no slash, no backslash — the results prove it, and
        // the Path check proves it again mechanically.
        assert_eq!(project_slug("../"), "project");
        assert_eq!(project_slug(".."), "project");
        assert_eq!(project_slug("..\\.."), "project");
        assert_eq!(project_slug("a../../b"), "a-b");
        assert_eq!(project_slug("../../../etc/passwd"), "etc-passwd");
        assert_eq!(project_slug("..."), "project");

        for hostile in HOSTILE {
            let slug = project_slug(hostile);
            assert!(!slug.contains('.'), "{hostile:?} → {slug:?} keeps a dot");
            assert!(!slug.contains('/'), "{hostile:?} → {slug:?} keeps a slash");
            assert!(
                !slug.contains('\\'),
                "{hostile:?} → {slug:?} keeps a backslash"
            );
            assert!(
                !slug.contains('\u{0}'),
                "{hostile:?} → {slug:?} keeps a NUL"
            );
            assert_ne!(slug, ".");
            assert_ne!(slug, "..");
            assert_all_normal(&slug);
        }
    }

    #[test]
    fn percent_spellings_are_literal_not_decoded() {
        // No percent-decoding ever happens: %2e is a percent, a
        // separator, and the literal digits 2e — the exact slug a
        // scanner of the directory can read back.
        assert_eq!(project_slug("%2e%2e%2f"), "2e-2e-2f");
        assert_eq!(project_slug("%00"), "00");
    }

    #[test]
    fn long_names_cut_to_40_cleaned_characters() {
        // "Up to 40 characters" describes the RESULT (D-079's
        // reading of the same wording): clean first, cut second.
        assert_eq!(project_slug(&"x".repeat(100)), "x".repeat(40));
        assert_eq!(project_slug(&"x".repeat(40)), "x".repeat(40)); // exactly at the bound
        assert_eq!(project_slug(&"x".repeat(41)), "x".repeat(40)); // one over
        // The cut keeps the first 40 KEPT characters — separators
        // the name spent before them do not eat the budget:
        assert_eq!(
            project_slug(&format!("{}-{}", "x".repeat(50), "y".repeat(50))),
            "x".repeat(40)
        );
    }

    #[test]
    fn the_cut_runs_on_the_cleaned_string_and_retrims_a_trailing_hyphen() {
        // Clean first, cut second — pinned by the emoji case below:
        // cutting the NAME at 40 code points first would yield the
        // 38 b's alone, while the cleaned-string cut keeps the inner
        // hyphen and a b after it.
        //
        // The walk to get there is by code points: a non-ASCII
        // character is ONE separator unit, never a byte pair.
        let with_accent = format!("{}é{}", "a".repeat(39), "b".repeat(5));
        // 39 a's + one hyphen + 5 b's = 45 characters; the cut at 40
        // takes the a's and the hyphen, and the re-trim drops it.
        assert_eq!(project_slug(&with_accent), "a".repeat(39));

        // The hyphen sits at the 40th slot: the cut keeps it only as
        // a trailing edge, which the re-trim drops.
        let edge = format!("{}-{}", "b".repeat(39), "b".repeat(5));
        assert_eq!(project_slug(&edge), "b".repeat(39));

        // One slot earlier the hyphen is an INNER separator and
        // survives the cut:
        let inner = format!("{}-{}", "b".repeat(38), "b".repeat(5));
        assert_eq!(project_slug(&inner), format!("{}-b", "b".repeat(38)));

        // Astral characters (4 bytes each) are one separator unit,
        // and this is the case that distinguishes clean-then-cut
        // from cut-then-clean: the name's first 40 code points are
        // the b's and the two emoji, but the CLEANED string's first
        // 40 characters reach past the hyphen into the b's.
        let with_emoji = format!("{}🔥🔥{}", "b".repeat(38), "b".repeat(5));
        assert_eq!(project_slug(&with_emoji), format!("{}-b", "b".repeat(38)));
    }

    #[test]
    fn non_ascii_is_dropped_not_transliterated() {
        // D-080: the spec owns no transliteration table — Cyrillic,
        // accents and emoji clean away entirely (the ID is the key).
        assert_eq!(project_slug("Проект"), "project"); // all non-ASCII → fallback
        assert_eq!(project_slug("EunomiaSync проект"), "eunomiasync");
        assert_eq!(project_slug(&format!("{}abc", "д".repeat(45))), "abc");
        assert_eq!(project_slug("café"), "caf"); // é rides the run with nothing
        assert_eq!(project_slug("🔥"), "project");
    }

    #[test]
    fn empty_and_specials_only_names_fall_back_to_project() {
        // The fallback of D-080: BKP-001's "the main key is
        // project_id" makes the slug display sugar.
        for emptying in [
            "", " ", "   ", "...", "/", "\\", "-", "_", "\u{0}", "—", "🔥", ". .. /",
        ] {
            assert_eq!(project_slug(emptying), "project", "{emptying:?}");
        }
    }

    #[test]
    fn edge_separator_runs_are_trimmed() {
        assert_eq!(project_slug("  EunomiaSync  "), "eunomiasync");
        assert_eq!(project_slug("-/-x-/-"), "x");
        assert_eq!(project_slug("__my_project__"), "my-project");
    }

    #[test]
    fn hostile_payloads_build_three_normal_path_components() {
        // SEC-006 is a rule about the PATH, so the hostile battery
        // runs through the whole builder too: every payload lands in
        // the slug slot, and the built path stays the flat §5.5
        // tree — exactly three components, all `Normal`.
        for hostile in HOSTILE {
            let path = backup_relative_path(
                &project(),
                hostile,
                Scheme::A,
                BaseName::AgentsMd,
                time_2026_11_19(),
                SHA256,
            )
            .unwrap_or_else(|error| panic!("{hostile:?} refused: {error}"));
            assert_all_normal(&path);
            assert_eq!(
                Path::new(&path).components().count(),
                3,
                "{hostile:?} → {path:?} left the §5.5 tree"
            );
            assert!(
                path.starts_with("backups/"),
                "{hostile:?} → {path:?} lost the backups root"
            );
            assert!(
                path.contains("__"),
                "{hostile:?} → {path:?} lost the id/slug divider"
            );
        }

        // Two of them, pinned as whole literals: the dot-only
        // payload falls back to the slug, the mixed one keeps the
        // kebab shape — both inside the same safe tree.
        assert_eq!(
            backup_relative_path(
                &project(),
                "..",
                Scheme::A,
                BaseName::AgentsMd,
                time_2026_11_19(),
                SHA256,
            )
            .unwrap(),
            "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__project/\
             AGENTS.md.20261119T103000123Z.d2a2446b.bak"
        );
        assert_eq!(
            backup_relative_path(
                &project(),
                "..%2f..%2f..%2fetc%2fpasswd",
                Scheme::A,
                BaseName::AgentsMd,
                time_2026_11_19(),
                SHA256,
            )
            .unwrap(),
            "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__2f-2f-2fetc-2fpasswd/\
             AGENTS.md.20261119T103000123Z.d2a2446b.bak"
        );
    }

    #[test]
    fn every_unicode_scalar_value_cleans_into_the_charset() {
        // The full-scan lemma (D-079's posture for total rules): for
        // EVERY scalar value, a name built around it cleans into the
        // [a-z0-9-] charset — no dot, slash, backslash or NUL can
        // ride through any single character, so no future Unicode
        // table change reopens SEC-006's question.
        for code in 0..=u32::MAX {
            let Some(character) = char::from_u32(code) else {
                continue; // the surrogate gap is no character
            };
            let slug = project_slug(&format!("a{character}b"));
            assert!(
                slug.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "the scalar {character:?} escapes the charset: {slug:?}"
            );
            assert!(!slug.contains('.'), "{character:?} → {slug:?} keeps a dot");
            assert!(
                !slug.contains('/'),
                "{character:?} → {slug:?} keeps a slash"
            );
            assert!(
                !slug.contains('\\'),
                "{character:?} → {slug:?} keeps a backslash"
            );
            assert!(
                !slug.contains('\u{0}'),
                "{character:?} → {slug:?} keeps a NUL"
            );
            assert!(slug.chars().count() <= SLUG_MAX, "{character:?} → {slug:?}");
        }
    }

    #[test]
    fn every_slug_matches_the_path_safety_property() {
        // The battery: hostile and ordinary names alike clean to a
        // string over [a-z0-9-], at most 40 characters, never
        // hyphen-edged, and a single Normal path component.
        let names = [
            "My Project",
            "EunomiaSync",
            "../etc/passwd",
            "..\\..\\windows\\system32",
            "a/b\\c..d%00e%2ef g",
            "\u{0}\u{1}\u{2}",
            "NUL",
            "CON PRN AUX",
            "Проект",
            "проект №5",
            "🔥🔥🔥",
            &"x".repeat(200),
            &format!("{}é{}", "a".repeat(39), "b".repeat(5)),
            "  ..  ",
            "%2e%2e",
            "name\twith\ttabs",
            "name\nwith\nnewlines",
            "étrange",
            "straße",
            "İstanbul",
        ];
        for name in names {
            let slug = project_slug(name);
            assert!(
                !slug.is_empty(),
                "{name:?} cleaned to an empty slug — the fallback is owed"
            );
            assert!(
                slug.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{name:?} → {slug:?} leaves the [a-z0-9-] charset"
            );
            assert!(
                slug.chars().count() <= SLUG_MAX,
                "{name:?} → {slug:?} is longer than {SLUG_MAX} characters"
            );
            assert!(
                !slug.starts_with('-') && !slug.ends_with('-'),
                "{name:?} → {slug:?}"
            );
            assert!(!slug.contains('.'), "{name:?} → {slug:?} keeps a dot");
            assert_all_normal(&slug);
        }
    }

    // ---- the target file name: SEC-006's enum -----------------------

    #[test]
    fn target_file_name_maps_every_scheme_and_base_pair() {
        // §5.7's watched files, as the four literals: A and B write
        // the main file, C writes <STEM>.project.md (D-20).
        assert_eq!(target_file_name(Scheme::A, BaseName::AgentsMd), "AGENTS.md");
        assert_eq!(target_file_name(Scheme::A, BaseName::ClaudeMd), "CLAUDE.md");
        assert_eq!(target_file_name(Scheme::B, BaseName::AgentsMd), "AGENTS.md");
        assert_eq!(target_file_name(Scheme::B, BaseName::ClaudeMd), "CLAUDE.md");
        assert_eq!(
            target_file_name(Scheme::C, BaseName::AgentsMd),
            "AGENTS.project.md"
        );
        assert_eq!(
            target_file_name(Scheme::C, BaseName::ClaudeMd),
            "CLAUDE.project.md"
        );
    }

    // ---- the timestamp: format pins ---------------------------------

    #[test]
    fn the_stamp_is_the_bkp_001_template_verbatim() {
        // 18 digits-plus-T characters before the Z, 19 in total —
        // the uncompressed, lexically sorting UTC form.
        let name =
            backup_file_name(Scheme::A, BaseName::AgentsMd, time_2026_11_19(), SHA256).unwrap();
        assert_eq!(
            name,
            format!("AGENTS.md.20261119T103000123Z.{}.bak", &SHA256[..8])
        );

        let stamp = stamp_of(&name);
        assert_eq!(stamp, "20261119T103000123Z");
        assert_eq!(stamp.len(), 19);
        assert!(stamp.ends_with('Z'));
        assert_eq!(stamp.as_bytes()[8], b'T'); // the divider, not a separator
        assert!(stamp[..8].bytes().all(|b| b.is_ascii_digit()));
        assert!(stamp[9..18].bytes().all(|b| b.is_ascii_digit()));
    }

    #[test]
    fn every_field_is_zero_padded_to_its_width() {
        // Single-digit fields pad: YYYY=4, MM/DD/HH/MM/SS=2, mmm=3.
        let time = BackupTime {
            year: 26,
            month: 5,
            day: 3,
            hour: 7,
            minute: 8,
            second: 9,
            millisecond: 42,
        };
        let name = backup_file_name(Scheme::C, BaseName::ClaudeMd, time, SHA256).unwrap();
        assert_eq!(stamp_of(&name), "00260503T070809042Z");

        // The domain's floors and ceilings, all fixed-width:
        let floor = BackupTime {
            year: 1,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            millisecond: 0,
        };
        assert_eq!(
            stamp_of(&backup_file_name(Scheme::A, BaseName::AgentsMd, floor, SHA256).unwrap()),
            "00010101T000000000Z"
        );
        let ceiling = BackupTime {
            year: 9999,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
            millisecond: 999,
        };
        assert_eq!(
            stamp_of(&backup_file_name(Scheme::A, BaseName::AgentsMd, ceiling, SHA256).unwrap()),
            "99991231T235959999Z"
        );
    }

    #[test]
    #[should_panic(expected = "the year must be 1..=9999")]
    fn year_zero_panics() {
        let _ = utc_stamp(BackupTime {
            year: 0,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            millisecond: 0,
        });
    }

    #[test]
    #[should_panic(expected = "the year must be 1..=9999")]
    fn year_ten_thousand_panics() {
        let _ = utc_stamp(BackupTime {
            year: 10000,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            millisecond: 0,
        });
    }

    #[test]
    #[should_panic(expected = "the month must be 1..=12")]
    fn month_zero_panics() {
        let _ = utc_stamp(BackupTime {
            month: 0,
            ..time_2026_11_19()
        });
    }

    #[test]
    #[should_panic(expected = "the month must be 1..=12")]
    fn month_thirteen_panics() {
        let _ = utc_stamp(BackupTime {
            month: 13,
            ..time_2026_11_19()
        });
    }

    #[test]
    #[should_panic(expected = "the day must be 1..=31")]
    fn day_zero_panics() {
        let _ = utc_stamp(BackupTime {
            day: 0,
            ..time_2026_11_19()
        });
    }

    #[test]
    #[should_panic(expected = "the day must be 1..=31")]
    fn day_thirty_two_panics() {
        let _ = utc_stamp(BackupTime {
            day: 32,
            ..time_2026_11_19()
        });
    }

    #[test]
    #[should_panic(expected = "the hour must be 0..=23")]
    fn hour_twenty_four_panics() {
        let _ = utc_stamp(BackupTime {
            hour: 24,
            ..time_2026_11_19()
        });
    }

    #[test]
    #[should_panic(expected = "the minute must be 0..=59")]
    fn minute_sixty_panics() {
        let _ = utc_stamp(BackupTime {
            minute: 60,
            ..time_2026_11_19()
        });
    }

    #[test]
    #[should_panic(expected = "the second must be 0..=59")]
    fn second_sixty_panics() {
        // A time library's civil second never reaches a leap
        // second's 60 (D-061's posture); synthetic values stop here.
        let _ = utc_stamp(BackupTime {
            second: 60,
            ..time_2026_11_19()
        });
    }

    #[test]
    #[should_panic(expected = "the millisecond must be 0..=999")]
    fn millisecond_thousand_panics() {
        let _ = utc_stamp(BackupTime {
            millisecond: 1000,
            ..time_2026_11_19()
        });
    }

    /// The stamp segment of a built file name:
    /// `<file>.<stamp>.<hash8>.bak` — the segment three before the
    /// end (the file name itself may contain dots).
    fn stamp_of(name: &str) -> &str {
        let segments: Vec<&str> = name.split('.').collect();
        segments[segments.len() - 3]
    }

    // ---- hash8: the truncation and its gate -------------------------

    #[test]
    fn hash8_is_the_first_eight_characters_of_the_digest() {
        let name =
            backup_file_name(Scheme::A, BaseName::AgentsMd, time_2026_11_19(), SHA256).unwrap();
        assert_eq!(name, "AGENTS.md.20261119T103000123Z.d2a2446b.bak");
        // A different digest changes exactly the hash8 segment:
        let other_digest = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let name2 = backup_file_name(
            Scheme::A,
            BaseName::AgentsMd,
            time_2026_11_19(),
            other_digest,
        )
        .unwrap();
        assert_eq!(name2, "AGENTS.md.20261119T103000123Z.ffffffff.bak");
    }

    #[test]
    fn digests_that_are_not_64_lowercase_hex_are_refused() {
        // Length, case and charset — each failure mode separately.
        for bad in [
            "",
            "d2a2",                                                         // too short
            &SHA256[..63],                                                  // one under
            &format!("{SHA256}0"),                                          // one over
            &SHA256.to_uppercase(),                                         // uppercase hex
            &format!("{}{}z", &SHA256[..62], "g"),                          // non-hex character
            &SHA256.replace('a', "-"),                                      // separator inside
            &format!("{}{}", &SHA256[..32], " ".repeat(32)),                // spaces inside
            &format!("{}{}{}", &SHA256[..32], "\u{0}\u{0}", &SHA256[34..]), // NUL inside
        ] {
            assert_eq!(
                backup_file_name(Scheme::A, BaseName::AgentsMd, time_2026_11_19(), bad),
                Err(BackupNameError::InvalidSha256 {
                    value: bad.to_owned()
                }),
                "digest {bad:?} passed the gate"
            );
        }
    }

    #[test]
    fn the_sha256_error_names_the_shape_it_wants() {
        // The Display line the command layer will surface (D-062's
        // boundary: no AppError mapping lives in core).
        let error = BackupNameError::InvalidSha256 {
            value: "d2a2".to_owned(),
        };
        assert_eq!(
            error.to_string(),
            "sha256 value \"d2a2\" is not 64 lowercase hex characters"
        );
    }

    // ---- the project id gate ----------------------------------------

    #[test]
    fn uuid_shaped_ids_pass_the_gate() {
        assert_eq!(
            backup_dir_name(&project(), "My Project").unwrap(),
            format!("{ID}__my-project")
        );
        // The charset, not the UUID form, is the rule (D-042: core
        // never validates UUIDv7): uppercase hex and underscores pass.
        assert!(
            backup_dir_name(
                &ProjectId::new("0193F1A0-6D6D-7CCE-A3C0-3D1F2A4B5C6D".to_owned()),
                "x"
            )
            .is_ok()
        );
        assert!(backup_dir_name(&ProjectId::new("id_1-x".to_owned()), "x").is_ok());
    }

    #[test]
    fn unsafe_ids_are_refused_with_the_typed_error() {
        for unsafe_id in [
            "",       // empty
            "../etc", // traversal
            "..",
            ".",
            "a b",  // space
            "a\\b", // backslash
            "a/b",
            "a\u{0}b", // NUL
            "Проект",  // non-ASCII
            "a.b",     // dot — the .. root
            "a%2eb",   // percent stays literal, still not in the charset
        ] {
            assert_eq!(
                backup_dir_name(&ProjectId::new(unsafe_id.to_owned()), "x"),
                Err(BackupNameError::UnsafeProjectId {
                    value: unsafe_id.to_owned()
                }),
                "id {unsafe_id:?} passed the gate"
            );
        }
    }

    #[test]
    fn the_unsafe_id_error_names_the_charset_it_wants() {
        let error = BackupNameError::UnsafeProjectId {
            value: "../etc".to_owned(),
        };
        assert_eq!(
            error.to_string(),
            "project id \"../etc\" is not path-safe (expected only digits, ASCII letters, '-' and '_')"
        );
    }

    #[test]
    fn the_dir_name_carries_the_fallback_slug_too() {
        // A name that cleans to nothing still yields a real
        // directory — keyed by the id, sugared by the fallback.
        assert_eq!(
            backup_dir_name(&project(), "!!!").unwrap(),
            format!("{ID}__project")
        );
        // And the __ divider means the name can never equal a
        // Windows reserved device word (CON, NUL, … — no underscore).
        let dir = backup_dir_name(&project(), "CON").unwrap();
        assert!(dir.starts_with(&format!("{ID}__")));
        assert!(dir.contains("__"));
    }

    // ---- the whole layout: §5.5 oracles ------------------------------

    #[test]
    fn the_relative_path_is_the_55_layout_verbatim() {
        let path = backup_relative_path(
            &project(),
            "My Project",
            Scheme::A,
            BaseName::AgentsMd,
            time_2026_11_19(),
            SHA256,
        )
        .unwrap();
        assert_eq!(
            path,
            "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project/\
             AGENTS.md.20261119T103000123Z.d2a2446b.bak"
        );
        assert_all_normal(&path);
        // Exactly three components: the backups root, the project
        // directory, the file — the §5.5 tree, no deeper.
        assert_eq!(Path::new(&path).components().count(), 3);
    }

    #[test]
    fn the_scheme_c_project_backs_up_the_project_md_file() {
        // D-20: scheme C writes <STEM>.project.md — the backup name
        // shows that file, never the untouched main file.
        let path = backup_relative_path(
            &project(),
            "My Project",
            Scheme::C,
            BaseName::AgentsMd,
            time_2026_11_19(),
            SHA256,
        )
        .unwrap();
        assert!(path.ends_with("/AGENTS.project.md.20261119T103000123Z.d2a2446b.bak"));
    }

    #[test]
    fn hostile_names_build_the_same_safe_layout() {
        // End to end: the traversal payload as the project NAME —
        // SEC-006's whole point — lands in the slug slot, cleaned.
        let path = backup_relative_path(
            &project(),
            "../../../etc/passwd",
            Scheme::A,
            BaseName::AgentsMd,
            time_2026_11_19(),
            SHA256,
        )
        .unwrap();
        assert_eq!(
            path,
            "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__etc-passwd/\
             AGENTS.md.20261119T103000123Z.d2a2446b.bak"
        );
        assert_all_normal(&path);
    }

    #[test]
    fn every_built_name_is_length_bounded_and_flat() {
        // The §5.5 pieces have fixed shapes: the file name is
        // <file>.<19>.<8>.<3> plus three dots, the directory is at
        // most 36 (UUID) + 2 + 40 (slug) characters.
        for (scheme, base, file_len) in [
            (Scheme::A, BaseName::AgentsMd, "AGENTS.md".len()),
            (Scheme::A, BaseName::ClaudeMd, "CLAUDE.md".len()),
            (Scheme::B, BaseName::AgentsMd, "AGENTS.md".len()),
            (Scheme::B, BaseName::ClaudeMd, "CLAUDE.md".len()),
            (Scheme::C, BaseName::AgentsMd, "AGENTS.project.md".len()),
            (Scheme::C, BaseName::ClaudeMd, "CLAUDE.project.md".len()),
        ] {
            let name = backup_file_name(scheme, base, time_2026_11_19(), SHA256).unwrap();
            assert_eq!(name.len(), file_len + 1 + 19 + 1 + 8 + 1 + 3, "{name:?}");
            assert!(name.ends_with(".bak"));
            assert_all_normal(&name);
        }

        let dir = backup_dir_name(&project(), &"x".repeat(200)).unwrap();
        assert!(dir.len() <= 36 + 2 + SLUG_MAX, "{dir:?}");
        assert_all_normal(&dir);
    }

    // ---- the export suggestion (BKP-012, D-203 (2)) -----------------

    #[test]
    fn the_suggestion_carries_the_stem_stamp_and_extension() {
        // BKP-012's `<stem>.backup-<stamp>.<ext>`: the stem and ext of
        // the target file, the copy's own creation stamp.
        assert_eq!(
            export_suggested_name(
                "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project/\
                 AGENTS.md.20261119T103000123Z.d2a2446b.bak"
            )
            .unwrap(),
            "AGENTS.backup-20261119T103000123Z.md"
        );
        // The scheme C spelling: the target carries its own inner dot,
        // and the stem keeps it (CLAUDE.project, not CLAUDE).
        assert_eq!(
            export_suggested_name(
                "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project/\
                 CLAUDE.project.md.20260503T070809042Z.ffffffff.bak"
            )
            .unwrap(),
            "CLAUDE.project.backup-20260503T070809042Z.md"
        );
    }

    #[test]
    fn every_builder_output_decomposes_back_into_a_suggestion() {
        // The round trip the "one grammar" rule buys (no forked
        // parsing): every name the builders can spell — all six
        // scheme/base pairs, both grammar-legal digests — yields its
        // suggestion, and the stamp inside it equals the stamp the
        // builder was given.
        for (scheme, base) in [
            (Scheme::A, BaseName::AgentsMd),
            (Scheme::A, BaseName::ClaudeMd),
            (Scheme::B, BaseName::AgentsMd),
            (Scheme::B, BaseName::ClaudeMd),
            (Scheme::C, BaseName::AgentsMd),
            (Scheme::C, BaseName::ClaudeMd),
        ] {
            let path = backup_relative_path(
                &project(),
                "My Project",
                scheme,
                base,
                time_2026_11_19(),
                SHA256,
            )
            .unwrap();
            let suggestion = export_suggested_name(&path).unwrap_or_else(|error| {
                panic!("the builder's own output {path:?} was refused: {error}")
            });
            assert!(
                suggestion.starts_with("AGENTS") || suggestion.starts_with("CLAUDE"),
                "{suggestion:?} lost the target's stem"
            );
            assert!(
                suggestion.ends_with(".backup-20261119T103000123Z.md"),
                "{suggestion:?} lost the stamp or the extension"
            );
        }
    }

    #[test]
    fn cells_that_are_not_the_layout_are_refused_not_guessed() {
        // D-039: a stored cell outside the grammar is a typed refusal.
        // Every arm: wrong root, wrong component count, absolute path,
        // a directory with a dot (the `..` climb), a missing `.bak`,
        // a bad hash8, a bad stamp, a dotless target, an empty piece —
        // and the Windows-separator arms (the round-1 P1-1 fix): a
        // backslash rides in the FILE component, where a `'/'`-split
        // cannot see it but Windows' `Path` would parse it as a climb;
        // the file component's charset bars every separator byte on
        // every platform.
        for bad in [
            "AGENTS.md.20261119T103000123Z.d2a2446b.bak", // no layout
            "db-backups/20261119T103000123Z.d2a2446b.bak", // wrong root
            "backups/AGENTS.md.20261119T103000123Z.d2a2446b.bak", // 2 components
            "backups/dir/AGENTS.md.20261119T103000123Z.d2a2446b.bak/x", // 4
            "/backups/dir/AGENTS.md.20261119T103000123Z.d2a2446b.bak", // absolute
            "backups/../AGENTS.md.20261119T103000123Z.d2a2446b.bak", // dir climb
            "backups/dir../AGENTS.md.20261119T103000123Z.d2a2446b.bak",
            "backups//AGENTS.md.20261119T103000123Z.d2a2446b.bak", // empty dir
            "backups/dir/AGENTS.md.20261119T103000123Z.d2a2446b",  // no .bak
            "backups/dir/AGENTS.md.20261119T103000123Z.d2a2446.bak", // 7-hex
            "backups/dir/AGENTS.md.20261119T103000123Z.D2A2446B.bak", // uppercase
            "backups/dir/AGENTS.md.20261119103000123Z.d2a2446b.bak", // no T
            "backups/dir/AGENTS.md.20261119T10300012.d2a2446b.bak", // short stamp
            "backups/dir/AGENTS.md.2026111XT103000123Z.d2a2446b.bak", // non-digit
            "backups/dir/20261119T103000123Z.d2a2446b.bak",        // no target
            "backups/dir/AGENTS..20261119T103000123Z.d2a2446b.bak", // empty ext
            "backups/dir/AGENTSmd.20261119T103000123Z.d2a2446b.bak", // dotless
            // The Windows-separator climbs (P1-1): a backslash in the
            // file component a `'/'`-split leaves in place.
            "backups/dir/..\\..\\..\\..\\Users\\Public\\secret.md.20261119T103000123Z.d2a2446b.bak",
            "backups/dir/AGENTS.md\\..\\..\\escape.20261119T103000123Z.d2a2446b.bak",
            "backups/dir/AGENTS.md.20261119T103000123Z.d2a2446b.ba\\k",
            "backups/dir/AGENTS\u{0}md.20261119T103000123Z.d2a2446b.bak", // NUL
            "backups/dir/AGENTS%md.20261119T103000123Z.d2a2446b.bak",     // percent
            "backups/dir/AGENTS md.20261119T103000123Z.d2a2446b.bak",     // space
            "backups/dir/Проект.md.20261119T103000123Z.d2a2446b.bak",     // non-ASCII
            "",
        ] {
            assert_eq!(
                export_suggested_name(bad),
                Err(BackupNameError::NotABackupLayout {
                    value: bad.to_owned()
                }),
                "cell {bad:?} was not refused"
            );
        }
    }

    #[test]
    fn the_layout_refusal_names_the_shape_it_wants() {
        // The Display line the command layer logs at the boundary.
        let error = BackupNameError::NotABackupLayout {
            value: "/etc/passwd".to_owned(),
        };
        assert_eq!(
            error.to_string(),
            "backup path \"/etc/passwd\" is not a backup copy name \
             (backups/<dir>/<file>.<stamp>.<hash8>.bak)"
        );
    }

    #[test]
    fn the_layout_cannot_climb_out_of_the_backups_tree_on_any_platform() {
        // The P1-1 property, stated on the JOINED path: whatever the
        // cell says, the layout this module accepts — joined under a
        // root — stays inside `root/backups/...`, on both separator
        // spellings (`std::path` parses `\\` as a separator only on
        // Windows, `/` everywhere: the property is checked by the
        // charset, not by the platform running the test, so this arm
        // refuses the hostile cells BEFORE any platform sees them).
        for hostile in [
            "..\\..\\..\\..\\Users\\Public\\secret.md.20261119T103000123Z.d2a2446b.bak",
            "AGENTS.md\\..\\..\\escape.20261119T103000123Z.d2a2446b.bak",
            "../escape.md.20261119T103000123Z.d2a2446b.bak",
            "AGENTS.md 20261119T103000123Z.d2a2446b.bak", // a space rides in
            "AGENTS\u{0}.md.20261119T103000123Z.d2a2446b.bak", // a NUL rides in
        ] {
            let cell = format!("backups/{ID}__project/{hostile}");
            assert!(
                export_suggested_name(&cell).is_err(),
                "the tampered cell {cell:?} was accepted"
            );
        }
        // And the grammar's own output joins flat: three Normal
        // components under every parser std knows.
        let path = backup_relative_path(
            &project(),
            "My Project",
            Scheme::C,
            BaseName::ClaudeMd,
            time_2026_11_19(),
            SHA256,
        )
        .unwrap();
        assert_all_normal(&path);
        assert_eq!(Path::new(&path).components().count(), 3);
    }
}
