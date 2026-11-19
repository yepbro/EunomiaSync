//! The scheme-B file assembly (spec §7.5 "Scheme B", §9.3 ALG-031;
//! D-22, OUT-B-001…006; §15.2 T-F12).
//!
//! Scheme B writes into a marked area of a file the user has fenced
//! by hand with the D-19 markers. [`assemble`] is the whole step for
//! a target file's text: it finds the one marker pair (through
//! [`markers::parse`], T-040, D-062), builds the new managed block,
//! and returns the file's new content with that block spliced
//! **between the marker lines**:
//!
//! ```text
//! … everything before …
//! <!-- EUNOMIASYNC:BEGIN -->
//! <the new managed block>
//! <!-- EUNOMIASYNC:END -->
//! … everything after …
//! ```
//!
//! Everything outside the marker lines — the marker lines themselves,
//! a BOM at byte 0, the file's own line breaks, blank lines, the text
//! before BEGIN and after END — survives **byte for byte** (§7.5
//! OUT-B, D-22; T-F12's acceptance criterion). The splice is exactly
//! the one the marker parser's offsets exist for (D-062):
//!
//! ```text
//! content[..span.managed_start] + block + content[span.managed_end..]
//! ```
//!
//! so no outside byte is ever re-encoded, re-wrapped or normalized —
//! and the result is a [`String`], losslessly: every piece of the
//! splice is UTF-8 text, and a BOM is the character `\u{feff}`, which
//! a `String` carries unchanged. The caller writes the string's bytes
//! to disk as they are.
//!
//! # The block's layout: ALG-030's rules, carried into B by ALG-031
//!
//! ALG-031 inserts "the `managed` block and the marker (by the same
//! rules)" — the rules of ALG-030, §9.3: the managed text, an empty
//! line, the creation marker, one trailing line break. The marker
//! thus lands **at the end of the managed block, before the closing
//! marker** (OUT-016…018, D-23); scheme B has no separate marker
//! location. An empty managed block leaves only the marker between
//! the marker lines ("if `managed` is empty, the file has only the
//! marker"):
//!
//! ```text
//! managed non-empty:  <managed>⏎⏎<marker>⏎
//! managed empty:      <marker>⏎
//! ```
//!
//! The trailing break is what keeps the end marker on its own line:
//! without it the marker would glue to the END line and the file this
//! write produced would fail its own next [`markers::parse`] (D-062
//! states that precondition for exactly this splice). Every output of
//! [`assemble`] therefore re-parses, with the begin marker unmoved —
//! a property the tests below pin.
//!
//! # Line breaks: the file's dominant style, LF on a tie
//!
//! ALG-031: "The block's line breaks are converted to the ones most
//! used in the file (LF on a tie)." The count is over the file's
//! **preserved** terminators — everything outside the marker lines,
//! the marker lines' own terminators included; the region being
//! replaced is excluded, because its breaks vanish on write: counting
//! them would let a stale block's style outvote the file the user
//! actually keeps, and flip the chosen style between two syncs of the
//! same build — the second sync would then rewrite an unchanged file,
//! which D-24 ("status `unchanged` … is not overwritten", T-F9)
//! forbids. Counting the preserved outside makes the choice a fixed
//! point: the outside never changes, so neither does the style, and
//! every write of the same build into the same file produces the same
//! bytes (pinned by the idempotence tests). A `\r\n` counts once, as
//! CRLF; every `\n` not part of a pair counts once, as LF; a lone
//! `\r` is content, not a break (the `str::lines` semantics of
//! D-062). CRLF wins only when strictly more frequent — a tie takes
//! LF. All breaks the block introduces (the managed text's inner
//! breaks, the empty line, the trailing break) use that one style;
//! the marker line arrives without a break of its own
//! ([`creation_marker_line`] emits none). The managed text's own
//! breaks are converted with the same lines-semantics: one trailing
//! break is absorbed, not doubled; further trailing blank lines are
//! lines like any other and are kept.
//!
//! # What does not belong here
//!
//! - **No extra-file reading lines.** Those are scheme A only
//!   (§7.5's ALG-030 "(A only)"; OUT-B names none): the by-scheme
//!   builder for A/C (T-039) owns them. Scheme B's managed block is
//!   the body alone, from [`body::build_body`] (T-035) — ALG-002 has
//!   already normalized it to LF text with no trailing break, which
//!   is why the conversion here starts from LF in practice.
//! - **The marker line is an input**, verbatim: the sync pipeline
//!   formats it once per run via [`creation_marker_line`] (T-038,
//!   ALG-032) and hands it over. This module adds no clock and no
//!   format of its own.
//! - **No I/O, no `AppError`.** Reading the file, the raw-bytes
//!   `E_NOT_UTF8` check (upstream of the `&str` this module takes)
//!   and the mapping of [`MarkersError`] to `E_MARKERS_INVALID` are
//!   the sync pipeline's (§9.4 phase 1 step 4; D-062's boundary);
//!   writing the result atomically is the file layer's.
//! - **No reserved-line policing.** A managed text that itself
//!   contains a scheme-B marker *line* would make the written file
//!   grow a second pair and be skipped on the next sync (§7.5); that
//!   text is rejected earlier, at save time, by DB-016's
//!   `E_CONTENT_RESERVED` check — whose exact line rule is that
//!   task's decision (D-062 notes the same for `classify_line`).
//!
//! [`creation_marker_line`]: crate::marker::creation_marker_line

