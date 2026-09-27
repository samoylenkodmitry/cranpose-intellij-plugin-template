//! Convert parser character positions to source bytes and IDE UTF-16 offsets.
//! Build once per parse, then look up spans without rescanning source prefixes.
use crate::Range;
use proc_macro2::{LineColumn, Span};

pub(super) struct SourceIndex {
    line_scalars: Vec<usize>,
    non_ascii: Vec<Adjustment>,
}

struct Adjustment {
    scalar: usize,
    extra_bytes: usize,
    extra_utf16: usize,
}

impl SourceIndex {
    pub(super) fn new(source: &str) -> Self {
        let mut index = Self {
            line_scalars: vec![0],
            non_ascii: vec![],
        };
        let (mut extra_bytes, mut extra_utf16) = (0, 0);
        for (scalar, ch) in source.chars().enumerate() {
            if ch == '\n' {
                index.line_scalars.push(scalar + 1);
            } else if !ch.is_ascii() {
                extra_bytes += ch.len_utf8() - 1;
                extra_utf16 += ch.len_utf16() - 1;
                index.non_ascii.push(Adjustment {
                    scalar,
                    extra_bytes,
                    extra_utf16,
                });
            }
        }
        index
    }

    pub(super) fn offsets(&self, point: LineColumn) -> (usize, usize) {
        // proc_macro2 columns count Unicode scalar values, not encoded bytes.
        // All points come from spans in the source used to build this index.
        let scalar = self.line_scalars[point.line - 1] + point.column;
        let preceding = self.non_ascii.partition_point(|a| a.scalar < scalar);
        if preceding == 0 {
            (scalar, scalar)
        } else {
            let adjustment = &self.non_ascii[preceding - 1];
            (
                scalar + adjustment.extra_bytes,
                scalar + adjustment.extra_utf16,
            )
        }
    }

    pub(super) fn range(&self, span: Span) -> Range {
        let (start, start_utf16) = self.offsets(span.start());
        let (end, end_utf16) = self.offsets(span.end());
        Range {
            start,
            end,
            start_utf16,
            end_utf16,
            line: span.start().line,
            column: span.start().column + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_boundaries(source: &str) {
        let index = SourceIndex::new(source);
        let (mut line, mut column, mut utf16) = (1, 0, 0);
        for (byte, ch) in source.char_indices() {
            assert_eq!(index.offsets(LineColumn { line, column }), (byte, utf16));
            utf16 += ch.len_utf16();
            if ch == '\n' {
                line += 1;
                column = 0;
            } else {
                column += 1;
            }
        }
        assert_eq!(
            index.offsets(LineColumn { line, column }),
            (source.len(), utf16)
        );
    }

    #[test]
    fn every_character_boundary_matches_utf8_and_utf16() {
        for source in [
            "",
            "ascii",
            "\n",
            "\r\n\n",
            "a\r\nb\nc\r",
            "é🙂\n漢字\n",
            "\u{7f}\u{80}\u{7ff}\u{800}\u{ffff}\u{10000}\u{10ffff}",
            "Cafe\u{301} 👩\u{200d}💻 trailing\n",
        ] {
            check_boundaries(source);
        }
        // Many positions on one line and across empty / CRLF lines exercise
        // both binary-search boundaries and accumulated encoding differences.
        check_boundaries(&"aé🦀z".repeat(4096));
        check_boundaries(&"\n\r\nCafé 🦀\n".repeat(4096));
    }

    #[test]
    fn ascii_sources_need_only_line_starts() {
        let index = SourceIndex::new(&"Text(123);\n".repeat(4096));
        assert!(index.non_ascii.is_empty());
        assert_eq!(index.line_scalars.len(), 4097);
    }
}
