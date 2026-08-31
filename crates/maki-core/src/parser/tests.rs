use super::draft::{BlockDraft, ListItemDraft, PropertyItemDraft, PropertyKind, build_drafts};
use super::line::{LinePrefix, LineToken, scan_lines};
use super::*;
use crate::source::{SourceMap, SourceSpan};

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
fn parse_list_item_contains_quote_code_and_table_blocks() {
    let parsed = parse(
        r#"- nested blocks
  > quoted
  >
  > body
  : fn main() {}
  :
  | Name | Value |
  |---+---|
  | quote | nested |"#,
    );

    assert!(parsed.diagnostics.is_empty());
    let BlockKind::List { items } = &parsed.document.blocks[0].kind else {
        panic!("expected a list");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].children.len(), 3);

    let BlockKind::Quote { lines } = &items[0].children[0].kind else {
        panic!("expected a nested quote block");
    };
    assert_eq!(lines, &vec!["quoted", "", "body"]);

    let BlockKind::Code { lines, lang } = &items[0].children[1].kind else {
        panic!("expected a nested code block");
    };
    assert_eq!(lines, &vec!["fn main() {}", ""]);
    assert_eq!(*lang, None);

    let BlockKind::Table {
        header,
        alignments,
        rows,
    } = &items[0].children[2].kind
    else {
        panic!("expected a nested table block");
    };
    assert_eq!(header.cells[0].body, vec![Inline::Text("Name")]);
    assert_eq!(header.cells[1].body, vec![Inline::Text("Value")]);
    assert_eq!(alignments.len(), 2);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cells[0].body, vec![Inline::Text("quote")]);
    assert_eq!(rows[0].cells[1].body, vec![Inline::Text("nested")]);
}

#[test]
fn parse_todo_list_item_states_and_strict_fallbacks() {
    let parsed = parse(
        r#"- [ ]
- [x] done
- [x]
- [ ]  one leading space remains
- [X] uppercase stays text
- [ ]attached stays text
- ordinary

1. [ ] ordered stays text"#,
    );

    let BlockKind::List { items } = &parsed.document.blocks[0].kind else {
        panic!("expected an unordered list");
    };
    assert_eq!(items.len(), 7);
    assert_eq!(items[0].todo, Some(TodoState::Todo));
    assert!(items[0].body.is_empty());
    assert_eq!(items[1].todo, Some(TodoState::Done));
    assert_eq!(items[1].body, vec![Inline::Text("done")]);
    assert_eq!(items[2].todo, Some(TodoState::Done));
    assert!(items[2].body.is_empty());
    assert_eq!(items[3].todo, Some(TodoState::Todo));
    assert_eq!(
        items[3].body,
        vec![Inline::Text(" one leading space remains")]
    );
    assert_eq!(items[4].todo, None);
    assert_eq!(
        items[4].body,
        vec![Inline::Text("[X] uppercase stays text")]
    );
    assert_eq!(items[5].todo, None);
    assert_eq!(items[5].body, vec![Inline::Text("[ ]attached stays text")]);
    assert_eq!(items[6].todo, None);

    let BlockKind::List { items } = &parsed.document.blocks[1].kind else {
        panic!("expected an ordered list");
    };
    assert_eq!(items[0].todo, None);
    assert_eq!(items[0].body, vec![Inline::Text("[ ] ordered stays text")]);
}

#[test]
fn parse_nested_todo_list_items() {
    let parsed = parse(
        r#"- [ ] parent
  - [x] child"#,
    );

    let BlockKind::List { items } = &parsed.document.blocks[0].kind else {
        panic!("expected an unordered list");
    };
    assert_eq!(items[0].todo, Some(TodoState::Todo));
    let BlockKind::List { items } = &items[0].children[0].kind else {
        panic!("expected a nested unordered list");
    };
    assert_eq!(items[0].todo, Some(TodoState::Done));
    assert_eq!(items[0].body, vec![Inline::Text("child")]);
}