use crate::markers::{self, MarkerSpan, MarkersError};

/// Assembles a scheme-B target file's new content (spec §7.5, ALG-031).
///
/// `content` is the existing file's whole text (UTF-8 — the
/// `E_NOT_UTF8` check runs upstream, on the raw bytes), `managed` the
/// project's built body ([`build_body`](crate::body::build_body)
/// output), `marker_line` the formatted creation marker
/// ([`creation_marker_line`](crate::marker::creation_marker_line),
/// ALG-032 — a single line, no break). The marker pair is looked up
/// afresh; a file that breaks any §7.5 rule returns its
/// [`MarkersError`] unchanged for the pipeline to map to
/// `E_MARKERS_INVALID` — nothing is written in that case, which is
/// the caller's decision, not this function's.
///
/// The returned [`String`] is the file's new content: the new managed
/// block (see the module docs for its layout) between the marker
/// lines, everything outside them byte for byte (D-22, T-F12).
///
/// # Examples
///
/// ```
/// use eunomia_core::assembly_b::assemble;
///
/// let file = "Intro.\n<!-- EUNOMIASYNC:BEGIN -->\nOld.\n<!-- EUNOMIASYNC:END -->\n";
/// let marker = "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";
///
/// let written = assemble(file, "New body.", marker).expect("one pair, in order");
/// assert_eq!(
///     written,
///     format!("Intro.\n<!-- EUNOMIASYNC:BEGIN -->\nNew body.\n\n{marker}\n<!-- EUNOMIASYNC:END -->\n"),
/// );
/// ```
pub fn assemble(content: &str, managed: &str, marker_line: &str) -> Result<String, MarkersError> {
    let span = markers::parse(content)?;
    Ok(splice(content, &span, managed, marker_line))
}

/// The ALG-031 splice proper, for a caller that already holds the
/// [`MarkerSpan`] (spec §7.5, ALG-031; D-062's splice contract).
///
/// This is [`assemble`] without the parse: `span` is trusted the way
/// [`MarkerSpan::managed`](crate::markers::MarkerSpan::managed)
/// trusts it — a hand-built or mismatched span slices `content` where
/// its offsets point (or panics past its end), while a span from
/// [`markers::parse`] over this same `content` is always safe. The
/// dominant line-break style is counted over the splice's preserved
/// outside — `content[..span.managed_start]` and
/// `content[span.managed_end..]` — not the whole file: the replaced
/// region's breaks do not survive the write and must not outvote the
/// bytes that do (see the module docs).
///
/// # Panics
///
/// Panics if `marker_line` contains a line break — the marker is one
/// line by contract
/// ([`creation_marker_line`](crate::marker::creation_marker_line)
/// never emits one), and a broken one would glue the managed block to
/// the end marker (the check is a release `assert!`, matching
/// `marker.rs`'s posture for its own inputs' domains). Slicing panics
/// for an out-of-range span are as documented above.
///
/// # Examples
///
/// ```
/// use eunomia_core::assembly_b::splice;
/// use eunomia_core::markers;
///
/// let file = "Intro.\n<!-- EUNOMIASYNC:BEGIN -->\nOld.\n<!-- EUNOMIASYNC:END -->\n";
/// let span = markers::parse(file).expect("one pair, in order");
///
/// let written = splice(file, &span, "New body.", "<!-- Created by EunomiaSync … -->");
/// assert_eq!(
///     written,
///     "Intro.\n<!-- EUNOMIASYNC:BEGIN -->\nNew body.\n\n<!-- Created by EunomiaSync … -->\n<!-- EUNOMIASYNC:END -->\n",
/// );
/// ```
#[must_use]
pub fn splice(content: &str, span: &MarkerSpan, managed: &str, marker_line: &str) -> String {
    assert!(
        !marker_line.contains(['\r', '\n']),
        "the creation marker is one line (ALG-032); a break in it would glue the block to the end marker"
    );

    // The style comes from the preserved outside only: the region
    // between the offsets is about to be discarded, and its breaks
    // must not outvote the bytes that survive (module docs, D-24).
    let style = dominant_terminator(&content[..span.managed_start], &content[span.managed_end..]);
    let block = build_block(managed, marker_line, style);

    // D-062's splice: the three pieces are the whole output, so every
    // byte outside [managed_start, managed_end) is copied, never
    // recomputed — the T-F12 byte-exactness in one line.
    let mut written = String::with_capacity(content.len() + block.len());
    written.push_str(&content[..span.managed_start]);
    written.push_str(&block);
    written.push_str(&content[span.managed_end..]);
    written
}

