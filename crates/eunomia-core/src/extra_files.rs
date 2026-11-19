//! The extra-file reading lines of scheme A (spec §7.5 "Scheme A",
//! decisions D-01 and D-21; OUT-A-001…004).
//!
//! A scheme-A project may add two "read this file" lines to its main
//! file — one for `<STEM>.project.md`, one for `<STEM>.local.md` —
//! each with an "Include" flag and a Start or End position (the
//! [`ExtraFile`] pair of
//! [`ProjectWriteSettings`](crate::project::ProjectWriteSettings)).
//! The app only ever writes these lines: it does not create, read, or
//! change the extra files themselves (spec D-01, closing OUT-A-004).
//!
//! The line text is the D-21 sentence verbatim, with `<file>` bound to
//! the extra file's name:
//!
//! ```text
//! If AGENTS.project.md exists in the repository root, read it before starting work and follow its instructions.
//! ```
//!
//! [`extra_file_lines`] splits the enabled lines by position. When
//! both lines share a position, the `.project.md` line goes first and
//! the `.local.md` line follows — local rules are read last and take
//! priority (§7.5).
//!
//! Boundary: this module produces the lines and nothing else. The
//! scheme-A skeleton that places them — ALG-030's
//! `managed = join(parts, BLANK_LINE)` over the start lines, the body
//! ([`build_body`](crate::body::build_body), if not empty) and the end
//! lines, plus the creation marker — belongs to the by-scheme builder,
//! and schemes B and C never add these lines (ALG-030: "A only"), so
//! their builders do not call this function. The §7.5 hint about
//! `.local.md` and Git is a settings-UI concern, and the scheme-C
//! snippet reuses the same D-21 sentence without going through the
//! build.
//!
//! # Examples
//!
//! ```
//! use eunomia_core::extra_files::extra_file_lines;
//! use eunomia_core::project::{BaseName, ExtraFile, ExtraFilePosition};
//!
//! // Both extra files enabled at the same position: `.project.md`
//! // first, then `.local.md` (§7.5).
//! let lines = extra_file_lines(
//!     BaseName::AgentsMd,
//!     ExtraFile { enabled: true, position: ExtraFilePosition::Start },
//!     ExtraFile { enabled: true, position: ExtraFilePosition::Start },
//! );
//! assert_eq!(lines.start.len(), 2);
//! assert!(lines.end.is_empty());
//! assert!(lines.start[0].starts_with("If AGENTS.project.md exists"));
//! assert!(lines.start[1].starts_with("If AGENTS.local.md exists"));
//!
//! // Nothing enabled: no lines at all.
//! let off = extra_file_lines(
//!     BaseName::ClaudeMd,
//!     ExtraFile { enabled: false, position: ExtraFilePosition::End },
//!     ExtraFile { enabled: false, position: ExtraFilePosition::End },
//! );
//! assert!(off.is_empty());
//! ```

use crate::project::{BaseName, ExtraFile, ExtraFilePosition};

/// The extra-file reading lines of one project, split by position
/// (spec §7.5 "Scheme A"; consumed by ALG-030).
///
/// `start` holds the enabled lines with position `start`, `end` the
/// enabled ones with position `end`; each list keeps the §7.5 order —
/// `.project.md` before `.local.md`. The scheme-A builder splices them
/// exactly as ALG-030 reads: the parts are the `start` lines, the body
/// (if not empty), the `end` lines, joined with one empty line between
/// neighbours.
///
/// A pair of empty lists is a legitimate value — a scheme-A project
/// with both options disabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraFileLines {
    /// The enabled lines with position `start`, `.project.md` before
    /// `.local.md` (§7.5).
    pub start: Vec<String>,
    /// The enabled lines with position `end`, `.project.md` before
    /// `.local.md` (§7.5).
    pub end: Vec<String>,
}

impl ExtraFileLines {
    /// Whether no line is enabled at either position.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start.is_empty() && self.end.is_empty()
    }
}

