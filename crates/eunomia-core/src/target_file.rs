//! The content of the target file, built whole for the
//! full-overwrite schemes A and C (spec §9.3, ALG-030; decisions
//! D-20, D-22; OUT-C-001…004).
//!
//! [`build_content`] is the last pure step of a build: it joins the
//! three pieces earlier tasks own — the §9.2 body
//! ([`body::build_body`](crate::body::build_body)), the scheme-A
//! extra-file reading lines
//! ([`extra_files::extra_file_lines`](crate::extra_files::extra_file_lines))
//! and the creation-marker line
//! ([`marker::creation_marker_line`](crate::marker::creation_marker_line))
//! — into the exact content a sync writes to the target file of a
//! scheme-A or scheme-C project (ALG-030, verbatim):
//!
//! ```text
//! managed = join(parts, BLANK_LINE)   // §9.2's BLANK_LINE = one empty line
//! file    = managed, an empty line, the marker, one line break
//! ```
//!
//! The parts, in ALG-030's order:
//!
//! - the reading lines with position `start` — scheme A only;
//! - the body, when it is not empty;
//! - the reading lines with position `end` — scheme A only.
//!
//! One position's lines are ONE part — adjacent lines of a single
//! block, `.project.md` before `.local.md` (§7.5) — and only
//! neighbouring parts are separated by an empty line. A part that
//! does not exist (no lines at the position, an empty body) leaves no
//! trace; in particular an empty body never leaves an empty block
//! between the start and end lines.
//!
//! When no part exists, the managed block is empty and "the file has
//! only the marker" (ALG-030): the marker line with its one
//! terminating line break, and nothing before it — no managed block,
//! no leading empty line. Every file this function builds ends with
//! exactly one line break.
//!
//! # The scheme gate is here
//!
//! [`extra_files::extra_file_lines`](crate::extra_files::extra_file_lines)
//! is deliberately scheme-blind (DECISIONS.md D-063 left the gate to
//! this builder): scheme A splices the reading lines in, scheme C
//! ignores them even when every option is enabled (ALG-030's "(A
//! only)"; D-20 — the app does not edit the main file of a scheme-C
//! project, and the §7.5 snippet its settings show is UI text, never
//! file content). With no reading lines, schemes A and C build the
//! byte-identical file: the schemes differ in the target name
//! (`AGENTS.md`/`CLAUDE.md` vs `<STEM>.project.md`), not in the
//! content.
//!
//! # Encoding, line breaks, no clock
//!
//! D-22: UTF-8 without BOM, LF. The mechanics: every separator this
//! function adds is `\n`, and a Rust [`String`] is UTF-8 — no BOM is
//! ever prepended. No text is normalized, re-wrapped or trimmed here:
//! the body arrived ALG-002-normalized from
//! [`build_body`](crate::body::build_body), the reading lines are the
//! D-21 sentences, and the marker line is asserted single-line. The
//! marker itself is an input, not formatted here — ALG-032 takes the
//! time "once per run, at the start of the write phase", which is the
//! sync pipeline's clock, never a builder's; the caller formats the
//! line once per run and passes it to every project.
//!
//! # Boundary
//!
//! Scheme B builds no whole file: ALG-031 splices the managed block
//! and the marker into the existing marked file, a different builder
//! with different inputs (the existing bytes and the D-19 markers),
//! so `Scheme::B` panics here as a caller bug. Target naming and
//! resolution (`<root>/<base_name>` or `<root>/<STEM>.project.md`),
//! the atomic write (§9.5), the build hash and the marker-ignoring
//! comparison (ALG-033) and the D-25 empty-build confirmation are all
//! pipeline steps around this function, none of them its business.
//!
//! # Examples
//!
//! ```
//! use eunomia_core::extra_files::ExtraFileLines;
//! use eunomia_core::project::Scheme;
//! use eunomia_core::target_file::build_content;
//!
//! const MARKER: &str = "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";
//! let no_lines = ExtraFileLines { start: Vec::new(), end: Vec::new() };
//!
//! // No reading lines: schemes A and C build the identical file…
//! let a = build_content(Scheme::A, "Be brief.", &no_lines, MARKER);
//! let c = build_content(Scheme::C, "Be brief.", &no_lines, MARKER);
//! assert_eq!(a, c);
//! assert_eq!(a, "Be brief.\n\n<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n");
//!
//! // …and an empty managed block writes only the marker line.
//! let empty = build_content(Scheme::C, "", &no_lines, MARKER);
//! assert_eq!(empty, "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n");
//! ```

