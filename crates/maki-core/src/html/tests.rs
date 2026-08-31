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
        r#"Use /italic/, *strong*, ^{sup}, _{sub}, +{inserted}, -{deleted}, and ::highlight:: with <https://example.com> and <http://example.com/docs>.[^note]

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
    assert!(
        html.contains("<a class=\"external-link\" href=\"https://example.com\">example.com</a>")
    );
    assert!(html.contains(
        "<a class=\"external-link\" href=\"http://example.com/docs\">example.com/docs</a>"
    ));
    assert!(html.contains(
        "<a class=\"maki-reference-use maki-reference-footnote\" id=\"maki-reference-use-1-1\" href=\"#maki-reference-note-1\" role=\"doc-noteref\">note<sup class=\"maki-reference-number\">1</sup></a>"
    ));
    assert!(html.contains(
        "<section class=\"maki-reference-notes\" aria-labelledby=\"maki-reference-notes-title\"><h2 class=\"maki-reference-notes-title\" id=\"maki-reference-notes-title\">Notes</h2><ol><li id=\"maki-reference-note-1\" tabindex=\"-1\">Footnote <strong>body</strong>.<span class=\"maki-reference-backlinks\"><a class=\"maki-reference-backlink\" href=\"#maki-reference-use-1-1\" aria-label=\"Back to note reference 1\">&#8617;</a></span></li></ol></section>"
    ));
    assert!(!html.contains("[^note]:"));
}

#[test]
fn reference_value_shape_selects_link_or_annotation_presentation() {
    let parsed = parser::parse(
        r#"[site], [description], and [explicit path].

[site]: https://example.com/search?q=maki
[description]: Famous *search* portal & directory
[explicit path]: /notes/path with spaces
[unused]: Unused prose reference"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains(
        "<a class=\"external-link\" href=\"https://example.com/search?q=maki\">site</a>"
    ));
    assert!(html.contains("<a href=\"/notes/path with spaces\">explicit path</a>"));
    assert!(html.contains(
        "<a class=\"maki-reference-use maki-reference-annotation\" id=\"maki-reference-use-1-1\" href=\"#maki-reference-note-1\" role=\"doc-noteref\">description<sup class=\"maki-reference-number\">1</sup></a>"
    ));
    assert!(html.contains(
        "<li id=\"maki-reference-note-1\" tabindex=\"-1\">Famous <strong>search</strong> portal &amp; directory"
    ));
    assert_eq!(html.matches("<li id=\"maki-reference-note-").count(), 1);
    assert!(!html.contains("Unused prose reference"));
}

#[test]
fn reference_notes_follow_first_use_and_share_occurrence_backlinks() {
    let parsed = parser::parse(
        r#"[^source] [details] [^details] [details]

[details]: Detailed *prose* & context
[source]: https://example.com/?a=1&b=2"#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains(
        "id=\"maki-reference-use-1-1\" href=\"#maki-reference-note-1\" role=\"doc-noteref\">source<sup class=\"maki-reference-number\">1</sup>"
    ));
    assert!(html.contains(
        "id=\"maki-reference-use-2-1\" href=\"#maki-reference-note-2\" role=\"doc-noteref\">details<sup class=\"maki-reference-number\">2</sup>"
    ));
    assert!(html.contains(
        "id=\"maki-reference-use-2-2\" href=\"#maki-reference-note-2\" role=\"doc-noteref\">details<sup class=\"maki-reference-number\">2</sup>"
    ));
    assert!(html.contains("id=\"maki-reference-use-2-3\""));

    let source_note = html.find("<li id=\"maki-reference-note-1\"").unwrap();
    let details_note = html.find("<li id=\"maki-reference-note-2\"").unwrap();
    assert!(source_note < details_note);
    assert!(html.contains(
        "<a class=\"external-link\" href=\"https://example.com/?a=1&amp;b=2\">https://example.com/?a=1&amp;b=2</a>"
    ));
    assert!(html.contains("Detailed <strong>prose</strong> &amp; context"));
    assert_eq!(
        html.matches("aria-label=\"Back to details reference ")
            .count(),
        3
    );
    assert!(html.contains("href=\"#maki-reference-use-2-1\""));
    assert!(html.contains("href=\"#maki-reference-use-2-2\""));
    assert!(html.contains("href=\"#maki-reference-use-2-3\""));
}

