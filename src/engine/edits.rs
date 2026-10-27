//! Resolve operator ranges and issue transactions.
use super::visual::visual_extent;
use super::{
    normalized_register, transform_block_positions, Action, AppliedEdit, BlockInsertion,
    ByteOffset, Command, DeletedRegister, DocumentView, EditTarget, EditingState, Effect,
    EngineError, Mode, OperationKind, Operator, PlannedEdit, Replacement, ResolvedRange, Selection,
    Session, Shape, Step, TargetRequest, Transaction,
};
use crate::{Motion, MotionResolver};

impl Session {
    pub(super) fn resolved_command(
        &mut self,
        state: &mut EditingState,
        command: Command,
        mut range: ResolvedRange,
        view: DocumentView<'_>,
    ) -> Step {
        if range.ranges.is_empty()
            || range.ranges.iter().any(|r| {
                r.start > r.end
                    || r.end.0 > view.text.len_bytes()
                    || !view.text.is_char_boundary(r.start.0)
                    || !view.text.is_char_boundary(r.end.0)
            })
            || range.ranges.windows(2).any(|r| r[0].end > r[1].start)
        {
            self.abort_input(state);
            return Step::Error(EngineError::InvalidDocument);
        }
        let resolver = MotionResolver::from_view(view);
        for r in &mut range.ranges {
            *r = resolver.expand_opaque(*r);
        }
        if range
            .ranges
            .windows(2)
            .any(|rows| rows[0].end > rows[1].start)
        {
            self.abort_input(state);
            return Step::Consumed;
        }
        let columns = block_columns(view, range.shape);
        match command.action {
            Action::Operate { operator, .. } => {
                self.edit_operator(state, command, operator, range, view)
            }
            Action::SelectTarget(_) => {
                let first = range.ranges[0].start;
                let end = range.ranges.last().expect("validated nonempty range").end;
                let cursor = ByteOffset(if first < end {
                    resolver.normal_cursor(resolver.previous(end.0))
                } else {
                    first.0
                });
                let selection = Selection {
                    anchor: first,
                    cursor,
                    shape: range.shape,
                };
                self.move_to(
                    state,
                    view,
                    cursor,
                    super::visual::shape_mode(range.shape),
                    Some(selection),
                )
            }
            Action::Target(_) => {
                let Some(last) = range.ranges.last() else {
                    return Step::Error(EngineError::InvalidDocument);
                };
                let cursor = last.end;
                let cursor = ByteOffset(resolver.normal_cursor(cursor.0));
                let selection = view.selection.map(|s| Selection { cursor, ..s });
                self.move_to(state, view, cursor, state.mode, selection)
            }
            Action::ReplaceChar(c) => {
                let replacements = range
                    .ranges
                    .iter()
                    .map(|r| {
                        if c == '\n' && !state.mode.is_visual() {
                            return Replacement {
                                range: *r,
                                text: "\n".into(),
                            };
                        }
                        if let Some(columns) = columns.clone() {
                            let row = resolver.block_row(*r, columns);
                            let mut bytes = [0; 4];
                            let width = view.geometry.width(c.encode_utf8(&mut bytes), 0).max(1);
                            return Replacement {
                                range: *r,
                                text: " ".repeat(row.before)
                                    + &c.to_string().repeat(row.cells / width)
                                    + &" ".repeat(row.cells % width + row.after),
                            };
                        }
                        let mut text = String::new();
                        let mut p = r.start.0;
                        while p < r.end.0 {
                            if matches!(resolver.head(p), Some('\n' | '\r')) {
                                text.push_str(&view.text.slice(p, resolver.next(p)));
                            } else {
                                text.push(c);
                            }
                            p = resolver.next(p);
                        }
                        Replacement { range: *r, text }
                    })
                    .collect();
                let cursor = if !state.mode.is_visual() && c == '\n' {
                    ByteOffset(range.ranges[0].start.0 + 1)
                } else if !state.mode.is_visual() {
                    ByteOffset(range.ranges[0].start.0 + c.len_utf8() * (command.count.max(1) - 1))
                } else {
                    range.ranges[0].start
                };
                self.edit(
                    state,
                    command,
                    view,
                    PlannedEdit {
                        replacements,
                        cursor,
                        mode: Mode::Normal,
                        register: None,
                    },
                )
            }
            Action::ToggleCase => {
                self.edit_operator(state, command, Operator::ToggleCase, range, view)
            }
            Action::BlockInsert { append } => {
                let left = view.selection.map_or_else(
                    || resolver.column(view.cursor.0),
                    |s| resolver.column(s.anchor.0).min(resolver.column(s.cursor.0)),
                );
                let Shape::Block { width } = range.shape else {
                    return Step::Error(EngineError::UnexpectedResult);
                };
                let first = &range.ranges[0];
                let cursor = if append { first.end } else { first.start };
                state.block_origin = Some(first.start);
                state.block_insert = range
                    .ranges
                    .iter()
                    .skip(1)
                    .filter_map(|row| {
                        let end = view.text.line_end(row.start.0);
                        let columns = resolver.column(end);
                        if !append && (columns < left || resolver.column(first.start.0) < left) {
                            return None;
                        }
                        let target = if append { left + width } else { left };
                        let offset =
                            resolver.at_visual_column(view.text.line_start(row.start.0), target);
                        let padding = target.saturating_sub(resolver.column(offset));
                        let tab_tail = (padding > 0 && view.text.byte_at(offset) == Some(b'\t'))
                            .then(|| resolver.column_span(offset).1 + 1 - target);
                        Some(BlockInsertion {
                            offset: ByteOffset(offset),
                            padding,
                            tab_tail,
                        })
                    })
                    .collect();
                self.edit(
                    state,
                    command,
                    view,
                    PlannedEdit {
                        replacements: Vec::new(),
                        cursor,
                        mode: Mode::Insert,
                        register: None,
                    },
                )
            }
            // Neovim's dot after a Visual put repeats the removal of the selected extent.
            Action::Paste { .. } if state.replay_tail.is_some() => {
                self.edit_operator(state, command, Operator::Delete, range, view)
            }
            Action::Paste { before } => {
                let Some(register) = self.registers.get('"').cloned() else {
                    return Step::Consumed;
                };
                let mut replacements = if matches!(register.shape(), Shape::Block { .. }) {
                    super::put::block_replacements(
                        view,
                        range.ranges[0].start.0,
                        &range.ranges,
                        register.text(),
                        command.count,
                    )
                } else {
                    range
                        .ranges
                        .iter()
                        .map(|r| Replacement {
                            range: *r,
                            text: if register.shape() == Shape::Line {
                                String::new()
                            } else {
                                register.text().repeat(command.count.max(1))
                            },
                        })
                        .collect()
                };
                let mut cursor = range.ranges[0].start;
                if register.shape() == Shape::Line {
                    let last = range.ranges.last().expect("nonempty block");
                    let offset = view.text.next_line(last.end.0);
                    let removed: usize = range.ranges.iter().map(|r| r.end.0 - r.start.0).sum();
                    let mut text = register.text().repeat(command.count.max(1));
                    cursor = ByteOffset(offset - removed);
                    if offset == view.text.len_bytes()
                        && view.text.byte_at(offset.saturating_sub(1)) != Some(b'\n')
                    {
                        text.insert(0, '\n');
                        text.pop();
                        cursor.0 += 1;
                    }
                    if before {
                        let offset = view.text.line_start(range.ranges[0].start.0);
                        cursor = ByteOffset(offset);
                        replacements.insert(
                            0,
                            Replacement {
                                range: crate::TextRange::new(offset, offset),
                                text: register.text().repeat(command.count.max(1)),
                            },
                        );
                    } else {
                        replacements.push(Replacement {
                            range: crate::TextRange::new(offset, offset),
                            text,
                        });
                    }
                }
                let mut projected_rows = Vec::new();
                if let Some(columns) = columns {
                    for (r, replacement) in range.ranges.iter().zip(&mut replacements) {
                        let row = resolver.block_row(*r, columns.clone());
                        if replacement.range == *r {
                            replacement.text =
                                " ".repeat(row.before) + &replacement.text + &" ".repeat(row.after);
                        }
                        projected_rows.push((*r, row.selected));
                    }
                }
                self.edit(
                    state,
                    command,
                    view,
                    PlannedEdit {
                        replacements,
                        cursor,
                        mode: Mode::Normal,
                        register: (!before).then_some(DeletedRegister {
                            shape: range.shape,
                            numbered: false,
                            separator_bytes: 0,
                            ends_on_empty_line: false,
                            projected_rows,
                        }),
                    },
                )
            }
            _ => Step::Error(EngineError::UnexpectedResult),
        }
    }

