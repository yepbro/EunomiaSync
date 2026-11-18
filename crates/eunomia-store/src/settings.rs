//! Settings repository (spec §6.5).
//!
//! All user-managed settings live in the `settings` table — SQLite is
//! the only store for them (spec D-18; the machine-level `config.toml`
//! of spec §5.8 is deliberately not a setting). Values are stored as
//! JSON text in the `value` column; this module owns the canonical key
//! list, the default values and the ranges of the §6.5 table.
//!
//! Reading ([`get`], [`get_all`]) applies the §6.5 default when a key
//! has no row; a read never writes. Writing ([`set`]) first checks
//! that the value's variant belongs to the key, then validates the
//! value against the key's type and range through the same parse
//! ([`SettingValue::from_json`]) that the read path and the future IPC
//! commands (T-030) use, so whatever is stored can always be read
//! back. `set` runs inside the caller's transaction (it is a single
//! statement); a settings-save screen that must apply atomically wraps
//! its `set` calls in one transaction.
//!
//! A stored value that is not valid JSON, or valid JSON outside the
//! key's contract, fails the read with an explicit
//! [`StorageError::Settings`] — never a silent fallback to the
//! default. The settings table is machine-written (only [`set`]
//! writes it), so a bad cell means corruption or a version skew and
//! must surface; this deliberately differs from `config.toml`
//! (SYS-012), where a hand-edited broken file degrades to defaults
//! with a warning. Rows with keys outside the §6.5 set are ignored by
//! [`get_all`]: a newer app version may add keys, and this repository
//! serves the set it knows.
//!
//! `default.*` keys are defaults *for new projects* (spec §6.5,
//! DEF-005/PRJ-012): existing projects carry their own scheme, base
//! name and extra-file columns, and changing a default never rewrites
//! them. The copy into a newly created project belongs to the future
//! project-creation task; nothing in this module reads `default.*`
//! into any project row.

use std::collections::HashMap;
use std::fmt;

use rusqlite::Connection;
use rusqlite::OptionalExtension;

use crate::storage::StorageError;
use crate::values::now_utc;

/// The §6.5 `backup.retention` range: 1–200 (owner decision D-17).
const BACKUP_RETENTION_RANGE: (i64, i64) = (1, 200);
/// The §6.5 `scan.max_depth` range: 1–20 (owner decision D-04).
const SCAN_MAX_DEPTH_RANGE: (i64, i64) = (1, 20);
/// The §6.5 `history.max_revisions` range: 10–1000.
const HISTORY_MAX_REVISIONS_RANGE: (i64, i64) = (10, 1000);
/// The §6.5 `log.max_entries` range: 1000–100000.
const LOG_MAX_ENTRIES_RANGE: (i64, i64) = (1000, 100000);
/// The §6.5 `log.max_days` range: 7–3650.
const LOG_MAX_DAYS_RANGE: (i64, i64) = (7, 3650);

/// The §6.5 `scan.exclude_names` default (owner decision D-05).
const DEFAULT_SCAN_EXCLUDE_NAMES: [&str; 7] = [
    "node_modules",
    "vendor",
    ".git",
    "dist",
    "build",
    "target",
    ".venv",
];

/// The shape of a setting value — the §6.5 "Type" column, as one
/// typed vocabulary shared by [`SettingKey::value_kind`] and
/// [`SettingValue::kind`], so the pairing check of [`set`] compares
/// enum values rather than two hand-maintained string tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    /// A scheme member.
    Scheme,
    /// A base-name member.
    BaseName,
    /// An enabled-plus-position pair.
    ExtraFile,
    /// A boolean.
    Boolean,
    /// An integer within the key's range.
    Integer,
    /// A free-form string.
    Str,
    /// A list of strings.
    StrList,
}

impl ValueKind {
    /// The kind's name, for error messages.
    const fn as_str(&self) -> &'static str {
        match self {
            ValueKind::Scheme => "scheme",
            ValueKind::BaseName => "base name",
            ValueKind::ExtraFile => "extra file",
            ValueKind::Boolean => "boolean",
            ValueKind::Integer => "integer",
            ValueKind::Str => "string",
            ValueKind::StrList => "string list",
        }
    }
}

/// One key of the §6.5 settings table.
///
/// The variants are listed in §6.5 table order; [`SettingKey::ALL`]
/// preserves that order, so [`get_all`] returns the spec's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingKey {
    /// `default.scheme` — scheme for new projects, one of A, B, C.
    DefaultScheme,
    /// `default.base_name` — target file for new projects.
    DefaultBaseName,
    /// `default.extra_local` — the `AGENTS.local.md` hint for new
    /// projects: enabled plus position.
    DefaultExtraLocal,
    /// `default.extra_project` — the `AGENTS.project.md` hint for new
    /// projects: enabled plus position.
    DefaultExtraProject,
    /// `backup.retention` — how many per-file backups are kept.
    BackupRetention,
    /// `sync.skip_identical` — do not overwrite byte-identical targets
    /// (owner decision D-24).
    SyncSkipIdentical,
    /// `scan.max_depth` — repository scan depth (owner decision D-04).
    ScanMaxDepth,
    /// `scan.exclude_names` — directory names the scan skips (owner
    /// decision D-05).
    ScanExcludeNames,
    /// `history.max_revisions` — revisions kept per entity.
    HistoryMaxRevisions,
    /// `log.max_entries` — activity-log entries kept.
    LogMaxEntries,
    /// `log.max_days` — activity-log retention in days.
    LogMaxDays,
    /// `ui.language` — the interface language.
    UiLanguage,
    /// `update.check_on_start` — look for updates at launch.
    UpdateCheckOnStart,
}