use crate::extra_files::ExtraFileLines;
use crate::project::Scheme;

/// The separator between two neighbouring parts of the managed block
/// (spec §9.2: `BLANK_LINE` = two line breaks = exactly one empty
/// line — the same separator [`body`](crate::body) joins its blocks
/// with; ALG-030 joins the parts with it verbatim).
const BLANK_LINE: &str = "\n\n";

/// Builds the whole target-file content of a scheme-A or scheme-C
/// project (spec §9.3, ALG-030).
///
/// `body` is the project's §9.2 body ([`build_body`](crate::body::build_body)
/// output; an empty string is a legitimate empty build);
/// `extra_lines` the scheme-A reading lines
/// ([`extra_file_lines`](crate::extra_files::extra_file_lines)
/// output — spliced in under scheme A only, ignored under scheme C
/// even when non-empty); `marker_line` the ready creation-marker line
/// ([`creation_marker_line`](crate::marker::creation_marker_line)
/// output — used verbatim, never formatted here). The result is the
/// exact file content, D-22 discipline throughout: UTF-8 without
/// BOM, LF line breaks, ending with exactly one line break.
///
/// The managed block is the non-empty parts of ALG-030 joined with
/// one empty line between neighbours; the file is the managed block,
/// an empty line, the marker line, one line break. When the managed
/// block is empty the file is only the marker line with its
/// terminating line break (ALG-030: "the file has only the marker").
///
/// No input text is touched: the body and the reading lines enter
/// verbatim (their normalization happened upstream in
/// [`normalize`](crate::normalize) and the D-21 line shaping), and
/// only `\n` separators are added. Two byte-level contracts are
/// asserted — the marker line being a single non-empty line (it must
/// occupy exactly one line of the file for the ALG-033 scanner to
/// find later) and the body carrying no framing line break of its
/// own (a leading or trailing `\n` would turn ALG-030's one empty
/// line into two).
///
/// # Panics
///
/// Panics on `Scheme::B` — scheme B builds no whole file (ALG-031
/// splices into the existing marked file; a different builder), so a
/// B here is a caller bug. Panics when `marker_line` is empty or
/// contains a line break, or when `body` starts or ends with a line
/// break: the crate's broken-invariant posture (DECISIONS.md D-061)
/// — the marker arrives from
/// [`creation_marker_line`](crate::marker::creation_marker_line) and
/// the body from [`build_body`](crate::body::build_body), which
/// guarantee those shapes, so each marks a miswired caller, not a
/// data condition.
///
/// # Examples
///
/// ```
/// use eunomia_core::extra_files::extra_file_lines;
/// use eunomia_core::project::{BaseName, ExtraFile, ExtraFilePosition, Scheme};
/// use eunomia_core::target_file::build_content;
///
/// const MARKER: &str = "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";
///
/// // AGENTS.md with `.project.md` at start and `.local.md` at end.
/// let extra = extra_file_lines(
///     BaseName::AgentsMd,
///     ExtraFile { enabled: true, position: ExtraFilePosition::Start },
///     ExtraFile { enabled: true, position: ExtraFilePosition::End },
/// );
/// let file = build_content(Scheme::A, "Be brief.", &extra, MARKER);
/// assert_eq!(
///     file,
///     "If AGENTS.project.md exists in the repository root, read it before starting work and follow its instructions.\n\
///      \n\
///      Be brief.\n\
///      \n\
///      If AGENTS.local.md exists in the repository root, read it before starting work and follow its instructions.\n\
///      \n\
///      <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n"
/// );
///
/// // Scheme C ignores the same enabled lines (ALG-030: "A only").
/// let c = build_content(Scheme::C, "Be brief.", &extra, MARKER);
/// assert_eq!(c, "Be brief.\n\n<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n");
/// ```
#[must_use]
pub fn build_content(
    scheme: Scheme,
    body: &str,
    extra_lines: &ExtraFileLines,
    marker_line: &str,
) -> String {
    assert!(!marker_line.is_empty(), "the marker line must not be empty");
    assert!(
        !marker_line.contains('\n') && !marker_line.contains('\r'),
        "the marker line must be a single line"
    );
    assert!(
        !body.starts_with('\n') && !body.ends_with('\n'),
        "the body must arrive without framing line breaks — build_body shapes it so"
    );

    let managed = managed_block(scheme, body, extra_lines);
    if managed.is_empty() {
        format!("{marker_line}\n")
    } else {
        format!("{managed}{BLANK_LINE}{marker_line}\n")
    }
}

