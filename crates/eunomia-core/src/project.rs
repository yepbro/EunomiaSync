//! Project-side domain vocabulary (spec §2, §7.5).
//!
//! These types describe how a project's target file is written: the
//! write scheme, the target file base name, and the two extra-file
//! "read this file" options (spec D-01). They are domain types, not
//! settings: the §6.5 `default.*` settings and the §6.2 `projects`
//! columns both speak this vocabulary, but defaults and persistence
//! belong to `eunomia-store` (`eunomia_store::settings`, DECISIONS.md
//! D-039) — this crate defines the words, not the storage, and does not
//! depend on the store crate.
//!
//! The closed vocabularies serialize to their canonical database forms:
//! the `projects.scheme`, `projects.base_name` and `extra_*_position`
//! CHECK literals of schema §6.2, and the §6.5 JSON object shape
//! `{"enabled": …, "position": …}` for an extra file. The unit tests
//! pin every literal in both directions.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The identifier of a project (spec DB-002).
///
/// An opaque TEXT identifier. This crate never validates the UUIDv7
/// form: generating identifiers is the store layer's job
/// (`eunomia_store::values::new_id`), and the `uuid` crate is outside
/// this crate's dependency allow-list (DECISIONS.md D-017). The newtype
/// still keeps project, item and variant identifiers from being mixed
/// up at call sites.
///
/// # Examples
///
/// ```
/// use eunomia_core::project::ProjectId;
///
/// let id = ProjectId::new("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d".to_owned());
/// assert_eq!(id.as_str(), "0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d");
/// assert_eq!(id.to_string(), "0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(String);

impl ProjectId {
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

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ProjectId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

/// The write scheme of a project (spec §2 "Write scheme", §7.5).
///
/// The serde form is the schema §6.2 `projects.scheme` CHECK literal,
/// and it is case-sensitive in both directions.
///
/// # Examples
///
/// ```
/// use eunomia_core::project::Scheme;
///
/// assert_eq!(Scheme::A.as_str(), "A");
/// assert_eq!(serde_json::to_string(&Scheme::C).unwrap(), "\"C\"");
/// // Case-sensitive, like the schema CHECK:
/// assert!(serde_json::from_str::<Scheme>("\"a\"").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scheme {
    /// Full overwrite of the main file (`AGENTS.md` / `CLAUDE.md`).
    A,
    /// Managed block between the `EUNOMIASYNC` HTML comment markers
    /// (spec D-19); the file must already contain exactly one marker
    /// pair.
    B,
    /// Write to a separate `<STEM>.project.md` file; the main file is
    /// not edited (spec D-20).
    C,
}

impl Scheme {
    /// The canonical schema literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
        }
    }
}

/// The target file base name (spec §2 "File name (base name)", D-03).
///
/// The serde form is the schema §6.2 `projects.base_name` CHECK literal.
///
/// # Examples
///
/// ```
/// use eunomia_core::project::BaseName;
///
/// assert_eq!(BaseName::AgentsMd.as_str(), "AGENTS.md");
/// assert_eq!(BaseName::ClaudeMd.stem(), "CLAUDE");
/// assert_eq!(serde_json::to_string(&BaseName::AgentsMd).unwrap(), "\"AGENTS.md\"");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BaseName {
    /// `AGENTS.md`.
    #[serde(rename = "AGENTS.md")]
    AgentsMd,
    /// `CLAUDE.md`.
    #[serde(rename = "CLAUDE.md")]
    ClaudeMd,
}

impl BaseName {
    /// The canonical schema literal (the file name itself).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgentsMd => "AGENTS.md",
            Self::ClaudeMd => "CLAUDE.md",
        }
    }

    /// The stem without extension (spec §2: "The stem is `AGENTS` or
    /// `CLAUDE`"). Scheme C derives `<STEM>.project.md` and
    /// `<STEM>.local.md` from it (spec D-20).
    #[must_use]
    pub const fn stem(self) -> &'static str {
        match self {
            Self::AgentsMd => "AGENTS",
            Self::ClaudeMd => "CLAUDE",
        }
    }
}

