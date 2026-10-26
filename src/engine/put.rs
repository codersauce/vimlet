//! Put register contents and join logical lines.
use super::visual::selection_range;
use super::{
    ByteOffset, Command, DeletedRegister, DocumentView, EditingState, Mode, PlannedEdit, Register,
    Replacement, Session, Shape, Step, TargetRequest,
};
use crate::{MotionResolver, TextRange, TextSource};

impl Session {
    pub(super) fn paste(
        &mut self,
        state: &mut EditingState,
        command: Command,
        before: bool,
        view: DocumentView<'_>,
    ) -> Step {
        let register = self
            .registers
            .get('"')
            .cloned()
            .unwrap_or_else(|| Register::from_text("", Shape::Character));
        if register.text().is_empty() && view.selection.is_none() {
            return Step::Consumed;
        }
        let resolver = MotionResolver::from_view(view);
        let register_effect = view
            .selection
            .filter(|_| !before)
            .map(|selection| DeletedRegister {
                shape: selection.shape,
                numbered: false,
                separator_bytes: 0,
            });
        if let Some(selection) = view
            .selection
            .filter(|s| matches!(s.shape, Shape::Block { .. }))
        {
            return self.resolve_request(state, command, TargetRequest::Block { selection }, view);
        }
        let selected = view
            .selection
            .map(|s| selection_range(resolver, s).ranges[0]);
        if matches!(register.shape(), Shape::Block { .. })
            && !view.selection.is_some_and(|s| s.shape == Shape::Line)
        {
            let first = if let Some(range) = selected {
                range.start.0
            } else if before {
                view.cursor.0
            } else {
                resolver
                    .next(view.cursor.0)
                    .min(view.text.line_end(view.cursor.0))
            };
            let replacements = block_replacements(
                view,
                first,
                selected.as_slice(),
                register.text(),
                command.count,
            );
            return self.edit(
                state,
                command,
                view,
                PlannedEdit {
                    replacements,
                    cursor: ByteOffset(first),
                    mode: Mode::Normal,
                    register: register_effect,
                },
            );
        }
        let mut text = register.text().repeat(command.count.max(1));
        let (range, cursor) = if let Some(selection) = view.selection {
            let range = selected.expect("selection range");
            let cursor = if selection.shape == Shape::Line {
                if !text.ends_with('\n')
                    && view.text.byte_at(range.end.0.saturating_sub(1)) == Some(b'\n')
                {
                    text.push('\n');
                } else if range.end.0 == view.text.len_bytes()
                    && view.text.byte_at(range.end.0.saturating_sub(1)) != Some(b'\n')
                    && text.ends_with('\n')
                {
                    text.pop();
                }
                range.start.0 + text.len() - text.trim_start_matches([' ', '\t']).len()
            } else if register.shape() == Shape::Line {
                text.insert(0, '\n');
                range.start.0 + 1
            } else if text.contains('\n') {
                range.start.0
            } else {
                range.start.0 + text.prev_grapheme(text.len())
            };
            (range, ByteOffset(cursor))
        } else if register.shape() == Shape::Line {
            let p = if before {
                view.text.line_start(view.cursor.0)
            } else {
                view.text.next_line(view.cursor.0)
            };
            let mut cursor = p;
            if !before
                && p == view.text.len_bytes()
                && p > 0
                && view.text.byte_at(p - 1) != Some(b'\n')
            {
                text.insert(0, '\n');
                text.pop();
                cursor += 1;
            }
            (TextRange::new(p, p), ByteOffset(cursor))
        } else {
            let p = if before {
                view.cursor.0
            } else {
                resolver
                    .next(view.cursor.0)
                    .min(view.text.line_end(view.cursor.0))
            };
            let cursor = if text.contains('\n') {
                p
            } else {
                p + text.prev_grapheme(text.len())
            };
            (TextRange::new(p, p), ByteOffset(cursor))
        };
        self.edit(
            state,
            command,
            view,
            PlannedEdit {
                replacements: vec![Replacement { range, text }],
                cursor,
                mode: Mode::Normal,
                register: register_effect,
            },
        )
    }

    pub(super) fn join(
        &mut self,
        state: &mut EditingState,
        command: Command,
        spaces: bool,
        view: DocumentView<'_>,
    ) -> Step {
        let first = view
            .selection
            .map_or(view.cursor.0, |s| s.anchor.0.min(s.cursor.0));
        let start = view.text.line_start(first);
        let count = view.selection.map_or(command.count.max(2), |s| {
            view.text.line_index(s.anchor.0.max(s.cursor.0)) - view.text.line_index(first) + 1
        });
        let mut end = view.text.line_end(start);
        let mut text = view.text.slice(start, end).into_owned();
        let mut cursor = start + text.len();
        let mut joined = false;
        for _ in 1..count {
            let next = view.text.next_line(end);
            if next <= end || next >= view.text.len_bytes() {
                break;
            }
            cursor = start + text.len();
            end = view.text.line_end(next);
            let source = view.text.slice(next, end);
            let source = if spaces {
                source.trim_start_matches([' ', '\t'])
            } else {
                source.as_ref()
            };
            if spaces
                && !text.is_empty()
                && !text.ends_with([' ', '\t'])
                && !source.starts_with(')')
                && !source.is_empty()
            {
                text.push(' ');
            }
            text.push_str(source);
            joined = true;
        }
        if !joined {
            return Step::Consumed;
        }
        self.edit(
            state,
            command,
            view,
            PlannedEdit {
                replacements: vec![Replacement {
                    range: TextRange::new(start, end),
                    text,
                }],
                cursor: ByteOffset(cursor),
                mode: Mode::Normal,
                register: None,
            },
        )
    }
}

pub(super) fn block_replacements(
    view: DocumentView<'_>,
    first: usize,
    ranges: &[TextRange],
    text: &str,
    count: usize,
) -> Vec<Replacement> {
    let resolver = MotionResolver::from_view(view);
    let column = resolver.column(first);
    let mut line = Some(view.text.line_start(first));
    let mut rows = text.split('\n').peekable();
    let mut replacements: Vec<Replacement> = Vec::new();
    let mut extra = String::new();
    for index in 0.. {
        let selected = ranges.get(index).copied();
        if rows.peek().is_none() && selected.is_none() {
            break;
        }
        let row = rows.next().unwrap_or("").repeat(count.max(1));
        let Some(start) = line else {
            extra.push('\n');
            extra.push_str(&" ".repeat(column));
            extra.push_str(&row);
            continue;
        };
        let end = view.text.line_end(start);
        let end_column = resolver.column(end);
        let pos = if column >= end_column {
            end
        } else {
            resolver.at_column(start, column)
        };
        let range = selected.unwrap_or(TextRange::new(pos, pos));
        let padding = if row.is_empty() {
            0
        } else {
            column.saturating_sub(end_column)
        };
        replacements.push(Replacement {
            range,
            text: " ".repeat(padding) + &row,
        });
        line = (view.text.line_end(range.end.0) < view.text.len_bytes())
            .then(|| view.text.next_line(range.end.0));
    }
    if !extra.is_empty() {
        let eof = view.text.len_bytes();
        if let Some(last) = replacements.last_mut().filter(|r| r.range.end.0 == eof) {
            last.text.push_str(&extra);
        } else {
            replacements.push(Replacement {
                range: TextRange::new(eof, eof),
                text: extra,
            });
        }
    }
    replacements
}