    pub(super) fn edit_operator(
        &mut self,
        state: &mut EditingState,
        command: Command,
        operator: Operator,
        mut range: ResolvedRange,
        view: DocumentView<'_>,
    ) -> Step {
        let shape = range.shape;
        let resolver = MotionResolver::from_view(view);
        let columns = block_columns(view, shape);
        let rows: Vec<_> = columns
            .into_iter()
            .flat_map(|cols| {
                range
                    .ranges
                    .iter()
                    .map(move |r| resolver.block_row(*r, cols.clone()))
            })
            .collect();
        if operator == Operator::Yank {
            let text = range
                .ranges
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    rows.get(i).map_or_else(
                        || view.text.slice(r.start.0, r.end.0),
                        |row| std::borrow::Cow::Borrowed(row.selected.as_str()),
                    )
                })
                .collect::<Vec<_>>()
                .join(if matches!(shape, Shape::Block { .. }) {
                    "\n"
                } else {
                    ""
                });
            self.registers.yank(normalized_register(text, shape));
            state.previous_visual = view.selection.or(state.previous_visual);
            let cursor = if state.mode.is_visual()
                || shape == Shape::Character && range.ranges[0].start < view.cursor
            {
                range.ranges[0].start
            } else {
                view.cursor
            };
            return self.move_to(state, view, cursor, Mode::Normal, None);
        }
        if matches!(
            operator,
            Operator::Indent | Operator::Outdent | Operator::Format | Operator::Comment
        ) {
            let effect = Effect::Command {
                command,
                mode: Mode::Normal,
                register: None,
                visual: visual_extent(view),
            };
            return self.request(
                state,
                view,
                OperationKind::Transform {
                    operator,
                    range,
                    count: command.count.max(1),
                },
                effect,
            );
        }
        // At EOF the last empty line has no separator of its own.
        let ends_on_empty_line = shape == Shape::Line
            && view.cursor.0 == view.text.len_bytes()
            && view.cursor.0 > 0
            && view.text.byte_at(view.cursor.0 - 1) == Some(b'\n');
        let mut prefix = 0;
        let case = matches!(
            operator,
            Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase
        );
        if !case && shape == Shape::Line {
            let r = &mut range.ranges[0];
            if operator == Operator::Change {
                // Keep the last line ending so the replacement remains a line.
                if !ends_on_empty_line
                    && r.end.0 > r.start.0
                    && view.text.byte_at(r.end.0 - 1) == Some(b'\n')
                {
                    r.end.0 -= 1;
                    if r.end.0 > r.start.0 && view.text.byte_at(r.end.0 - 1) == Some(b'\r') {
                        r.end.0 -= 1;
                    }
                }
            } else if r.end.0 == view.text.len_bytes() && r.start.0 > 0 {
                let new_start = view.text.line_end(r.start.0 - 1);
                prefix = r.start.0 - new_start;
                r.start = ByteOffset(new_start);
            }
        }
        let replacements: Vec<_> = range
            .ranges
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let text = if case {
                    transform_case(&view.text.slice(r.start.0, r.end.0), operator)
                } else if operator == Operator::Change && shape == Shape::Line {
                    view.text
                        .slice(r.start.0, resolver.first_non_blank(r.start.0))
                        .into_owned()
                } else if let Some(row) = rows.get(i) {
                    " ".repeat(row.before + row.after)
                } else {
                    String::new()
                };
                Replacement { range: *r, text }
            })
            .collect();
        let mut cursor = range.ranges[0].start;
        if operator == Operator::Change && shape == Shape::Line {
            cursor.0 += replacements[0].text.len();
        }
        if case && shape == Shape::Line {
            cursor = ByteOffset(resolver.first_non_blank(cursor.0));
        }
        if case && !state.mode.is_visual() && matches!(command.action, Action::ToggleCase) {
            cursor = range.ranges[0].end;
        }
        if matches!(operator, Operator::Delete | Operator::Change) && !rows.is_empty() {
            cursor.0 += rows[0].before;
        }
        if operator == Operator::Change && matches!(shape, Shape::Block { .. }) {
            state.block_origin = Some(cursor);
            state.block_insert = range
                .ranges
                .iter()
                .zip(&rows)
                .skip(1)
                .filter(|(_, row)| !row.short)
                .map(|(r, _)| BlockInsertion {
                    offset: r.start,
                    padding: 0,
                    tab_tail: None,
                })
                .collect();
            let removed: Vec<_> = replacements
                .iter()
                .map(|r| AppliedEdit {
                    range: r.range,
                    removed: String::new(),
                    inserted: r.text.clone(),
                })
                .collect();
            transform_block_positions(&mut state.block_insert, &removed);
            for (insert, row) in state
                .block_insert
                .iter_mut()
                .zip(rows.iter().skip(1).filter(|row| !row.short))
            {
                insert.offset.0 += row.before;
            }
        }
        let mode = if operator == Operator::Change {
            Mode::Insert
        } else {
            Mode::Normal
        };
        let numbered = matches!(
            command.action,
            Action::Operate {
                target: EditTarget::Motion(
                    Motion::MatchDelimiter | Motion::Sentence { .. } | Motion::Paragraph { .. }
                ) | EditTarget::Host(TargetRequest::Search { .. }),
                ..
            }
        );
        let register = (!case && !matches!(command.action, Action::Paste { before: true }))
            .then_some(DeletedRegister {
                shape,
                numbered,
                separator_bytes: prefix,
                ends_on_empty_line,
                projected_rows: range
                    .ranges
                    .iter()
                    .copied()
                    .zip(rows.into_iter().map(|row| row.selected))
                    .collect(),
            });
        if operator == Operator::Delete && shape == Shape::Line && prefix == 0 {
            let end = range.ranges[0].end.0;
            cursor.0 += resolver.first_non_blank(end).saturating_sub(end);
        }
        if prefix > 0 {
            cursor = ByteOffset(resolver.first_non_blank(range.ranges[0].start.0));
        }
        if mode == Mode::Normal && cursor.0 == view.text.len_bytes() {
            cursor = ByteOffset(resolver.normal_cursor(cursor.0));
        }
        self.edit(
            state,
            command,
            view,
            PlannedEdit {
                replacements,
                cursor,
                mode,
                register,
            },
        )
    }

    pub(super) fn edit(
        &mut self,
        state: &mut EditingState,
        command: Command,
        view: DocumentView<'_>,
        edit: PlannedEdit,
    ) -> Step {
        let PlannedEdit {
            replacements,
            cursor,
            mode,
            register,
        } = edit;
        let transaction = match mode {
            Mode::Insert | Mode::Replace => Transaction::Begin,
            _ => Transaction::Atomic,
        };
        let visual = visual_extent(view);
        if state.mode.is_visual() {
            state.previous_visual = view.selection;
        }
        self.request(
            state,
            view,
            OperationKind::Edit {
                replacements,
                cursor,
                selection: None,
                transaction,
            },
            Effect::Command {
                command,
                mode,
                register,
                visual,
            },
        )
    }
}

fn transform_case(text: &str, operator: Operator) -> String {
    let mut result = String::with_capacity(text.len());
    for character in text.chars() {
        if operator == Operator::Uppercase
            || (operator == Operator::ToggleCase && character.is_lowercase())
        {
            result.extend(character.to_uppercase());
        } else {
            result.extend(character.to_lowercase());
        }
    }
    result
}

fn block_columns(view: DocumentView<'_>, shape: Shape) -> Option<std::ops::Range<usize>> {
    let Shape::Block { width } = shape else {
        return None;
    };
    let resolver = MotionResolver::from_view(view);
    let left = view.selection.map_or_else(
        || resolver.column(view.cursor.0),
        |s| resolver.block_columns(s).start,
    );
    Some(left..left.saturating_add(width))
}
