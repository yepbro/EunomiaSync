//! The scheme-B marker parser (spec §7.5 "Scheme B", D-19, ALG-031).
//!
//! Scheme B never creates, moves or removes the two HTML comment
//! markers that fence the managed block — the user pastes them by hand
//! (§7.5, OUT-B), so every sync first has to find **exactly one pair**
//! of the D-19 markers
//!
//! - [`BEGIN_MARKER`] = `<!-- EUNOMIASYNC:BEGIN -->` and
//! - [`END_MARKER`] = `<!-- EUNOMIASYNC:END -->`,
//!
//! each on its own line, spaces at the line ends allowed. [`parse`]
//! is that lookup: a pure linear scan over the file's text (no I/O —
//! the crate boundary of D-006/D-010) returning the managed region
//! between the marker lines as byte offsets into the input, or one
//! error variant per way the file can fail the rule. The spec §7.5
//! names the four failures ("No markers, several of them, reversed
//! order, or one of them missing"); the T-F4 scenario of §15.2
//! exercises three of them (no markers, two pairs, reversed order),
//! and the unit suite below covers all four plus the edges.
//!
//! Three readings of D-19/§7.5 this module fixes (the reasoning is
//! recorded in DECISIONS.md D-062):
//!
//! - **"Spaces at the ends"** is horizontal editor padding: ASCII
//!   space and tab, on either side of the marker, and nothing else.
//!   Other ASCII whitespace (`\f`, `\v`) and Unicode spaces (NBSP,
//!   ideographic space, …) are line content, so a line carrying them
//!   is not a marker line. A `\r` is not padding either: it counts as
//!   a line terminator only as part of `\r\n` (see the next point).
//! - **CRLF files are first-class.** D-22 keeps the file's line
//!   breaks outside the block byte for byte, so a marker line may end
//!   in LF or CRLF. Lines are split with the semantics of
//!   [`str::lines`]: on `\n`, a `\r` directly before it belongs to
//!   the terminator; a lone `\r` anywhere else — including the last
//!   line of a file with no final `\n` — is content.
//! - **A BOM at byte 0 is an encoding artifact, not line content.**
//!   D-22 lets a scheme-B file keep its BOM outside the block, and
//!   the only place a BOM can sit before the begin marker is the very
//!   start of line 1, so [`parse`] skips one `\u{feff}` there. A BOM
//!   anywhere else is content and keeps its line from being a marker
//!   line. All returned offsets stay absolute in the input either
//!   way, so the BOM is preserved by any splice made at them.
//!
//! A marker written inside a longer line — before text, after text,
//! or both markers on one line — is *not* a marker line ("each on its
//! own line"), and a marker spelled differently (case, inner spaces,
//! missing spaces around `-->`) never matches: the D-19 literals are
//! compared byte for byte after the padding trim.
//!
//! When several rules fail at once, the error is classified by
//! precedence: **duplicates outrank a missing half, which outranks
//! reversed order** (`DuplicateMarkers` carries the begin/end counts,
//! so a file with two begins and no end loses no information to the
//! classification).
//!
//! # The boundary with `E_MARKERS_INVALID`
//!
//! The spec's error code lives in the §7.10 `AppError` registry
//! (`src-tauri/src/errors.rs`, DECISIONS.md D-034): every variant of
//! [`MarkersError`] maps to `E_MARKERS_INVALID` there. That mapping
//! belongs to the sync pipeline (§9.4 phase 1 step 4 reads the file,
//! checks UTF-8 — `E_NOT_UTF8`, upstream of this module, whose input
//! is already `&str` — then calls [`parse`] and translates), and is
//! deliberately absent here: this crate depends on neither Tauri nor
//! the app's error DTO. DB-016's reserved-marker-line validation can
//! likewise reuse [`classify_line`] without dragging `AppError` in —
//! with the caveat that DB-016's own wording is "a line **equal to**
//! the marker" while [`classify_line`] also matches padded marker
//! lines; whether that widening is DB-016's rule is that task's
//! decision, not a fact fixed here.
//!
//! # Consumption by the write path
//!
//! ALG-031 (scheme B, T-041) splices the new managed block between
//! the marker lines: everything outside
//! `content[..span.managed_start] + block + content[span.managed_end..]`
//! — the marker lines, the BOM, the outside line breaks — stays
//! byte for byte (§7.5, D-22). One precondition sits on the inserted
//! `block`, not on the span: it must itself end with the file's line
//! terminator (ALG-030's trailing break, which ALG-031 carries into
//! scheme B), or the end marker would glue to the block's last line
//! and the next [`parse`] would reject the file this write produced.
//! ALG-033's build-hash comparison reads the same
//! `content[managed_start..managed_end]` slice. Line numbers are
//! 1-based, as editors and diagnostics show them.