impl SettingKey {
    /// Every §6.5 key, in table order.
    pub const ALL: [SettingKey; 13] = [
        SettingKey::DefaultScheme,
        SettingKey::DefaultBaseName,
        SettingKey::DefaultExtraLocal,
        SettingKey::DefaultExtraProject,
        SettingKey::BackupRetention,
        SettingKey::SyncSkipIdentical,
        SettingKey::ScanMaxDepth,
        SettingKey::ScanExcludeNames,
        SettingKey::HistoryMaxRevisions,
        SettingKey::LogMaxEntries,
        SettingKey::LogMaxDays,
        SettingKey::UiLanguage,
        SettingKey::UpdateCheckOnStart,
    ];

    /// The key's dotted form — the literal stored in the `settings`
    /// table and carried over the IPC (spec §6.5).
    pub const fn as_str(&self) -> &'static str {
        match self {
            SettingKey::DefaultScheme => "default.scheme",
            SettingKey::DefaultBaseName => "default.base_name",
            SettingKey::DefaultExtraLocal => "default.extra_local",
            SettingKey::DefaultExtraProject => "default.extra_project",
            SettingKey::BackupRetention => "backup.retention",
            SettingKey::SyncSkipIdentical => "sync.skip_identical",
            SettingKey::ScanMaxDepth => "scan.max_depth",
            SettingKey::ScanExcludeNames => "scan.exclude_names",
            SettingKey::HistoryMaxRevisions => "history.max_revisions",
            SettingKey::LogMaxEntries => "log.max_entries",
            SettingKey::LogMaxDays => "log.max_days",
            SettingKey::UiLanguage => "ui.language",
            SettingKey::UpdateCheckOnStart => "update.check_on_start",
        }
    }

    /// Parses a dotted key string back into the enum.
    ///
    /// Returns `None` for anything outside the §6.5 set — the caller
    /// (the future IPC layer) turns that into its unknown-key error.
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "default.scheme" => SettingKey::DefaultScheme,
            "default.base_name" => SettingKey::DefaultBaseName,
            "default.extra_local" => SettingKey::DefaultExtraLocal,
            "default.extra_project" => SettingKey::DefaultExtraProject,
            "backup.retention" => SettingKey::BackupRetention,
            "sync.skip_identical" => SettingKey::SyncSkipIdentical,
            "scan.max_depth" => SettingKey::ScanMaxDepth,
            "scan.exclude_names" => SettingKey::ScanExcludeNames,
            "history.max_revisions" => SettingKey::HistoryMaxRevisions,
            "log.max_entries" => SettingKey::LogMaxEntries,
            "log.max_days" => SettingKey::LogMaxDays,
            "ui.language" => SettingKey::UiLanguage,
            "update.check_on_start" => SettingKey::UpdateCheckOnStart,
            _ => return None,
        })
    }

    /// The kind of value the key stores (§6.5 "Type" column), for the
    /// pairing check of [`set`]. Keys sharing a shape (`Integer`
    /// across the five ranged keys, `Boolean` across the two switches,
    /// the two extra files) share a kind; the per-key range and
    /// members are checked by [`SettingValue::from_json`].
    const fn value_kind(&self) -> ValueKind {
        match self {
            SettingKey::DefaultScheme => ValueKind::Scheme,
            SettingKey::DefaultBaseName => ValueKind::BaseName,
            SettingKey::DefaultExtraLocal | SettingKey::DefaultExtraProject => ValueKind::ExtraFile,
            SettingKey::BackupRetention
            | SettingKey::ScanMaxDepth
            | SettingKey::HistoryMaxRevisions
            | SettingKey::LogMaxEntries
            | SettingKey::LogMaxDays => ValueKind::Integer,
            SettingKey::SyncSkipIdentical | SettingKey::UpdateCheckOnStart => ValueKind::Boolean,
            SettingKey::ScanExcludeNames => ValueKind::StrList,
            SettingKey::UiLanguage => ValueKind::Str,
        }
    }

    /// The key's §6.5 default value.
    pub fn default_value(&self) -> SettingValue {
        match self {
            SettingKey::DefaultScheme => SettingValue::Scheme(Scheme::A),
            SettingKey::DefaultBaseName => SettingValue::BaseName(BaseName::AgentsMd),
            SettingKey::DefaultExtraLocal | SettingKey::DefaultExtraProject => {
                SettingValue::ExtraFile(ExtraFile {
                    enabled: false,
                    position: Position::End,
                })
            }
            SettingKey::BackupRetention => SettingValue::Int(10),
            SettingKey::SyncSkipIdentical => SettingValue::Bool(true),
            SettingKey::ScanMaxDepth => SettingValue::Int(6),
            SettingKey::ScanExcludeNames => SettingValue::StrList(
                DEFAULT_SCAN_EXCLUDE_NAMES
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
            ),
            SettingKey::HistoryMaxRevisions => SettingValue::Int(200),
            SettingKey::LogMaxEntries => SettingValue::Int(10000),
            SettingKey::LogMaxDays => SettingValue::Int(180),
            SettingKey::UiLanguage => SettingValue::Str("en".to_owned()),
            SettingKey::UpdateCheckOnStart => SettingValue::Bool(true),
        }
    }
}