/// Builds a project's extra-file reading lines (spec §7.5 "Scheme A",
/// D-01, D-21).
///
/// Each enabled option contributes one line — the D-21 sentence with
/// `<file>` bound to `<STEM>.project.md` or `<STEM>.local.md`, the
/// stem from `base_name` — placed in [`ExtraFileLines::start`] or
/// [`ExtraFileLines::end`] by the option's position. A disabled option
/// contributes nothing: its position is ignored while `enabled` is
/// false (see
/// [`ExtraFile`]). When both options are
/// enabled in the same position, the `.project.md` line goes first and
/// the `.local.md` line follows — local rules are read last and take
/// priority (§7.5).
///
/// The function reads no scheme: the reading lines exist on scheme A
/// only (ALG-030), and the by-scheme builder makes that choice — this
/// pure shaping of the two options stays scheme-blind on purpose.
///
/// The argument order is project-then-local — the §7.5 tie order, and
/// the reverse of `ProjectWriteSettings`' field order (`extra_local`
/// is declared first). Both arguments share a type, so keep the order
/// straight at the call site; the oracle tests here catch an internal
/// swap, not a swapped call.
///
/// # Examples
///
/// ```
/// use eunomia_core::extra_files::extra_file_lines;
/// use eunomia_core::project::{BaseName, ExtraFile, ExtraFilePosition};
///
/// let lines = extra_file_lines(
///     BaseName::AgentsMd,
///     ExtraFile { enabled: true, position: ExtraFilePosition::Start },
///     ExtraFile { enabled: true, position: ExtraFilePosition::End },
/// );
/// assert_eq!(
///     lines.start,
///     ["If AGENTS.project.md exists in the repository root, read it before starting work and follow its instructions."]
/// );
/// assert_eq!(
///     lines.end,
///     ["If AGENTS.local.md exists in the repository root, read it before starting work and follow its instructions."]
/// );
/// ```
#[must_use]
pub fn extra_file_lines(
    base_name: BaseName,
    extra_project: ExtraFile,
    extra_local: ExtraFile,
) -> ExtraFileLines {
    let mut lines = ExtraFileLines {
        start: Vec::new(),
        end: Vec::new(),
    };

    // §7.5: in a shared position `.project.md` goes before
    // `.local.md`, so the project pair is visited first and each list
    // fills in that order.
    for (extra, suffix) in [(extra_project, ".project.md"), (extra_local, ".local.md")] {
        if !extra.enabled {
            continue;
        }
        let file_name = format!("{}{suffix}", base_name.stem());
        match extra.position {
            ExtraFilePosition::Start => lines.start.push(reading_line(&file_name)),
            ExtraFilePosition::End => lines.end.push(reading_line(&file_name)),
        }
    }

    lines
}