#[test]
fn parse_document_recognizes_all_explicit_reference_uses_before_definitions() {
    let parsed = parse(
        r#"Read [djot][], [ the language ][djot], [^source][], [^ 출처 ][source], [^][source], and [ inline ](https://example.com).

[djot]: <https://github.com/jgm/djot>
[source]: Published on [2026-08-25]."#,
    );

    assert!(parsed.diagnostics.is_empty());
    let BlockKind::Paragraph { body } = &parsed.document.blocks[0].kind else {
        panic!("expected a paragraph");
    };
    assert_eq!(
        body,
        &vec![
            Inline::Text("Read "),
            Inline::Reference {
                raw: "[djot][]",
                title: "djot",
                key: "djot",
            },
            Inline::Text(", "),
            Inline::Reference {
                raw: "[ the language ][djot]",
                title: "the language",
                key: "djot",
            },
            Inline::Text(", "),
            Inline::Footnote {
                raw: "[^source][]",
                title: Some("source"),
                key: "source",
            },
            Inline::Text(", "),
            Inline::Footnote {
                raw: "[^ 출처 ][source]",
                title: Some("출처"),
                key: "source",
            },
            Inline::Text(", "),
            Inline::Footnote {
                raw: "[^][source]",
                title: None,
                key: "source",
            },
            Inline::Text(", and "),
            Inline::DirectLink {
                raw: "[ inline ](https://example.com)",
                title: "inline",
                target: "https://example.com",
            },
            Inline::Text("."),
        ]
    );
}

#[test]
fn canonical_definitions_trim_keys_and_do_not_accept_footnote_aliases() {
    let parsed = parse("[ foo bar ]: value\n[^legacy]: body");

    let definition = parsed.document.reference("foo bar").unwrap();
    assert_eq!(definition.key, "foo bar");
    assert_eq!(definition.raw_value, "value");
    assert!(parsed.document.reference("legacy").is_none());
    let BlockKind::Paragraph { body } = &parsed.document.blocks[1].kind else {
        panic!("expected the former alias syntax to remain text");
    };
    assert_eq!(body, &vec![Inline::Text("[^legacy]: body")]);
}