use thiserror::Error;

/// The begin marker of the managed block, byte for byte as spec D-19
/// fixes it: `<!-- EUNOMIASYNC:BEGIN -->`.
///
/// The §7.5 "Copy marker block" button and DB-016's reserved-line
/// validation read this constant instead of retyping the literal.
pub const BEGIN_MARKER: &str = "<!-- EUNOMIASYNC:BEGIN -->";

/// The end marker of the managed block, byte for byte as spec D-19
/// fixes it: `<!-- EUNOMIASYNC:END -->`.
pub const END_MARKER: &str = "<!-- EUNOMIASYNC:END -->";

/// The UTF-8 BOM (spec D-22: a scheme-B file may keep it; only the
/// one at byte 0 is an encoding artifact for this parser).
const BOM: char = '\u{feff}';

/// Which marker a line holds, when it holds one at all.
///
/// A line is a begin-marker line or an end-marker line when its
/// content, after ASCII space and tab are trimmed from both ends,
/// equals [`BEGIN_MARKER`] or [`END_MARKER`] exactly ([`classify_line`]).
/// No line can be both: the two literals differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerLine {
    /// The line equals [`BEGIN_MARKER`] modulo padding.
    Begin,
    /// The line equals [`END_MARKER`] modulo padding.
    End,
}

/// The managed region a scheme-B file's marker pair fences in
/// (spec §7.5, ALG-031).
///
/// Produced only by [`parse`]; the fields are the splice points the
/// scheme-B write (ALG-031, T-041) and the build-hash comparison
/// (ALG-033) consume. All offsets are absolute byte offsets into the
/// exact `&str` handed to [`parse`], so splicing at them preserves
/// everything outside the pair byte for byte — the marker lines, a
/// leading BOM, and the file's own line breaks (§7.5, D-22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerSpan {
    /// The 1-based line number of the begin-marker line.
    pub begin_line: usize,
    /// The 1-based line number of the end-marker line (always later
    /// than `begin_line` — a valid span has no other order).
    pub end_line: usize,
    /// The byte offset of the first byte **after** the begin-marker
    /// line's terminator: where the managed block starts. The
    /// begin-marker line itself (padding, marker, LF or CRLF) lies
    /// entirely before this offset.
    pub managed_start: usize,
    /// The byte offset of the end-marker line's first byte: where the
    /// managed block ends. Everything from here on — the end-marker
    /// line included — is outside the managed block.
    pub managed_end: usize,
}

impl MarkerSpan {
    /// The managed block between the marker lines, as a slice of the
    /// parsed content.
    ///
    /// For a pair of adjacent marker lines this is the empty string:
    /// an empty managed region is a valid state a sync fills in
    /// (ALG-031 inserts between the lines).
    ///
    /// # Panics
    ///
    /// Panics if `managed_start > managed_end` or an offset is past
    /// the end of `content` — the debug assertion above checks both,
    /// so a release build panics only on slicing (`content` too short
    /// for the offsets), and a hand-built span with in-range offsets
    /// over the wrong content slices that content instead of
    /// panicking. None of this is reachable for a span returned by
    /// [`parse`], which validates the offsets; the panic marks a
    /// hand-built or mismatched span, not a data condition.
    #[must_use]
    pub fn managed<'a>(&self, content: &'a str) -> &'a str {
        debug_assert!(
            self.managed_start <= self.managed_end && self.managed_end <= content.len(),
            "a parse-returned span is always inside its content"
        );
        &content[self.managed_start..self.managed_end]
    }
}

