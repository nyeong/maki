use super::draft::{BlockDraft, ListItemDraft, PropertyItemDraft, PropertyKind, build_drafts};
use super::line::{LinePrefix, LineToken, scan_lines};
use super::*;

#[test]
fn nested_unordered_list() {
    let source = r#"- first
  - second
  - second-sibling
    - third
    - third-sibling
  - fourth but second depth

- another list"#;

    let parsed = parse(source);
    let doc = parsed.document;

    assert_eq!(doc.blocks.len(), 2);

    let BlockKind::List { items } = &doc.blocks[0].kind else {
        panic!("expected first block to be a list");
    };

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].body, vec![Inline::Text("first")]);
    assert_eq!(items[0].children.len(), 1);

    let BlockKind::List { items } = &items[0].children[0].kind else {
        panic!("expected first item to contain a nested list");
    };

    assert_eq!(items.len(), 3);
    assert_eq!(items[0].body, vec![Inline::Text("second")]);
    assert_eq!(items[1].body, vec![Inline::Text("second-sibling")]);
    assert_eq!(items[2].body, vec![Inline::Text("fourth but second depth")]);
    assert!(items[0].children.is_empty());
    assert!(items[2].children.is_empty());
    assert_eq!(items[1].children.len(), 1);

    let BlockKind::List { items } = &items[1].children[0].kind else {
        panic!("expected second-sibling to contain a nested list");
    };

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].body, vec![Inline::Text("third")]);
    assert_eq!(items[1].body, vec![Inline::Text("third-sibling")]);
    assert!(items[0].children.is_empty());
    assert!(items[1].children.is_empty());

    let BlockKind::List { items } = &doc.blocks[1].kind else {
        panic!("expected blank-separated list to become a separate block");
    };

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].body, vec![Inline::Text("another list")]);
    assert!(items[0].children.is_empty());
}

#[test]
fn parse_inline_supports_markdown_style_links() {
    assert_eq!(
        parse_inline("Read [djot](https://github.com/jgm/djot) and [[Maki]]."),
        vec![
            Inline::Text("Read "),
            Inline::Link {
                title: "djot",
                target: "https://github.com/jgm/djot"
            },
            Inline::Text(" and "),
            Inline::NoteLink { target: "Maki" },
            Inline::Text(".")
        ]
    );
}

#[test]
fn parse_inline_supports_star_delimited_strong_text() {
    assert_eq!(
        parse_inline("Use *bold `code` and [link](/target)* now."),
        vec![
            Inline::Text("Use "),
            Inline::Strong(vec![
                Inline::Text("bold "),
                Inline::Code("code"),
                Inline::Text(" and "),
                Inline::Link {
                    title: "link",
                    target: "/target"
                }
            ]),
            Inline::Text(" now.")
        ]
    );
}

#[test]
fn parse_inline_keeps_loose_stars_as_text() {
    assert_eq!(
        parse_inline("Do not parse * loose * or **double**."),
        vec![Inline::Text("Do not parse * loose * or **double**.")]
    );
}

#[test]
fn parse_inline_autolinks_plain_http_urls() {
    assert_eq!(
        parse_inline("Read https://example.com/docs, then `https://example.com/code`."),
        vec![
            Inline::Text("Read "),
            Inline::Link {
                title: "https://example.com/docs",
                target: "https://example.com/docs"
            },
            Inline::Text(", then "),
            Inline::Code("https://example.com/code"),
            Inline::Text(".")
        ]
    );
}