/// The D-21 "read this file" sentence with `<file>` bound:
/// "If <file> exists in the repository root, read it before starting
/// work and follow its instructions."
fn reading_line(file_name: &str) -> String {
    format!(
        "If {file_name} exists in the repository root, read it before starting work and follow its instructions."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The D-21 oracle literals: the spec sentence verbatim with each
    // file name the settings can produce (§7.5: `<STEM>.project.md`
    // and `<STEM>.local.md`, STEM = AGENTS | CLAUDE). Nothing in the
    // tests rebuilds them from a template.
    const AGENTS_PROJECT_LINE: &str = "If AGENTS.project.md exists in the repository root, read it before starting work and follow its instructions.";
    const AGENTS_LOCAL_LINE: &str = "If AGENTS.local.md exists in the repository root, read it before starting work and follow its instructions.";
    const CLAUDE_PROJECT_LINE: &str = "If CLAUDE.project.md exists in the repository root, read it before starting work and follow its instructions.";
    const CLAUDE_LOCAL_LINE: &str = "If CLAUDE.local.md exists in the repository root, read it before starting work and follow its instructions.";

    fn extra(enabled: bool, position: ExtraFilePosition) -> ExtraFile {
        ExtraFile { enabled, position }
    }

    // ---- the D-21 oracle --------------------------------------------------

    #[test]
    fn the_line_is_the_d_21_sentence_with_the_file_name_bound() {
        // One option enabled at a time; the single line produced must
        // equal the spec literal byte for byte, for all four names.
        let agents_project = extra_file_lines(
            BaseName::AgentsMd,
            extra(true, ExtraFilePosition::Start),
            extra(false, ExtraFilePosition::End),
        );
        assert_eq!(agents_project.start, [AGENTS_PROJECT_LINE]);
        assert!(agents_project.end.is_empty());

        let agents_local = extra_file_lines(
            BaseName::AgentsMd,
            extra(false, ExtraFilePosition::Start),
            extra(true, ExtraFilePosition::End),
        );
        assert!(agents_local.start.is_empty());
        assert_eq!(agents_local.end, [AGENTS_LOCAL_LINE]);

        let claude_project = extra_file_lines(
            BaseName::ClaudeMd,
            extra(true, ExtraFilePosition::End),
            extra(false, ExtraFilePosition::Start),
        );
        assert_eq!(claude_project.end, [CLAUDE_PROJECT_LINE]);
        assert!(claude_project.start.is_empty());

        let claude_local = extra_file_lines(
            BaseName::ClaudeMd,
            extra(false, ExtraFilePosition::End),
            extra(true, ExtraFilePosition::Start),
        );
        assert!(claude_local.end.is_empty());
        assert_eq!(claude_local.start, [CLAUDE_LOCAL_LINE]);
    }

    // ---- §7.5 placement -----------------------------------------------------

    #[test]
    fn the_same_position_puts_project_before_local() {
        // §7.5 verbatim: "If both lines are in the same position, the
        // line for `.project.md` goes first, then the one for
        // `.local.md` (local rules are read last and take priority)."
        // Both ties, both stems.
        let agents_both_start = extra_file_lines(
            BaseName::AgentsMd,
            extra(true, ExtraFilePosition::Start),
            extra(true, ExtraFilePosition::Start),
        );
        assert_eq!(
            agents_both_start.start,
            [AGENTS_PROJECT_LINE, AGENTS_LOCAL_LINE]
        );
        assert!(agents_both_start.end.is_empty());

        let claude_both_end = extra_file_lines(
            BaseName::ClaudeMd,
            extra(true, ExtraFilePosition::End),
            extra(true, ExtraFilePosition::End),
        );
        assert!(claude_both_end.start.is_empty());
        assert_eq!(
            claude_both_end.end,
            [CLAUDE_PROJECT_LINE, CLAUDE_LOCAL_LINE]
        );
    }

    #[test]
    fn different_positions_put_each_line_at_its_own_position() {
        // project → start, local → end, and the mirror. The body the
        // scheme-A builder splices between the two lists is out of
        // scope here (ALG-030, a later task).
        let split = extra_file_lines(
            BaseName::AgentsMd,
            extra(true, ExtraFilePosition::Start),
            extra(true, ExtraFilePosition::End),
        );
        assert_eq!(split.start, [AGENTS_PROJECT_LINE]);
        assert_eq!(split.end, [AGENTS_LOCAL_LINE]);

        let mirrored = extra_file_lines(
            BaseName::ClaudeMd,
            extra(true, ExtraFilePosition::End),
            extra(true, ExtraFilePosition::Start),
        );
        assert_eq!(mirrored.start, [CLAUDE_LOCAL_LINE]);
        assert_eq!(mirrored.end, [CLAUDE_PROJECT_LINE]);
    }

    #[test]
    fn a_disabled_option_adds_no_line_whatever_its_position() {
        // Both off: no lines, even though both options carry a position
        // (the schema columns are NOT NULL; the position is ignored
        // while enabled is false — see project::ExtraFile).
        let none = extra_file_lines(
            BaseName::AgentsMd,
            extra(false, ExtraFilePosition::Start),
            extra(false, ExtraFilePosition::End),
        );
        assert!(none.is_empty());

        // A disabled option leaves no trace beside an enabled partner,
        // in either pairing and either position.
        let only_local = extra_file_lines(
            BaseName::AgentsMd,
            extra(false, ExtraFilePosition::Start),
            extra(true, ExtraFilePosition::Start),
        );
        assert_eq!(only_local.start, [AGENTS_LOCAL_LINE]);
        assert!(only_local.end.is_empty());

        let only_project = extra_file_lines(
            BaseName::ClaudeMd,
            extra(true, ExtraFilePosition::End),
            extra(false, ExtraFilePosition::Start),
        );
        assert!(only_project.start.is_empty());
        assert_eq!(only_project.end, [CLAUDE_PROJECT_LINE]);
    }

    // ---- the full matrix ------------------------------------------------------

    #[test]
    fn every_flag_and_position_combination_for_both_stems_matches_the_spec() {
        // The card's acceptance matrix, exhaustive: enabled/disabled ×2
        // and start/end ×2 for both stems — 16 combinations per stem,
        // 32 cases. The expectation restates §7.5 by hand from the
        // oracle literals: an enabled line lands at its position, a
        // disabled one nowhere, and inside one position `.project.md`
        // precedes `.local.md`.
        let positions = [ExtraFilePosition::Start, ExtraFilePosition::End];
        for (base, project_line, local_line) in [
            (BaseName::AgentsMd, AGENTS_PROJECT_LINE, AGENTS_LOCAL_LINE),
            (BaseName::ClaudeMd, CLAUDE_PROJECT_LINE, CLAUDE_LOCAL_LINE),
        ] {
            for &project_enabled in &[false, true] {
                for &project_position in &positions {
                    for &local_enabled in &[false, true] {
                        for &local_position in &positions {
                            let lines = extra_file_lines(
                                base,
                                extra(project_enabled, project_position),
                                extra(local_enabled, local_position),
                            );

                            let (mut want_start, mut want_end): (Vec<&str>, Vec<&str>) =
                                (Vec::new(), Vec::new());
                            if project_enabled {
                                match project_position {
                                    ExtraFilePosition::Start => want_start.push(project_line),
                                    ExtraFilePosition::End => want_end.push(project_line),
                                }
                            }
                            if local_enabled {
                                match local_position {
                                    ExtraFilePosition::Start => want_start.push(local_line),
                                    ExtraFilePosition::End => want_end.push(local_line),
                                }
                            }

                            let case = format!(
                                "{base:?} project={}({project_position:?}) local={}({local_position:?})",
                                project_enabled, local_enabled
                            );
                            assert_eq!(lines.start, want_start, "start lines: {case}");
                            assert_eq!(lines.end, want_end, "end lines: {case}");
                            assert_eq!(
                                lines.is_empty(),
                                want_start.is_empty() && want_end.is_empty(),
                                "is_empty: {case}"
                            );
                        }
                    }
                }
            }
        }
    }
}