/// Why a file is not a valid scheme-B target (spec §7.5).
///
/// One variant per failure the spec names — no markers, several
/// marker lines, reversed order, one of the pair missing — with the
/// lines or counts the diagnostics need. The sync pipeline maps every
/// variant to the app-level `E_MARKERS_INVALID` (§7.10, DECISIONS.md
/// D-034); this crate stays independent of that registry (see the
/// module docs).
///
/// # Examples
///
/// ```
/// use eunomia_core::markers::MarkersError;
///
/// let error = MarkersError::ReversedOrder { begin_line: 4, end_line: 2 };
/// assert_eq!(
///     error.to_string(),
///     "the end marker on line 2 precedes the begin marker on line 4"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MarkersError {
    /// No marker line at all: neither marker is present as a line
    /// (T-F4's "no markers"; markers inside longer lines do not
    /// count, see the module docs).
    #[error("no EUNOMIASYNC marker line in the file")]
    NoMarkers,
    /// The begin-marker line is present, the end-marker line is not
    /// (§7.5 "one of them missing").
    #[error("a begin marker on line {begin_line} but no end marker line")]
    MissingEndMarker {
        /// The line the lone begin marker sits on.
        begin_line: usize,
    },
    /// The end-marker line is present, the begin-marker line is not
    /// (§7.5 "one of them missing").
    #[error("an end marker on line {end_line} but no begin marker line")]
    MissingBeginMarker {
        /// The line the lone end marker sits on.
        end_line: usize,
    },
    /// More than one begin- or end-marker line — "two pairs" (T-F4)
    /// or any other surplus. Classification precedence: duplicates
    /// outrank a missing half and reversed order (module docs), and
    /// the counts keep both facts visible.
    #[error("expected one marker pair, found {begins} begin and {ends} end marker lines")]
    DuplicateMarkers {
        /// How many begin-marker lines the file holds.
        begins: usize,
        /// How many end-marker lines the file holds.
        ends: usize,
    },
    /// Exactly one of each marker, but the end-marker line comes
    /// before the begin-marker line (T-F4's "reversed order").
    #[error("the end marker on line {end_line} precedes the begin marker on line {begin_line}")]
    ReversedOrder {
        /// The line the begin marker sits on.
        begin_line: usize,
        /// The line the end marker sits on.
        end_line: usize,
    },
}

/// Classifies one line of a file as a marker line (spec D-19).
///
/// The input is a single line's content — no `\n` line terminator;
/// a `\r` that was part of a `\r\n` terminator must already be
/// stripped (that splitting is [`parse`]'s job). The line matches
/// when, after ASCII spaces and tabs are trimmed from both ends, it
/// equals [`BEGIN_MARKER`] or [`END_MARKER`] byte for byte: padding
/// is allowed, every other difference (case, inner spacing, extra
/// text around the marker) is not.
///
/// # Examples
///
/// ```
/// use eunomia_core::markers::{classify_line, MarkerLine};
///
/// assert_eq!(classify_line("<!-- EUNOMIASYNC:BEGIN -->"), Some(MarkerLine::Begin));
/// assert_eq!(classify_line("  \t<!-- EUNOMIASYNC:END -->\t\t"), Some(MarkerLine::End));
/// assert_eq!(classify_line("text <!-- EUNOMIASYNC:END -->"), None);
/// ```
#[must_use]
pub fn classify_line(line: &str) -> Option<MarkerLine> {
    match line.trim_matches(|c: char| c == ' ' || c == '\t') {
        BEGIN_MARKER => Some(MarkerLine::Begin),
        END_MARKER => Some(MarkerLine::End),
        _ => None,
    }
}