#[test]
fn parse_inline_date_stamps_and_ranges() {
    assert_eq!(
        parse_inline("On [2026-08-15 토] and <2026-08-16> through [2026-08-17]--[2026-08-19]."),
        vec![
            Inline::Text("On "),
            Inline::DateStamp(DateStamp {
                kind: DateStampKind::Date,
                date: Date::new(2026, 8, 15).unwrap(),
                body: "2026-08-15 토",
            }),
            Inline::Text(" and "),
            Inline::DateStamp(DateStamp {
                kind: DateStampKind::Event,
                date: Date::new(2026, 8, 16).unwrap(),
                body: "2026-08-16",
            }),
            Inline::Text(" through "),
            Inline::DateRange(DateRange {
                start: DateStamp {
                    kind: DateStampKind::Date,
                    date: Date::new(2026, 8, 17).unwrap(),
                    body: "2026-08-17",
                },
                end: DateStamp {
                    kind: DateStampKind::Date,
                    date: Date::new(2026, 8, 19).unwrap(),
                    body: "2026-08-19",
                },
            }),
            Inline::Text("."),
        ]
    );
}

#[test]
fn test_scan_lines() {
    let source = r#"--^ title: Maki
== Heading

- list
  - nested list

: This is Code Line

--- code
Container Block
---

plain text"#;

    assert_eq!(
        scan_lines(source),
        vec![
            LineToken::Line {
                indent: 0,
                kind: LinePrefix::EnCaret,
                raw_line: "--^ title: Maki"
            },
            LineToken::Line {
                indent: 0,
                kind: LinePrefix::EqualsRun(2),
                raw_line: "== Heading"
            },
            LineToken::Blank { raw_line: "" },
            LineToken::Line {
                indent: 0,
                kind: LinePrefix::Hyphen,
                raw_line: "- list"
            },
            LineToken::Line {
                indent: 2,
                kind: LinePrefix::Hyphen,
                raw_line: "  - nested list"
            },
            LineToken::Blank { raw_line: "" },
            LineToken::Line {
                indent: 0,
                kind: LinePrefix::Colon,
                raw_line: ": This is Code Line"
            },
            LineToken::Blank { raw_line: "" },
            LineToken::Line {
                indent: 0,
                kind: LinePrefix::HyphenFence(3),
                raw_line: "--- code"
            },
            LineToken::Line {
                indent: 0,
                kind: LinePrefix::None,
                raw_line: "Container Block"
            },
            LineToken::Line {
                indent: 0,
                kind: LinePrefix::HyphenFence(3),
                raw_line: "---"
            },
            LineToken::Blank { raw_line: "" },
            LineToken::Line {
                indent: 0,
                kind: LinePrefix::None,
                raw_line: "plain text"
            },
        ]
    );
}

#[test]
fn test_build_drafts() {
    let source = r#"--^ title: Maki
--^ description: This is a simple example.
== Heading

- list
  - nested list

: This is Code Line

--- code
Container Block
---

plain text"#;

    let lines = scan_lines(source);
    let mut diagnostics = vec![];

    assert_eq!(
        build_drafts(&lines, &mut diagnostics),
        vec![
            BlockDraft::Property {
                indent: 0,
                kind: PropertyKind::Previous,
                items: vec![
                    PropertyItemDraft::new("title", "Maki"),
                    PropertyItemDraft::new("description", "This is a simple example.")
                ],
            },
            BlockDraft::Heading {
                level: 2,
                body: "Heading",
            },
            BlockDraft::List {
                items: vec![ListItemDraft {
                    kind: ListKind::Unordered,
                    indent: 0,
                    body: "list",
                    children: vec![BlockDraft::List {
                        items: vec![ListItemDraft {
                            kind: ListKind::Unordered,
                            indent: 0,
                            body: "nested list",
                            children: vec![],
                        }],
                    }],
                }],
            },
            BlockDraft::Code {
                raw_lines: vec!["This is Code Line"],
            },
            BlockDraft::Container {
                kind: "code",
                args: vec![],
                raw_lines: vec!["Container Block"],
            },
            BlockDraft::Paragraph {
                raw_lines: vec!["plain text"],
            },
        ]
    );
    assert_eq!(diagnostics, vec![]);
}

