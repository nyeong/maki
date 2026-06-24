//! Maki parser

use std::collections::HashMap;

pub(crate) fn parse(content: &str) -> Document {
    Document {
        blocks: parse_blocks(content),
    }
}

fn parse_blocks(source: &str) -> Vec<Block> {
    let mut blocks = vec![];
    let mut paragraph_lines = vec![];

    for line in source.lines() {
        if line.trim().is_empty() {
            flush_paragraph(&mut paragraph_lines, &mut blocks);
            continue;
        }

        if let Some(heading) = parse_heading_line(line) {
            flush_paragraph(&mut paragraph_lines, &mut blocks);
            blocks.push(Block::new(heading));
            continue;
        }
        paragraph_lines.push(line.to_string());
    }

    flush_paragraph(&mut paragraph_lines, &mut blocks);

    blocks
}

fn flush_paragraph(paragraph_lines: &mut Vec<String>, blocks: &mut Vec<Block>) {
    if !paragraph_lines.is_empty() {
        blocks.push(Block::new(BlockBody::Paragraph {
            content: parse_inlines(&paragraph_lines.join("\n")),
        }));
        paragraph_lines.clear();
    }
}

fn parse_heading_line(line: &str) -> Option<BlockBody> {
    let marker_len = line.chars().take_while(|&c| c == '=').count();

    if marker_len == 0 || marker_len > 6 {
        return None;
    }
    let rest = &line[marker_len..];
    if !rest.starts_with(' ') {
        return None;
    }

    let text = rest.trim_start();

    Some(BlockBody::Heading {
        level: marker_len.into(),
        content: parse_inlines(text),
    })
}

fn parse_inlines(mut rest: &str) -> Vec<Inline> {
    let mut inlines = vec![];

    while let Some(start) = rest.find("[[") {
        let before = &rest[..start];
        if !before.is_empty() {
            inlines.push(Inline::Text(before.to_string()));
        }

        let inner_start = &rest[start + 2..];
        let Some(end) = inner_start.find("]]") else {
            inlines.push(Inline::Text(rest[start..].to_string()));
            return inlines;
        };

        let raw_target = &inner_start[..end];

        if raw_target.is_empty() {
            inlines.push(Inline::Text(rest[start..].to_string()));
        } else {
            let note_link = parse_note_link(raw_target);
            inlines.push(note_link);
        }

        rest = &inner_start[end + 2..];
    }

    if !rest.is_empty() {
        inlines.push(Inline::Text(rest.to_string()));
    }

    inlines
}

/// NoteLink에서 구분자를 제외한 구문을 입력으로 받아 파싱합니다
fn parse_note_link(raw_target: &str) -> Inline {
    Inline::NoteLink {
        target: raw_target.to_string(),
    }
}

type Props = HashMap<String, String>;

/// Parsed document written in Maki syntax.
pub(crate) struct Document {
    blocks: Vec<Block>,
}

impl Document {
    pub(crate) fn blocks(&self) -> &[Block] {
        &self.blocks
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum HeadingLevel {
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
}

impl From<HeadingLevel> for usize {
    fn from(level: HeadingLevel) -> usize {
        match level {
            HeadingLevel::H1 => 1,
            HeadingLevel::H2 => 2,
            HeadingLevel::H3 => 3,
            HeadingLevel::H4 => 4,
            HeadingLevel::H5 => 5,
            HeadingLevel::H6 => 6,
        }
    }
}

impl From<usize> for HeadingLevel {
    fn from(level: usize) -> Self {
        match level {
            1 => HeadingLevel::H1,
            2 => HeadingLevel::H2,
            3 => HeadingLevel::H3,
            4 => HeadingLevel::H4,
            5 => HeadingLevel::H5,
            6 => HeadingLevel::H6,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum Inline {
    Text(String),
    NoteLink { target: String },
}

#[derive(Debug, PartialEq)]
pub(crate) struct Block {
    body: BlockBody,
    props: Props,
}

impl Block {
    fn new(body: BlockBody) -> Self {
        let props = HashMap::new();
        Self { body, props }
    }

    pub(crate) fn body(&self) -> &BlockBody {
        &self.body
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum BlockBody {
    Heading {
        level: HeadingLevel,
        content: Vec<Inline>,
    },
    Paragraph {
        content: Vec<Inline>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h1(raw: &str) -> Block {
        Block::new(BlockBody::Heading {
            level: HeadingLevel::H1,
            content: parse_inlines(raw),
        })
    }

    fn h2(raw: &str) -> Block {
        Block::new(BlockBody::Heading {
            level: HeadingLevel::H2,
            content: parse_inlines(raw),
        })
    }

    fn h3(raw: &str) -> Block {
        Block::new(BlockBody::Heading {
            level: HeadingLevel::H3,
            content: parse_inlines(raw),
        })
    }

    fn text(raw: &str) -> Inline {
        Inline::Text(raw.to_string())
    }

    fn note_link(raw: &str) -> Inline {
        Inline::NoteLink {
            target: raw.to_string(),
        }
    }

    fn p(raw: &str) -> Block {
        Block::new(BlockBody::Paragraph {
            content: parse_inlines(raw),
        })
    }

    #[test]
    fn parse_paragraph() {
        let content = "Hello, World!";
        let doc = parse(content);

        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0], p("Hello, World!"));
        assert!(matches!(doc.blocks[0].body, BlockBody::Paragraph { .. }));
    }

    #[test]
    fn parse_headings() {
        let content = r#"
= heading 1
== heading 2
=== heading 3
"#;

        let doc = parse(content);
        assert_eq!(doc.blocks[0], h1("heading 1"));
        assert_eq!(doc.blocks[1], h2("heading 2"));
        assert_eq!(doc.blocks[2], h3("heading 3"));
    }

    #[test]
    fn parse_note_link() {
        let content = r#"Hello, [[World]]!"#;
        let doc = parse_inlines(content);

        assert_eq!(doc, vec![text("Hello, "), note_link("World"), text("!")])
    }
}
