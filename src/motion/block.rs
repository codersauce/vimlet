//! Project an inclusive pair of cursors onto the display cells of each logical row.

use super::{MotionResolver, ResolvedRange};
use crate::{Selection, Shape, TextRange};
use std::ops::Range;

impl MotionResolver<'_> {
    /// Return the half-open display columns of the inclusive rectangle.
    ///
    /// Both endpoint graphemes contribute their full width. A cursor at line end spans one cell.
    pub fn block_columns(&self, selection: Selection) -> Range<usize> {
        let (a, end_a) = self.column_span(selection.anchor.0);
        let (b, end_b) = self.column_span(selection.cursor.0);
        a.min(b)..end_a.max(end_b) + 1
    }

    /// Resolve a rectangular selection into one half-open UTF-8 byte range per logical line.
    ///
    /// A partially covered grapheme is included in its row. A row ending at or before the left
    /// edge contributes an empty range at its end, so no operation can consume its newline.
    pub fn block_range(&self, selection: Selection) -> ResolvedRange {
        let columns = self.block_columns(selection);
        let mut line = self
            .text
            .line_start(selection.anchor.0.min(selection.cursor.0));
        let last = self
            .text
            .line_start(selection.anchor.0.max(selection.cursor.0));
        let mut ranges = Vec::new();
        loop {
            let end = self.text.line_end(line);
            let from = self.at_visual_column(line, columns.start).min(end);
            let to = self.at_visual_column(line, columns.end - 1);
            ranges.push(TextRange::new(from, self.next(to).min(end)));
            if line == last {
                break;
            }
            line = self.text.next_line(line);
        }
        ResolvedRange {
            ranges,
            shape: Shape::Block {
                width: columns.len(),
            },
        }
    }

    pub(crate) fn block_row(&self, range: TextRange, columns: Range<usize>) -> BlockRow {
        let start = self.column(range.start.0);
        let mut column = start;
        let mut selected = String::new();
        let mut pos = range.start.0;
        while pos < range.end.0 {
            let next = self.next(pos).min(range.end.0);
            let text = self.text.slice(pos, next);
            let end = column + self.geometry.width(&text, column);
            if column >= columns.start && end <= columns.end || self.span(pos).is_some() {
                selected.push_str(&text);
            } else {
                let overlap = end
                    .min(columns.end)
                    .saturating_sub(column.max(columns.start));
                selected.extend(std::iter::repeat_n(' ', overlap));
            }
            column = end;
            pos = next;
        }
        // A row strictly shorter than the starting column yanks as virtual spaces.
        if range.is_empty() && start < columns.start {
            selected = " ".repeat(columns.len());
        }
        let atomic = self
            .opaque
            .iter()
            .any(|span| span.start < range.end && range.start < span.end);
        BlockRow {
            selected,
            before: if atomic || range.is_empty() {
                0
            } else {
                columns.start.saturating_sub(start)
            },
            after: if atomic {
                0
            } else {
                column.saturating_sub(columns.end)
            },
            cells: column
                .min(columns.end)
                .saturating_sub(start.max(columns.start)),
            short: range.is_empty() && start < columns.start,
        }
    }
}

pub(crate) struct BlockRow {
    pub(crate) selected: String,
    pub(crate) before: usize,
    pub(crate) after: usize,
    pub(crate) cells: usize,
    pub(crate) short: bool,
}