#[test]
fn parse_table_with_inline_cells_and_numeric_alignment() {
    let parsed = parse(
        r#"| 이름 | 점수 | 취득일 |
|---+---+---|
| `Alice` | 10 | [2026-08-15] |
| Bob | 2 | [2026-08-16] |"#,
    );

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.document.blocks.len(), 1);

    let BlockKind::Table {
        header,
        alignments,
        rows,
    } = &parsed.document.blocks[0].kind
    else {
        panic!("expected a table block");
    };

    assert_eq!(header.cells[0].body, vec![Inline::Text("이름")]);
    assert_eq!(header.cells[1].body, vec![Inline::Text("점수")]);
    assert_eq!(
        alignments,
        &vec![
            TableColumnAlignment::Text,
            TableColumnAlignment::Number,
            TableColumnAlignment::Text
        ]
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].kind, TableRowKind::Data);
    assert_eq!(rows[0].cells[0].body, vec![Inline::Code("Alice")]);
    assert_eq!(rows[0].cells[1].body, vec![Inline::Text("10")]);
    assert_eq!(
        rows[0].cells[2].body,
        vec![Inline::DateStamp(DateStamp {
            kind: DateStampKind::Date,
            date: Date::new(2026, 8, 15).unwrap(),
            body: "2026-08-15",
        })]
    );
}

#[test]
fn parse_table_starts_after_paragraph_without_blank_line() {
    let parsed = parse(
        r#"intro
| 이름 | 점수 |
|---+---|
| Alice | 10 |"#,
    );

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.document.blocks.len(), 2);

    let BlockKind::Paragraph { body } = &parsed.document.blocks[0].kind else {
        panic!("expected the first block to stay paragraph");
    };
    assert_eq!(body, &vec![Inline::Text("intro")]);

    let BlockKind::Table { rows, .. } = &parsed.document.blocks[1].kind else {
        panic!("expected the second block to become table");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, TableRowKind::Data);
}

#[test]
fn parse_table_keeps_hyphen_cell_when_separator_width_differs() {
    let parsed = parse(
        r#"| 이름 | 값 |
|---+---|
| dash | --- |"#,
    );

    assert!(parsed.diagnostics.is_empty());

    let BlockKind::Table { rows, .. } = &parsed.document.blocks[0].kind else {
        panic!("expected a table block");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, TableRowKind::Data);
    assert_eq!(rows[0].cells[1].body, vec![Inline::Text("---")]);
}

#[test]
fn parse_table_keeps_middle_separator_inside_table() {
    let parsed = parse(
        r#"| 일시 | 시간 |
|---+---|
| [2025-11-05 Wed] | 5H |
|---+---|
| [2026-04-04 Sat] | 5H |"#,
    );

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.document.blocks.len(), 1);

    let BlockKind::Table {
        alignments, rows, ..
    } = &parsed.document.blocks[0].kind
    else {
        panic!("expected a table block");
    };

    assert_eq!(
        alignments,
        &vec![TableColumnAlignment::Text, TableColumnAlignment::Text]
    );
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].kind, TableRowKind::Data);
    assert_eq!(rows[1].kind, TableRowKind::Separator);
    assert!(rows[1].cells.is_empty());
    assert_eq!(rows[2].kind, TableRowKind::Data);
    assert_eq!(
        rows[2].cells[0].body,
        vec![Inline::DateStamp(DateStamp {
            kind: DateStampKind::Date,
            date: Date::new(2026, 4, 4).unwrap(),
            body: "2026-04-04 Sat",
        })]
    );
}

#[test]
fn parse_reports_no_diagnostics_for_supported_document() {
    let parsed = parse(
        r#"--^ title: Maki

= Heading

- list
  - nested

--v lang: html
: <main></main>"#,
    );

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.document.title(), Some("Maki"));
}

#[test]
fn parse_quote_lines_strip_prefix_for_inner_maki() {
    let parsed = parse(
        r#"> = Quoted
>
> Body with `code`
> - item
> > nested"#,
    );

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.document.blocks.len(), 1);

    let BlockKind::Quote { lines } = &parsed.document.blocks[0].kind else {
        panic!("expected a quote block");
    };

    assert_eq!(
        lines,
        &vec!["= Quoted", "", "Body with `code`", "- item", "> nested"]
    );
}