#[test]
fn reference_note_labels_and_values_are_html_escaped() {
    let parsed = parser::parse(
        r#"[odd & "<key>"] and [odd & "<key>"]

[odd & "<key>"]: Value with <unsafe> & "quotes""#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains("role=\"doc-noteref\">odd &amp; &quot;&lt;key&gt;&quot;<sup"));
    assert!(html.contains("Value with &lt;unsafe&gt; &amp; &quot;quotes&quot;"));
    assert_eq!(
        html.matches("aria-label=\"Back to odd &amp; &quot;&lt;key&gt;&quot; reference ")
            .count(),
        2
    );
}

#[test]
fn nested_reference_notes_use_the_nested_document_definition() {
    let parsed = parser::parse(
        r#"[^same]

> [^same]
>
> [same]: Nested value.

[same]: Outer value."#,
    );

    let html = render_document(&parsed.document);
    let quote_start = html.find("<blockquote>").unwrap();
    let quote_end = html.find("</blockquote>").unwrap();
    let quote = &html[quote_start..quote_end];
    let outer = &html[quote_end..];

    assert!(quote.contains("Nested value."), "{quote}");
    assert!(!quote.contains("Outer value."), "{quote}");
    assert!(outer.contains("Outer value."), "{outer}");
    assert!(!outer.contains("Nested value."), "{outer}");
    assert!(quote.contains("id=\"maki-reference-note-1-2\""), "{quote}");
    assert!(outer.contains("id=\"maki-reference-note-1\""), "{outer}");
}

#[test]
fn reference_note_bodies_contribute_late_backlinks_before_notes_are_emitted() {
    let parsed = parser::parse(
        r#"[^first] [^second]

[first]: First note.
[second]: Second note cites [first]."#,
    );

    let html = render_document(&parsed.document);
    let first_note_start = html.find("<li id=\"maki-reference-note-1\"").unwrap();
    let second_note_start = html.find("<li id=\"maki-reference-note-2\"").unwrap();
    let first_note = &html[first_note_start..second_note_start];

    assert!(
        first_note.contains("href=\"#maki-reference-use-1-1\""),
        "{first_note}"
    );
    assert!(
        first_note.contains("href=\"#maki-reference-use-1-2\""),
        "{first_note}"
    );
    assert_eq!(
        first_note
            .matches("aria-label=\"Back to first reference ")
            .count(),
        2,
        "{first_note}"
    );
}

#[test]
fn reference_note_body_discovery_handles_cycles_once_per_definition() {
    let parsed = parser::parse(
        r#"[first]

[first]: First cites [second].
[second]: Second cites [first]."#,
    );

    let html = render_document(&parsed.document);

    assert_eq!(html.matches("tabindex=\"-1\"").count(), 2);
    assert!(html.contains("First cites"));
    assert!(html.contains("Second cites"));
    assert!(html.contains("id=\"maki-reference-use-1-2\""));
    assert_eq!(
        html.matches("aria-label=\"Back to first reference ")
            .count(),
        2
    );
    assert_eq!(
        html.matches("aria-label=\"Back to second reference ")
            .count(),
        1
    );
}

#[test]
fn generated_reference_ids_avoid_all_authored_document_ids() {
    let parsed = parser::parse(
        r#"--^ title: maki-reference-notes-title

Anchored block
--^ id: maki-reference-note-1

[^note] [^note]

= maki-reference-use-1-1

> = maki-reference-use-1-2

[note]: Note body."#,
    );

    let html = render_document(&parsed.document);

    assert!(html.contains("<h1 id=\"maki-reference-notes-title\""));
    assert!(html.contains("class=\"maki-block-anchor\" id=\"maki-reference-note-1\""));
    assert!(html.contains("<h2 id=\"maki-reference-use-1-1\""));
    assert!(html.contains("<h2 id=\"maki-reference-use-1-2\""));
    assert!(html.contains("id=\"maki-reference-note-1-2\""));
    assert!(html.contains("id=\"maki-reference-use-1-1-2\""));
    assert!(html.contains("id=\"maki-reference-use-1-2-2\""));
    assert!(html.contains("aria-labelledby=\"maki-reference-notes-title-2\""));
    assert!(html.contains("id=\"maki-reference-notes-title-2\""));

    let ids: Vec<_> = html
        .split(" id=\"")
        .skip(1)
        .filter_map(|html| html.split('"').next())
        .collect();
    let unique_ids: std::collections::BTreeSet<_> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique_ids.len(), "duplicate IDs in {ids:?}");
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
fn document_hierarchy_renders_breadcrumb_and_subdocuments_after_title() {
    let parsed = parser::parse("--^ title: Topic\n\nAuthored content");
    let navigation = DocumentNavigation::from_ancestors(
        vec![
            DocumentNavigationItem::new("Root & home", "/root"),
            DocumentNavigationItem::new("Parent <overview>", "/root/parent"),
        ],
        vec![
            DocumentNavigationItem::new("Child <one>", "/topic/one"),
            DocumentNavigationItem::new("Child two", "/topic/two"),
        ],
    );
    assert_eq!(
        navigation.parent().map(DocumentNavigationItem::title),
        Some("Parent <overview>")
    );

    let html = render_document_with_context(
        &parsed.document,
        RenderContext::default().with_document_navigation(navigation),
    );

    let title = html.find("<h1 id=\"Topic\">Topic</h1>").unwrap();
    let breadcrumb = html
        .find("<nav class=\"maki-document-breadcrumb\" aria-label=\"Breadcrumb\">")
        .unwrap();
    let subdocuments = html
        .find("<nav class=\"maki-document-navigation\" aria-label=\"Subdocuments\">")
        .unwrap();
    let authored = html.find("<p>Authored content</p>").unwrap();
    assert!(title < breadcrumb && breadcrumb < subdocuments && subdocuments < authored);
    assert!(html.contains(
        "<ol><li><a href=\"/root\">Root &amp; home</a><span class=\"maki-document-breadcrumb-separator\" aria-hidden=\"true\">›</span></li><li><a href=\"/root/parent\">Parent &lt;overview&gt;</a></li></ol>"
    ));
    assert!(html.contains(
        "<span class=\"maki-document-navigation-label\">Subdocuments</span><ul><li><a href=\"/topic/one\">Child &lt;one&gt;</a></li><li><a href=\"/topic/two\">Child two</a></li></ul>"
    ));
}