/// Where an extra file's "read this file" line goes (spec §6.2
/// `extra_local_position` / `extra_project_position`, §6.5).
///
/// The serde form is the schema CHECK literal.
///
/// # Examples
///
/// ```
/// use eunomia_core::project::ExtraFilePosition;
///
/// assert_eq!(ExtraFilePosition::Start.as_str(), "start");
/// assert_eq!(serde_json::to_string(&ExtraFilePosition::End).unwrap(), "\"end\"");
/// assert!(serde_json::from_str::<ExtraFilePosition>("\"middle\"").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExtraFilePosition {
    /// Before the managed content.
    #[serde(rename = "start")]
    Start,
    /// After the managed content.
    #[serde(rename = "end")]
    End,
}

impl ExtraFilePosition {
    /// The canonical schema literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
        }
    }
}

/// One extra-file option of a project (spec §7.5 "Scheme A", D-01):
/// whether the "read this file" line for `<STEM>.local.md` or
/// `<STEM>.project.md` is added to the main file at all, and where.
///
/// The serde shape is the §6.5 JSON object
/// `{"enabled": <bool>, "position": "start" | "end"}` — the same form
/// the settings table stores (DECISIONS.md D-039). A disabled option
/// still carries a position: the schema columns are `NOT NULL` with a
/// default, and the position is simply ignored while `enabled` is
/// false.
///
/// The options apply to scheme A only (ALG-030); under schemes B and C
/// the builder ignores them, mirroring the spec.
///
/// # Examples
///
/// ```
/// use eunomia_core::project::{ExtraFile, ExtraFilePosition};
///
/// let extra = ExtraFile { enabled: true, position: ExtraFilePosition::Start };
/// assert_eq!(
///     serde_json::to_string(&extra).unwrap(),
///     r#"{"enabled":true,"position":"start"}"#
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtraFile {
    /// Whether the line is added.
    pub enabled: bool,
    /// Where the line is added.
    pub position: ExtraFilePosition,
}

