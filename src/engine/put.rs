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
        if matches!(register.shape(), Shape::Block { .. }) && view.selection.is_none() {
            let first = if before {
                view.cursor.0
            } else {
                resolver
                    .next(view.cursor.0)
                    .min(view.text.line_end(view.cursor.0))
            };
            let column = resolver.column(first);
            let mut line = view.text.line_start(first);
            let mut replacements: Vec<Replacement> = Vec::new();
            let mut extra_lines = String::new();
            let mut beyond_eof = false;
            for row in register.text().split('\n') {
                let contents = row.repeat(command.count.max(1));
                if beyond_eof {
                    extra_lines.push('\n');
                    extra_lines.push_str(&" ".repeat(column));
                    extra_lines.push_str(&contents);
                    continue;
                }
                let end = view.text.line_end(line);
                let end_column = resolver.column(end);
                let pos = if column >= end_column {
                    end
                } else {
                    resolver.at_column(line, column)
                };
                let padding = " ".repeat(column.saturating_sub(end_column));
                replacements.push(Replacement {
                    range: TextRange::new(pos, pos),
                    text: padding + &contents,
                });
                beyond_eof = end == view.text.len_bytes();
                line = view.text.next_line(line);
            }
            if !extra_lines.is_empty() {
                let eof = view.text.len_bytes();
                if let Some(last) = replacements.last_mut().filter(|r| r.range.start.0 == eof) {
                    last.text.push_str(&extra_lines);
                } else {
                    replacements.push(Replacement {
                        range: TextRange::new(eof, eof),
                        text: extra_lines,
                    });
                }
            }
            return self.edit(
                state,
                command,
                view,
                PlannedEdit {
                    replacements,
                    cursor: ByteOffset(first),
                    mode: Mode::Normal,
                    register: None,
                },
            );
        }
        let mut text = register.text().repeat(command.count.max(1));
        let (range, cursor) = if let Some(selection) = view.selection {
            if matches!(selection.shape, Shape::Block { .. }) {
                return self.resolve_request(
                    state,
                    command,
                    TargetRequest::Block { selection },
                    view,
                );
            }
            let range = selection_range(resolver, selection).ranges[0];
            let cursor = if register.shape() == Shape::Line {
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
            let cursor = p + text.prev_grapheme(text.len());
            (TextRange::new(p, p), ByteOffset(cursor))
        };
        let register_effect = view.selection.map(|s| DeletedRegister {
            shape: s.shape,
            numbered: false,
            separator_bytes: 0,
        });
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
        let cursor = start + text.len();
        let mut joined = false;
        for _ in 1..count {
            let next = view.text.next_line(end);
            if next <= end || next >= view.text.len_bytes() {
                break;
            }
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
