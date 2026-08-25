#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

impl SourceSpan {
    pub const fn new(start: usize, end: usize) -> Self {
        assert!(start <= end, "source span start must not exceed end");
        Self { start, end }
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub const fn len(self) -> usize {
        self.end - self.start
    }

    pub const fn contains(self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePosition {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utf16Position {
    pub line: usize,
    pub character: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMap<'a> {
    source: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> SourceMap<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(
            source
                .bytes()
                .enumerate()
                .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset + 1)),
        );

        Self {
            source,
            line_starts,
        }
    }

    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn line_start(&self, line: usize) -> Option<usize> {
        self.line_starts.get(line).copied()
    }

    pub fn line_span(&self, line: usize) -> Option<SourceSpan> {
        let start = self.line_start(line)?;
        let mut end = self
            .line_starts
            .get(line + 1)
            .map_or(self.source.len(), |next| next - 1);
        if end > start && self.source.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }

        Some(SourceSpan::new(start, end))
    }

    pub fn position(&self, offset: usize) -> Option<SourcePosition> {
        if offset > self.source.len() || !self.source.is_char_boundary(offset) {
            return None;
        }

        let line = self.line_index(offset);
        Some(SourcePosition {
            line,
            column: offset - self.line_starts[line],
        })
    }

    pub fn utf16_position(&self, offset: usize) -> Option<Utf16Position> {
        let position = self.position(offset)?;
        let line_start = self.line_starts[position.line];
        let character = self.source[line_start..offset].encode_utf16().count();

        Some(Utf16Position {
            line: position.line,
            character,
        })
    }

    pub fn offset(&self, position: SourcePosition) -> Option<usize> {
        let span = self.line_span(position.line)?;
        let offset = span.start.checked_add(position.column)?;

        (offset <= span.end && self.source.is_char_boundary(offset)).then_some(offset)
    }

    pub fn offset_utf16(&self, position: Utf16Position) -> Option<usize> {
        let span = self.line_span(position.line)?;
        let line = &self.source[span.start..span.end];
        let mut utf16_column = 0;

        for (byte_column, character) in line.char_indices() {
            if utf16_column == position.character {
                return Some(span.start + byte_column);
            }
            utf16_column += character.len_utf16();
            if utf16_column > position.character {
                return None;
            }
        }

        (utf16_column == position.character).then_some(span.end)
    }

    fn line_index(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|start| *start <= offset) - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_map_tracks_lf_crlf_and_trailing_empty_line() {
        let map = SourceMap::new("one\r\ntwo\n");

        assert_eq!(map.line_count(), 3);
        assert_eq!(map.line_span(0), Some(SourceSpan::new(0, 3)));
        assert_eq!(map.line_span(1), Some(SourceSpan::new(5, 8)));
        assert_eq!(map.line_span(2), Some(SourceSpan::new(9, 9)));
    }

    #[test]
    fn source_map_converts_multibyte_text_to_utf16_positions() {
        let source = "한글 日本語 😀x";
        let map = SourceMap::new(source);
        let x_offset = source.find('x').expect("x should exist");

        assert_eq!(
            map.position(x_offset),
            Some(SourcePosition {
                line: 0,
                column: x_offset,
            })
        );
        assert_eq!(
            map.utf16_position(x_offset),
            Some(Utf16Position {
                line: 0,
                character: 9,
            })
        );
        assert_eq!(
            map.offset_utf16(Utf16Position {
                line: 0,
                character: 9,
            }),
            Some(x_offset)
        );
        assert_eq!(
            map.offset_utf16(Utf16Position {
                line: 0,
                character: 8,
            }),
            None,
            "a UTF-16 position cannot split an emoji surrogate pair"
        );
    }
}
