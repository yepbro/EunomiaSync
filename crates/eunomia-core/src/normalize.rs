//! Variant text normalization for the build (spec ALG-002, spec
//! decision D-22).
//!
//! Built files are UTF-8 without a BOM and use LF line breaks (spec
//! D-22), so the raw stored text of a variant (see
//! [`crate::library::Variant`]; the snapshot carries it raw, per
//! DECISIONS.md D-042) passes through this module before it can reach
//! a file. ALG-002 fixes the rules:
//!
//! - CRLF and a single CR are replaced with LF;
//! - the BOM is removed;
//! - leading empty lines and all trailing whitespace of the block are
//!   dropped;
//! - spaces inside the text are not touched (they matter in Markdown).
//!
//! The store already folds CRLF into LF when saving (spec §6.9), but a
//! lone CR, a BOM and edge whitespace can still arrive — through
//! import (SEC-009), older databases, or direct SQL — and scheme B
//! (ALG-031) preserves the surrounding file's line breaks only outside
//! the managed block, so the build-time pass is unconditional and this
//! module is the single owner of the rule.
//!
//! Two boundaries ALG-002 names but does not define; both are recorded
//! in DECISIONS.md D-044:
//!
//! - the whitespace class is ASCII whitespace (`space`, `\t`, `\n`,
//!   `\f`, `\r` — Rust's `char::is_ascii_whitespace`), the characters
//!   that shape lines and blank lines in a UTF-8/LF file. Unicode
//!   whitespace (NBSP `U+00A0`, the `U+2000`…`U+200A` block, `U+3000`,
//!   …) and the vertical tab are Markdown text, not line structure, so
//!   they survive even at the block edges;
//! - an "empty line" is a line that is empty or holds only characters
//!   of that ASCII class — the Markdown blank line.

/// The UTF-8 BOM character (spec D-22: built files carry none).
const BOM: char = '\u{feff}';

/// Normalizes variant text for building (spec ALG-002).
///
/// The input is a variant's stored `content` as-is; the output is the
/// block a project file receives: CRLF and lone CR become LF, a
/// leading BOM is removed, leading empty lines and all trailing
/// whitespace of the block are dropped. Everything inside the text is
/// preserved exactly — leading indentation of the first line, empty
/// lines in the middle, and trailing spaces on inner lines (a
/// Markdown hard break) all survive.
///
/// The BOM rule is positional: a BOM counts as leading only at the
/// start of the block or right behind its leading empty lines
/// (`"\n\u{feff}text"` normalizes to `"text"` in one call); anywhere
/// else — after the first line's own characters, or inside the text —
/// it is content and stays.
///
/// The function is idempotent — the result is already normalized, so
/// `variant_text(variant_text(x)) == variant_text(x)` — and linear in
/// the input length.
///
/// # Examples
///
/// ```
/// use eunomia_core::library::{Variant, VariantId};
/// use eunomia_core::normalize;
///
/// let variant = Variant {
///     id: VariantId::new("v1".to_owned()),
///     name: "Main".to_owned(),
///     // Stored raw (D-042): a BOM, Windows line breaks, edge junk.
///     content: "\u{feff}\r\n\r\nBe brief.\r\n\n".to_owned(),
/// };
/// assert_eq!(normalize::variant_text(&variant.content), "Be brief.");
///
/// // Inside the text nothing moves.
/// let text = "first  \n\n    code";
/// assert_eq!(normalize::variant_text(text), text);
/// ```
#[must_use]
pub fn variant_text(content: &str) -> String {
    let converted = convert_line_breaks(content);
    let trimmed = converted.trim_end_matches(|c: char| c.is_ascii_whitespace());
    drop_leading_edge(trimmed).to_owned()
}

/// Replaces CRLF and a lone CR with LF (spec ALG-002).
fn convert_line_breaks(text: &str) -> String {
    let mut converted = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            // A CRLF pair becomes one LF; a lone CR becomes one too.
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            converted.push('\n');
        } else {
            converted.push(ch);
        }
    }
    converted
}

