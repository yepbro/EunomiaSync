//! The Markdown preview renderer (spec §7.10 Library module
//! `markdown_preview`, §5.3 "Markdown: `pulldown-cmark` — Preview and
//! warnings. Not used when building files", SEC-005, D-15, D-009).
//!
//! [`preview_html`] is the whole pipeline, in order:
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

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html::push_html};

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

/// Renders Markdown text into sanitized preview HTML (§7.10 Library
/// module `markdown_preview`, SEC-005) — the pipeline in the module
/// docs. Total for any input: parsing never fails, and empty text
/// renders to an empty string.
pub(crate) fn preview_html(text: &str) -> String {
    let mut html = String::with_capacity(text.len());
    let parser = Parser::new_ext(text, RENDER_OPTIONS);
    let neutralized = parser.filter_map(|event| match event {
        // Raw HTML from the text — whole blocks and inline tags:
        // re-typed as plain text so the renderer escapes it into a
        // visible literal (SEC-005). Ammonia would *strip* this markup
        // rather than escape it, which is why the transform happens
        // here, before rendering.
        Event::Html(markup) | Event::InlineHtml(markup) => Some(Event::Text(markup)),
        // Markdown image syntax `![alt](url)`: the element is dropped;
        // the parser's inner content (the alt text) is kept, so the
        // author sees *what* the image claimed to be but no `img`
        // element — and with it no `src` — is ever generated
        // (SEC-005: external resources are not loaded).
        Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => None,
        event => Some(event),
    });
    push_html(&mut html, neutralized);
    sanitize(html)
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
    //! [`preview_html`] — what the command's caller observes: hostile
    //! markup never survives as markup, raw HTML survives as escaped
    //! *text*, and ordinary Markdown demonstrably renders. The one
    //! exception drives [`sanitize`] directly: the sanitizer is the
    //! second gate, and its policy must hold even against input that
    //! the escaping transform would have neutralized first (defense in
    //! depth is pinned per gate, so removing one gate cannot silently
    //! disarm the other).

    use super::*;

    #[test]
    fn a_script_block_from_the_text_is_escaped_not_executable() {
        // Arrange: the `<script>` half of the acceptance pair, in both
        // shapes the parser can emit — an HTML block (its own line)
        // and an inline tag inside a paragraph.
        let text = "<script>alert('xss')</script>\n\ninline <script>alert(1)</script> tag";

        // Act
        let html = preview_html(text);

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
        let html = preview_html("![logo](https://tracker.example/pixel.png)");

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
        let html = preview_html(text);

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
        let html = preview_html("see <b>bold</b> words");

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
        assert_eq!(preview_html("# Title"), "<h1>Title</h1>\n");
    }

    #[test]
    fn emphasis_and_strong_renders() {
        // Arrange & Act & Assert
        let html = preview_html("*point* and **strong**");

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
        let html = preview_html("```rust\nlet s = \"<here> & more\";\n```");

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
        let html = preview_html("[click](javascript:alert(1))");

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
        let html = preview_html("[open](data:text/html;base64,PHNjcmlwdD4=)");

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
        let html = preview_html("[docs](https://example.com/guide)");

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
        let html = preview_html("- [x] done\n- [ ] todo\n");

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
    fn empty_text_renders_to_empty_html() {
        // Arrange & Act & Assert: the §6.9-empty variant text is a
        // real input (LIB-002's empty variant), and the honest preview
        // of nothing is nothing.
        assert_eq!(preview_html(""), "");
    }
}