#[test]
fn reference_value_kind_requires_one_exact_semantic_target() {
    let parsed = parse(
        r#"[web]: <https://example.com/path>
[note]: [[target]]
[date]: [2026-08-25]
[range]: [2026-08-25]--[2026-08-26]
[bare-url]: https://example.com/path
[direct]: [title](https://example.com)
[prose]: Published on [2026-08-25]."#,
    );

    assert_eq!(
        parsed.document.reference("web").unwrap().value_kind(),
        ReferenceValueKind::HyperLink
    );
    assert_eq!(
        parsed.document.reference("note").unwrap().value_kind(),
        ReferenceValueKind::NoteLink
    );
    assert_eq!(
        parsed.document.reference("date").unwrap().value_kind(),
        ReferenceValueKind::DateStamp
    );
    assert_eq!(
        parsed.document.reference("range").unwrap().value_kind(),
        ReferenceValueKind::DateRange
    );
    for key in ["bare-url", "direct", "prose"] {
        assert_eq!(
            parsed.document.reference(key).unwrap().value_kind(),
            ReferenceValueKind::Prose
        );
    }
}

#[test]
fn explicit_compounds_precede_dates_and_note_links_keep_priority() {
    assert!(matches!(
        parse_inline("[2026-08-25][date]").as_slice(),
        [Inline::Reference {
            title: "2026-08-25",
            key: "date",
            ..
        }]
    ));
    assert!(matches!(
        parse_inline("[[note]]").as_slice(),
        [Inline::NoteLink { target: "note" }]
    ));
    assert!(matches!(
        parse_inline("[2026-08-25]").as_slice(),
        [Inline::DateStamp(_)]
    ));
}

#[test]
fn bare_markers_are_text_but_explicit_uses_do_not_require_a_definition() {
    assert_eq!(
        parse_inline("[missing] [^missing] "),
        vec![Inline::Text("[missing] [^missing] ")]
    );
    assert_eq!(
        parse_inline("[missing][] [^missing][]"),
        vec![
            Inline::Reference {
                raw: "[missing][]",
                title: "missing",
                key: "missing",
            },
            Inline::Text(" "),
            Inline::Footnote {
                raw: "[^missing][]",
                title: Some("missing"),
                key: "missing",
            },
        ]
    );
}

#[test]
fn direct_links_reserve_leading_caret_and_require_nonempty_trimmed_titles() {
    assert_eq!(
        parse_inline("[^^](target)"),
        vec![Inline::Text("[^^](target)")]
    );
    assert_eq!(
        parse_inline("[ ^title ](target) [   ](target) [   ][key] [^   ][key] [^^title][key]"),
        vec![Inline::Text(
            "[ ^title ](target) [   ](target) [   ][key] [^   ][key] [^^title][key]"
        )]
    );
}

#[test]
fn parse_inline_handles_long_unclosed_delimiter_runs_as_text() {
    let cases = [
        "[".repeat(16_384),
        "<".repeat(16_384),
        "^{body ".repeat(2_048),
        "*body ".repeat(2_048),
        "::body ".repeat(2_048),
    ];

    for source in &cases {
        assert_eq!(parse_inline(source), vec![Inline::Text(source)]);
    }
}

#[test]
fn parse_inline_supports_star_delimited_strong_text() {
    assert_eq!(
        parse_inline("Use *bold `code`* now."),
        vec![
            Inline::Text("Use "),
            Inline::Strong(vec![Inline::Text("bold "), Inline::Code("code"),]),
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
fn parse_inline_supports_hyper_links_but_not_bare_urls() {
    assert_eq!(
        parse_inline("Read <https://example.com/docs>, not https://example.com/bare."),
        vec![
            Inline::Text("Read "),
            Inline::HyperLink {
                target: "https://example.com/docs"
            },
            Inline::Text(", not https://example.com/bare.")
        ]
    );
}

#[test]
fn parse_inline_supports_stable_formatting_syntax() {
    assert_eq!(
        parse_inline("/italic `code`/ *strong* ^{sup} _{sub} +{inserted} -{deleted} ::highlight::"),
        vec![
            Inline::Italic(vec![Inline::Text("italic "), Inline::Code("code")]),
            Inline::Text(" "),
            Inline::Strong(vec![Inline::Text("strong")]),
            Inline::Text(" "),
            Inline::Superscript("sup"),
            Inline::Text(" "),
            Inline::Subscript("sub"),
            Inline::Text(" "),
            Inline::Insertion("inserted"),
            Inline::Text(" "),
            Inline::Deletion("deleted"),
            Inline::Text(" "),
            Inline::Highlight(vec![Inline::Text("highlight")]),
        ]
    );
    assert_eq!(parse_inline("//italic//"), vec![Inline::Text("//italic//")]);
    assert_eq!(
        parse_inline("=highlight="),
        vec![Inline::Text("=highlight=")]
    );
}

#[test]
fn parse_inline_braced_modifiers_require_a_nonempty_raw_body() {
    assert_eq!(
        parse_inline("+{insert * raw} -{delete / raw}"),
        vec![
            Inline::Insertion("insert * raw"),
            Inline::Text(" "),
            Inline::Deletion("delete / raw"),
        ]
    );

    for source in ["+{}", "-{}", "+{open", "-{a{b}"] {
        assert_eq!(parse_inline(source), vec![Inline::Text(source)]);
    }
    assert_eq!(
        parse_inline("+{a}b}"),
        vec![Inline::Insertion("a"), Inline::Text("b}")]
    );
}

#[test]
fn parse_inline_uses_nearest_opener_and_first_closer() {
    assert_eq!(
        parse_inline("*not strong *strong*"),
        vec![
            Inline::Text("*not strong "),
            Inline::Strong(vec![Inline::Text("strong")]),
        ]
    );
    assert_eq!(
        parse_inline("*first*second*"),
        vec![
            Inline::Strong(vec![Inline::Text("first")]),
            Inline::Text("second*"),
        ]
    );
}

#[test]
fn parse_inline_invalidates_overlapping_openers_but_preserves_nesting() {
    assert_eq!(
        parse_inline("/outer *inner/ tail*"),
        vec![
            Inline::Italic(vec![Inline::Text("outer *inner")]),
            Inline::Text(" tail*"),
        ]
    );
    assert_eq!(
        parse_inline("*outer /inner* tail/"),
        vec![
            Inline::Strong(vec![Inline::Text("outer /inner")]),
            Inline::Text(" tail/"),
        ]
    );
    assert_eq!(
        parse_inline("/outer *inner* tail/"),
        vec![Inline::Italic(vec![
            Inline::Text("outer "),
            Inline::Strong(vec![Inline::Text("inner")]),
            Inline::Text(" tail"),
        ])]
    );
    assert_eq!(
        parse_inline("::outer *inner* tail::"),
        vec![Inline::Highlight(vec![
            Inline::Text("outer "),
            Inline::Strong(vec![Inline::Text("inner")]),
            Inline::Text(" tail"),
        ])]
    );
    assert_eq!(
        parse_inline(r"*outer \* literal*"),
        vec![Inline::Strong(vec![
            Inline::Text("outer "),
            Inline::Text("* literal"),
        ])]
    );
}

#[test]
fn parse_inline_backslash_escapes_ascii_punctuation() {
    let punctuation = (b'!'..=b'/')
        .chain(b':'..=b'@')
        .chain(b'['..=b'`')
        .chain(b'{'..=b'~');

    for byte in punctuation {
        let source = format!("\\{}", char::from(byte));
        assert_eq!(parse_inline(&source), vec![Inline::Text(&source[1..])]);
    }

    assert_eq!(parse_inline(r"\a \한"), vec![Inline::Text(r"\a \한")]);
}

#[test]
fn parse_inline_escape_keeps_reference_syntax_literal() {
    let parsed = parse(
        r#"Escaped \[known][], normal [known][].

[known]: <https://example.com>"#,
    );
    let BlockKind::Paragraph { body } = &parsed.document.blocks[0].kind else {
        panic!("expected a paragraph");
    };

    assert_eq!(
        body,
        &vec![
            Inline::Text("Escaped "),
            Inline::Text("[known][], normal "),
            Inline::Reference {
                raw: "[known][]",
                title: "known",
                key: "known",
            },
            Inline::Text("."),
        ]
    );
}

#[test]
fn parse_inline_keeps_standalone_unix_path_as_text() {
    assert_eq!(
        parse_inline("/usr/local/bin"),
        vec![Inline::Text("/usr/local/bin")]
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
                target: DateStampTarget::Date(Date::new(2026, 8, 15).unwrap()),
                body: "2026-08-15 토",
            }),
            Inline::Text(" and "),
            Inline::DateStamp(DateStamp {
                kind: DateStampKind::Event,
                target: DateStampTarget::Date(Date::new(2026, 8, 16).unwrap()),
                body: "2026-08-16",
            }),
            Inline::Text(" through "),
            Inline::DateRange(DateRange {
                start: DateStamp {
                    kind: DateStampKind::Date,
                    target: DateStampTarget::Date(Date::new(2026, 8, 17).unwrap()),
                    body: "2026-08-17",
                },
                end: DateStamp {
                    kind: DateStampKind::Date,
                    target: DateStampTarget::Date(Date::new(2026, 8, 19).unwrap()),
                    body: "2026-08-19",
                },
            }),
            Inline::Text("."),
        ]
    );
}

#[test]
fn parse_inline_month_iso_week_and_iso_weekday_date_markers() {
    assert_eq!(
        parse_inline("[2026-08] <2026-W23> [2026-W23-1 Mon]"),
        vec![
            Inline::DateStamp(DateStamp {
                kind: DateStampKind::Date,
                target: DateStampTarget::Month(DateMonth::new(2026, 8).unwrap()),
                body: "2026-08",
            }),
            Inline::Text(" "),
            Inline::DateStamp(DateStamp {
                kind: DateStampKind::Event,
                target: DateStampTarget::IsoWeek(IsoWeek::new(2026, 23).unwrap()),
                body: "2026-W23",
            }),
            Inline::Text(" "),
            Inline::DateStamp(DateStamp {
                kind: DateStampKind::Date,
                target: DateStampTarget::Date(Date::new(2026, 6, 1).unwrap()),
                body: "2026-W23-1 Mon",
            }),
        ]
    );
}

#[test]
fn parse_inline_rejects_invalid_month_and_iso_week_markers() {
    assert_eq!(
        parse_inline("[2026-13] [2026-W00] [2026-W54] [2026-W23-8]"),
        vec![Inline::Text("[2026-13] [2026-W00] [2026-W54] [2026-W23-8]")]
    );
}

#[test]
fn iso_week_dates_follow_iso_8601_boundaries() {
    let week = IsoWeek::new(2026, 1).unwrap();

    assert_eq!(week.monday(), Date::new(2025, 12, 29).unwrap());
    assert_eq!(week.sunday(), Date::new(2026, 1, 4));
    assert!(IsoWeek::new(2026, 53).is_some());
    assert!(IsoWeek::new(2027, 53).is_none());
}

#[test]
fn iso_week_dates_handle_representable_year_boundaries() {
    let first_week = IsoWeek::new(1, 1).unwrap();
    assert_eq!(first_week.monday(), Date::new(1, 1, 1).unwrap());
    assert!(first_week.previous().is_none());

    let last_week = IsoWeek::new(9999, 52).unwrap();
    for (weekday, day) in [(1, 27), (2, 28), (3, 29), (4, 30), (5, 31)] {
        assert_eq!(
            last_week.date_for_weekday(weekday),
            Date::new(9999, 12, day)
        );
    }
    assert!(last_week.date_for_weekday(6).is_none());
    assert!(last_week.sunday().is_none());
    assert_eq!(
        last_week.representable_date_range(),
        (
            Date::new(9999, 12, 27).unwrap(),
            Date::new(9999, 12, 31).unwrap()
        )
    );
    assert!(last_week.next().is_none());

    assert_eq!(
        parse_inline("[9999-W52-5]"),
        vec![Inline::DateStamp(DateStamp {
            kind: DateStampKind::Date,
            target: DateStampTarget::Date(Date::new(9999, 12, 31).unwrap()),
            body: "9999-W52-5",
        })]
    );
    assert_eq!(
        parse_inline("[9999-W52-6]"),
        vec![Inline::Text("[9999-W52-6]")]
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

    let lines = scan_lines(source);
    let tokens = lines
        .iter()
        .map(|line| match line {
            LineToken::Blank { raw_line, .. } => (0, None, *raw_line),
            LineToken::Line {
                indent,
                kind,
                raw_line,
                ..
            } => (*indent, Some(*kind), *raw_line),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tokens,
        vec![
            (0, Some(LinePrefix::EnCaret), "--^ title: Maki"),
            (0, Some(LinePrefix::EqualsRun(2)), "== Heading"),
            (0, None, ""),
            (0, Some(LinePrefix::Hyphen), "- list"),
            (2, Some(LinePrefix::Hyphen), "  - nested list"),
            (0, None, ""),
            (0, Some(LinePrefix::Colon), ": This is Code Line"),
            (0, None, ""),
            (0, Some(LinePrefix::HyphenFence(3)), "--- code"),
            (0, Some(LinePrefix::None), "Container Block"),
            (0, Some(LinePrefix::HyphenFence(3)), "---"),
            (0, None, ""),
            (0, Some(LinePrefix::None), "plain text"),
        ]
    );
    let source_map = SourceMap::new(source);
    assert_eq!(
        lines.iter().map(LineToken::span).collect::<Vec<_>>(),
        (0..lines.len())
            .map(|line| source_map.line_span(line).expect("line should exist"))
            .collect::<Vec<_>>()
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
                line: 1,
                span: SourceSpan::new(0, 15),
                raw_line: "--^ title: Maki",
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
                    todo: None,
                    indent: 0,
                    body: "list",
                    children: vec![BlockDraft::List {
                        items: vec![ListItemDraft {
                            kind: ListKind::Unordered,
                            todo: None,
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
            target: DateStampTarget::Date(Date::new(2026, 8, 15).unwrap()),
            body: "2026-08-15",
        })]
    );
}

#[test]
fn parse_table_keeps_escaped_pipe_inside_inline_cell() {
    let parsed = parse(
        r#"| Text | Mark |
|---+---|
| \| literal | ::marked:: |"#,
    );
    let BlockKind::Table { rows, .. } = &parsed.document.blocks[0].kind else {
        panic!("expected a table block");
    };

    assert_eq!(rows[0].cells.len(), 2);
    assert_eq!(rows[0].cells[0].body, vec![Inline::Text("| literal")]);
    assert_eq!(
        rows[0].cells[1].body,
        vec![Inline::Highlight(vec![Inline::Text("marked")])]
    );
}

#[test]
fn parse_table_accepts_pipe_separator_columns() {
    let parsed = parse(
        r#"| Name | Score |
|---|---|
| Alice | 10 |"#,
    );

    assert!(parsed.diagnostics.is_empty());
    let BlockKind::Table { rows, .. } = &parsed.document.blocks[0].kind else {
        panic!("expected a table block");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cells[0].body, vec![Inline::Text("Alice")]);
}

#[test]
fn parse_table_rejects_mixed_separator_columns() {
    let parsed = parse(
        r#"| Name | Score | Owner |
|---+---|---|"#,
    );

    assert!(matches!(
        parsed.document.blocks[0].kind,
        BlockKind::Paragraph { .. }
    ));
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
            target: DateStampTarget::Date(Date::new(2026, 4, 4).unwrap()),
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
fn lone_heading_marker_is_plain_text() {
    let parsed = parse("=");

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(
        parsed.document.blocks[0].kind,
        BlockKind::Paragraph {
            body: vec![Inline::Text("=")],
        }
    );
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
            span: SourceSpan::new(0, 20),
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
            span: SourceSpan::new(0, 8),
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
fn parse_treats_empty_kind_container_as_unknown_container() {
    let parsed = parse(
        r#"---
plain
---"#,
    );

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.document.blocks.len(), 1);

    let BlockKind::Container { kind, args, lines } = &parsed.document.blocks[0].kind else {
        panic!("expected a container block");
    };
    assert_eq!(*kind, "");
    assert!(args.is_empty());
    assert_eq!(lines, &vec!["plain"]);
}

#[test]
fn parse_container_kind_without_header_whitespace() {
    let parsed = parse(
        r#"---code rust
fn main() {}
---"#,
    );

    assert!(parsed.diagnostics.is_empty());
    let BlockKind::Container { kind, args, lines } = &parsed.document.blocks[0].kind else {
        panic!("expected a container block");
    };
    assert_eq!(*kind, "code");
    assert_eq!(args, &vec!["rust"]);
    assert_eq!(lines, &vec!["fn main() {}"]);
}

#[test]
fn parse_keeps_invalid_container_kinds_as_paragraphs() {
    for source in ["--- [@woohyong]", "--- code/bash", "--- 코드"] {
        let parsed = parse(source);

        assert!(parsed.diagnostics.is_empty(), "{source}");
        assert_eq!(parsed.document.blocks.len(), 1, "{source}");
        assert!(
            matches!(parsed.document.blocks[0].kind, BlockKind::Paragraph { .. }),
            "{source}"
        );
    }
}

#[test]
fn parse_accepts_container_kind_character_boundaries() {
    for kind in ["code", "custom-kind", "custom_kind", "123"] {
        let source = format!("---{kind}\nbody\n---");
        let parsed = parse(&source);

        assert!(parsed.diagnostics.is_empty(), "{kind}");
        let BlockKind::Container {
            kind: parsed_kind,
            args,
            lines,
        } = &parsed.document.blocks[0].kind
        else {
            panic!("expected {kind} to be a container kind");
        };
        assert_eq!(*parsed_kind, kind);
        assert!(args.is_empty());
        assert_eq!(lines, &vec!["body"]);
    }
}

#[test]
fn parse_removes_escape_before_block_start_prefixes() {
    let parsed = parse(
        r#"\= heading text
\- list text
\[link]: target
\---"#,
    );

    assert!(parsed.diagnostics.is_empty());
    let BlockKind::Paragraph { body } = &parsed.document.blocks[0].kind else {
        panic!("expected escaped prefixes to remain a paragraph");
    };
    assert_eq!(
        body,
        &vec![
            Inline::Text("= heading text"),
            Inline::SoftBreak,
            Inline::Text("- list text"),
            Inline::SoftBreak,
            Inline::Text("[link]: target"),
            Inline::SoftBreak,
            Inline::Text("---"),
        ]
    );
}

#[test]
fn parse_reports_property_on_property_and_ignores_the_second_property() {
    let parsed = parse(
        r#"--v title: pending
--^ title: ignored
= Heading"#,
    );

    assert_eq!(
        parsed.diagnostics,
        vec![ParseDiagnostic {
            line: 2,
            span: SourceSpan::new(19, 37),
            kind: ParseDiagnosticKind::PropertyOnProperty {
                raw_line: "--^ title: ignored",
            },
        }]
    );
    assert_eq!(
        parsed.document.blocks[0].properties().next(),
        Some(("title", "pending"))
    );
}

#[test]
fn parse_reports_duplicate_reference_definitions_and_uses_the_first() {
    let parsed = parse(
        r#"[link][]
[link]: first
[link]: second"#,
    );

    assert_eq!(
        parsed.document.reference("link").unwrap().raw_value,
        "first"
    );
    assert_eq!(parsed.diagnostics.len(), 1);
    assert!(matches!(
        parsed.diagnostics[0],
        ParseDiagnostic {
            line: 3,
            kind: ParseDiagnosticKind::DuplicateReferenceDefinition { .. },
            ..
        }
    ));
}

#[test]
fn trimmed_definition_keys_share_a_first_wins_namespace() {
    let parsed = parse(
        r#"[same][] [^same][]
[ same ]: first
[same]: second"#,
    );

    assert_eq!(
        parsed.document.reference("same").unwrap().raw_value,
        "first"
    );
    assert_eq!(parsed.diagnostics.len(), 1);
    assert!(matches!(
        parsed.diagnostics[0],
        ParseDiagnostic {
            line: 3,
            kind: ParseDiagnosticKind::DuplicateReferenceDefinition { .. },
            ..
        }
    ));
    let BlockKind::Paragraph { body } = &parsed.document.blocks[0].kind else {
        panic!("expected a paragraph");
    };
    assert!(matches!(
        body.as_slice(),
        [
            Inline::Reference {
                raw: "[same][]",
                title: "same",
                key: "same"
            },
            Inline::Text(" "),
            Inline::Footnote {
                raw: "[^same][]",
                title: Some("same"),
                key: "same"
            }
        ]
    ));
}

#[test]
fn former_footnote_alias_definition_is_plain_text() {
    let parsed = parse("[^same]: first");

    assert!(parsed.document.reference("same").is_none());
    assert!(parsed.diagnostics.is_empty());
    let BlockKind::Paragraph { body } = &parsed.document.blocks[0].kind else {
        panic!("expected a paragraph");
    };
    assert_eq!(body, &vec![Inline::Text("[^same]: first")]);
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
