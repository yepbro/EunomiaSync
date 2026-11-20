//! The Markdown preview renderer (spec §7.10 Library module
//! `markdown_preview`, §5.3 "Markdown: `pulldown-cmark` — Preview and
//! warnings. Not used when building files", SEC-005, D-15, D-009).
//!
//! [`preview`] is the whole pipeline, in ONE parse pass:
//!
//! 1. **Parse** the text with pulldown-cmark (the §5.3 choice; the
//!    GitHub-flavored basics — tables, strikethrough, task lists — are
//!    enabled, see [`RENDER_OPTIONS`]).
//! 2. **Escape the raw HTML** (SEC-005's "raw HTML from instruction
//!    text is escaped"): every raw-HTML event the parser emits (whole
//!    HTML *blocks* and inline tags alike) is re-typed as a plain-text
//!    event, and pulldown-cmark's own renderer escapes plain text — so
//!    whatever markup the author typed reaches the preview as a
//!    visible literal, never as markup. Markdown image syntax gets the
//!    same treatment one level up: the image element is dropped and
//!    its alt text (the parser's inner content) survives as text.
//! 3. **Render** to HTML with `html::push_html` — the output now
//!    contains only markup *this renderer generated*.
//! 4. **Sanitize** (SEC-005's "the HTML is sanitized (`ammonia`)"):
//!    ammonia re-parses and re-serializes the render under its
//!    allow-list (see [`sanitize`]). This is the second gate, not the
//!    only one: the escaping in step 2 already keeps the source text's
//!    markup out, and the sanitizer guarantees the generated markup
//!    carries no resource-loading element — with `img` removed from the
//!    allow-list, no `src` attribute survives anywhere (SEC-005's
//!    "External resources are not loaded"; D-022's CSP `img-src 'self'`
//!    is the browser-side twin of that rule, and its deferred live
//!    runtime check belongs to the frontend Preview tab task).
//!
//! Riding the same pass (step 1's event stream, T-057) the pipeline
//! also collects the **warnings** D-15 and §7.3 LIB-002/009…016 ask
//! the editor to show — "Markdown warnings (for example, an unclosed
//! code block) that do not block saving" — see [`MarkdownWarning`] for
//! the taxonomy and the detection invariant below. Warnings are pure
//! response data: they ride alongside the HTML, never into it (the
//! sanitize path of steps 2–4 is byte-for-byte the pipeline D-083
//! pinned), and they can never fail the command — D-15's "warnings
//! only … does not block writing".
//!
//! # The warnings: one parse, no second pass
//!
//! The tracker ([`FenceWarnings`]) observes the same event stream the
//! renderer consumes — `Parser::into_offset_iter()`, the same parser
//! with the same options, each event annotated with its source range.
//! Detection invariant, pinned against pulldown-cmark 0.13's block-end
//! semantics and by tests for every shape: a fenced code block is
//! **unclosed** unless the source between its content's end and the
//! block's own end holds a closing fence — a fence line is never
//! whitespace, so an empty or whitespace-only gap means no closer was
//! ever consumed: the block ran to end-of-input, or an enclosing
//! container (list item, blockquote) broke it first. A consumed closer
//! always leaves that non-whitespace gap (`content_end < block_end`).
//! The closing-fence rules themselves
//! (same character, at least as many fence characters, nothing else on
//! the line) are pulldown-cmark's, not reimplemented here: a line too
//! short to close, or of the wrong character, stays *content* and
//! keeps the content running to the block's end — exactly the warning
//! case. Two pulldown specifics the gap form must survive: a final
//! non-newline-terminated line of 1–3 spaces is a synthesized
//! zero-length item (no content event — the whitespace gap is the
//! unclosed verdict, not a closed block), and degenerate one-or-two-
//! line blocks emit no content events at all (`"```"`, `"```\n"`,
//! `"```\n```"`, `"```\n```\n"`) — those are decided by a bounded read
//! of the block's own source range: closed only if a line follows the
//! opening fence line inside it.
//!
//! # Placement: src-tauri, not eunomia-core
//!
//! The spec pins eunomia-core to a closed dependency allow-list —
//! §5.2: "only `serde`, `sha2`, `thiserror`" (DECISIONS.md D-017). §5.3
//! names `pulldown-cmark` (and SEC-005 names `ammonia`) for the app
//! without assigning either to a crate; they are outside core's list,
//! so the renderer lives in the app crate, beside the command — the
//! same reading that keeps `rusqlite`/`tracing`/`specta` out of the
//! library crates.
//!
//! # The build path never calls this
//!
//! §5.3's "Not used when building files" is enforced structurally: this
//! module's only caller is the `markdown_preview` command
//! (`commands.rs`), and nothing in the assembly/target-file code
//! (`eunomia-core`'s `assembly_*`/`target_file`, `eunomia-fs`) reaches
//! it — those paths keep building plain text exactly as before.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html::push_html};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The Markdown extensions the preview renders. §5.3 pins the crate,
/// not a dialect; these are the GitHub-flavored basics an instruction
/// text plausibly uses — tables, strikethrough, task lists. Deliberately
/// NOT enabled: footnotes, math and metadata blocks (no instruction
/// feature calls for them), heading attributes (`id`/`class` output the
/// sanitizer would strip anyway), definition/superscript/subscript and
/// wikilinks, and smart punctuation (it would silently rewrite the
/// author's literal quotes and dashes). `union` because the bitflags
/// operator is not `const`.
const RENDER_OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS);