/// The managed block of ALG-030: the existing parts of the scheme,
/// joined with one empty line between neighbours, before the marker
/// tail is attached.
///
/// The `match` is exhaustive on purpose: the scheme decides the parts
/// (ALG-030's "(A only)" gate — scheme A splices the reading-line
/// parts around the body, scheme C takes the body alone), scheme B is
/// ALG-031's business, and a scheme variant added to
/// [`Scheme`](crate::project::Scheme) later must be decided HERE, at
/// compile time, not silently folded into scheme C's shape. The
/// scheme-B arm carries the documented caller-bug panic of
/// [`build_content`]'s `# Panics` contract.
fn managed_block(scheme: Scheme, body: &str, extra_lines: &ExtraFileLines) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(3);
    match scheme {
        Scheme::A => {
            push_lines_part(&mut parts, &extra_lines.start);
            if !body.is_empty() {
                parts.push(body.to_owned());
            }
            push_lines_part(&mut parts, &extra_lines.end);
        }
        Scheme::C => {
            if !body.is_empty() {
                parts.push(body.to_owned());
            }
        }
        Scheme::B => panic!(
            "scheme B builds no whole file — ALG-031 splices the managed block into the existing marked file"
        ),
    }
    parts.join(BLANK_LINE)
}

/// Adds one reading-lines part: all lines of one position joined into
/// a single block of adjacent lines — ALG-030 counts the position's
/// lines as ONE part, so only the block's neighbours get the empty
/// line. A position with no lines adds no part at all.
fn push_lines_part(parts: &mut Vec<String>, lines: &[String]) {
    if !lines.is_empty() {
        parts.push(lines.join("\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extra_files::extra_file_lines;
    use crate::project::{BaseName, ExtraFile, ExtraFilePosition};

    // The oracle literals, hardcoded from the spec and independent of
    // the code under test: the ALG-032 / D-23 example marker line,
    // and the D-21 reading lines for the AGENTS stem.
    const MARKER: &str = "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";
    const PROJECT_LINE: &str = "If AGENTS.project.md exists in the repository root, read it before starting work and follow its instructions.";
    const LOCAL_LINE: &str = "If AGENTS.local.md exists in the repository root, read it before starting work and follow its instructions.";

    fn lines(start: &[&str], end: &[&str]) -> ExtraFileLines {
        ExtraFileLines {
            start: start.iter().map(|line| (*line).to_owned()).collect(),
            end: end.iter().map(|line| (*line).to_owned()).collect(),
        }
    }

    fn none_enabled() -> ExtraFileLines {
        lines(&[], &[])
    }

    fn file(scheme: Scheme, body: &str, extra: &ExtraFileLines) -> String {
        build_content(scheme, body, extra, MARKER)
    }

    // Every option enabled, split across the positions — the shape
    // the sync pipeline hands over for a fully configured project.
    fn fully_enabled() -> ExtraFileLines {
        extra_file_lines(
            BaseName::AgentsMd,
            ExtraFile {
                enabled: true,
                position: ExtraFilePosition::Start,
            },
            ExtraFile {
                enabled: true,
                position: ExtraFilePosition::End,
            },
        )
    }

    // ---- the ALG-030 oracles: whole files, byte for byte ----------

    #[test]
    fn scheme_a_with_start_and_end_lines_builds_the_alg_030_file() {
        // The full ALG-030 composition: start part, body part, end
        // part — one empty line between neighbours, then an empty
        // line, the marker, one line break. Expected string hardcoded
        // from the spec's own D-21 / ALG-032 literals.
        let extra = lines(&[PROJECT_LINE], &[LOCAL_LINE]);
        assert_eq!(
            file(Scheme::A, "Body text.", &extra),
            "If AGENTS.project.md exists in the repository root, read it before starting work and follow its instructions.\n\
             \n\
             Body text.\n\
             \n\
             If AGENTS.local.md exists in the repository root, read it before starting work and follow its instructions.\n\
             \n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n"
        );
    }

    #[test]
    fn scheme_c_builds_the_body_marker_file_of_out_c() {
        // Scheme C: the same file minus the reading lines (OUT-C per
        // D-20 — content goes to `<STEM>.project.md`, fully
        // overwritten; no reading lines ever).
        assert_eq!(
            file(Scheme::C, "## Style\n\nBe brief.", &none_enabled()),
            "## Style\n\nBe brief.\n\
             \n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n"
        );
    }

    #[test]
    fn one_positions_lines_are_one_part_of_adjacent_lines() {
        // Both reading lines at start: they stay adjacent lines of
        // ONE block (`.project.md` before `.local.md`, §7.5), and
        // only the block's neighbours get the empty line. The mirror
        // case (both at end) is the same shape at the other end.
        let both_start = lines(&[PROJECT_LINE, LOCAL_LINE], &[]);
        assert_eq!(
            file(Scheme::A, "Body text.", &both_start),
            "If AGENTS.project.md exists in the repository root, read it before starting work and follow its instructions.\n\
             If AGENTS.local.md exists in the repository root, read it before starting work and follow its instructions.\n\
             \n\
             Body text.\n\
             \n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n"
        );

        let both_end = lines(&[], &[PROJECT_LINE, LOCAL_LINE]);
        assert_eq!(
            file(Scheme::A, "Body text.", &both_end),
            "Body text.\n\
             \n\
             If AGENTS.project.md exists in the repository root, read it before starting work and follow its instructions.\n\
             If AGENTS.local.md exists in the repository root, read it before starting work and follow its instructions.\n\
             \n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n"
        );
    }

    #[test]
    fn every_reading_position_combination_matches_the_oracle() {
        // The card's matrix: start-only / end-only / both / neither
        // around a non-empty body, scheme A. Expectations restated by
        // hand from the literals — each existing part in ALG-030's
        // order, one empty line between neighbours.
        let body = "Body text.";
        let body_tail = format!("{body}\n\n{MARKER}\n");

        // Neither position enabled.
        assert_eq!(file(Scheme::A, body, &none_enabled()), body_tail);

        // Start only.
        assert_eq!(
            file(Scheme::A, body, &lines(&[PROJECT_LINE], &[])),
            format!("{PROJECT_LINE}\n\n{body_tail}")
        );

        // End only.
        assert_eq!(
            file(Scheme::A, body, &lines(&[], &[LOCAL_LINE])),
            format!("{body}\n\n{LOCAL_LINE}\n\n{MARKER}\n")
        );

        // Both (split positions) — the full oracle of the first test.
        assert_eq!(
            file(Scheme::A, body, &lines(&[PROJECT_LINE], &[LOCAL_LINE])),
            format!("{PROJECT_LINE}\n\n{body}\n\n{LOCAL_LINE}\n\n{MARKER}\n")
        );
    }

    // ---- the empty body ------------------------------------------------

    #[test]
    fn an_empty_body_leaves_no_empty_block_between_the_lines() {
        // "the body (if not empty)" — an empty body part simply does
        // not exist: start and end parts become neighbours.
        let split = lines(&[PROJECT_LINE], &[LOCAL_LINE]);
        assert_eq!(
            file(Scheme::A, "", &split),
            format!("{PROJECT_LINE}\n\n{LOCAL_LINE}\n\n{MARKER}\n")
        );

        // A lone part needs no separator at all.
        assert_eq!(
            file(Scheme::A, "", &lines(&[PROJECT_LINE], &[])),
            format!("{PROJECT_LINE}\n\n{MARKER}\n")
        );
        assert_eq!(
            file(Scheme::A, "", &lines(&[], &[LOCAL_LINE])),
            format!("{LOCAL_LINE}\n\n{MARKER}\n")
        );

        // A body of only whitespace is NOT empty — it is spliced
        // verbatim (normalization is upstream, never here).
        assert_eq!(
            file(Scheme::A, " ", &none_enabled()),
            format!(" \n\n{MARKER}\n")
        );
    }

    #[test]
    fn an_empty_managed_block_writes_only_the_marker_line() {
        // ALG-030: "If `managed` is empty, the file has only the
        // marker." Both full-overwrite schemes; the marker line with
        // its one terminating line break and nothing before it — no
        // leading empty line. Under scheme C the gate and the empty
        // branch meet: even a fully enabled line pair changes
        // nothing.
        let only_marker = format!("{MARKER}\n");
        assert_eq!(file(Scheme::A, "", &none_enabled()), only_marker);
        assert_eq!(file(Scheme::C, "", &none_enabled()), only_marker);
        assert_eq!(
            build_content(Scheme::C, "", &fully_enabled(), MARKER),
            only_marker
        );

        let content = file(Scheme::A, "", &none_enabled());
        assert!(content.starts_with("<!--"));
        assert!(!content.starts_with('\n'));
    }

    // ---- the scheme gate (ALG-030 "A only"; the card's criterion) ----

    #[test]
    fn scheme_c_ignores_even_fully_enabled_reading_lines() {
        // The gate lives here (D-063): the lines reach the builder
        // with every option enabled, and scheme C drops them all.
        // Built through the real extra_file_lines, exactly as the
        // sync pipeline would hand them over.
        let enabled = fully_enabled();
        assert!(!enabled.is_empty());

        let with_lines = build_content(Scheme::C, "Body text.", &enabled, MARKER);
        assert_eq!(
            with_lines,
            "Body text.\n\
             \n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n"
        );
        // The same lines under scheme A are spliced — the gate is the
        // scheme, not the caller pre-filtering.
        let a = build_content(Scheme::A, "Body text.", &enabled, MARKER);
        assert_eq!(
            a,
            format!("{PROJECT_LINE}\n\nBody text.\n\n{LOCAL_LINE}\n\n{MARKER}\n")
        );
        assert_ne!(a, with_lines);
    }

    #[test]
    fn without_reading_lines_schemes_a_and_c_build_the_identical_file() {
        // The managed block of A and C is identical (ALG-030 builds
        // both the same way); the schemes differ only in the target
        // file name, which is not this function's concern.
        for body in ["", "Body text.", "One.\n\nTwo."] {
            assert_eq!(
                file(Scheme::A, body, &none_enabled()),
                file(Scheme::C, body, &none_enabled()),
                "bodies must build identically: {body:?}"
            );
        }
    }

    // ---- D-22: encoding and line breaks ----------------------------------

    #[test]
    fn every_file_ends_with_exactly_one_line_break() {
        // "…the marker, one line break" — one final \n after the
        // marker line, never two, never zero.
        let cases = [
            file(Scheme::A, "Body text.", &none_enabled()),
            file(Scheme::A, "", &none_enabled()),
            file(
                Scheme::A,
                "",
                &lines(&[PROJECT_LINE, LOCAL_LINE], &[PROJECT_LINE, LOCAL_LINE]),
            ),
            file(Scheme::C, "Body text.", &none_enabled()),
            file(Scheme::C, "", &none_enabled()),
        ];
        for content in cases {
            assert!(content.ends_with('\n'), "no final line break: {content:?}");
            assert!(
                !content.ends_with("\n\n"),
                "more than one final line break: {content:?}"
            );
        }
    }

    #[test]
    fn no_bom_is_ever_prepended() {
        // D-22: UTF-8 without BOM. The first byte is never EF BB BF.
        for content in [
            file(Scheme::A, "Body text.", &none_enabled()),
            file(Scheme::A, "", &none_enabled()),
            file(Scheme::C, "", &none_enabled()),
        ] {
            assert!(!content.starts_with('\u{feff}'));
            assert_ne!(&content.as_bytes()[..3], &[0xEF, 0xBB, 0xBF]);
        }
    }

    #[test]
    fn the_body_is_spliced_verbatim_with_no_normalization() {
        // A multi-line body with inner blank lines and trailing
        // spaces passes through byte for byte; this function adds
        // only the \n separators and the marker tail. Normalization
        // is ALG-002's, upstream in build_body.
        let body = "Line one.\n\nLine two.  \nTail";
        assert_eq!(
            file(Scheme::C, body, &none_enabled()),
            "Line one.\n\nLine two.  \nTail\n\
             \n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n"
        );
    }

    // ---- the marker is an input ------------------------------------------

    #[test]
    fn the_marker_line_is_used_verbatim_and_never_formatted() {
        // No clock, no formatting: whatever line arrives is the tail.
        // A marker that is not the ALG-032 shape still passes through
        // untouched — this function neither knows nor checks the
        // comment's grammar, only its single-line shape.
        let placeholder = "<!-- MARK -->";
        assert_eq!(
            build_content(Scheme::A, "Body.", &none_enabled(), placeholder),
            "Body.\n\n<!-- MARK -->\n"
        );
    }

    // ---- broken invariants panic -----------------------------------------

    #[test]
    #[should_panic(expected = "scheme B builds no whole file")]
    fn scheme_b_is_rejected_as_a_caller_bug() {
        // ALG-031's builder splices into an existing marked file;
        // there is no whole scheme-B file to build here.
        let _ = file(Scheme::B, "Body text.", &none_enabled());
    }

    #[test]
    #[should_panic(expected = "the marker line must not be empty")]
    fn an_empty_marker_line_panics() {
        let _ = build_content(Scheme::A, "Body.", &none_enabled(), "");
    }

    #[test]
    #[should_panic(expected = "the marker line must be a single line")]
    fn a_marker_line_with_a_line_break_panics() {
        let _ = build_content(Scheme::A, "Body.", &none_enabled(), "<!-- a\nb -->");
    }

    #[test]
    #[should_panic(expected = "the marker line must be a single line")]
    fn a_marker_line_with_a_carriage_return_panics() {
        let _ = build_content(Scheme::A, "Body.", &none_enabled(), "<!-- a\rb -->");
    }

    #[test]
    #[should_panic(expected = "the body must arrive without framing line breaks")]
    fn a_body_with_a_framing_line_break_panics() {
        // A trailing \n would turn ALG-030's one empty line before
        // the marker into two; build_body never emits one.
        let _ = file(Scheme::A, "Trailing break.\n", &none_enabled());
    }
}
