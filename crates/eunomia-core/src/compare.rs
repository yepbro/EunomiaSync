//! Comparing an existing file with a new build (spec §9.3, ALG-033;
//! §2 D-24, §8 SYS-008).
//!
//! D-24 decides whether a sync may skip the write: "if the new
//! content matches the existing file, ignoring the marker line, the
//! file is not overwritten (status `unchanged`)". ALG-033 fixes the
//! comparison this module implements, verbatim:
//!
//! > in the existing file, the last non-empty line that starts with
//! > `<!-- Created by EunomiaSync ` and ends with ` -->` is dropped;
//! > the rest is compared with the new content byte for byte.
//!
//! # The drop rule, made precise
//!
//! Three readings the sentence leaves open are pinned here:
//!
//! - **Which line.** The last line *anywhere in the file* whose
//!   content starts with `<!-- Created by EunomiaSync ` and ends with
//!   ` -->` — a content match, not a position: the scan runs over
//!   every line and does not require the marker to be the file's
//!   last line. A stray marker copy inside the managed text survives
//!   (only the last one goes); edits after the marker stay in the
//!   rest and make it differ. "Non-empty" needs no separate check: a
//!   line carrying that prefix cannot be empty. Recognition is
//!   prefix and suffix only — the timestamp between them is not
//!   re-validated against ALG-032.
//! - **What dropping removes.** The line's span: its content bytes
//!   plus its terminator (`\r\n` or `\n`; nothing when the final line
//!   is unterminated). The empty line ALG-030 places before the
//!   marker is a line of its own and stays. The `\r` of a `\r\n`
//!   terminator belongs to the terminator, not the content — the line
//!   model of Rust's `str::lines()`, on bytes. ALG-031 lets the app
//!   itself write CRLF marker lines into CRLF scheme-B files; without
//!   that absorption a second sync of such a file could never be
//!   `unchanged`, against D-24's acceptance ("a second sync with no
//!   edits changes no files"). A `\r` anywhere else is an ordinary
//!   byte and breaks the suffix.
//! - **No marker, nothing dropped.** The whole existing content is
//!   then compared as is — a file without any marker line matches
//!   exactly when its bytes are already the new build's markerless
//!   content.
//!
//! The result: a file freshly written by this app matches its next
//! build, marker time aside, and any other difference — one body
//! byte, an extra line, a BOM, CRLF where the build has LF — does
//! not.
//!
//! # Inputs
//!
//! `existing` is raw file **bytes**: the comparison is byte for byte,
//! and only scheme B is guaranteed UTF-8 (§9.4 phase 1 checks
//! `E_NOT_UTF8` there), so the A/C reader hands over whatever the
//! disk holds. The prefix and suffix are ASCII, which never occurs
//! inside a multi-byte UTF-8 sequence, so the scan needs no decoded
//! text — and a file whose compared **rest** contains invalid UTF-8
//! never matches: those bytes stay in the rest while the build is
//! valid UTF-8. Bytes between the prefix and the suffix of a dropped
//! marker line go with the line — recognition never looks there.
//!
//! `new_without_marker_line` is the new build **with its own marker
//! line's span removed the same way** — the markerless form of the
//! very bytes the sync would write. For the ALG-030 shape (`managed`,
//! an empty line, the marker, one line break) that is
//! `managed + "\n\n"`; when `managed` is empty, the empty string. The
//! comparison does not scan the new side: the build knows where its
//! marker line is, so re-finding it here would only invite drift.
//!
//! `existing` is the bytes of a file that **exists**. A missing
//! target is §8's write case, not a comparison (OUT-005: a missing
//! file is created): an empty-managed build's markerless form is the
//! empty string, so empty bytes standing in for a missing file would
//! read as `unchanged` and the file would never appear.
//!
//! # Boundary
//!
//! The return value is the plain D-24 answer — match or no match.
//! SYS-008's externally-modified state, the DB-021 dirty flag and any
//! journal detail around a difference are later consumers' concerns
//! and deliberately not shaped here; reading the file from disk and
//! assembling the build (ALG-030/031) are other tasks' seams.

/// The exact ALG-033 prefix of a creation-marker line — the fixed
/// phrase of ALG-032 up to and including the space after
/// `EunomiaSync`.
const MARKER_PREFIX: &[u8] = b"<!-- Created by EunomiaSync ";

/// The exact ALG-033 suffix of a creation-marker line — a space, the
/// comment close.
const MARKER_SUFFIX: &[u8] = b" -->";

