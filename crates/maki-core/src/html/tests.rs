use super::assets::{
    DEFAULT_CSS, EXTERNAL_LINKS_SCRIPT, PROJECT_NAVIGATION_HTML, SEARCH_SCRIPT, TOC_SCRIPT,
};
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

== [home.maki]

[home.maki]: /home"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains("<h3 id=\"[home.maki]\"><a href=\"/home\">home.maki</a></h3>"));
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
fn render_ordered_schedule_with_nested_lists() {
    let parsed = parser::parse(
        r#"1. [2026-09-04 Fri 14:30]
   - 시간
     - 14:30 국립중앙박물관 어메이징 타일랜드
     - 15:30 국립중앙박물관 한국인의 밥상
     - 16:30 저녁
   - 현장 결제
     - [ ] 국립중앙박물관 어메이징 타일랜드 전시 (13,000 원)
     - [ ] 국립중앙박물관 한국인의 밥상 (13,000 원)
   - 비고
     - 파란색을 입고 와야함"#,
    );

    let html = render_document_with_context(
        &parsed.document,
        RenderContext::default().with_date_source_path(std::path::Path::new("index.maki")),
    );
    let body = html.split_once("</head>").unwrap().1;

    assert!(
        body.starts_with(
            "<body><ol><li><a class=\"maki-date-location maki-date-stamp maki-date-stamp-reference\" id=\"date-inline-index-maki-1\" href=\"/@/dates/2026-09-04#date-inline-index-maki-1\">[2026-09-04 Fri 14:30]</a><ul><li>시간<ul><li>14:30 국립중앙박물관"
        ),
        "{body}"
    );
    assert!(!body.contains("<li><span class=\"maki-date-location\""));
    assert!(body.contains(
        "<li class=\"maki-todo-item\" data-todo-state=\"todo\"><input class=\"maki-todo-checkbox\""
    ));
}

#[test]
fn render_todo_list_items_as_disabled_checkboxes() {
    let parsed = parser::parse(
        r#"- [ ] todo
- [x] done
- ordinary"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains(
        "<li class=\"maki-todo-item\" data-todo-state=\"todo\"><input class=\"maki-todo-checkbox\" type=\"checkbox\" disabled aria-label=\"todo\">todo</li>"
    ));
    assert!(html.contains(
        "<li class=\"maki-todo-item\" data-todo-state=\"done\"><input class=\"maki-todo-checkbox\" type=\"checkbox\" disabled checked aria-label=\"done\">done</li>"
    ));
    assert!(html.contains("<li>ordinary</li>"));
}

#[test]
fn render_table_with_inline_cells_and_numeric_alignment() {
    let parsed = parser::parse(
        r#"| 이름 | 점수 |
|---+---|
| `Alice` | 10 |
| [Bob] | 2 |

[Bob]: /bob"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains(
            "<table><thead><tr><th scope=\"col\">이름</th><th class=\"maki-table-number\" scope=\"col\">점수</th></tr></thead><tbody><tr><td><code>Alice</code></td><td class=\"maki-table-number\">10</td></tr><tr><td><a href=\"/bob\">Bob</a></td><td class=\"maki-table-number\">2</td></tr></tbody></table>"
        ));
}