/// Finds the one marker pair a scheme-B file must hold (spec §7.5).
///
/// The input is the file's whole text as UTF-8 (the `E_NOT_UTF8`
/// check of §9.4 phase 1 runs upstream, in the file layer); the
/// output is the [`MarkerSpan`] between the marker lines — the region
/// ALG-031's write splices and ALG-033's comparison hashes — or the
/// [`MarkersError`] telling which §7.5 rule the file broke. The scan
/// is linear and single-pass; nothing outside the marker lines is
/// read beyond recognizing them.
///
/// Every offset in the returned span is absolute in `content`, so
/// `content[..span.managed_start] + new_block + content[span.managed_end..]`
/// is the ALG-031 splice: marker lines, a leading BOM and the file's
/// own line breaks all survive byte for byte (§7.5, D-22).
///
/// # Examples
///
/// ```
/// use eunomia_core::markers::{parse, MarkersError};
///
/// let file = "# Project\n\n<!-- EUNOMIASYNC:BEGIN -->\nOld block.\n<!-- EUNOMIASYNC:END -->\n";
/// let span = parse(file).expect("exactly one pair, in order");
/// assert_eq!((span.begin_line, span.end_line), (3, 5));
/// assert_eq!(span.managed(file), "Old block.\n");
///
/// // T-F4's reversed file is one of the error shapes:
/// let reversed = "<!-- EUNOMIASYNC:END -->\n<!-- EUNOMIASYNC:BEGIN -->\n";
/// assert_eq!(
///     parse(reversed),
///     Err(MarkersError::ReversedOrder { begin_line: 2, end_line: 1 })
/// );
/// ```
pub fn parse(content: &str) -> Result<MarkerSpan, MarkersError> {
    // Every marker line in file order, each carrying exactly what the
    // outcome needs from it: a begin line its number and the first
    // byte after its terminator (the splice point), an end line its
    // number and first byte. Nothing is read back outside the match
    // below, so the span can only be built from the pair the match
    // actually accepted.
    let mut begins: Vec<(usize, usize)> = Vec::new();
    let mut ends: Vec<(usize, usize)> = Vec::new();

    // D-062: a BOM at byte 0 is an encoding artifact (D-22), not the
    // first line's content; offsets stay absolute regardless.
    let mut line_start = usize::from(content.starts_with(BOM)) * BOM.len_utf8();
    let mut line_no = 0;

    while line_start < content.len() {
        line_no += 1;
        let rest = &content[line_start..];
        let (body, next_start) = match rest.find('\n') {
            Some(newline) => {
                // str::lines() semantics: a '\r' right before the
                // '\n' is the CRLF terminator, not content.
                let mut body = &rest[..newline];
                if let Some(without_cr) = body.strip_suffix('\r') {
                    body = without_cr;
                }
                (body, line_start + newline + 1)
            }
            // The final line of a file without a trailing newline.
            None => (rest, content.len()),
        };

        match classify_line(body) {
            Some(MarkerLine::Begin) => begins.push((line_no, next_start)),
            Some(MarkerLine::End) => ends.push((line_no, line_start)),
            None => {}
        }

        line_start = next_start;
    }

    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => Err(MarkersError::NoMarkers),
        ([(begin_line, _)], []) => Err(MarkersError::MissingEndMarker {
            begin_line: *begin_line,
        }),
        ([], [(end_line, _)]) => Err(MarkersError::MissingBeginMarker {
            end_line: *end_line,
        }),
        // More of either marker than the one allowed pair — whatever
        // else is also wrong with the file (a missing half, a
        // reversed order), D-062's precedence reports the counts.
        (many_begins, many_ends) if many_begins.len() > 1 || many_ends.len() > 1 => {
            Err(MarkersError::DuplicateMarkers {
                begins: many_begins.len(),
                ends: many_ends.len(),
            })
        }
        ([(begin_line, managed_start)], [(end_line, managed_end)]) => {
            if end_line < begin_line {
                return Err(MarkersError::ReversedOrder {
                    begin_line: *begin_line,
                    end_line: *end_line,
                });
            }
            // end_line > begin_line always holds here: the lines are
            // numbered in file order and a line matches at most one
            // marker, so the pair cannot share a line.
            let span = MarkerSpan {
                begin_line: *begin_line,
                end_line: *end_line,
                managed_start: *managed_start,
                managed_end: *managed_end,
            };
            debug_assert!(span.managed_start <= span.managed_end);
            debug_assert!(span.managed_end <= content.len());
            Ok(span)
        }
        // The four shapes above exhaust (begins, ends) up to counts,
        // which the guard arm handles first.
        _ => unreachable!("counts of zero and one are matched exhaustively"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- the D-19 oracle --------------------------------------------------

    #[test]
    fn the_marker_constants_are_the_spec_d19_literals() {
        // Hardcoded from spec D-19, independent of the constants: any
        // drift (case, spacing, a doubled space) fails here first.
        assert_eq!(BEGIN_MARKER, "<!-- EUNOMIASYNC:BEGIN -->");
        assert_eq!(END_MARKER, "<!-- EUNOMIASYNC:END -->");
    }

    // ---- the valid pair ---------------------------------------------------

    #[test]
    fn a_valid_pair_locates_the_managed_region() {
        // The oracle is built from named literal parts; the expected
        // offsets are their byte lengths, not parser feedback.
        let head = "Intro text.\n";
        let begin_line = "<!-- EUNOMIASYNC:BEGIN -->\n";
        let managed = "Old managed text.\nSecond line.\n";
        let end_line = "<!-- EUNOMIASYNC:END -->\n";
        let tail = "Outro text.\n";
        let content = format!("{head}{begin_line}{managed}{end_line}{tail}");

        let span = parse(&content).expect("exactly one pair, in order");

        assert_eq!(span.begin_line, 2);
        assert_eq!(span.end_line, 5);
        assert_eq!(span.managed_start, head.len() + begin_line.len());
        assert_eq!(
            span.managed_end,
            head.len() + begin_line.len() + managed.len()
        );
        assert_eq!(span.managed(&content), managed);
        // The ALG-031 splice keeps both sides byte for byte (§7.5,
        // D-22): the boundary slices are exactly the marker lines'
        // surroundings.
        assert_eq!(
            &content[..span.managed_start],
            "Intro text.\n<!-- EUNOMIASYNC:BEGIN -->\n"
        );
        assert_eq!(
            &content[span.managed_end..],
            "<!-- EUNOMIASYNC:END -->\nOutro text.\n"
        );
    }

    #[test]
    fn spaces_and_tabs_around_the_markers_are_allowed() {
        // §7.5: "each on its own line (spaces at the ends are
        // allowed)". D-062 reads that as horizontal padding — space
        // and tab, either side — while the CRLF after the first
        // marker line is a terminator, not padding.
        let content = format!("  \t{BEGIN_MARKER}\t\r\nbody\n\t {END_MARKER}  ");

        let span = parse(&content).expect("padded markers are marker lines");

        assert_eq!(span.begin_line, 1);
        assert_eq!(span.end_line, 3);
        assert_eq!(span.managed(&content), "body\n");
    }

    #[test]
    fn an_empty_managed_region_is_valid() {
        // Adjacent marker lines: nothing between them yet. ALG-031
        // fills such a file in; the parser must not reject it.
        let content = format!("{BEGIN_MARKER}\n{END_MARKER}");

        let span = parse(&content).expect("an empty managed block is valid");

        assert_eq!(span.begin_line, 1);
        assert_eq!(span.end_line, 2);
        assert_eq!(span.managed_start, BEGIN_MARKER.len() + 1);
        assert_eq!(span.managed_start, span.managed_end);
        assert_eq!(span.managed(&content), "");
    }

    #[test]
    fn an_end_marker_without_a_trailing_line_break_is_valid() {
        let content = format!("head\n{BEGIN_MARKER}\nbody\n{END_MARKER}");

        let span = parse(&content).expect("a final line needs no newline");

        assert_eq!(span.end_line, 4);
        assert_eq!(span.managed(&content), "body\n");
        assert_eq!(&content[span.managed_end..], END_MARKER);
    }

    // ---- T-F4 (§15.2) and §7.5's fourth failure ----------------------------

    #[test]
    fn t_f4_a_file_without_markers_reports_no_markers() {
        let content = "# Just markdown\n\nNothing to fence here.\n";
        assert_eq!(parse(content), Err(MarkersError::NoMarkers));
    }

    #[test]
    fn t_f4_two_pairs_report_the_surplus() {
        let content = format!("{BEGIN_MARKER}\n{END_MARKER}\n{BEGIN_MARKER}\nx\n{END_MARKER}\n");
        assert_eq!(
            parse(&content),
            Err(MarkersError::DuplicateMarkers { begins: 2, ends: 2 })
        );
    }

    #[test]
    fn t_f4_a_reversed_pair_reports_reversed_order() {
        let content = format!("above\n{END_MARKER}\nbetween\n{BEGIN_MARKER}\nbelow\n");
        assert_eq!(
            parse(&content),
            Err(MarkersError::ReversedOrder {
                begin_line: 4,
                end_line: 2
            })
        );
    }

    #[test]
    fn one_marker_of_the_pair_alone_is_reported_as_missing() {
        let only_begin = format!("text\n{BEGIN_MARKER}\nmore\n");
        assert_eq!(
            parse(&only_begin),
            Err(MarkersError::MissingEndMarker { begin_line: 2 })
        );

        let only_end = format!("{END_MARKER}\ntext\n");
        assert_eq!(
            parse(&only_end),
            Err(MarkersError::MissingBeginMarker { end_line: 1 })
        );
    }

    // ---- "each on its own line" and byte-exactness -------------------------

    #[test]
    fn a_marker_inside_a_longer_line_is_not_a_marker_line() {
        // Text before, between or after the markers: none of it makes
        // a marker line ("each on its own line", D-19).
        let inline = format!("text {BEGIN_MARKER} {END_MARKER} more\n");
        assert_eq!(parse(&inline), Err(MarkersError::NoMarkers));

        let prefixed = format!("see: {BEGIN_MARKER}\n{END_MARKER}\n");
        assert_eq!(
            parse(&prefixed),
            Err(MarkersError::MissingBeginMarker { end_line: 2 })
        );
    }

    #[test]
    fn a_valid_pair_with_an_inline_mention_stays_valid() {
        // The positive half of "each on its own line": prose quoting
        // a marker is content, so a file whose head or managed block
        // mentions the markers inline still parses (D-062's fifth
        // reading).
        let content = format!(
            "Paste {BEGIN_MARKER} and {END_MARKER} like this:\n{BEGIN_MARKER}\nManaged text quoting {END_MARKER} inline.\n{END_MARKER}\n"
        );

        let span = parse(&content).expect("an inline mention is not a marker line");

        assert_eq!(span.begin_line, 2);
        assert_eq!(span.end_line, 4);
        assert_eq!(
            span.managed(&content),
            format!("Managed text quoting {END_MARKER} inline.\n")
        );
    }

    #[test]
    fn near_miss_spellings_are_not_markers() {
        let near_misses = [
            "<!--EUNOMIASYNC:BEGIN -->",        // no space after <!--
            "<!-- EUNOMIASYNC:BEGIN-->",        // no space before -->
            "<!--  EUNOMIASYNC:BEGIN  -->",     // doubled inner spaces
            "<!-- EUNOMIASYNC: BEGIN -->",      // space after the colon
            "<!-- eunomiasync:begin -->",       // case matters
            "\u{a0}<!-- EUNOMIASYNC:BEGIN -->", // NBSP is not padding
            "<!-- EUNOMIASYNC:END -->\u{a0}",   // …on either side
            "\u{c}<!-- EUNOMIASYNC:BEGIN -->",  // form feed: ASCII whitespace, not padding
            "\u{b}<!-- EUNOMIASYNC:BEGIN -->",  // vertical tab likewise
        ];
        for miss in near_misses {
            assert_eq!(parse(miss), Err(MarkersError::NoMarkers), "line: {miss:?}");
        }
    }

    // ---- D-22: BOM and line breaks ------------------------------------------

    #[test]
    fn a_bom_right_before_the_first_marker_is_an_encoding_artifact() {
        // The one place a BOM can precede the begin marker is byte 0
        // (D-062); the offsets stay absolute, so the BOM is outside
        // the span and survives the ALG-031 splice (D-22).
        let content = format!("\u{feff}{BEGIN_MARKER}\nmanaged\n{END_MARKER}\n");

        let span = parse(&content).expect("a byte-0 BOM is not line content");

        assert_eq!(span.begin_line, 1);
        assert_eq!(span.managed(&content), "managed\n");
        assert!(content[..span.managed_start].starts_with('\u{feff}'));
    }

    #[test]
    fn a_bom_anywhere_else_is_content() {
        let content = format!("{BEGIN_MARKER}\nmanaged\n\u{feff}{END_MARKER}\n");
        assert_eq!(
            parse(&content),
            Err(MarkersError::MissingEndMarker { begin_line: 1 })
        );
    }

    #[test]
    fn only_one_bom_at_byte_zero_is_an_artifact() {
        // A second BOM is content (D-062): line 1 then carries it and
        // is not a marker line, so the file misses its begin marker.
        let content = format!("\u{feff}\u{feff}{BEGIN_MARKER}\n{END_MARKER}\n");
        assert_eq!(
            parse(&content),
            Err(MarkersError::MissingBeginMarker { end_line: 2 })
        );
    }

    #[test]
    fn a_bom_before_an_ordinary_first_line_keeps_the_numbering() {
        // The only line the byte-0 skip can affect is line 1; here
        // line 1 is ordinary content and the pair sits behind it.
        let content = format!("\u{feff}head\n{BEGIN_MARKER}\nbody\n{END_MARKER}\n");

        let span = parse(&content).expect("a valid pair behind a BOM");

        assert_eq!(span.begin_line, 2);
        assert_eq!(span.managed(&content), "body\n");
    }

    #[test]
    fn crlf_line_breaks_are_recognized_and_stay_outside_the_span() {
        let content =
            format!("Header.\r\n{BEGIN_MARKER}\r\nOld block.\r\n{END_MARKER}\r\nTail.\r\n");

        let span = parse(&content).expect("CRLF terminators are line breaks");

        assert_eq!(span.begin_line, 2);
        assert_eq!(span.end_line, 4);
        assert_eq!(span.managed(&content), "Old block.\r\n");
        assert_eq!(
            &content[..span.managed_start],
            format!("Header.\r\n{BEGIN_MARKER}\r\n")
        );
        assert_eq!(
            &content[span.managed_end..],
            format!("{END_MARKER}\r\nTail.\r\n")
        );
    }

    #[test]
    fn t_f12_a_bom_and_crlf_file_keeps_its_outside_bytes() {
        // §15.2 T-F12: a scheme-B file with BOM and CRLF — the area
        // outside the markers is kept byte for byte. Both mechanisms
        // at once: the byte-0 BOM skip and CRLF terminators.
        let content =
            format!("\u{feff}Header.\r\n{BEGIN_MARKER}\r\nOld block.\r\n{END_MARKER}\r\nTail.\r\n");

        let span = parse(&content).expect("BOM and CRLF compose");

        assert_eq!(span.begin_line, 2);
        assert_eq!(span.end_line, 4);
        assert_eq!(span.managed(&content), "Old block.\r\n");
        assert_eq!(
            &content[..span.managed_start],
            format!("\u{feff}Header.\r\n{BEGIN_MARKER}\r\n")
        );
        assert_eq!(
            &content[span.managed_end..],
            format!("{END_MARKER}\r\nTail.\r\n")
        );
    }

    #[test]
    fn a_lone_cr_is_line_content_not_a_terminator() {
        // str::lines() semantics: '\r' belongs to the line unless a
        // '\n' follows it. A file ending "END\r" has no end-marker
        // line, and "BEGIN\r\n" with the padding rule alone would
        // leave the '\r' as trailing content.
        let trailing_cr = format!("{BEGIN_MARKER}\n{END_MARKER}\r");
        assert_eq!(
            parse(&trailing_cr),
            Err(MarkersError::MissingEndMarker { begin_line: 1 })
        );
    }

    // ---- classification precedence (D-062) ----------------------------------

    #[test]
    fn duplicates_outrank_a_missing_half_and_reversed_order() {
        // Two begins, no end: DuplicateMarkers, not MissingEndMarker
        // — the counts carry both facts.
        let two_begins = format!("{BEGIN_MARKER}\n{BEGIN_MARKER}\n");
        assert_eq!(
            parse(&two_begins),
            Err(MarkersError::DuplicateMarkers { begins: 2, ends: 0 })
        );

        // Duplicated AND reversed: multiplicity still wins.
        let end_begin_end = format!("{END_MARKER}\n{BEGIN_MARKER}\n{END_MARKER}\n");
        assert_eq!(
            parse(&end_begin_end),
            Err(MarkersError::DuplicateMarkers { begins: 1, ends: 2 })
        );
    }

    // ---- degenerate inputs ---------------------------------------------------

    #[test]
    fn an_empty_file_and_a_blank_file_have_no_markers() {
        assert_eq!(parse(""), Err(MarkersError::NoMarkers));
        assert_eq!(parse("\n\n\n"), Err(MarkersError::NoMarkers));
    }

    // ---- classify_line -------------------------------------------------------

    #[test]
    fn classify_line_matches_padded_marker_lines_only() {
        assert_eq!(classify_line(BEGIN_MARKER), Some(MarkerLine::Begin));
        assert_eq!(classify_line(END_MARKER), Some(MarkerLine::End));
        assert_eq!(
            classify_line(" \t<!-- EUNOMIASYNC:END -->\t "),
            Some(MarkerLine::End)
        );
        assert_eq!(classify_line(""), None);
        assert_eq!(classify_line("   "), None);
        assert_eq!(classify_line("text"), None);
        assert_eq!(classify_line("x<!-- EUNOMIASYNC:BEGIN -->"), None);
    }
}