#[test]
fn document_hierarchy_renders_breadcrumb_and_subdocuments_independently() {
    let parsed = parser::parse("--^ title: Topic\n\nAuthored content");
    let resolve_note_link = |_target: &str| NoteLinkResolution::Broken;
    let get_note_info = |_note_ref: &NoteRef| None;

    let parent_only = render_document_with_context(
        &parsed.document,
        RenderContext::project(&resolve_note_link, &get_note_info).with_document_navigation(
            DocumentNavigation::new(
                Some(DocumentNavigationItem::new("Parent", "/parent")),
                Vec::new(),
            ),
        ),
    );
    assert!(parent_only.contains("aria-label=\"Breadcrumb\""));
    assert!(!parent_only.contains("aria-label=\"Subdocuments\""));

    let children_only = render_document_with_context(
        &parsed.document,
        RenderContext::project(&resolve_note_link, &get_note_info).with_document_navigation(
            DocumentNavigation::new(
                None,
                vec![DocumentNavigationItem::new("Child", "/topic/child")],
            ),
        ),
    );
    assert!(!children_only.contains("aria-label=\"Breadcrumb\""));
    assert!(children_only.contains("aria-label=\"Subdocuments\""));
}

#[test]
fn empty_or_unspecified_document_navigation_is_not_rendered() {
    let parsed = parser::parse("--^ title: Page\n\nbody");

    let standalone = render_document(&parsed.document);
    let explicitly_empty = render_document_with_context(
        &parsed.document,
        RenderContext::default().with_document_navigation(DocumentNavigation::default()),
    );

    assert!(!standalone.contains("<nav class=\"maki-document-navigation\""));
    assert!(!explicitly_empty.contains("<nav class=\"maki-document-navigation\""));
    assert!(!standalone.contains("<nav class=\"maki-document-breadcrumb\""));
    assert!(!explicitly_empty.contains("<nav class=\"maki-document-breadcrumb\""));
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
fn reparsed_quote_bodies_do_not_publish_unlocated_explicit_id_anchors() {
    let parsed = parser::parse("> quoted block\n> --^ id: quote-local");
    let html = render_document(&parsed.document);

    assert!(html.contains("<blockquote><p>quoted block</p></blockquote>"));
    assert!(!html.contains("id=\"quote-local\""));
}

#[test]
fn standalone_rendering_maps_current_document_id_links_to_html_fragments() {
    let parsed = parser::parse("Addressable\n--^ id: local-id\n\n[[@local-id]]");
    let html = render_document(&parsed.document);

    assert!(html.contains("id=\"local-id\""));
    assert!(html.contains("<a href=\"#local-id\">@local-id</a>"));
}

#[test]
fn quote_attribution_separator_is_not_an_unknown_container() {
    let parsed = parser::parse(
        r#"> 인간은 생체컴퓨터이기 때문에 항온항습이 중요하다..
>
> --- [@woohyong]"#,
    );

    let html = render_document(&parsed.document);

    assert!(!html.contains("maki-container-unknown"), "{html}");
    assert!(html.contains("---"), "{html}");
    assert!(html.contains("@woohyong"), "{html}");
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