/// The line-break style a scheme-B file most uses (ALG-031: "the
/// ones most used in the file (LF on a tie)"), counted over the
/// splice's preserved outside: `head` is `content[..managed_start]`,
/// `tail` is `content[managed_end..]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Terminator {
    /// `\n` — the tie winner, and the default of a break-less text.
    Lf,
    /// `\r\n` — only when strictly more frequent than LF.
    CrLf,
}

impl Terminator {
    /// The terminator's bytes, as spliced into the block.
    fn as_str(self) -> &'static str {
        match self {
            Terminator::Lf => "\n",
            Terminator::CrLf => "\r\n",
        }
    }
}

/// Picks the dominant style over the preserved outside of the splice.
///
/// Both sides are scanned with [`count_breaks`] and summed; no line
/// terminator can straddle the head/tail boundary, because the head
/// ends right after the begin line's terminator and the tail starts
/// at the end line's first byte. CRLF must be strictly ahead — a tie,
/// and an outside with no breaks at all, takes LF.
fn dominant_terminator(head: &str, tail: &str) -> Terminator {
    let (head_crlf, head_lf) = count_breaks(head.as_bytes());
    let (tail_crlf, tail_lf) = count_breaks(tail.as_bytes());
    if head_crlf + tail_crlf > head_lf + tail_lf {
        Terminator::CrLf
    } else {
        Terminator::Lf
    }
}

/// Counts a text's line terminators: every `\r\n` once, as CRLF;
/// every `\n` not part of a pair once, as LF; a lone `\r` is content,
/// not a break (D-062's `str::lines` semantics).
fn count_breaks(bytes: &[u8]) -> (usize, usize) {
    let mut crlf = 0usize;
    let mut lf = 0usize;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'\n' => {
                lf += 1;
                at += 1;
            }
            b'\r' if bytes.get(at + 1) == Some(&b'\n') => {
                crlf += 1;
                at += 2;
            }
            _ => at += 1,
        }
    }
    (crlf, lf)
}