#[test]
fn render_stable_inline_and_footnote_syntax() {
    let parsed = parser::parse(
        r#"Use /italic/, *strong*, ^{sup}, _{sub}, +{inserted}, -{deleted}, and ::highlight:: with <https://example.com>.[^note]

[^note]: Footnote *body*."#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains("<em>italic</em>"));
    assert!(html.contains("<strong>strong</strong>"));
    assert!(html.contains("<sup>sup</sup>"));
    assert!(html.contains("<sub>sub</sub>"));
    assert!(html.contains("<ins>inserted</ins>"));
    assert!(html.contains("<del>deleted</del>"));
    assert!(html.contains("<mark>highlight</mark>"));
    assert!(html.contains(
        "<a class=\"external-link\" href=\"https://example.com\">https://example.com</a>"
    ));
    assert!(html.contains("<sup class=\"footnote-ref\"><a href=\"#fn-note\">[note]</a></sup>"));
    assert!(html.contains("<section class=\"footnotes\"><ol><li id=\"fn-note\">Footnote <strong>body</strong>.</li></ol></section>"));
    assert!(!html.contains("[^note]:"));
}

#[test]
fn render_inline_backslash_escape_without_the_backslash() {
    let parsed = parser::parse(r"Literal \[text], \*stars\*, and \::marks\::.");
    let html = render_document(&parsed.document);

    assert!(html.contains("<p>Literal [text], *stars*, and ::marks::.</p>"));
    assert!(!html.contains("<strong>"));
    assert!(!html.contains("<mark>"));
}

#[test]
fn external_link_favicon_css_uses_the_script_state_class() {
    assert!(DEFAULT_CSS.contains("a.maki-external-link-has-favicon >"));
    assert!(DEFAULT_CSS.contains("a.maki-external-link-has-favicon::after"));
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
        "{PROJECT_NAVIGATION_HTML}<script>{EXTERNAL_LINKS_SCRIPT}</script><script>{SEARCH_SCRIPT}</script><script>{TOC_SCRIPT}</script><h1"
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
        "<script src=\"{EXTERNAL_LINKS_SCRIPT_ASSET_PATH}\"></script>"
    )));
    assert!(html.contains(&format!(
        "<script src=\"{SEARCH_SCRIPT_ASSET_PATH}\"></script>"
    )));
    assert!(html.contains(&format!(
        "<script src=\"{TOC_SCRIPT_ASSET_PATH}\"></script>"
    )));
    assert!(!html.contains("<style>:root"));
    assert!(!html.contains(EXTERNAL_LINKS_SCRIPT));
    assert!(!html.contains(SEARCH_SCRIPT));
    assert!(!html.contains(TOC_SCRIPT));
}

#[test]
fn project_rendering_can_suffix_browser_title_with_site_title() {
    let parsed = parser::parse("--^ title: Page\n\nbody");
    let resolve_note_link = |_target: &str| NoteLinkResolution::Broken;
    let get_note_info = |_note_ref: &NoteRef| None;

    let html = render_document_with_context(
        &parsed.document,
        RenderContext::project(&resolve_note_link, &get_note_info)
            .with_site_title(Some("Maki & Co")),
    );

    assert!(html.contains("<title>Page | Maki &amp; Co</title>"));
    assert!(html.contains("<h1 id=\"Page\">Page</h1>"));
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
fn nested_maki_blocks_resolve_their_reference_definitions() {
    let parsed = parser::parse(
        r#"> [quoted]
> [quoted]: https://quoted.example
> [outer]
> > [outer]

--- quote
[contained]
[contained]: https://contained.example
[outer]
---

[outer]: https://outer.example"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains("href=\"https://quoted.example\">quoted</a>"));
    assert!(html.contains("href=\"https://contained.example\">contained</a>"));
    assert_eq!(
        html.matches("href=\"https://outer.example\">outer</a>")
            .count(),
        3
    );
}

#[test]
fn quote_raw_modes_and_container_properties_are_rendered() {
    let parsed = parser::parse(
        r#"--v mode: pre
> = Raw heading
> [site]

--v lang: rust
---code
fn main() {}
---

--v mode: text
---quote
= Raw container heading
[site]
---

--v mode: block
> = Parsed heading
> [site]

[site]: https://example.com"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains("<blockquote><pre>= Raw heading\n[site]</pre></blockquote>"));
    assert!(html.contains("<pre><code class=\"language-rust\">fn main() {}</code></pre>"));
    assert!(html.contains(
        "<blockquote><div class=\"maki-quote-text\">= Raw container heading\n[site]</div></blockquote>"
    ));
    assert!(html.contains("<blockquote><h2 id=\"Parsed heading\">Parsed heading</h2>"));
    assert_eq!(html.matches("href=\"https://example.com\"").count(), 1);
}

#[test]
fn quote_text_mode_css_preserves_newlines_and_wraps_long_words() {
    let rule = DEFAULT_CSS
        .split(".maki-quote-text {")
        .nth(1)
        .and_then(|css| css.split('}').next())
        .unwrap();

    assert!(rule.contains("white-space: pre-wrap"));
    assert!(rule.contains("overflow-wrap: anywhere"));
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