/// The managed-block scheme of a project (§6.5 `default.scheme`; the
/// members are the `projects.scheme` CHECK values of schema §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scheme {
    /// Managed block around the whole file content.
    A,
    /// Managed block between HTML comment markers (spec D-19).
    B,
    /// No file edits; a snippet is shown instead (spec D-20).
    C,
}

impl Scheme {
    /// The stored member string.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Scheme::A => "A",
            Scheme::B => "B",
            Scheme::C => "C",
        }
    }

    /// Parses a member string, rejecting anything outside `A`/`B`/`C`
    /// (case-sensitive, like the schema CHECK).
    fn from_member(member: &str) -> Result<Self, SettingsError> {
        match member {
            "A" => Ok(Scheme::A),
            "B" => Ok(Scheme::B),
            "C" => Ok(Scheme::C),
            _ => Err(SettingsError::Member {
                found: member.to_owned(),
            }),
        }
    }
}

/// The target file base name (§6.5 `default.base_name`; the two values
/// of spec D-03 and the `projects.base_name` CHECK of schema §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BaseName {
    /// `AGENTS.md`.
    AgentsMd,
    /// `CLAUDE.md`.
    ClaudeMd,
}

impl BaseName {
    /// The stored member string.
    pub const fn as_str(&self) -> &'static str {
        match self {
            BaseName::AgentsMd => "AGENTS.md",
            BaseName::ClaudeMd => "CLAUDE.md",
        }
    }

    /// Parses a member string, rejecting anything outside the two
    /// allowed names (case-sensitive, like the schema CHECK).
    fn from_member(member: &str) -> Result<Self, SettingsError> {
        match member {
            "AGENTS.md" => Ok(BaseName::AgentsMd),
            "CLAUDE.md" => Ok(BaseName::ClaudeMd),
            _ => Err(SettingsError::Member {
                found: member.to_owned(),
            }),
        }
    }
}

/// Where an extra file's "read this file" line goes (§6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Position {
    /// Before the managed content.
    Start,
    /// After the managed content.
    End,
}

impl Position {
    /// The stored member string.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Position::Start => "start",
            Position::End => "end",
        }
    }

    /// Parses a member string, rejecting anything outside
    /// `start`/`end` (case-sensitive, like the schema CHECKs).
    fn from_member(member: &str) -> Result<Self, SettingsError> {
        match member {
            "start" => Ok(Position::Start),
            "end" => Ok(Position::End),
            _ => Err(SettingsError::Member {
                found: member.to_owned(),
            }),
        }
    }
}

/// An extra-file default for new projects (§6.5 `default.extra_local`
/// and `default.extra_project`): whether the "read this file" line is
/// added at all, and where. Stored as one JSON object
/// `{"enabled": <bool>, "position": "start"|"end"}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExtraFile {
    /// Whether the line is added.
    pub enabled: bool,
    /// Where the line is added.
    pub position: Position,
}

/// The value of one setting, typed per the §6.5 "Type" column.
///
/// Each key accepts exactly one variant (see [`SettingValue::from_json`]);
/// a mismatched pairing is a validation error, not a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    /// A scheme member (`default.scheme`).
    Scheme(Scheme),
    /// A base-name member (`default.base_name`).
    BaseName(BaseName),
    /// An enabled-plus-position pair (`default.extra_*`).
    ExtraFile(ExtraFile),
    /// A boolean (`sync.skip_identical`, `update.check_on_start`).
    Bool(bool),
    /// An integer within the key's range (the five ranged keys).
    Int(i64),
    /// A free-form string (`ui.language`).
    Str(String),
    /// A list of strings (`scan.exclude_names`).
    StrList(Vec<String>),
}

impl SettingValue {
    /// The value's kind, for the pairing check of [`set`].
    const fn kind(&self) -> ValueKind {
        match self {
            SettingValue::Scheme(_) => ValueKind::Scheme,
            SettingValue::BaseName(_) => ValueKind::BaseName,
            SettingValue::ExtraFile(_) => ValueKind::ExtraFile,
            SettingValue::Bool(_) => ValueKind::Boolean,
            SettingValue::Int(_) => ValueKind::Integer,
            SettingValue::Str(_) => ValueKind::Str,
            SettingValue::StrList(_) => ValueKind::StrList,
        }
    }

