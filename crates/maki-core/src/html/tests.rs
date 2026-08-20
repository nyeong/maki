use super::assets::{PROJECT_NAVIGATION_HTML, SEARCH_SCRIPT, TOC_SCRIPT};
use super::pages::format_unix_seconds_kst;
use super::*;
use crate::{
    maki::{NoteLinkResolution, NoteRef},
    parser,
};

#[test]
fn test_render_document() {
    let parsed = parser::parse(
        r#"--^ title: Maki

= Heading

hello <maki> & friends

--v lang: html
: <main>
: </main>

- one
- two

1. first
2. second"#,
    );

    let html = render_document(&parsed.document);

    assert!(
        html.contains("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">")
    );
    assert!(html.contains("<title>Maki</title>"));
    assert!(html.contains("<h2"));
    assert!(html.contains("<p>hello &lt;maki&gt; &amp; friends</p>"));
    assert!(
        html.contains(
            "<pre><code class=\"language-html\">&lt;main&gt;\n&lt;/main&gt;</code></pre>"
        )
    );
    assert!(html.contains("<ul><li>one</li><li>two</li></ul>"));
    assert!(html.contains("<ol><li>first</li><li>second</li></ol>"));
}

#[test]
fn test_render_heading_supports_inline_links() {
    let parsed = parser::parse(
        r#"--^ title: Diagnostics

== [home.maki](/home)"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains("<h3 id=\"[home.maki](/home)\"><a href=\"/home\">home.maki</a></h3>"));
}

#[test]
fn render_star_delimited_strong_text() {
    let parsed = parser::parse("This is *bold & `code`*.");

    let html = render_document(&parsed.document);

    assert!(html.contains("<p>This is <strong>bold &amp; <code>code</code></strong>.</p>"));
}

#[test]
fn render_ordered_list_with_child_paragraph() {
    let parsed = parser::parse(
        r#"1. Glider 활용 증진

   현재 Glider의 CloudData를 더 넓게 활용합니다."#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains(
        "<ol><li>Glider 활용 증진<p>현재 Glider의 CloudData를 더 넓게 활용합니다.</p></li></ol>"
    ));
}

#[test]
fn render_table_with_inline_cells_and_numeric_alignment() {
    let parsed = parser::parse(
        r#"| 이름 | 점수 |
|---+---|
| `Alice` | 10 |
| [Bob](/bob) | 2 |"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains(
            "<table><thead><tr><th scope=\"col\">이름</th><th class=\"maki-table-number\" scope=\"col\">점수</th></tr></thead><tbody><tr><td><code>Alice</code></td><td class=\"maki-table-number\">10</td></tr><tr><td><a href=\"/bob\">Bob</a></td><td class=\"maki-table-number\">2</td></tr></tbody></table>"
        ));
}

#[test]
fn render_table_with_middle_separator() {
    let parsed = parser::parse(
        r#"| 일시 | 시간 |
|---+---|
| [2025-11-05 Wed] | 5H |
|---+---|
| [2026-04-04 Sat] | 5H |"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains(
        "<tr class=\"maki-table-separator\" aria-hidden=\"true\"><td colspan=\"2\"></td></tr>"
    ));
    assert!(html.contains("2026-04-04 Sat"));
}