/// Drops the block's leading edge: the BOM run and the empty lines,
/// interleaved, in one linear scan (spec ALG-002).
///
/// The two rules interleave — an empty line can hide a BOM that only
/// becomes leading once the line is dropped (`"\n\u{feff}text"`), and a
/// BOM run can precede further empty lines — so the scan repeats
/// {strip the BOM run, drop empty lines} until neither consumes
/// anything. Every character is consumed once or examined a bounded
/// number of times, which keeps the whole normalization linear (a
/// leading-edge fixed point re-running the full pass would be
/// quadratic on inputs like `"\u{feff}\n"` repeated — the round-1
/// review measured 13.6 s on a 131k-character crafted input).
///
/// A line is empty when it holds nothing but ASCII whitespace (the
/// Markdown blank line; see the module docs). The first line that
/// carries any other character starts the text — including its own
/// leading spaces, which are inside the text from there on.
fn drop_leading_edge(text: &str) -> &str {
    let mut start = 0;
    loop {
        let before = start;
        while text[start..].starts_with(BOM) {
            start += BOM.len_utf8();
        }
        while let Some(offset) = text[start..].find('\n') {
            let line_end = start + offset;
            if text[start..line_end]
                .chars()
                .all(|c: char| c.is_ascii_whitespace())
            {
                start = line_end + 1;
            } else {
                break;
            }
        }
        if start == before {
            return &text[start..];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::variant_text;

    // ---- Line breaks (ALG-002: CRLF and a single CR become LF) ----

    #[test]
    fn crlf_becomes_lf() {
        assert_eq!(variant_text("one\r\ntwo\r\nthree"), "one\ntwo\nthree");
    }

    #[test]
    fn lone_cr_becomes_lf() {
        assert_eq!(variant_text("one\rtwo\rthree"), "one\ntwo\nthree");
        assert_eq!(variant_text("a\rb"), "a\nb");
    }

    #[test]
    fn mixed_line_breaks_all_become_lf() {
        assert_eq!(variant_text("a\r\nb\rc\nd"), "a\nb\nc\nd");
    }

    // ---- BOM (ALG-002: the BOM is removed; D-22: files carry none) ----

    #[test]
    fn leading_bom_is_removed() {
        assert_eq!(variant_text("\u{feff}text"), "text");
        assert_eq!(variant_text("\u{feff}\u{feff}text"), "text");
    }

    #[test]
    fn bom_only_input_normalizes_to_empty() {
        assert_eq!(variant_text("\u{feff}"), "");
    }

    #[test]
    fn bom_inside_the_text_is_content() {
        assert_eq!(variant_text("a\u{feff}b"), "a\u{feff}b");
    }

    #[test]
    fn a_bom_behind_a_leading_empty_line_is_removed_in_one_call() {
        // The empty line hides the BOM from a plain position-0 check;
        // the leading-edge scan still removes both (D-044).
        assert_eq!(variant_text("\n\u{feff}text"), "text");
        assert_eq!(variant_text("\r\u{feff}text"), "text");
    }

    #[test]
    fn a_bom_after_the_first_lines_indentation_is_content() {
        // The BOM rule is positional: leading means the very start of
        // the block (behind its empty lines), never after the first
        // line's own characters (D-044).
        assert_eq!(variant_text("  \u{feff}text"), "  \u{feff}text");
    }

    // ---- Leading empty lines (ALG-002) ----

    #[test]
    fn leading_empty_lines_are_dropped() {
        assert_eq!(variant_text("\n\n\nfirst\nsecond"), "first\nsecond");
    }

    #[test]
    fn leading_whitespace_only_lines_are_dropped() {
        // A whitespace-only line is the Markdown blank line: it is
        // empty for the block's leading edge (DECISIONS.md D-044).
        assert_eq!(variant_text(" \n\t\n  \nfirst"), "first");
    }

    #[test]
    fn leading_spaces_of_the_first_content_line_are_inside_the_text() {
        assert_eq!(variant_text("\n\n    indented"), "    indented");
    }

    // ---- Trailing whitespace of the block (ALG-002: all of it) ----

    #[test]
    fn trailing_line_breaks_are_dropped() {
        assert_eq!(variant_text("text\n\n\n"), "text");
    }

    #[test]
    fn trailing_spaces_and_tabs_are_dropped() {
        assert_eq!(variant_text("text  \t"), "text");
    }

    #[test]
    fn a_trailing_whitespace_line_is_dropped_whole() {
        assert_eq!(variant_text("text\n \t\n \n"), "text");
    }

    #[test]
    fn a_hard_break_at_the_block_end_is_trailing_whitespace() {
        // "All trailing whitespace of the block" is unconditional:
        // even the two-space Markdown hard break goes at the very end.
        assert_eq!(variant_text("text  "), "text");
    }

    // ---- Spaces inside the text are not touched (ALG-002) ----

    #[test]
    fn spaces_inside_the_text_are_kept() {
        let text = "    indented code\n  - a list item  with  double spaces";
        assert_eq!(variant_text(text), text);
    }

    #[test]
    fn empty_lines_inside_the_text_are_kept() {
        assert_eq!(
            variant_text("first\n\nsecond\n\nthird"),
            "first\n\nsecond\n\nthird"
        );
    }

    #[test]
    fn trailing_spaces_on_inner_lines_are_kept() {
        // Two trailing spaces are a Markdown hard line break; the line
        // is inner, so its spaces are inside the text.
        assert_eq!(variant_text("first  \nsecond"), "first  \nsecond");
    }

    #[test]
    fn tabs_inside_the_text_are_kept() {
        assert_eq!(variant_text("use tabs:\tinner."), "use tabs:\tinner.");
    }

    // ---- Whitespace class boundaries (DECISIONS.md D-044) ----

    #[test]
    fn unicode_whitespace_is_content_even_at_the_edges() {
        // NBSP and the em space render in Markdown; they are not ASCII
        // whitespace, so the edge rules never touch them.
        assert_eq!(variant_text("\u{a0}text\u{2003}"), "\u{a0}text\u{2003}");
    }

    #[test]
    fn vertical_tab_is_content() {
        // U+000B is outside the WhatWG ASCII whitespace class, so it
        // survives the edge rules like any other character.
        assert_eq!(variant_text("\u{b}text"), "\u{b}text");
    }

    #[test]
    fn a_form_feed_only_line_is_blank_at_the_leading_edge() {
        // One ASCII class serves both edge rules, so the blank-line
        // class is one notch broader than CommonMark's space/tab:
        // a form-feed-only line counts as empty here (D-044). Inside
        // the text the same line stays.
        assert_eq!(variant_text("\u{c}\nfoo"), "foo");
        assert_eq!(variant_text("a\n\u{c}\nb"), "a\n\u{c}\nb");
    }

    // ---- Degenerate inputs ----

    #[test]
    fn empty_input_normalizes_to_empty() {
        assert_eq!(variant_text(""), "");
    }

    #[test]
    fn whitespace_only_input_normalizes_to_empty() {
        assert_eq!(variant_text(" \n\t\n \n"), "");
        assert_eq!(variant_text("\r"), "");
    }

    // ---- Idempotency ----

    #[test]
    fn normalization_is_idempotent() {
        let inputs = [
            "",
            "plain",
            "\u{feff}\r\n\r\nmixed\rtext  \r\n\n",
            // A BOM after a leading empty line surfaces only once that
            // line is dropped; the result must still be stable.
            "\n\u{feff}bom after an empty line",
            "\u{feff}\u{feff}double bom",
            "  \r\n\r\n\t\n\u{feff}\rleading junk",
        ];
        for input in inputs {
            let once = variant_text(input);
            assert_eq!(variant_text(&once), once, "input: {input:?}");
        }
    }

    #[test]
    fn the_worst_interleaving_stays_linear() {
        // A crafted BOM/empty-line chain (SEC-009 treats imports as
        // untrusted): it once pushed the leading-edge loop into
        // quadratic runtime; the single-scan version eats it instantly.
        let mut evil = String::with_capacity(65_536 * 4 + 4);
        for _ in 0..65_536 {
            evil.push('\u{feff}');
            evil.push('\n');
        }
        evil.push_str("tail");
        assert_eq!(variant_text(&evil), "tail");
    }

    // ---- The whole pipeline over a realistic paste ----

    #[test]
    fn a_windows_paste_is_normalized_in_one_call() {
        let pasted = "\u{feff}\r\n\r\nBe brief.\r\nUse tabs:\tinner.\r\n  \r\n\r\n";
        assert_eq!(variant_text(pasted), "Be brief.\nUse tabs:\tinner.");
    }
}