/// How a project's target file is written (spec §7.5, PRJ-012).
///
/// Every project carries its own copy of these choices (spec PRJ-012);
/// new projects are seeded from the §6.5 `default.*` settings by the
/// project-creation task — deliberately no `Default` impl lives here,
/// because the §6.5 default values have one owner: the settings
/// repository (DECISIONS.md D-039).
///
/// # Examples
///
/// ```
/// use eunomia_core::project::{
///     BaseName, ExtraFile, ExtraFilePosition, ProjectWriteSettings, Scheme,
/// };
///
/// let settings = ProjectWriteSettings {
///     scheme: Scheme::A,
///     base_name: BaseName::AgentsMd,
///     extra_local: ExtraFile { enabled: false, position: ExtraFilePosition::End },
///     extra_project: ExtraFile { enabled: false, position: ExtraFilePosition::End },
/// };
/// assert_eq!(settings.scheme.as_str(), "A");
/// assert_eq!(settings.base_name.stem(), "AGENTS");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectWriteSettings {
    /// The write scheme (§2 "Write scheme").
    pub scheme: Scheme,
    /// The target file base name (§2 "File name (base name)").
    pub base_name: BaseName,
    /// The `<STEM>.local.md` option (§7.5 Scheme A).
    pub extra_local: ExtraFile,
    /// The `<STEM>.project.md` option (§7.5 Scheme A).
    pub extra_project: ExtraFile,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_id_wraps_and_displays_the_identifier() {
        let id = ProjectId::new("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d".to_owned());
        assert_eq!(id.as_str(), "0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d");
        assert_eq!(id.to_string(), "0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d");
        assert_eq!(
            ProjectId::from("id-2".to_owned()),
            ProjectId::new("id-2".to_owned())
        );
    }

    #[test]
    fn scheme_literals_round_trip_and_reject_other_strings() {
        for (scheme, literal) in [(Scheme::A, "A"), (Scheme::B, "B"), (Scheme::C, "C")] {
            assert_eq!(scheme.as_str(), literal);
            assert_eq!(
                serde_json::to_string(&scheme).unwrap(),
                format!("\"{literal}\"")
            );
            assert_eq!(
                serde_json::from_str::<Scheme>(&format!("\"{literal}\"")).unwrap(),
                scheme
            );
        }
        for rejected in ["a", "b", " D", ""] {
            assert!(serde_json::from_str::<Scheme>(&format!("\"{rejected}\"")).is_err());
        }
    }

    #[test]
    fn base_name_literals_and_stems_round_trip() {
        for (name, literal, stem) in [
            (BaseName::AgentsMd, "AGENTS.md", "AGENTS"),
            (BaseName::ClaudeMd, "CLAUDE.md", "CLAUDE"),
        ] {
            assert_eq!(name.as_str(), literal);
            assert_eq!(name.stem(), stem);
            assert_eq!(
                serde_json::to_string(&name).unwrap(),
                format!("\"{literal}\"")
            );
            assert_eq!(
                serde_json::from_str::<BaseName>(&format!("\"{literal}\"")).unwrap(),
                name
            );
        }
        assert!(serde_json::from_str::<BaseName>("\"agents.md\"").is_err());
        assert!(serde_json::from_str::<BaseName>("\"GEMINI.md\"").is_err());
    }

    #[test]
    fn extra_file_position_literals_round_trip() {
        for (position, literal) in [
            (ExtraFilePosition::Start, "start"),
            (ExtraFilePosition::End, "end"),
        ] {
            assert_eq!(position.as_str(), literal);
            assert_eq!(
                serde_json::to_string(&position).unwrap(),
                format!("\"{literal}\"")
            );
            assert_eq!(
                serde_json::from_str::<ExtraFilePosition>(&format!("\"{literal}\"")).unwrap(),
                position
            );
        }
        assert!(serde_json::from_str::<ExtraFilePosition>("\"middle\"").is_err());
        assert!(serde_json::from_str::<ExtraFilePosition>("\"Start\"").is_err());
    }

    #[test]
    fn extra_file_serializes_to_the_settings_json_shape() {
        let extra = ExtraFile {
            enabled: false,
            position: ExtraFilePosition::End,
        };
        assert_eq!(
            serde_json::to_string(&extra).unwrap(),
            r#"{"enabled":false,"position":"end"}"#
        );
        let parsed: ExtraFile =
            serde_json::from_str(r#"{"enabled":true,"position":"start"}"#).unwrap();
        assert_eq!(
            parsed,
            ExtraFile {
                enabled: true,
                position: ExtraFilePosition::Start
            }
        );
        // Missing members, unknown members and wrong types are
        // rejected, not defaulted — as strict as the settings
        // repository's own §6.5 shape check.
        assert!(serde_json::from_str::<ExtraFile>(r#"{"enabled":true}"#).is_err());
        assert!(
            serde_json::from_str::<ExtraFile>(r#"{"enabled":"yes","position":"end"}"#).is_err()
        );
        assert!(serde_json::from_str::<ExtraFile>(r#"{"enabled":true,"position":"top"}"#).is_err());
        assert!(
            serde_json::from_str::<ExtraFile>(r#"{"enabled":true,"position":"end","note":1}"#)
                .is_err()
        );
    }

    #[test]
    fn project_write_settings_round_trips_as_one_object() {
        let settings = ProjectWriteSettings {
            scheme: Scheme::B,
            base_name: BaseName::ClaudeMd,
            extra_local: ExtraFile {
                enabled: true,
                position: ExtraFilePosition::Start,
            },
            extra_project: ExtraFile {
                enabled: false,
                position: ExtraFilePosition::End,
            },
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<ProjectWriteSettings>(&json).unwrap(),
            settings
        );
        assert!(json.contains(r#""scheme":"B""#));
    }
}