/// Compares an existing file with a new build by ALG-033 (spec §9.3;
/// the D-24 `unchanged` decision).
///
/// In `existing`, the last line whose content starts with
/// `<!-- Created by EunomiaSync ` and ends with ` -->` is dropped —
/// content and terminator together; without such a line nothing is
/// dropped. The rest is compared with `new_without_marker_line` byte
/// for byte (see the module docs for the full rule and the meaning of
/// the new side).
///
/// Pure: no I/O, no clock, no allocation. The marker's timestamp is
/// never looked at, which is the whole point — two builds of the same
/// content at different times match.
///
/// # Examples
///
/// ```
/// use eunomia_core::compare::contents_match;
/// use eunomia_core::marker::{MarkerTime, creation_marker_line};
///
/// let time = MarkerTime {
///     year: 2026,
///     month: 11,
///     day: 17,
///     hour: 14,
///     minute: 3,
///     second: 22,
/// };
/// let file = format!(
///     "Be brief.\n\n{}\n",
///     creation_marker_line(time, Some("Europe/Prague"), 60),
/// );
///
/// // Same build, whatever the marker says: match.
/// assert!(contents_match(file.as_bytes(), "Be brief.\n\n"));
///
/// // Any other byte of the body: no match.
/// assert!(!contents_match(
///     file.replace("Be brief.", "Be very brief.").as_bytes(),
///     "Be brief.\n\n",
/// ));
/// ```
#[must_use]
pub fn contents_match(existing: &[u8], new_without_marker_line: &str) -> bool {
    let new = new_without_marker_line.as_bytes();
    match last_marker_line_span(existing) {
        Some(span) => {
            let before = &existing[..span.start];
            let after = &existing[span.end..];
            new.len() == before.len() + after.len()
                && new.starts_with(before)
                && new.ends_with(after)
        }
        None => existing == new,
    }
}

/// Finds the byte span of the last creation-marker line of `existing`
/// — its content plus its terminator (`\r\n` or `\n`; a bare content
/// range when the final line is unterminated) — or `None` when no
/// line of the file is a marker line.
///
/// Lines are the segments between `\n` bytes, with one `\r` absorbed
/// into a `\n` terminator (`str::lines()` semantics, on bytes). The
/// scan walks forward and remembers the last match, so a stray marker
/// copy earlier in the file loses to the real one after it.
fn last_marker_line_span(existing: &[u8]) -> Option<std::ops::Range<usize>> {
    let mut last = None;
    let mut start = 0;
    while start <= existing.len() {
        let content_start = start;
        let Some(offset) = existing[start..].iter().position(|&b| b == b'\n') else {
            // Final unterminated line: the span is the content alone.
            if is_marker_line(&existing[content_start..]) {
                last = Some(content_start..existing.len());
            }
            break;
        };
        let line_end = content_start + offset; // index of the '\n'
        let content_end = if line_end > content_start && existing[line_end - 1] == b'\r' {
            line_end - 1
        } else {
            line_end
        };
        if is_marker_line(&existing[content_start..content_end]) {
            last = Some(content_start..(line_end + 1)); // content + terminator
        }
        start = line_end + 1;
    }
    last
}