    /// Parses and validates a JSON value for `key` (§6.5 Type and
    /// Range columns). This is the single validation boundary: the
    /// read path, the write path and the future IPC layer all go
    /// through it, so a value is storable exactly when it is readable.
    ///
    /// Integers must be JSON integers within the key's inclusive
    /// range; enum members are case-sensitive strings matching the
    /// schema CHECK vocabularies; the extra-file shape is an object
    /// with exactly `enabled` (boolean) and `position`
    /// (`"start"`/`"end"`); `scan.exclude_names` is an array whose
    /// every element is a string (§6.5 pins no further content rules,
    /// so an empty list and empty names are accepted).
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::Type`] when the JSON value is not of
    /// the key's type, [`SettingsError::Range`] when an integer is
    /// outside the key's range, [`SettingsError::Member`] when a
    /// string is not one of the key's allowed members, and
    /// [`SettingsError::Shape`] when the extra-file object does not
    /// have exactly the two required fields.
    pub fn from_json(key: SettingKey, json: &serde_json::Value) -> Result<Self, SettingsError> {
        match key {
            SettingKey::DefaultScheme => Ok(SettingValue::Scheme(Scheme::from_member(
                expect_string(json)?,
            )?)),
            SettingKey::DefaultBaseName => Ok(SettingValue::BaseName(BaseName::from_member(
                expect_string(json)?,
            )?)),
            SettingKey::DefaultExtraLocal | SettingKey::DefaultExtraProject => {
                Ok(SettingValue::ExtraFile(extra_file_from_json(json)?))
            }
            SettingKey::BackupRetention => Ok(SettingValue::Int(integer_in_range(
                json,
                BACKUP_RETENTION_RANGE,
            )?)),
            SettingKey::SyncSkipIdentical | SettingKey::UpdateCheckOnStart => match json {
                serde_json::Value::Bool(flag) => Ok(SettingValue::Bool(*flag)),
                other => Err(type_error("boolean", other)),
            },
            SettingKey::ScanMaxDepth => Ok(SettingValue::Int(integer_in_range(
                json,
                SCAN_MAX_DEPTH_RANGE,
            )?)),
            SettingKey::ScanExcludeNames => Ok(SettingValue::StrList(string_list(json)?)),
            SettingKey::HistoryMaxRevisions => Ok(SettingValue::Int(integer_in_range(
                json,
                HISTORY_MAX_REVISIONS_RANGE,
            )?)),
            SettingKey::LogMaxEntries => Ok(SettingValue::Int(integer_in_range(
                json,
                LOG_MAX_ENTRIES_RANGE,
            )?)),
            SettingKey::LogMaxDays => Ok(SettingValue::Int(integer_in_range(
                json,
                LOG_MAX_DAYS_RANGE,
            )?)),
            SettingKey::UiLanguage => Ok(SettingValue::Str(expect_string(json)?.to_owned())),
        }
    }

    /// The canonical JSON form of the value — what [`set`] stores in
    /// the `value` column. Compact when serialized: `"C"`, `10`,
    /// `{"enabled":true,"position":"end"}`,
    /// `["node_modules","vendor"]`.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            SettingValue::Scheme(scheme) => serde_json::Value::String(scheme.as_str().to_owned()),
            SettingValue::BaseName(name) => serde_json::Value::String(name.as_str().to_owned()),
            SettingValue::ExtraFile(extra) => serde_json::json!({
                "enabled": extra.enabled,
                "position": extra.position.as_str(),
            }),
            SettingValue::Bool(flag) => serde_json::Value::Bool(*flag),
            SettingValue::Int(number) => serde_json::Value::from(*number),
            SettingValue::Str(text) => serde_json::Value::String(text.clone()),
            SettingValue::StrList(items) => serde_json::Value::Array(
                items
                    .iter()
                    .map(|item| serde_json::Value::String(item.clone()))
                    .collect(),
            ),
        }
    }
}

/// Why a setting value failed validation or decoding.
#[derive(Debug)]
pub enum SettingsError {
    /// The JSON value is not of the key's type (§6.5 "Type" column):
    /// a string where a boolean belongs, a fractional number where an
    /// integer belongs, an object where an array belongs.
    Type {
        /// The type the key stores.
        expected: &'static str,
        /// What the JSON value actually is.
        found: &'static str,
    },
    /// An integer outside the key's inclusive range (§6.5 "Range"
    /// column). `value` is wide because a JSON integer can exceed
    /// i64 while still being stored exactly (serde_json keeps up to
    /// u64; anything larger is held as a float and therefore reported
    /// as a type error instead).
    Range {
        /// The rejected integer.
        value: i128,
        /// The range's inclusive lower bound.
        min: i64,
        /// The range's inclusive upper bound.
        max: i64,
    },
    /// A string that is not one of the key's allowed members (scheme,
    /// base name, extra-file position).
    Member {
        /// The offending string.
        found: String,
    },
    /// The extra-file object does not have exactly the fields
    /// `enabled` and `position`: a field is missing or unknown.
    Shape {
        /// What is wrong, naming the field(s).
        problem: String,
    },
    /// The stored text is not valid JSON at all.
    Json(serde_json::Error),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Type { expected, found } => {
                write!(f, "expected {expected}, found {found}")
            }
            SettingsError::Range { value, min, max } => {
                write!(f, "{value} is outside the range {min}..={max}")
            }
            SettingsError::Member { found } => {
                write!(f, "{found:?} is not an allowed value")
            }
            SettingsError::Shape { problem } => write!(f, "malformed object: {problem}"),
            SettingsError::Json(err) => write!(f, "invalid JSON: {err}"),
        }
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SettingsError::Json(err) => Some(err),
            _ => None,
        }
    }
}

/// The JSON kind of a value, for [`SettingsError::Type`] messages.
/// Numbers split into exactly-stored integers and floating-point
/// values, so a fractional input reads "expected integer, found
/// floating-point number" instead of contradicting itself.
fn json_kind(json: &serde_json::Value) -> &'static str {
    match json {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        serde_json::Value::Number(_) => "floating-point number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Builds the type-mismatch error for `json`.
fn type_error(expected: &'static str, json: &serde_json::Value) -> SettingsError {
    SettingsError::Type {
        expected,
        found: json_kind(json),
    }
}

/// Extracts a JSON string, rejecting other kinds.
fn expect_string(json: &serde_json::Value) -> Result<&str, SettingsError> {
    match json {
        serde_json::Value::String(text) => Ok(text),
        other => Err(type_error("string", other)),
    }
}

/// Extracts a JSON integer within `range` (inclusive bounds), so that
/// out-of-range integers are distinguishable from non-integers.
fn integer_in_range(json: &serde_json::Value, range: (i64, i64)) -> Result<i64, SettingsError> {
    let serde_json::Value::Number(number) = json else {
        return Err(type_error("integer", json));
    };
    let Some(value) = number.as_i128() else {
        // Fractional, or an integer beyond u64 precision (which
        // serde_json already holds as a float): not an integer.
        return Err(type_error("integer", json));
    };
    let (min, max) = range;
    if (i128::from(min)..=i128::from(max)).contains(&value) {
        Ok(i64::try_from(value).expect("the range check bounds the value within i64"))
    } else {
        Err(SettingsError::Range { value, min, max })
    }
}

/// Extracts an array in which every element is a string.
fn string_list(json: &serde_json::Value) -> Result<Vec<String>, SettingsError> {
    let serde_json::Value::Array(items) = json else {
        return Err(type_error("array of strings", json));
    };
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let serde_json::Value::String(name) = item else {
            return Err(type_error("string array element", item));
        };
        names.push(name.clone());
    }
    Ok(names)
}

/// Parses the `default.extra_*` object: exactly `enabled` (boolean)
/// and `position` (`"start"`/`"end"`), no other fields.
fn extra_file_from_json(json: &serde_json::Value) -> Result<ExtraFile, SettingsError> {
    let serde_json::Value::Object(fields) = json else {
        return Err(type_error("object with \"enabled\" and \"position\"", json));
    };
    let enabled = match fields.get("enabled") {
        Some(serde_json::Value::Bool(enabled)) => *enabled,
        Some(other) => return Err(type_error("boolean", other)),
        None => {
            return Err(SettingsError::Shape {
                problem: "missing field \"enabled\"".to_owned(),
            });
        }
    };
    let position = match fields.get("position") {
        Some(serde_json::Value::String(member)) => Position::from_member(member)?,
        Some(other) => return Err(type_error("string (\"start\" or \"end\")", other)),
        None => {
            return Err(SettingsError::Shape {
                problem: "missing field \"position\"".to_owned(),
            });
        }
    };
    let unknown: Vec<&str> = fields
        .keys()
        .filter(|name| *name != "enabled" && *name != "position")
        .map(|name| name.as_str())
        .collect();
    if !unknown.is_empty() {
        let listed = unknown
            .iter()
            .map(|name| format!("{name:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let plural = if unknown.len() == 1 { "" } else { "s" };
        return Err(SettingsError::Shape {
            problem: format!("unknown field{plural} {listed}"),
        });
    }
    Ok(ExtraFile { enabled, position })
}

/// Wraps a [`SettingsError`] with the key it belongs to.
fn invalid(key: SettingKey, source: SettingsError) -> StorageError {
    StorageError::Settings {
        key: key.as_str().to_owned(),
        source,
    }
}

/// Parses and validates a stored `value` cell for `key`.
fn parse_stored(key: SettingKey, text: &str) -> Result<SettingValue, StorageError> {
    let json = serde_json::from_str(text).map_err(|err| invalid(key, SettingsError::Json(err)))?;
    SettingValue::from_json(key, &json).map_err(|source| invalid(key, source))
}

/// Reads one setting, applying its §6.5 default when no row exists.
///
/// # Errors
///
/// Returns [`StorageError::Settings`] when a row exists but its value
/// is not valid JSON or outside the key's contract (an explicit error,
/// never a silent default), and [`StorageError::Sql`] when the query
/// fails.
pub fn get(conn: &Connection, key: SettingKey) -> Result<SettingValue, StorageError> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [key.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    match stored {
        Some(text) => parse_stored(key, &text),
        None => Ok(key.default_value()),
    }
}

/// Reads every §6.5 setting with defaults applied, in table order.
///
/// Rows with keys outside [`SettingKey::ALL`] are ignored (a newer app
/// version may have written them); every known key is present in the
/// result.
///
/// # Errors
///
/// Returns [`StorageError::Settings`] when any stored value of a known
/// key is invalid, and [`StorageError::Sql`] when the query fails.
pub fn get_all(conn: &Connection) -> Result<Vec<(SettingKey, SettingValue)>, StorageError> {
    let mut statement = conn.prepare("SELECT key, value FROM settings")?;
    let stored: HashMap<String, String> = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<_, _>>()?;
    SettingKey::ALL
        .iter()
        .map(|&key| match stored.get(key.as_str()) {
            Some(text) => parse_stored(key, text).map(|value| (key, value)),
            None => Ok((key, key.default_value())),
        })
        .collect()
}

/// Validates and stores one setting (an upsert of the single row).
///
/// Two checks run before the write. First, the value's variant must
/// belong to the key ([`SettingValue::kind`] versus the key's §6.5
/// type): a Rust caller mistake like pairing [`SettingValue::Scheme`]
/// with `ui.language` is rejected instead of silently coercing through
/// the JSON form. Second, the value's canonical JSON is re-parsed
/// through [`SettingValue::from_json`] — the same parse the read path
/// and the future IPC layer use — so a value is storable exactly when
/// it is readable. The write is one statement inside the caller's
/// transaction; `updated_at` is stamped with the DB-002 UTC format.
///
/// # Errors
///
/// Returns [`StorageError::Settings`] when the value does not fit the
/// key's type or range (the row is left untouched — both checks run
/// before the write), and [`StorageError::Sql`] when the statement
/// fails.
pub fn set(conn: &Connection, key: SettingKey, value: SettingValue) -> Result<(), StorageError> {
    if value.kind() != key.value_kind() {
        return Err(invalid(
            key,
            SettingsError::Type {
                expected: key.value_kind().as_str(),
                found: value.kind().as_str(),
            },
        ));
    }
    let json = value.to_json();
    SettingValue::from_json(key, &json).map_err(|source| invalid(key, source))?;
    let text = serde_json::to_string(&json).expect("a serde_json::Value always serializes");
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (key) DO UPDATE SET
             value = excluded.value,
             updated_at = excluded.updated_at",
        rusqlite::params![key.as_str(), text, now_utc()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §6.5 key literals, in table order.
    const SPEC_KEYS: [&str; 13] = [
        "default.scheme",
        "default.base_name",
        "default.extra_local",
        "default.extra_project",
        "backup.retention",
        "sync.skip_identical",
        "scan.max_depth",
        "scan.exclude_names",
        "history.max_revisions",
        "log.max_entries",
        "log.max_days",
        "ui.language",
        "update.check_on_start",
    ];

    /// Asserts `json` parses for `key` and returns the value
    /// (`SettingsError` carries a `serde_json::Error` and cannot be
    /// compared wholesale, so failures render through the panic).
    fn parsed(key: SettingKey, json: &serde_json::Value) -> SettingValue {
        SettingValue::from_json(key, json)
            .unwrap_or_else(|err| panic!("{}: {json} should parse, got {err}", key.as_str()))
    }

    #[test]
    fn keys_round_trip_the_spec_literals() {
        assert_eq!(SettingKey::ALL.len(), SPEC_KEYS.len());
        for (key, literal) in SettingKey::ALL.iter().zip(SPEC_KEYS) {
            assert_eq!(key.as_str(), literal);
            assert_eq!(SettingKey::from_key(literal), Some(*key));
        }
        for unknown in [
            "",
            "default",
            "default.extra",
            "default.extra_local.enabled",
            "backup.retention ",
            "UI.language",
        ] {
            assert_eq!(
                SettingKey::from_key(unknown),
                None,
                "{unknown:?} must not parse as a setting key"
            );
        }
    }

    #[test]
    fn every_keys_default_value_has_the_keys_kind() {
        // Cross-checks the two kind tables through the defaults: a
        // mismatch would make `set` reject the key's own default.
        for key in SettingKey::ALL {
            assert_eq!(
                key.default_value().kind(),
                key.value_kind(),
                "{}",
                key.as_str()
            );
        }
    }

    #[test]
    fn defaults_are_the_spec_6_5_values() {
        // Expected values hardcoded from the spec §6.5 table — an
        // oracle independent of the production constants.
        for key in SettingKey::ALL {
            let expected = match key {
                SettingKey::DefaultScheme => SettingValue::Scheme(Scheme::A),
                SettingKey::DefaultBaseName => SettingValue::BaseName(BaseName::AgentsMd),
                SettingKey::DefaultExtraLocal | SettingKey::DefaultExtraProject => {
                    SettingValue::ExtraFile(ExtraFile {
                        enabled: false,
                        position: Position::End,
                    })
                }
                SettingKey::BackupRetention => SettingValue::Int(10),
                SettingKey::SyncSkipIdentical => SettingValue::Bool(true),
                SettingKey::ScanMaxDepth => SettingValue::Int(6),
                SettingKey::ScanExcludeNames => SettingValue::StrList(
                    [
                        "node_modules",
                        "vendor",
                        ".git",
                        "dist",
                        "build",
                        "target",
                        ".venv",
                    ]
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
                ),
                SettingKey::HistoryMaxRevisions => SettingValue::Int(200),
                SettingKey::LogMaxEntries => SettingValue::Int(10000),
                SettingKey::LogMaxDays => SettingValue::Int(180),
                SettingKey::UiLanguage => SettingValue::Str("en".to_owned()),
                SettingKey::UpdateCheckOnStart => SettingValue::Bool(true),
            };
            assert_eq!(key.default_value(), expected, "default of {}", key.as_str());
        }
    }

    #[test]
    fn canonical_json_forms_round_trip_through_from_json() {
        let cases: Vec<(SettingKey, SettingValue, serde_json::Value)> = vec![
            (
                SettingKey::DefaultScheme,
                SettingValue::Scheme(Scheme::C),
                serde_json::json!("C"),
            ),
            (
                SettingKey::DefaultBaseName,
                SettingValue::BaseName(BaseName::ClaudeMd),
                serde_json::json!("CLAUDE.md"),
            ),
            (
                SettingKey::DefaultExtraLocal,
                SettingValue::ExtraFile(ExtraFile {
                    enabled: true,
                    position: Position::Start,
                }),
                serde_json::json!({"enabled": true, "position": "start"}),
            ),
            (
                SettingKey::BackupRetention,
                SettingValue::Int(200),
                serde_json::json!(200),
            ),
            (
                SettingKey::SyncSkipIdentical,
                SettingValue::Bool(false),
                serde_json::json!(false),
            ),
            (
                SettingKey::ScanMaxDepth,
                SettingValue::Int(1),
                serde_json::json!(1),
            ),
            (
                SettingKey::ScanExcludeNames,
                SettingValue::StrList(vec![".cache".to_owned(), "tmp".to_owned()]),
                serde_json::json!([".cache", "tmp"]),
            ),
            (
                SettingKey::HistoryMaxRevisions,
                SettingValue::Int(1000),
                serde_json::json!(1000),
            ),
            (
                SettingKey::LogMaxEntries,
                SettingValue::Int(100000),
                serde_json::json!(100000),
            ),
            (
                SettingKey::LogMaxDays,
                SettingValue::Int(3650),
                serde_json::json!(3650),
            ),
            (
                SettingKey::UiLanguage,
                SettingValue::Str("cs".to_owned()),
                serde_json::json!("cs"),
            ),
            (
                SettingKey::UpdateCheckOnStart,
                SettingValue::Bool(false),
                serde_json::json!(false),
            ),
        ];
        assert_eq!(cases.len(), SettingKey::ALL.len() - 1); // extra_project shares its shape

        for (key, value, json) in cases {
            assert_eq!(value.to_json(), json, "to_json of {}", key.as_str());
            assert_eq!(parsed(key, &json), value, "from_json of {}", key.as_str());
        }
    }

    #[test]
    fn integer_ranges_accept_their_bounds_and_reject_one_past() {
        // Bounds hardcoded from the spec §6.5 table — an oracle
        // independent of the production range constants, so a mutated
        // constant fails here.
        let ranges: [(SettingKey, (i64, i64)); 5] = [
            (SettingKey::BackupRetention, (1, 200)),
            (SettingKey::ScanMaxDepth, (1, 20)),
            (SettingKey::HistoryMaxRevisions, (10, 1000)),
            (SettingKey::LogMaxEntries, (1000, 100000)),
            (SettingKey::LogMaxDays, (7, 3650)),
        ];
        for (key, (min, max)) in ranges {
            for bound in [min, max] {
                assert_eq!(
                    parsed(key, &serde_json::json!(bound)),
                    SettingValue::Int(bound),
                    "{}: {bound} is inside the range",
                    key.as_str()
                );
            }
            for rejected in [min - 1, max + 1] {
                match SettingValue::from_json(key, &serde_json::json!(rejected)) {
                    Err(SettingsError::Range {
                        value,
                        min: lo,
                        max: hi,
                    }) => {
                        assert_eq!(value, i128::from(rejected));
                        assert_eq!((lo, hi), (min, max));
                    }
                    other => panic!(
                        "{}: {rejected} must be rejected as out of range, got {other:?}",
                        key.as_str()
                    ),
                }
            }
        }
    }

    #[test]
    fn non_integers_are_type_errors_not_range_errors() {
        for (key, json, found_kind) in [
            (
                SettingKey::BackupRetention,
                serde_json::json!(1.5),
                "floating-point number",
            ),
            (
                SettingKey::BackupRetention,
                serde_json::json!(10.0),
                "floating-point number",
            ),
            (SettingKey::ScanMaxDepth, serde_json::json!("6"), "string"),
            (SettingKey::ScanMaxDepth, serde_json::json!(true), "boolean"),
        ] {
            match SettingValue::from_json(key, &json) {
                Err(SettingsError::Type { expected, found }) => {
                    assert_eq!(expected, "integer");
                    assert_eq!(found, found_kind, "{key:?} {json}");
                }
                other => panic!("{key:?} {json}: expected a type error, got {other:?}"),
            }
        }
    }

    #[test]
    fn integers_beyond_i64_still_report_their_range() {
        // serde_json keeps u64 exactly; a retention of ~10^19 is out
        // of range, not "not an integer".
        let huge = serde_json::json!(9_999_999_999_999_999_999u64);
        match SettingValue::from_json(SettingKey::BackupRetention, &huge) {
            Err(SettingsError::Range { value, min, max }) => {
                assert_eq!(value, 9_999_999_999_999_999_999i128);
                assert_eq!((min, max), (1, 200));
            }
            other => panic!("expected a range error, got {other:?}"),
        }
    }

    #[test]
    fn every_key_rejects_null_and_wrong_shapes() {
        for key in SettingKey::ALL {
            let err = SettingValue::from_json(key, &serde_json::Value::Null)
                .expect_err("null is never a setting value");
            assert!(
                matches!(err, SettingsError::Type { .. }),
                "{}: {err:?}",
                key.as_str()
            );
        }
        let mismatched: [(SettingKey, serde_json::Value, &str); 7] = [
            (SettingKey::DefaultScheme, serde_json::json!(true), "string"),
            (SettingKey::DefaultBaseName, serde_json::json!(1), "string"),
            (
                SettingKey::DefaultExtraLocal,
                serde_json::json!("end"),
                "object with \"enabled\" and \"position\"",
            ),
            (
                SettingKey::BackupRetention,
                serde_json::json!({}),
                "integer",
            ),
            (
                SettingKey::SyncSkipIdentical,
                serde_json::json!("true"),
                "boolean",
            ),
            (
                SettingKey::ScanExcludeNames,
                serde_json::json!("node_modules"),
                "array of strings",
            ),
            (SettingKey::UiLanguage, serde_json::json!(5), "string"),
        ];
        for (key, json, expected) in mismatched {
            match SettingValue::from_json(key, &json) {
                Err(SettingsError::Type { expected: want, .. }) => {
                    assert_eq!(want, expected, "{}", key.as_str());
                }
                other => panic!("{key:?} {json}: expected a type error, got {other:?}"),
            }
        }
    }

    #[test]
    fn string_lists_require_every_element_to_be_a_string() {
        match SettingValue::from_json(
            SettingKey::ScanExcludeNames,
            &serde_json::json!(["node_modules", 7]),
        ) {
            Err(SettingsError::Type { expected, .. }) => {
                assert_eq!(expected, "string array element");
            }
            other => panic!("expected a type error, got {other:?}"),
        }
        // §6.5 pins no content rules: an empty list is valid.
        assert_eq!(
            parsed(SettingKey::ScanExcludeNames, &serde_json::json!([])),
            SettingValue::StrList(Vec::new())
        );
    }

    #[test]
    fn enum_members_are_case_sensitive_and_closed() {
        for (key, json) in [
            (SettingKey::DefaultScheme, serde_json::json!("D")),
            (SettingKey::DefaultScheme, serde_json::json!("a")),
            (SettingKey::DefaultBaseName, serde_json::json!("agents.md")),
            (SettingKey::DefaultBaseName, serde_json::json!("AGENTS.MD")),
            (SettingKey::DefaultBaseName, serde_json::json!("README.md")),
        ] {
            match SettingValue::from_json(key, &json) {
                Err(SettingsError::Member { found }) => {
                    assert_eq!(found, json.as_str().unwrap());
                }
                other => panic!("{key:?} {json}: expected a member error, got {other:?}"),
            }
        }
    }

    #[test]
    fn extra_file_objects_need_exactly_enabled_and_position() {
        let key = SettingKey::DefaultExtraLocal;
        let malformed: [(serde_json::Value, &str); 3] = [
            (serde_json::json!({}), "missing field"),
            (serde_json::json!({"enabled": true}), "missing field"),
            (
                serde_json::json!({"enabled": true, "position": "end", "note": 1}),
                "unknown field",
            ),
        ];
        for (json, needle) in malformed {
            match SettingValue::from_json(key, &json) {
                Err(SettingsError::Shape { problem }) => {
                    assert!(
                        problem.contains(needle),
                        "problem {problem:?} should mention {needle:?}"
                    );
                }
                other => panic!("{json}: expected a shape error, got {other:?}"),
            }
        }
        for (json, expected) in [
            (
                serde_json::json!({"position": "end"}),
                "missing field \"enabled\"",
            ),
            (
                serde_json::json!({"enabled": true, "position": 3}),
                "expected string",
            ),
            (
                serde_json::json!({"enabled": "yes", "position": "end"}),
                "expected boolean",
            ),
        ] {
            let err = SettingValue::from_json(key, &json)
                .expect_err("a malformed extra-file object must be rejected");
            assert!(
                err.to_string().contains(expected),
                "error {err} should contain {expected:?}"
            );
        }
        // The position member itself stays a closed vocabulary.
        match SettingValue::from_json(
            key,
            &serde_json::json!({"enabled": true, "position": "End"}),
        ) {
            Err(SettingsError::Member { found }) => assert_eq!(found, "End"),
            other => panic!("expected a member error, got {other:?}"),
        }
    }
}