#[test]
fn date_ranges_render_separator_as_en_dash() {
    let parsed = parser::parse(
        r#"References [2026-08-17]--[2026-08-19].
Events <2026-08-20>--<2026-08-22>."#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains("[2026-08-17]&ndash;[2026-08-19]"));
    assert!(html.contains("<2026-08-20>&ndash;<2026-08-22>"));
}

#[test]
fn project_rendering_includes_home_navigation() {
    let parsed = parser::parse("--^ title: Page\n\nbody");
    let resolve_note_link = |_target: &str| NoteLinkResolution::Broken;
    let get_note_info = |_note_ref: &NoteRef| None;

    let html = render_document_with_context(
        &parsed.document,
        RenderContext::project(&resolve_note_link, &get_note_info),
    );

    assert!(html.contains(&format!(
        "{PROJECT_NAVIGATION_HTML}<script>{SEARCH_SCRIPT}</script><script>{TOC_SCRIPT}</script><h1"
    )));
}

#[test]
fn project_rendering_can_use_external_assets() {
    let parsed = parser::parse("--^ title: Page\n\nbody");
    let resolve_note_link = |_target: &str| NoteLinkResolution::Broken;
    let get_note_info = |_note_ref: &NoteRef| None;

    let html = render_document_with_context(
        &parsed.document,
        RenderContext::project(&resolve_note_link, &get_note_info)
            .with_asset_mode(AssetMode::External),
    );

    assert!(html.contains(&format!(
        "<link rel=\"stylesheet\" href=\"{CSS_ASSET_PATH}\">"
    )));
    assert!(html.contains(&format!(
        "<script src=\"{SEARCH_SCRIPT_ASSET_PATH}\"></script>"
    )));
    assert!(html.contains(&format!(
        "<script src=\"{TOC_SCRIPT_ASSET_PATH}\"></script>"
    )));
    assert!(!html.contains("<style>:root"));
    assert!(!html.contains(SEARCH_SCRIPT));
    assert!(!html.contains(TOC_SCRIPT));
}

#[test]
fn format_unix_seconds_kst_formats_known_instants() {
    assert_eq!(format_unix_seconds_kst(0), "1970-01-01 09:00 KST");
    assert_eq!(format_unix_seconds_kst(951_782_400), "2000-02-29 09:00 KST");
    assert_eq!(
        format_unix_seconds_kst(1_704_067_199),
        "2024-01-01 08:59 KST"
    );
}

#[test]
fn test_render_builtin_containers() {
    let parsed = parser::parse(
        r#"--- code rust
fn main() {}
---

--- text
line <one>
line two
---

--- quote
= Quoted

quote body
---"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains("<pre><code class=\"language-rust\">fn main() {}</code></pre>"));
    assert!(html.contains("<pre>line &lt;one&gt;\nline two</pre>"));
    assert!(
        html.contains("<blockquote><h2 id=\"Quoted\">Quoted</h2><p>quote body</p></blockquote>")
    );
}

#[test]
fn quote_line_renders_inner_maki_blocks() {
    let parsed = parser::parse(
        r#"> = Quoted
>
> Body with `code`
> - item
> > nested"#,
    );

    let html = render_document(&parsed.document);
    let expected = "<blockquote><h2 id=\"Quoted\">Quoted</h2><p>Body with <code>code</code></p><ul><li>item</li></ul><blockquote><p>nested</p></blockquote></blockquote>";

    assert!(html.contains(expected));
}

#[test]
fn nested_unordered_list() {
    let source = r#"- first
  - second
  - second-sibling
    - third
    - third-sibling
  - fourth but second depth

- another list"#;

    let parsed = parser::parse(source);
    let html = render_document(&parsed.document);

    assert!(html.contains(
            "<ul><li>first<ul><li>second</li><li>second-sibling<ul><li>third</li><li>third-sibling</li></ul></li><li>fourth but second depth</li></ul></li></ul><ul><li>another list</li></ul>"
        ));
}

#[test]
fn test_render_ordered_list() {
    let parsed = parser::parse(
        r#"1. 블록에 property를 붙일 수 있음
2. 쿼리를 통해 검색할 수 있음
3. 컴파일, 서빙을 통해 다른 포맷이나 서비스에 붙일 수 있음"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains("<ol><li>블록에 property를 붙일 수 있음</li><li>쿼리를 통해 검색할 수 있음</li><li>컴파일, 서빙을 통해 다른 포맷이나 서비스에 붙일 수 있음</li></ol>"));
}

#[test]
fn list_items_render_indented_code_children_inside_list() {
    let parsed = parser::parse(
        r#"- unordered
  : quoted <text>
  : second line

1. ordered
   : ordered text"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains(
        "<ul><li>unordered<pre><code>quoted &lt;text&gt;\nsecond line</code></pre></li></ul>"
    ));
    assert!(html.contains("<ol><li>ordered<pre><code>ordered text</code></pre></li></ol>"));
}