/// One line's content against the ALG-033 prefix and suffix. A line
/// with this prefix cannot be empty, so the spec's "non-empty" holds
/// by construction; the bytes between prefix and suffix — ALG-032's
/// timestamp and zone — are deliberately not inspected.
fn is_marker_line(content: &[u8]) -> bool {
    content.starts_with(MARKER_PREFIX) && content.ends_with(MARKER_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::{MarkerTime, creation_marker_line};
    use crate::markers::{BEGIN_MARKER, END_MARKER};

    /// The §9.2-style managed text the tests build files around.
    const MANAGED: &str = "## Style\n\nBe brief.";

    /// A marker line through the real ALG-032 formatter — the format
    /// under test is exactly what the app writes.
    fn marker(time: MarkerTime, zone: Option<&str>, offset_minutes: i32) -> String {
        creation_marker_line(time, zone, offset_minutes)
    }

    /// `MarkerTime` shorthand for the six civil fields.
    fn at(year: i16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> MarkerTime {
        MarkerTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    /// The spec's example marker: 2026-11-17 14:03:22 Europe/Prague.
    fn spec_marker() -> String {
        marker(at(2026, 11, 17, 14, 3, 22), Some("Europe/Prague"), 60)
    }

    /// A file in the ALG-030 shape: managed, an empty line, the
    /// marker, one line break — the bytes a sync leaves on disk.
    /// Its markerless form (what the caller would pass as the new
    /// content) is `MANAGED + "\n\n"`.
    fn fresh_file(marker_line: &str) -> String {
        format!("{MANAGED}\n\n{marker_line}\n")
    }

    // ---- the criterion: match across marker times -------------------

    #[test]
    fn files_differing_only_in_the_marker_match() {
        // Markers from the real formatter: another time, another date
        // and zone, the D-23 UTC fallback, a western negative offset.
        let markers = [
            spec_marker(),
            marker(at(2031, 1, 2, 3, 4, 5), Some("Europe/Prague"), 60),
            marker(at(2026, 11, 17, 14, 3, 22), None, 60),
            marker(at(2026, 11, 17, 8, 3, 22), Some("America/New_York"), -300),
        ];
        // They really are different files…
        let files: Vec<String> = markers.iter().map(|m| fresh_file(m)).collect();
        for pair in files.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
        // …and each one matches the same markerless new content, the
        // build prefix up to where its own marker line would start.
        for file in &files {
            assert!(contents_match(file.as_bytes(), &format!("{MANAGED}\n\n")));
        }
    }

    #[test]
    fn a_marker_line_without_its_final_break_still_drops() {
        // A file whose last line break after the marker was stripped:
        // the span is the bare content, the drop happens all the same.
        let file = format!("{MANAGED}\n\n{}", spec_marker());
        assert!(contents_match(file.as_bytes(), &format!("{MANAGED}\n\n")));
    }

    #[test]
    fn a_crlf_marker_line_in_a_crlf_file_is_recognized() {
        // ALG-031 converts a scheme-B block's breaks — marker
        // included — to the file's own; the terminator's `\r` is not
        // part of the content, so such a file matches its build.
        let file = format!("{MANAGED}\r\n\r\n{}\r\n", spec_marker());
        assert!(contents_match(
            file.as_bytes(),
            &format!("{MANAGED}\r\n\r\n")
        ));
    }

    #[test]
    fn recognition_does_not_parse_the_timestamp() {
        // Prefix and suffix alone decide; whatever sits between — an
        // old format, a note — is not re-validated against ALG-032.
        let file = format!("{MANAGED}\n\n<!-- Created by EunomiaSync not a timestamp -->\n");
        assert!(contents_match(file.as_bytes(), &format!("{MANAGED}\n\n")));
    }

    #[test]
    fn the_last_marker_line_is_dropped_not_an_earlier_one() {
        // A stray marker copy inside the managed text is content and
        // stays; the scan drops the last marker line, the file's real
        // one — so the rest equals the new build, stray line and all.
        let stray = "<!-- Created by EunomiaSync 2020-01-01 00:00:00 UTC +00:00 -->";
        let file = format!("Note.\n{stray}\n\n{}\n", spec_marker());
        let new = format!("Note.\n{stray}\n\n");
        assert!(contents_match(file.as_bytes(), &new));
    }

    #[test]
    fn a_scheme_b_file_matches_with_the_marker_between_block_and_end() {
        // ALG-031/OUT-016 place the creation marker between the
        // managed block and the closing line — not at the file's end.
        // The drop removes the marker line alone: the END line keeps
        // the break in front of it, and the file matches its
        // markerless twin in both break styles.
        let cases = [
            (
                format!(
                    "{BEGIN_MARKER}\n{MANAGED}\n\n{}\n{END_MARKER}\n",
                    spec_marker()
                ),
                format!("{BEGIN_MARKER}\n{MANAGED}\n\n{END_MARKER}\n"),
            ),
            (
                format!(
                    "{BEGIN_MARKER}\r\n{}\r\n\r\n{}\r\n{END_MARKER}\r\n",
                    MANAGED.replace('\n', "\r\n"),
                    spec_marker()
                ),
                format!(
                    "{BEGIN_MARKER}\r\n{}\r\n\r\n{END_MARKER}\r\n",
                    MANAGED.replace('\n', "\r\n")
                ),
            ),
        ];
        for (file, new) in cases {
            assert!(
                contents_match(file.as_bytes(), &new),
                "no match for {file:?}"
            );
        }
    }

    // ---- the criterion: any other edit differs ----------------------

    #[test]
    fn a_body_edit_differs() {
        let edited = fresh_file(&spec_marker()).replace("Be brief.", "Be very brief.");
        assert!(!contents_match(
            edited.as_bytes(),
            &format!("{MANAGED}\n\n")
        ));
        // Even one trailing byte of extra content in the file:
        let grown = format!("{MANAGED}.\n\n{}\n", spec_marker());
        assert!(!contents_match(grown.as_bytes(), &format!("{MANAGED}\n\n"),));
    }

    #[test]
    fn an_extra_line_differs() {
        // An extra reading line before the managed text:
        let extra = format!("<!-- read: extra.txt -->\n{MANAGED}\n\n{}\n", spec_marker());
        assert!(!contents_match(extra.as_bytes(), &format!("{MANAGED}\n\n"),));
        // A blank line appended after the marker is content too:
        let padded = format!("{MANAGED}\n\n{}\n\n", spec_marker());
        assert!(!contents_match(
            padded.as_bytes(),
            &format!("{MANAGED}\n\n"),
        ));
    }

    #[test]
    fn a_bom_outside_the_marker_differs() {
        // The build has no BOM, so a marked file carrying one is not
        // its match — the marker drops, the BOM stays in the rest.
        let file = format!("\u{FEFF}{MANAGED}\n\n{}\n", spec_marker());
        assert!(!contents_match(file.as_bytes(), &format!("{MANAGED}\n\n"),));
    }

    #[test]
    fn crlf_breaks_against_an_lf_build_differ() {
        // Byte for byte means no line-break normalization: the CRLF
        // twin of a file does not match the LF build.
        let file = format!("{MANAGED}\r\n\r\n{}\r\n", spec_marker());
        assert!(!contents_match(file.as_bytes(), &format!("{MANAGED}\n\n")));
    }

    #[test]
    fn a_damaged_marker_line_is_not_dropped() {
        // Near-miss marker lines: a double space inside the phrase, a
        // leading indent, a trailing space after the close — each
        // fails the exact prefix or suffix, stays in the comparison
        // as content, and makes the file differ from the build.
        let damaged = [
            "<!-- Created  by EunomiaSync 2026-11-17 14:03:22 UTC +00:00 -->",
            "  <!-- Created by EunomiaSync 2026-11-17 14:03:22 UTC +00:00 -->",
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 UTC +00:00 --> ",
            // The prefix's trailing space and the suffix's leading
            // space are part of the ALG-033 strings, not separators:
            "<!-- Created by EunomiaSync2026-11-17 14:03:22 UTC +00:00 -->",
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 UTC+00:00-->",
        ];
        for line in damaged {
            let file = format!("{MANAGED}\n\n{line}\n");
            assert!(
                !contents_match(file.as_bytes(), &format!("{MANAGED}\n\n")),
                "a damaged marker line must not be dropped: {line:?}"
            );
        }
    }

    #[test]
    fn content_added_after_the_marker_differs() {
        // The marker not at the end — an edit below it. The drop is
        // by content, not position: the marker goes, the edit stays.
        let file = format!("{MANAGED}\n\n{}\nAdded later.\n", spec_marker());
        assert!(!contents_match(file.as_bytes(), &format!("{MANAGED}\n\n"),));
    }

    #[test]
    fn non_utf8_bytes_outside_the_marker_differ() {
        // Only scheme B promises UTF-8; an A/C file may hold other
        // bytes. The scan still finds the ASCII marker, the rest
        // keeps what the build does not have and cannot match it.
        // A foreign but valid character first:
        let file = format!("\u{FFFD}{MANAGED}\n\n{}\n", spec_marker());
        assert!(!contents_match(file.as_bytes(), &format!("{MANAGED}\n\n"),));
        // Raw invalid bytes, not expressible as a &str fixture:
        let mut raw = b"\xFF".to_vec();
        raw.extend_from_slice(fresh_file(&spec_marker()).as_bytes());
        assert!(!contents_match(&raw, &format!("{MANAGED}\n\n")));
        // The flip side: invalid bytes INSIDE the marker line drop
        // with it — recognition never looks between the prefix and
        // the suffix, so this marker-only file matches the empty
        // build.
        assert!(contents_match(
            b"<!-- Created by EunomiaSync \xFF -->\n",
            ""
        ));
    }

    // ---- no marker, empty file, marker-only file ---------------------

    #[test]
    fn a_file_without_a_marker_is_compared_whole() {
        // The marker line removed by hand: the rest is exactly the
        // markerless build, so the file matches and stays untouched.
        assert!(contents_match(
            format!("{MANAGED}\n\n").as_bytes(),
            &format!("{MANAGED}\n\n"),
        ));
        // With the separator's breaks gone too, it is no longer the
        // build's markerless bytes and differs:
        assert!(!contents_match(
            MANAGED.as_bytes(),
            &format!("{MANAGED}\n\n")
        ));
    }

    #[test]
    fn an_empty_existing_file_matches_only_an_empty_build() {
        assert!(contents_match(b"", ""));
        assert!(!contents_match(b"", &format!("{MANAGED}\n\n")));
    }

    #[test]
    fn a_file_of_only_the_marker_matches_an_empty_build() {
        // ALG-030: an empty managed part leaves the marker alone in
        // the file; dropping it leaves nothing to compare against
        // but the empty build.
        let file = format!("{}\n", spec_marker());
        assert!(contents_match(file.as_bytes(), ""));
        let unterminated = spec_marker();
        assert!(contents_match(unterminated.as_bytes(), ""));
    }
}