#[test]
fn parse_reports_invalid_property_without_panicking() {
    let parsed = parse(
        r#"--^ invalid-property
--^ title: Maki

= Heading"#,
    );

    assert_eq!(parsed.document.title(), Some("Maki"));
    assert_eq!(
        parsed.diagnostics,
        vec![ParseDiagnostic {
            line: 1,
            kind: ParseDiagnosticKind::InvalidProperty {
                raw_line: "--^ invalid-property"
            },
        }]
    );
}

#[test]
fn parse_reports_unclosed_container() {
    let parsed = parse(
        r#"--- code
fn main() {}"#,
    );

    assert_eq!(
        parsed.diagnostics,
        vec![ParseDiagnostic {
            line: 1,
            kind: ParseDiagnosticKind::UnclosedContainer {
                raw_line: "--- code"
            },
        }]
    );
}

#[test]
fn parse_preserves_shorter_fence_inside_long_container() {
    let parsed = parse(
        r#"----- code
---
body
-----"#,
    );

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.document.blocks.len(), 1);

    let BlockKind::Container { kind, args, lines } = &parsed.document.blocks[0].kind else {
        panic!("expected a container block");
    };

    assert_eq!(*kind, "code");
    assert!(args.is_empty());
    assert_eq!(lines, &vec!["---", "body"]);
}

#[test]
fn parse_treats_headerless_hyphen_run_as_paragraph() {
    let parsed = parse(
        r#"---
plain"#,
    );

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.document.blocks.len(), 1);

    let BlockKind::Paragraph { body } = &parsed.document.blocks[0].kind else {
        panic!("expected a paragraph block");
    };

    assert_eq!(
        body,
        &vec![
            Inline::Text("---"),
            Inline::SoftBreak,
            Inline::Text("plain")
        ]
    );
}

#[test]
fn ordered_list_strips_marker_width_from_body() {
    let parsed = parse(
        r#"9. ninth
10. tenth"#,
    );

    assert!(parsed.diagnostics.is_empty());

    let BlockKind::List { items } = &parsed.document.blocks[0].kind else {
        panic!("expected an ordered list block");
    };

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].kind, ListKind::Ordered);
    assert_eq!(items[0].body, vec![Inline::Text("ninth")]);
    assert_eq!(items[1].kind, ListKind::Ordered);
    assert_eq!(items[1].body, vec![Inline::Text("tenth")]);
}

#[test]
fn ordered_list_keeps_indented_paragraphs_inside_items() {
    let parsed = parse(
        r#"1. Glider 활용 증진

   현재 Glider의 CloudData는 여러 제약사항으로 다양한 곳에 활용하지 못하고 있습니다.

2. Datadog 활용 증진

   Datadog 사용을 위해 고비용을 지불하고 있습니다.

3. 사내 라이브러리 도입

   사내 서버 개발 미팅을 통해 도출된 반복되는 업무를 라이브러리화합니다."#,
    );

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.document.blocks.len(), 1);

    let BlockKind::List { items } = &parsed.document.blocks[0].kind else {
        panic!("expected an ordered list block");
    };

    assert_eq!(items.len(), 3);
    assert_eq!(items[0].kind, ListKind::Ordered);
    assert_eq!(items[0].body, vec![Inline::Text("Glider 활용 증진")]);
    assert_eq!(items[0].children.len(), 1);

    let BlockKind::Paragraph { body } = &items[0].children[0].kind else {
        panic!("expected the indented line to become a child paragraph");
    };

    assert_eq!(
        body,
        &vec![Inline::Text(
            "현재 Glider의 CloudData는 여러 제약사항으로 다양한 곳에 활용하지 못하고 있습니다."
        )]
    );
    assert_eq!(items[1].body, vec![Inline::Text("Datadog 활용 증진")]);
    assert_eq!(items[1].children.len(), 1);
    assert_eq!(items[2].body, vec![Inline::Text("사내 라이브러리 도입")]);
    assert_eq!(items[2].children.len(), 1);
}