/// Builds the block inserted between the marker lines: ALG-030's
/// layout, with the marker at its end (OUT-016…018) and every break
/// in the file's dominant style (ALG-031).
///
/// The managed text is split with `str::lines` semantics and rejoined
/// with `style`, so whatever breaks it arrives with (LF from
/// [`build_body`](crate::body::build_body), a pasted CRLF, a trailing
/// break) become exactly the file's style — one trailing break is
/// absorbed, not doubled, further trailing blank lines are lines like
/// any other and are kept, and a lone `\r` inside a line is content
/// and stays. An empty managed text (no lines) yields the marker
/// alone: "if `managed` is empty, the file has only the marker".
fn build_block(managed: &str, marker_line: &str, style: Terminator) -> String {
    let terminator = style.as_str();
    let mut block = String::new();

    let mut lines = managed.lines();
    if let Some(first) = lines.next() {
        block.push_str(first);
        for line in lines {
            block.push_str(terminator);
            block.push_str(line);
        }
        // End the managed text, then ALG-030's empty line before the
        // marker — two terminators, both in the file's style.
        block.push_str(terminator);
        block.push_str(terminator);
    }
    block.push_str(marker_line);
    // ALG-030's trailing break, carried into scheme B by ALG-031:
    // keeps the end marker on its own line (D-062's precondition).
    block.push_str(terminator);
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markers::{self, MarkersError};

    /// The ALG-032 / D-23 example marker, verbatim — the pipeline's
    /// one-per-run input in miniature.
    const MARKER: &str = "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";

    /// T-F12's byte-for-byte property, checked through each side's own
    /// parse: the area before the begin marker and after the end
    /// marker is the same slice of text in input and output. (The two
    /// spans' offsets differ — the block between them is replaced —
    /// so slicing the output at the input's offsets would be wrong.)
    fn outside_survives(input: &str, output: &str) {
        let before = markers::parse(input).expect("the input has one pair");
        let after = markers::parse(output).expect("the output has one pair");
        assert_eq!(
            &output[..after.managed_start],
            &input[..before.managed_start],
            "the area before the begin marker changed"
        );
        assert_eq!(
            &output[after.managed_end..],
            &input[before.managed_end..],
            "the area after the end marker changed"
        );
    }

    /// Every write must leave a file its own parser accepts, with the
    /// begin marker unmoved (ALG-031 / D-062).
    fn reparses_with_moved_end(input: &str, output: &str, expected_end_line: usize) {
        let before = markers::parse(input).expect("the input has one pair");
        let after = markers::parse(output).expect("a written file re-parses");
        assert_eq!(after.begin_line, before.begin_line);
        assert_eq!(after.end_line, expected_end_line);
    }

    // ---- T-F12 (§15.2): BOM + CRLF, byte for byte -------------------------

    #[test]
    fn t_f12_bom_and_crlf_file_keeps_outside_bytes_verbatim() {
        // §15.2 T-F12: "A scheme B file with BOM and CRLF — the area
        // outside the markers is kept byte for byte." The oracle below
        // is hardcoded, independent of the implementation: BOM, CRLF
        // head and tail, and the block converted to the file's style.
        let input = "\u{feff}Header.\r\n\
                     <!-- EUNOMIASYNC:BEGIN -->\r\n\
                     Old block.\r\n\
                     <!-- EUNOMIASYNC:END -->\r\n\
                     Tail.\r\n";

        let output =
            assemble(input, "New managed text.\nSecond line.", MARKER).expect("one pair, in order");

        let oracle = "\u{feff}Header.\r\n\
                      <!-- EUNOMIASYNC:BEGIN -->\r\n\
                      New managed text.\r\n\
                      Second line.\r\n\
                      \r\n\
                      <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\r\n\
                      <!-- EUNOMIASYNC:END -->\r\n\
                      Tail.\r\n";
        assert_eq!(output, oracle);
        outside_survives(input, &output);
        reparses_with_moved_end(input, &output, 7);
    }

    // ---- ALG-031's dominant-break rule -------------------------------------

    #[test]
    fn a_crlf_majority_converts_an_lf_block_to_crlf() {
        // Four CRLF breaks outside the pair, one stray LF: CRLF wins.
        let input = "One.\r\nTwo.\r\n<!-- EUNOMIASYNC:BEGIN -->\r\nOld.\r\n<!-- EUNOMIASYNC:END -->\r\nThree.\r\nFour.\n";
        let output = assemble(input, "First line.\nSecond line.", MARKER).expect("valid pair");

        assert_eq!(
            output,
            "One.\r\nTwo.\r\n<!-- EUNOMIASYNC:BEGIN -->\r\n\
             First line.\r\nSecond line.\r\n\r\n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\r\n\
             <!-- EUNOMIASYNC:END -->\r\nThree.\r\nFour.\n"
        );
        outside_survives(input, &output);
    }

    #[test]
    fn an_lf_majority_converts_a_crlf_block_to_lf() {
        // The mirror case: LF outside, a CRLF-carrying managed body.
        let input = "One.\nTwo.\n<!-- EUNOMIASYNC:BEGIN -->\nOld.\n<!-- EUNOMIASYNC:END -->\nThree.\nFour.\r\n";
        let output = assemble(input, "First line.\r\nSecond line.", MARKER).expect("valid pair");

        assert_eq!(
            output,
            "One.\nTwo.\n<!-- EUNOMIASYNC:BEGIN -->\n\
             First line.\nSecond line.\n\n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n\
             <!-- EUNOMIASYNC:END -->\nThree.\nFour.\r\n"
        );
        outside_survives(input, &output);
    }

    #[test]
    fn a_tie_outside_takes_lf_even_when_the_old_block_disagrees() {
        // ALG-031: "LF on a tie". The count covers the preserved
        // outside only — two CRLF and two LF out there — so the old
        // block's CRLF breaks (which the write discards) cannot tip
        // the choice; the new block's CRLF breaks still become LF.
        // The begin line's own LF is outside and keeps its bytes.
        let input =
            "One.\r\nTwo.\r\n<!-- EUNOMIASYNC:BEGIN -->\nOld.\r\n<!-- EUNOMIASYNC:END -->\n";
        let output = assemble(input, "A\r\nB", MARKER).expect("valid pair");

        assert_eq!(
            output,
            "One.\r\nTwo.\r\n<!-- EUNOMIASYNC:BEGIN -->\nA\nB\n\n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n\
             <!-- EUNOMIASYNC:END -->\n"
        );
        outside_survives(input, &output);
    }

    #[test]
    fn the_terminator_count_follows_lines_semantics() {
        // A `\r\n` pair is one CRLF, not a CRLF plus an LF: counting
        // the pair on both sides would flip this case to LF (2 CRLF,
        // 2 LF). A lone `\r` is content (D-062) and counts for
        // neither style.
        assert_eq!(count_breaks(b"a\r\nb\r\nc\nd"), (2, 1));
        assert_eq!(count_breaks(b"a\r\nb\nc"), (1, 1));
        assert_eq!(count_breaks(b"a\rb\rc"), (0, 0));
        assert_eq!(dominant_terminator("a\r\nb\r\nc\nd", ""), Terminator::CrLf);
        // A tie takes LF, and so does an outside with no breaks at
        // all (unreachable through `assemble` — the head always ends
        // with the begin line's terminator — but the counter stands
        // alone).
        assert_eq!(dominant_terminator("a\r\nb\r\n", "c\nd\n"), Terminator::Lf);
        assert_eq!(dominant_terminator("no breaks", ""), Terminator::Lf);
        assert_eq!(dominant_terminator("", ""), Terminator::Lf);
    }

    // ---- first insertion, replacement, empty body ---------------------------

    #[test]
    fn an_empty_managed_region_receives_its_first_block() {
        // Adjacent marker lines — nothing between them yet (the state
        // after the user pastes the block by hand, §7.5).
        let input = "# Notes\n<!-- EUNOMIASYNC:BEGIN -->\n<!-- EUNOMIASYNC:END -->\n";
        let output = assemble(input, "First body.", MARKER).expect("valid pair");

        assert_eq!(
            output,
            "# Notes\n<!-- EUNOMIASYNC:BEGIN -->\nFirst body.\n\n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n\
             <!-- EUNOMIASYNC:END -->\n"
        );
        outside_survives(input, &output);
        reparses_with_moved_end(input, &output, 6);
    }

    #[test]
    fn an_existing_managed_region_is_replaced_whole() {
        // However much old content sits between the markers, none of
        // it survives: the block is the new managed text and marker,
        // nothing else.
        let input = "<!-- EUNOMIASYNC:BEGIN -->\n\
                     stale line one\n\
                     stale line two\n\
                     <!-- Created by EunomiaSync 2020-01-01 00:00:00 UTC+00:00 -->\n\
                     stale line three\n\
                     <!-- EUNOMIASYNC:END -->\n";
        let output = assemble(input, "Fresh.", MARKER).expect("valid pair");

        assert_eq!(
            output,
            "<!-- EUNOMIASYNC:BEGIN -->\nFresh.\n\n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\n\
             <!-- EUNOMIASYNC:END -->\n"
        );
        assert!(!output.contains("stale"));
        outside_survives(input, &output);
    }

    #[test]
    fn an_empty_managed_body_leaves_only_the_marker_between_the_markers() {
        // ALG-030: "If `managed` is empty, the file has only the
        // marker" — in scheme B, between the marker lines.
        let input =
            "Head.\n<!-- EUNOMIASYNC:BEGIN -->\nWhatever was here.\n<!-- EUNOMIASYNC:END -->\n";
        let output = assemble(input, "", MARKER).expect("valid pair");

        assert_eq!(
            output,
            format!("Head.\n<!-- EUNOMIASYNC:BEGIN -->\n{MARKER}\n<!-- EUNOMIASYNC:END -->\n")
        );
        outside_survives(input, &output);
        reparses_with_moved_end(input, &output, 4);
    }

    #[test]
    fn a_managed_body_of_blank_lines_keeps_them_between_the_block_breaks() {
        // build_body never emits this (ALG-022 separates blocks with
        // exactly one empty line), but the splice is faithful to the
        // lines it is given: two blank lines stay two blank lines.
        let input = "<!-- EUNOMIASYNC:BEGIN -->\nold\n<!-- EUNOMIASYNC:END -->\n";
        let output = assemble(input, "\n\n", MARKER).expect("valid pair");

        assert_eq!(
            output,
            format!("<!-- EUNOMIASYNC:BEGIN -->\n\n\n\n{MARKER}\n<!-- EUNOMIASYNC:END -->\n")
        );
        outside_survives(input, &output);
    }

    #[test]
    fn a_trailing_break_in_the_managed_body_is_not_doubled() {
        // The body arrives without a trailing break from build_body;
        // one that slips in is absorbed, not turned into an extra
        // empty line before the marker. A second trailing break is a
        // blank line of its own and is kept as one.
        let input = "<!-- EUNOMIASYNC:BEGIN -->\nold\n<!-- EUNOMIASYNC:END -->\n";
        let without = assemble(input, "Body.", MARKER).expect("valid pair");
        let with = assemble(input, "Body.\n", MARKER).expect("valid pair");
        assert_eq!(with, without);

        let two = assemble(input, "Body.\n\n", MARKER).expect("valid pair");
        assert_eq!(
            two,
            format!("<!-- EUNOMIASYNC:BEGIN -->\nBody.\n\n\n{MARKER}\n<!-- EUNOMIASYNC:END -->\n")
        );
    }

    // ---- the parser's verdicts are proxied (§7.5, T-F4) ---------------------

    #[test]
    fn a_file_without_markers_is_reported_as_such() {
        // The card's error case: a file the parser rejects returns
        // its MarkersError unchanged — the pipeline maps it to
        // E_MARKERS_INVALID; nothing here decides to write anyway.
        assert_eq!(
            assemble("# Just markdown\n", "Body.", MARKER),
            Err(MarkersError::NoMarkers)
        );
    }

    #[test]
    fn every_other_parser_verdict_is_proxied_too() {
        let reversed = "<!-- EUNOMIASYNC:END -->\n<!-- EUNOMIASYNC:BEGIN -->\n";
        assert_eq!(
            assemble(reversed, "Body.", MARKER),
            Err(MarkersError::ReversedOrder {
                begin_line: 2,
                end_line: 1
            })
        );

        let two_pairs = "<!-- EUNOMIASYNC:BEGIN -->\n<!-- EUNOMIASYNC:END -->\n<!-- EUNOMIASYNC:BEGIN -->\nx\n<!-- EUNOMIASYNC:END -->\n";
        assert_eq!(
            assemble(two_pairs, "Body.", MARKER),
            Err(MarkersError::DuplicateMarkers { begins: 2, ends: 2 })
        );

        let half = "text\n<!-- EUNOMIASYNC:BEGIN -->\nmore\n";
        assert_eq!(
            assemble(half, "Body.", MARKER),
            Err(MarkersError::MissingEndMarker { begin_line: 2 })
        );
    }

    // ---- the outside's awkward bytes ----------------------------------------

    #[test]
    fn padded_marker_lines_and_a_bom_survive_outside() {
        // D-062: padding is space/tab around the marker; a BOM before
        // the padded first line also sits outside the splice. The end
        // marker's line here is the file's last, padding and all.
        let input = "\u{feff}  <!-- EUNOMIASYNC:BEGIN -->\t\nold\n\t <!-- EUNOMIASYNC:END -->  ";
        let output = assemble(input, "Body.", MARKER).expect("valid pair");

        assert_eq!(
            output,
            format!(
                "\u{feff}  <!-- EUNOMIASYNC:BEGIN -->\t\nBody.\n\n{MARKER}\n\t <!-- EUNOMIASYNC:END -->  "
            )
        );
        outside_survives(input, &output);
    }

    #[test]
    fn an_end_marker_without_a_trailing_break_keeps_the_tail_verbatim() {
        // The file's last line needs no newline (D-062); the block
        // still ends with its own break, so END stays a line.
        let input = "head\n<!-- EUNOMIASYNC:BEGIN -->\nold\n<!-- EUNOMIASYNC:END -->";
        let output = assemble(input, "Body.", MARKER).expect("valid pair");

        assert_eq!(
            output,
            format!(
                "head\n<!-- EUNOMIASYNC:BEGIN -->\nBody.\n\n{MARKER}\n<!-- EUNOMIASYNC:END -->"
            )
        );
        outside_survives(input, &output);
    }

    #[test]
    fn the_outside_breaks_of_a_mixed_file_are_not_touched() {
        // Byte-exactness is per byte, not per style: a mixed file
        // keeps its mixed breaks outside the pair — including the
        // begin line's own LF after four CRLF head lines — while the
        // block itself takes the majority style (CRLF here, 5:3
        // across the file).
        let input = "one\r\ntwo\r\nthree\r\nfour\r\n<!-- EUNOMIASYNC:BEGIN -->\nold\n<!-- EUNOMIASYNC:END -->\nfive\r\n";
        let output = assemble(input, "X\nY", MARKER).expect("valid pair");

        assert_eq!(
            output,
            "one\r\ntwo\r\nthree\r\nfour\r\n<!-- EUNOMIASYNC:BEGIN -->\nX\r\nY\r\n\r\n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\r\n\
             <!-- EUNOMIASYNC:END -->\nfive\r\n"
        );
        outside_survives(input, &output);
    }

    // ---- the write's own stability ------------------------------------------

    #[test]
    fn a_second_sync_of_the_written_file_is_a_no_op() {
        // The dominant style is a fixed point: it is counted over the
        // preserved outside, which no write changes, so every sync of
        // the same build into the same file produces the same bytes —
        // what D-24's "unchanged" comparison needs.
        let input =
            "a\r\nb\r\n<!-- EUNOMIASYNC:BEGIN -->\r\nold\r\n<!-- EUNOMIASYNC:END -->\r\nc\n";
        let once = assemble(input, "One.\nTwo.", MARKER).expect("valid pair");
        let twice = assemble(&once, "One.\nTwo.", MARKER).expect("the written file re-parses");
        assert_eq!(twice, once);
    }

    #[test]
    fn a_stale_block_style_cannot_flip_the_files_choice() {
        // The R1-review regression: a file whose outside is CRLF but
        // whose old managed block was LF (a foreign or hand-made
        // block). Counting the whole file would call this a tie, pick
        // LF, and write a file whose next count picks CRLF — one
        // spurious rewrite of an unchanged build (D-24/T-F9). The
        // count ignores the block: CRLF wins outright, immediately
        // and forever.
        let input =
            "A\r\nB\r\n<!-- EUNOMIASYNC:BEGIN -->\r\nx\ny\nz\nw\n<!-- EUNOMIASYNC:END -->\r\n";

        let once = assemble(input, "Body.", MARKER).expect("valid pair");

        assert_eq!(
            once,
            "A\r\nB\r\n<!-- EUNOMIASYNC:BEGIN -->\r\nBody.\r\n\r\n\
             <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->\r\n\
             <!-- EUNOMIASYNC:END -->\r\n"
        );
        outside_survives(input, &once);

        let twice = assemble(&once, "Body.", MARKER).expect("the written file re-parses");
        assert_eq!(twice, once, "the choice is a fixed point");
    }

    #[test]
    fn the_written_managed_region_is_exactly_the_block() {
        // Reading the output back through the parser: the managed
        // region between the markers is precisely the new block, so
        // ALG-033's hash comparison and the next sync see one body.
        let input = "Intro.\n<!-- EUNOMIASYNC:BEGIN -->\nOld.\n<!-- EUNOMIASYNC:END -->\n";
        let output = assemble(input, "New body.", MARKER).expect("valid pair");

        let span = markers::parse(&output).expect("the written file re-parses");
        assert_eq!(span.managed(&output), format!("New body.\n\n{MARKER}\n"));
    }
}