/// The `markdown_preview` pipeline result: the sanitized preview HTML
/// and the non-blocking Markdown warnings (D-15), both computed in the
/// single parse pass above.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Preview {
    /// SEC-005's sanitized render — the pipeline steps 2–4.
    pub(crate) html: String,
    /// The warnings (D-15) in source order; empty for clean text.
    pub(crate) warnings: Vec<MarkdownWarning>,
}

/// One Markdown warning (D-15's "validation gives warnings only (for
/// example, an unclosed code fence) and does not block writing";
/// §7.3 LIB-002/009…016's "Markdown warnings (for example, an unclosed
/// code block) that do not block saving").
///
/// The taxonomy is deliberately minimal: the spec pins the category
/// ("warnings only", "do not block saving") and names exactly one kind
/// by example — the unclosed code fence — leaving the rest of the set
/// open; until a requirement names another kind, this stays the whole
/// set (a guessed taxonomy would invent UI for problems nobody asked
/// about). A warning is data for the editor's Preview UI: it never
/// blocks saving (nothing in the save path reads it) and never fails
/// the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) struct MarkdownWarning {
    /// What was found.
    pub(crate) kind: MarkdownWarningKind,
    /// The 1-based source line where the problem starts — for the
    /// unclosed fence, the line of the OPENING fence, which is what an
    /// author fixes. Counted over the raw text's bytes (the editor's
    /// textarea is line-addressable; a byte offset would need a second
    /// coordinate system).
    pub(crate) line: u32,
}

/// The closed set of Markdown warnings the preview reports. The wire
/// strings are a contract for the editor UI (the `ErrorCode` rule:
/// renamed explicitly, pinned by the bindings test) — `snake_case`,
/// one entry per spec example.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MarkdownWarningKind {
    /// A fenced code block that never sees a closing fence: everything
    /// after its opening ` ``` ` line renders as code (D-15's and
    /// LIB-002's named example).
    UnclosedCodeFence,
}

/// Renders Markdown text into sanitized preview HTML plus the
/// non-blocking Markdown warnings (§7.10 Library module
/// `markdown_preview`, SEC-005, D-15) — the pipeline in the module
/// docs. Total for any input: parsing never fails, and empty text
/// renders to an empty string with no warnings.
pub(crate) fn preview(text: &str) -> Preview {
    let mut html = String::with_capacity(text.len());
    let parser = Parser::new_ext(text, RENDER_OPTIONS);
    let mut fences = FenceWarnings::new(text);
    // The offsets ride along for the tracker only; the transformed
    // event stream the renderer consumes is exactly D-083's.
    let neutralized = parser.into_offset_iter().filter_map(|(event, range)| {
        fences.observe(&event, &range);
        match event {
            // Raw HTML from the text — whole blocks and inline tags:
            // re-typed as plain text so the renderer escapes it into a
            // visible literal (SEC-005). Ammonia would *strip* this
            // markup rather than escape it, which is why the transform
            // happens here, before rendering.
            Event::Html(markup) | Event::InlineHtml(markup) => Some(Event::Text(markup)),
            // Markdown image syntax `![alt](url)`: the element is
            // dropped; the parser's inner content (the alt text) is
            // kept, so the author sees *what* the image claimed to be
            // but no `img` element — and with it no `src` — is ever
            // generated (SEC-005: external resources are not loaded).
            Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => None,
            event => Some(event),
        }
    });
    push_html(&mut html, neutralized);
    let html = sanitize(html);
    Preview {
        html,
        warnings: fences.finish(),
    }
}

/// The fenced-code-block tracker riding the parse pass — the warning
/// half of [`preview`], with the detection invariant from the module
/// docs. Fenced blocks do not nest (their content is plain text), so a
/// single open block is the whole state.
struct FenceWarnings<'a> {
    text: &'a str,
    open: Option<OpenFence>,
    warnings: Vec<MarkdownWarning>,
    /// The line-counting cursor: `line_at_counted` is the 1-based line
    /// of `counted_up_to`. Warnings arrive in source order, so each
    /// line number counts only the bytes since the last one instead of
    /// rescanning the text from byte 0 (a max-size §6.9 draft full of
    /// container-broken fences would otherwise make the counting
    /// quadratic).
    counted_up_to: usize,
    line_at_counted: u32,
}

/// A fenced block between its `Start` and `End` events.
struct OpenFence {
    /// Source offset of the opening fence line's first fence character
    /// (the `Start` event's range start — pulldown-cmark begins the
    /// block item there, past any container/indent prefix).
    start: usize,
    /// The furthest source end reached by the block's content events so
    /// far. `None` until content arrives — the degenerate one-line
    /// forms emit none.
    content_end: Option<usize>,
}

impl<'a> FenceWarnings<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            open: None,
            warnings: Vec::new(),
            counted_up_to: 0,
            line_at_counted: 1,
        }
    }

    /// Observes one event of the parse stream. Read-only for the
    /// renderer: the event passes through untouched.
    fn observe(&mut self, event: &Event<'_>, range: &std::ops::Range<usize>) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                // Fenced blocks do not nest; an open block here would
                // mean a parser invariant broke. Ignore defensively
                // (the outer block keeps tracking) rather than panic.
                if self.open.is_none() {
                    self.open = Some(OpenFence {
                        start: range.start,
                        content_end: None,
                    });
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                // Only a fenced block can be open; `End` of an indented
                // code block arrives with nothing open and is a no-op.
                if let Some(open) = self.open.take()
                    && Self::is_unclosed(self.text, open.start, open.content_end, range.end)
                {
                    let line = self.line_of(open.start);
                    self.warnings.push(MarkdownWarning {
                        kind: MarkdownWarningKind::UnclosedCodeFence,
                        line,
                    });
                }
            }
            // Everything else while a block is open is its content
            // (plain text, possibly split into pieces — CRLF input
            // arrives split): track the furthest end.
            _ => {
                if let Some(open) = self.open.as_mut() {
                    open.content_end =
                        Some(open.content_end.map_or(range.end, |end| end.max(range.end)));
                }
            }
        }
    }

    /// The detection invariant: a fenced block is unclosed unless a
    /// closing fence lies between its content and its end — all offsets
    /// absolute source offsets into `text` (they must never be mixed
    /// with slice-relative lengths in one comparison). Closed ⟺ the
    /// gap `text[content_end..block_end]` holds that fence (a fence
    /// line is never whitespace); unclosed ⟺ the gap is empty or
    /// whitespace-only — the block ran to end-of-input or a container
    /// broke it. `content_end > block_end` cannot happen (the parser's
    /// `End` range encloses its content's); the arm is ordered so it
    /// would degrade to "unclosed" rather than slice-invert if a
    /// pulldown upgrade ever broke that guarantee. Blocks that emitted
    /// no content events (`block` = the block's own source range) are
    /// decided by that range: a block holding a line after its opening
    /// fence line was closed by it (anything else following the opening
    /// line would have been content), a block holding nothing but the
    /// opening fence line never closed.
    fn is_unclosed(
        text: &str,
        block_start: usize,
        content_end: Option<usize>,
        block_end: usize,
    ) -> bool {
        match content_end {
            Some(content_end) => {
                content_end >= block_end || text[content_end..block_end].trim().is_empty()
            }
            None => !text[block_start..block_end]
                .split_once('\n')
                .is_some_and(|(_, rest)| !rest.trim().is_empty()),
        }
    }

    /// The 1-based line of a source offset, counted over the raw bytes.
    /// Counts only the bytes since the previous warning's offset (see
    /// the cursor fields) — every count advances the cursor, so a
    /// max-size draft full of warnings still pays O(text) once in
    /// total, not per warning.
    fn line_of(&mut self, offset: usize) -> u32 {
        if offset >= self.counted_up_to {
            let tail = &self.text[self.counted_up_to..offset];
            let newlines =
                u32::try_from(tail.bytes().filter(|&b| b == b'\n').count()).unwrap_or(u32::MAX);
            self.line_at_counted = self.line_at_counted.saturating_add(newlines);
            self.counted_up_to = offset;
            self.line_at_counted
        } else {
            // Defensive: offsets arrive in source order, so this arm is
            // not reachable today; count the head without moving the
            // cursor backwards.
            u32::try_from(self.text[..offset].bytes().filter(|&b| b == b'\n').count())
                .unwrap_or(u32::MAX)
                .saturating_add(1)
        }
    }

    fn finish(self) -> Vec<MarkdownWarning> {
        self.warnings
    }
}

/// SEC-005's ammonia gate over the rendered HTML. The config, on top of
/// the crate defaults:
///
/// - `img` is removed from the allowed tags — no `<img>` (and no `src`
///   attribute) can survive, whatever produced it. No relative/external
///   image policy is needed because no image policy exists (§7.10's
///   preview carries text; D-022's CSP holds the browser side).
/// - Task-list checkboxes get a closed carve-out: `input` joins the
///   allowed tags, `disabled`/`checked` are whitelisted as plain
///   attributes (boolean attributes of the checkboxes pulldown-cmark
///   emits; their value carries no behavior), and `type` is whitelisted
///   ONLY through the value set — the single value `checkbox` —
///   because ammonia's attribute check is a disjunction (`tag_attributes`
///   or `tag_attribute_values`), so an attribute listed in the plain set
///   would pass with any value. An `input` with no `type` attribute at
///   all can still survive this gate for a caller that bypasses the
///   escaping transform — it renders no executable or
///   resource-loading behavior (a browser would show a default text
///   field, nothing more), and gate 1 never lets one
///   through to here.
///
/// The crate defaults carry the rest of the policy: `script`/`style`
/// contents are removed wholesale (`clean_content_tags`), HTML comments
/// are stripped (`strip_comments` defaults to `true` in ammonia 4),
/// URLs are limited to the default safe-scheme set
/// (`http`/`https`/`mailto`/… — no `javascript:`, no `data:`), every
/// kept link is forced to `rel="noopener noreferrer"` (`link_rel`), and
/// `class` is not an allowed attribute, so even the code-fence language
/// class of the render is dropped (the MVP preview styles fenced code
/// as plain `<pre><code>`; no highlighting is in scope).
fn sanitize(html: String) -> String {
    ammonia::Builder::new()
        .rm_tags(&["img"])
        .add_tags(&["input"])
        .add_tag_attributes("input", &["disabled", "checked"])
        .add_tag_attribute_values("input", "type", &["checkbox"])
        .clean(&html)
        .to_string()
}

#[cfg(test)]
mod tests {
    //! Behavior tests of the pipeline through its one entry point
    //! [`preview`] — what the command's caller observes: hostile
    //! markup never survives as markup, raw HTML survives as escaped
    //! *text*, ordinary Markdown demonstrably renders, and the warning
    //! half (D-15, T-057) reports exactly the unclosed fences — every
    //! shape pinned in the module docs — while the HTML stays the
    //! pipeline D-083 pinned. The one exception drives [`sanitize`]
    //! directly: the sanitizer is the second gate, and its policy must
    //! hold even against input that the escaping transform would have
    //! neutralized first (defense in depth is pinned per gate, so
    //! removing one gate cannot silently disarm the other).

    use super::*;

    #[test]
    fn a_script_block_from_the_text_is_escaped_not_executable() {
        // Arrange: the `<script>` half of the acceptance pair, in both
        // shapes the parser can emit — an HTML block (its own line)
        // and an inline tag inside a paragraph.
        let text = "<script>alert('xss')</script>\n\ninline <script>alert(1)</script> tag";

        // Act
        let html = preview(text).html;

        // Assert: no script markup anywhere, the block's full source —
        // tags AND payload — survives as escaped visible text.
        assert!(
            !html.contains("<script"),
            "a script element must not reach the result as markup: {html}"
        );
        assert!(
            html.contains("&lt;script&gt;alert('xss')&lt;/script&gt;"),
            "the script block must survive as escaped visible text: {html}"
        );
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "the inline script tag must be escaped with its payload kept: {html}"
        );
    }

    #[test]
    fn an_external_image_from_markdown_syntax_is_reduced_to_its_alt_text() {
        // Arrange: the `![…](https://…)` spelling of the acceptance
        // pair — the URL must not survive anywhere, because the image
        // element (the only thing that would load it) is dropped.
        let html = preview("![logo](https://tracker.example/pixel.png)").html;

        // Assert: exactly the alt text remains — no img element, no
        // `src`, no trace of the remote URL (the trailing newline is
        // push_html's, kept by the sanitizer's re-serialization).
        assert_eq!(
            html, "<p>logo</p>\n",
            "the image must be reduced to its alt text: {html}"
        );
    }

    #[test]
    fn an_external_image_from_raw_html_survives_only_as_escaped_text() {
        // Arrange: the `<img src=…>` spelling of the acceptance pair —
        // lower-case and upper-case (HTML tags are case-insensitive, so
        // the escaping must be too). As escaped text the URL is inert:
        // visible to the author, unreferenceable by the browser.
        let text = "<img src=\"https://ads.example/banner.png\"> and <IMG SRC=\"https://ads.example/tall.png\">";

        // Act
        let html = preview(text).html;

        // Assert: no img markup in either case, the escaped literal
        // with its URL kept as text.
        assert!(
            !html.contains("<img") && !html.contains("<IMG"),
            "an img element must not reach the result as markup: {html}"
        );
        assert!(
            html.contains("&lt;img src=\"https://ads.example/banner.png\"&gt;"),
            "the raw img tag must survive as escaped visible text: {html}"
        );
        assert!(
            html.contains("&lt;IMG SRC=\"https://ads.example/tall.png\"&gt;"),
            "tag case must not change the escaping: {html}"
        );
    }

    #[test]
    fn raw_html_is_escaped_even_when_the_tag_would_be_harmless() {
        // Arrange: benign markup an author might style with — the
        // requirement is *all* raw HTML is escaped, not only the
        // dangerous kind (ammonia alone would let `<b>` through as
        // markup; the escaping transform is the stricter gate).
        let html = preview("see <b>bold</b> words").html;

        // Assert
        assert!(
            html.contains("&lt;b&gt;bold&lt;/b&gt;"),
            "benign raw HTML must also arrive as escaped text: {html}"
        );
        assert!(
            !html.contains("<b>"),
            "no raw tag may pass as markup: {html}"
        );
    }

    #[test]
    fn a_heading_renders_as_a_heading() {
        // Arrange & Act & Assert: the basic render pin — Markdown in,
        // real (sanitizer-serialized) HTML out.
        assert_eq!(preview("# Title").html, "<h1>Title</h1>\n");
    }

    #[test]
    fn emphasis_and_strong_renders() {
        // Arrange & Act & Assert
        let html = preview("*point* and **strong**").html;

        assert!(html.contains("<em>point</em>"), "emphasis pin: {html}");
        assert!(
            html.contains("<strong>strong</strong>"),
            "strong pin: {html}"
        );
    }

    #[test]
    fn a_fenced_code_block_renders_with_escaped_content_and_no_class() {
        // Arrange: a TAGGED fence — pulldown-cmark emits
        // `<pre><code class="language-rust">` for it, which gives the
        // sanitizer's no-class policy something real to strip — whose
        // body contains markup-shaped text.
        let html = preview("```rust\nlet s = \"<here> & more\";\n```").html;

        // Assert: the code block renders WITHOUT the language class
        // (the exact `<pre><code>` pins the class-stripping policy),
        // and its content is escaped (the fence is *display*, never
        // markup).
        assert!(html.contains("<pre><code>"), "code block pin: {html}");
        assert!(
            !html.contains("class="),
            "no class attribute may survive sanitization: {html}"
        );
        assert!(
            html.contains("&lt;here&gt; &amp; more"),
            "code content must be escaped: {html}"
        );
        assert!(
            !html.contains("<here>"),
            "no raw markup from the fence: {html}"
        );
    }

    #[test]
    fn a_javascript_link_is_reduced_to_its_text() {
        // Arrange: a Markdown link whose target is a `javascript:` URL
        // — the one external-reference form the preview keeps (links)
        // meeting the scheme gate: ammonia's `url_schemes` default has
        // no `javascript`, so the href is dropped and nothing
        // executable remains.
        let html = preview("[click](javascript:alert(1))").html;

        // Assert
        assert!(
            !html.contains("javascript") && !html.contains("alert"),
            "the hostile scheme must not survive anywhere: {html}"
        );
        assert!(
            !html.contains("href="),
            "no href may survive the scheme gate: {html}"
        );
        assert!(
            html.contains(">click</a>"),
            "the link text survives: {html}"
        );
    }

    #[test]
    fn a_data_link_is_reduced_to_its_text() {
        // Arrange: the `data:` smuggler — also outside ammonia's
        // default scheme set, and the classic way to carry a script
        // payload inside an href (base64 here, so the payload is one
        // opaque token the output must not contain).
        let html = preview("[open](data:text/html;base64,PHNjcmlwdD4=)").html;

        // Assert
        assert!(
            !html.contains("data:") && !html.contains("PHNjcmlwdD4="),
            "the data scheme and its payload must not survive: {html}"
        );
        assert!(
            !html.contains("href="),
            "no href may survive the scheme gate: {html}"
        );
        assert!(html.contains(">open</a>"), "the link text survives: {html}");
    }

    #[test]
    fn a_link_keeps_its_href_with_the_noopener_rel() {
        // Arrange: links are the one external reference the preview
        // keeps — inert on their own (nothing loads until clicked).
        let html = preview("[docs](https://example.com/guide)").html;

        // Assert: the href survives with the forced `rel` (ammonia's
        // `link_rel` default) — the pieces, not the attribute order,
        // are pinned.
        assert!(
            html.contains("href=\"https://example.com/guide\""),
            "the link target survives: {html}"
        );
        assert!(
            html.contains("rel=\"noopener noreferrer\""),
            "every link carries the noopener rel: {html}"
        );
        assert!(html.contains(">docs</a>"), "the link text survives: {html}");
    }

    #[test]
    fn a_task_list_checkbox_survives_with_only_its_safe_attributes() {
        // Arrange & Act: the TASKLISTS extension's one non-default tag.
        let html = preview("- [x] done\n- [ ] todo\n").html;

        // Assert: checkboxes render, and nothing beyond the three
        // emitted attributes could have survived even if the source
        // tried to add more (the `type` value is pinned to `checkbox`).
        assert!(html.contains("<input"), "the checkbox renders: {html}");
        assert!(
            html.contains("type=\"checkbox\""),
            "the checkbox type is kept: {html}"
        );
        assert!(
            html.contains("disabled"),
            "the checkbox is disabled: {html}"
        );
        assert!(
            html.contains("checked"),
            "the checked state is kept: {html}"
        );
        assert!(
            html.contains("done") && html.contains("todo"),
            "items text: {html}"
        );
    }

    #[test]
    fn the_sanitizer_itself_strips_hostile_markup() {
        // Arrange: fed directly (bypassing the escaping transform) —
        // the gate's own policy must hold against markup that never
        // went through pulldown-cmark, e.g. if a future caller handed
        // it a pre-built HTML string. Covers every policy lever the
        // gate documents: script/style content dropping, img removal,
        // event-handler stripping, comment stripping, the URL-scheme
        // gate (`javascript:` and the `data:` smuggler), a
        // non-allowlisted element (iframe), and the checkbox carve-out's
        // negative half — a non-checkbox `type` is dropped, so no
        // useful `<input>` survives this gate.
        let hostile = "<script>alert(1)</script><img src=x onerror=alert(1)>\
<p onclick=\"x\">hi</p><!-- c --><a href=\"javascript:alert(1)\">link</a>\
<a href=\"data:text/html,<script>alert(2)</script>\">d</a>\
<iframe src=\"https://evil.example/frame\"></iframe>\
<input type=\"text\" value=\"smuggled\">";

        // Act
        let cleaned = sanitize(hostile.to_owned());

        // Assert: nothing executable, loadable or smuggled survives —
        // the script payloads and the hostile hrefs (`javascript:` and
        // the `data:` smuggler) are gone with their attributes, the
        // iframe is gone, the input's `type="text"` (outside the
        // value-pinned carve-out) and its `value` are dropped — while
        // the harmless text, its paragraph and the href-less anchors
        // (with ammonia's forced `rel`, the crate's every-anchor
        // default) are kept.
        assert_eq!(
            cleaned,
            "<p>hi</p>\
<a rel=\"noopener noreferrer\">link</a>\
<a rel=\"noopener noreferrer\">d</a>\
<input>",
            "the gate: {cleaned}"
        );
        assert!(
            !cleaned.contains("alert") && !cleaned.contains("javascript"),
            "no script payload or hostile scheme: {cleaned}"
        );
        assert!(
            !cleaned.contains("evil.example") && !cleaned.contains("iframe"),
            "no embeddable frame: {cleaned}"
        );
        assert!(
            !cleaned.contains("type=") && !cleaned.contains("smuggled"),
            "the non-checkbox input carries nothing: {cleaned}"
        );
    }

    #[test]
    fn empty_text_renders_to_empty_html_with_no_warnings() {
        // Arrange & Act & Assert: the §6.9-empty variant text is a
        // real input (LIB-002's empty variant), and the honest preview
        // of nothing is nothing — no HTML, no warnings.
        assert_eq!(
            preview(""),
            Preview {
                html: String::new(),
                warnings: Vec::new(),
            }
        );
    }

    // ------------------------------------------------------------------
    // The warnings (T-057, D-15, §7.3 LIB-002/009…016)
    // ------------------------------------------------------------------

    #[test]
    fn an_unclosed_code_fence_warns_at_its_opening_line() {
        // Arrange: the acceptance pair's warning half — a fence opened
        // after a paragraph and never closed; everything after it is
        // code. The warning points at the OPENING fence's line (line
        // 3: one-based, over the raw text), which is what the author
        // fixes.
        let text = "para\n\n```\ncode that never closes\n";

        // Act
        let result = preview(text);

        // Assert: exactly one warning of the spec's named kind, at the
        // opening line — and the preview still renders the swallowed
        // text as a code block (warnings do not dim the render).
        assert_eq!(
            result.warnings,
            vec![MarkdownWarning {
                kind: MarkdownWarningKind::UnclosedCodeFence,
                line: 3,
            }],
            "the unclosed fence must warn exactly once at its opening line"
        );
        assert!(
            result.html.contains("<pre><code>") && result.html.contains("code that never closes"),
            "the swallowed text still renders as a code block: {}",
            result.html
        );
    }

    #[test]
    fn clean_text_produces_no_warnings() {
        // Arrange: the acceptance pair's clean half — ordinary
        // Markdown including a CLOSED fence, the shape §7.3's editor
        // shows without a warning.
        let text = "# Guide\n\nSome *point* and **strong**.\n\n```rust\nlet x = 1;\n```\n\nDone.\n";

        // Act & Assert
        assert!(
            preview(text).warnings.is_empty(),
            "closed fences and ordinary Markdown must not warn"
        );
    }

    #[test]
    fn an_unclosed_fence_at_eof_without_a_trailing_newline_also_warns() {
        // Arrange: the file simply ends inside the fence — the most
        // common accidental shape.
        let result = preview("```\ncode");

        // Assert
        assert_eq!(
            result.warnings,
            vec![MarkdownWarning {
                kind: MarkdownWarningKind::UnclosedCodeFence,
                line: 1,
            }],
            "EOF inside the fence is the same warning: {:?}",
            result.warnings
        );
    }

    #[test]
    fn a_shorter_or_foreign_closer_does_not_close_the_fence() {
        // Arrange: the closing-fence rules are pulldown-cmark's, and
        // the tracker inherits them: a line of too few fence
        // characters ("``" cannot close a 3-backtick fence) stays
        // content, as does a line of the wrong character ("```" cannot
        // close a tilde fence) — both keep the block unclosed.
        let short = preview("```\n``\n");
        let foreign = preview("~~~\n```\n");

        // Assert: both warn, at the opening line.
        let expected = MarkdownWarning {
            kind: MarkdownWarningKind::UnclosedCodeFence,
            line: 1,
        };
        assert_eq!(
            short.warnings,
            vec![expected],
            "short closer: {:?}",
            short.warnings
        );
        assert_eq!(
            foreign.warnings,
            vec![expected],
            "foreign closer: {:?}",
            foreign.warnings
        );
    }

    #[test]
    fn an_unclosed_tilde_fence_warns_the_same_way() {
        // Arrange & Act & Assert: the fence character is not the
        // point — an open block of any fence character warns.
        assert_eq!(
            preview("~~~\ncode\n").warnings,
            vec![MarkdownWarning {
                kind: MarkdownWarningKind::UnclosedCodeFence,
                line: 1,
            }]
        );
    }

    #[test]
    fn bare_and_one_line_fence_forms_are_decided_by_their_source() {
        // Arrange: the degenerate shapes that emit no content events
        // at all. `"```"` and `"```\n"` hold nothing but the opening
        // fence line — unclosed; `"```\n```"` and `"```\n```\n"` hold
        // the closer line after it — closed.
        let bare = preview("```");
        let bare_nl = preview("```\n");
        let closed = preview("```\n```");
        let closed_nl = preview("```\n```\n");
        let expected = MarkdownWarning {
            kind: MarkdownWarningKind::UnclosedCodeFence,
            line: 1,
        };

        // Assert
        assert_eq!(
            bare.warnings,
            vec![expected],
            "bare ```: {:?}",
            bare.warnings
        );
        assert_eq!(
            bare_nl.warnings,
            vec![expected],
            "bare ``` with a newline: {:?}",
            bare_nl.warnings
        );
        assert!(
            closed.warnings.is_empty(),
            "``` then ``` closes: {:?}",
            closed.warnings
        );
        assert!(
            closed_nl.warnings.is_empty(),
            "``` then ``` then newline closes: {:?}",
            closed_nl.warnings
        );
    }

    #[test]
    fn a_fence_broken_by_its_container_still_warns() {
        // Arrange: the unclosed fence inside a list item or a
        // blockquote does not reach end-of-input — the enclosing
        // container breaks first (a new item, or the quote ending) and
        // pulldown-cmark ends the block there, with no closer ever
        // consumed. The content runs to the block's end, the same
        // signature as EOF, and the warning fires at the opening line.
        let in_list = preview("- ```\n  code\n\n- next\n");
        let in_quote = preview("> ```\n> code\n\npara\n");
        // The opposite, more dangerous half: CLOSED fences inside
        // containers must stay quiet — this is exactly where an
        // offsets-based invariant would misfire if a consumed closer
        // stopped leaving its non-whitespace gap.
        let closed_in_list = preview("- ```\n  a\n  ```\n");
        let closed_in_quote = preview("> ```\n> a\n> ```\n");

        // Assert
        let expected = MarkdownWarning {
            kind: MarkdownWarningKind::UnclosedCodeFence,
            line: 1,
        };
        assert_eq!(
            in_list.warnings,
            vec![expected],
            "list-item container break: {:?}",
            in_list.warnings
        );
        assert_eq!(
            in_quote.warnings,
            vec![expected],
            "blockquote container break: {:?}",
            in_quote.warnings
        );
        assert!(
            closed_in_list.warnings.is_empty(),
            "a closed fence in a list item must not warn: {:?}",
            closed_in_list.warnings
        );
        assert!(
            closed_in_quote.warnings.is_empty(),
            "a closed fence in a blockquote must not warn: {:?}",
            closed_in_quote.warnings
        );
    }

    #[test]
    fn a_whitespace_only_tail_line_does_not_close_and_still_warns() {
        // Arrange: the round-1 review catch — pulldown-cmark emits NO
        // content event for a final non-newline-terminated line of
        // 1–3 spaces (a synthesized zero-length item), so the content
        // stops short of the block's end with no closer consumed: the
        // gap form of the invariant (the gap holds no fence) is what
        // keeps this an unclosed verdict. The tab, 4-space and
        // trailing-newline shapes exercise the boundary of the
        // synthesized-item behavior; the closed fence with trailing
        // whitespace after the closer pins the no-false-warning half.
        let spaces = preview("```\na\n   ");
        let one_space = preview("```\na\n ");
        let with_newline = preview("```\na\n   \n");
        let tab = preview("```\na\n\t");
        let four_spaces = preview("```\na\n    ");
        let closed_trailing_ws = preview("```\na\n```   ");
        let expected = MarkdownWarning {
            kind: MarkdownWarningKind::UnclosedCodeFence,
            line: 1,
        };

        // Assert
        assert_eq!(
            spaces.warnings,
            vec![expected],
            "3-space tail: {:?}",
            spaces.warnings
        );
        assert_eq!(
            one_space.warnings,
            vec![expected],
            "1-space tail: {:?}",
            one_space.warnings
        );
        assert_eq!(
            with_newline.warnings,
            vec![expected],
            "whitespace tail with a newline: {:?}",
            with_newline.warnings
        );
        assert_eq!(tab.warnings, vec![expected], "tab tail: {:?}", tab.warnings);
        assert_eq!(
            four_spaces.warnings,
            vec![expected],
            "4-space tail: {:?}",
            four_spaces.warnings
        );
        assert!(
            closed_trailing_ws.warnings.is_empty(),
            "a closed fence with trailing whitespace after the closer must not warn: {:?}",
            closed_trailing_ws.warnings
        );
    }

    #[test]
    fn multiple_unclosed_fences_warn_in_source_order() {
        // Arrange: two unclosed fences — a blockquote one broken by the
        // quote's end, then a top-level one running to end-of-input.
        // The response is ordered by position in the source (the parse
        // order), each pointing at its own opening line.
        let result = preview("> ```\n> a\n```\n");

        // Assert
        assert_eq!(
            result.warnings,
            vec![
                MarkdownWarning {
                    kind: MarkdownWarningKind::UnclosedCodeFence,
                    line: 1,
                },
                MarkdownWarning {
                    kind: MarkdownWarningKind::UnclosedCodeFence,
                    line: 3,
                },
            ],
            "both fences, in source order: {:?}",
            result.warnings
        );
    }

    #[test]
    fn the_warning_kind_wire_literal_is_pinned() {
        // Arrange: the kind's wire string is a contract for the editor
        // UI (the errors.rs rule — the literal is the API, the variant
        // name is not), so the serde spelling is asserted directly
        // rather than only through the regenerated bindings.
        let wire = serde_json::to_value(MarkdownWarningKind::UnclosedCodeFence)
            .expect("serializing a warning kind must work");

        // Assert
        assert_eq!(
            wire,
            serde_json::json!("unclosed_code_fence"),
            "the wire literal must stay `unclosed_code_fence`"
        );
    }

    #[test]
    fn an_indented_code_block_is_not_a_fence_and_never_warns() {
        // Arrange: four spaces make an indented code block — even when
        // the lines look like fences (CommonMark: the fence is only a
        // fence outside indented-code context); a warning here would
        // be a false alarm about working text.
        let indented_text = preview("para\n\n    indented code\n");
        let indented_fence = preview("    ```\n    c\n");

        // Assert
        assert!(
            indented_text.warnings.is_empty(),
            "indented code is not a fence: {:?}",
            indented_text.warnings
        );
        assert!(
            indented_fence.warnings.is_empty(),
            "a 4-space ``` line is indented content, not a fence: {:?}",
            indented_fence.warnings
        );
    }

    #[test]
    fn crlf_input_warns_and_stays_quiet_exactly_like_lf() {
        // Arrange: the editor may hold CRLF (§6.9 normalizes only on
        // save); inside the fence the content arrives as split text
        // pieces, which the furthest-end tracking must survive — and a
        // closed CRLF fence must stay quiet.
        let unclosed = preview("```\r\ncode\r\n");
        let closed = preview("```\r\ncode\r\n```\r\n");
        let expected = MarkdownWarning {
            kind: MarkdownWarningKind::UnclosedCodeFence,
            line: 1,
        };

        // Assert
        assert_eq!(
            unclosed.warnings,
            vec![expected],
            "CRLF: {:?}",
            unclosed.warnings
        );
        assert!(
            closed.warnings.is_empty(),
            "closed CRLF fence: {:?}",
            closed.warnings
        );
    }

    #[test]
    fn one_warning_per_unclosed_fence_wherever_it_opens() {
        // Arrange: a closed fence first (quiet), then an unclosed one
        // opening on line 7; however much text the open fence swallows
        // (paragraphs, blank lines), it is still ONE fence and ONE
        // warning — the line is the opening fence's, not the swallow
        // site's.
        let text = "```\na\n```\n\npara\n\n```\nswallowed\n\nmore\n";
        let result = preview(text);

        // Assert
        assert_eq!(
            result.warnings,
            vec![MarkdownWarning {
                kind: MarkdownWarningKind::UnclosedCodeFence,
                line: 7,
            }],
            "the second fence, at its opening line 7: {:?}",
            result.warnings
        );
    }

    #[test]
    fn warnings_and_html_are_independent_outputs() {
        // Arrange: an unclosed fence whose swallowed content holds
        // markup-shaped text — the warning rides ALONGSIDE the
        // pipeline, never into it: the HTML must be byte-for-byte the
        // render D-083 pinned (raw HTML inside code is escaped display
        // text; nothing leaks as markup), and the taxonomy's wire
        // spelling is pinned by exact equality here too.
        let result = preview("```\nc <here> <b>bold</b>\n");

        // Assert: the exact sanitized render — content escaped, no
        // markup from the swallowed text — and the one warning.
        assert_eq!(
            result.html, "<pre><code>c &lt;here&gt; &lt;b&gt;bold&lt;/b&gt;\n</code></pre>\n",
            "the HTML path is unchanged by the warnings work: {}",
            result.html
        );
        assert_eq!(
            result.warnings,
            vec![MarkdownWarning {
                kind: MarkdownWarningKind::UnclosedCodeFence,
                line: 1,
            }],
            "the warning still fires alongside: {:?}",
            result.warnings
        );
    }
}
