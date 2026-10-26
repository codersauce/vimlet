//! Resolve operator ranges and issue transactions.
use super::visual::visual_extent;
use super::{
    normalized_register, transform_block_positions, Action, AppliedEdit, ByteOffset, Command,
    DeletedRegister, DocumentView, EditTarget, EditingState, Effect, EngineError, Mode,
    OperationKind, Operator, PlannedEdit, Replacement, ResolvedRange, Selection, Session, Shape,
    Step, TargetRequest, Transaction,
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
        match command.action {
            Action::Operate { operator, .. } => {
                self.edit_operator(state, command, operator, range, view)
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
                let cursor = if !state.mode.is_visual() && c != '\n' {
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
                let positions: Vec<_> = range
                    .ranges
                    .iter()
                    .map(|r| if append { r.end } else { r.start })
                    .collect();
                let cursor = positions[0];
                state.block_insert = positions[1..].to_vec();
                state.block_origin = Some(range.ranges[0].start);
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
            Action::Paste { .. } => {
                let Some(register) = self.registers.get('"').cloned() else {
                    return Step::Consumed;
                };
                let lines: Vec<_> = register.text().lines().collect();
                let replacements = range
                    .ranges
                    .iter()
                    .zip(lines.iter().cycle())
                    .map(|(r, text)| Replacement {
                        range: *r,
                        text: text.repeat(command.count.max(1)),
                    })
                    .collect();
                self.edit(
                    state,
                    command,
                    view,
                    PlannedEdit {
                        replacements,
                        cursor: range.ranges[0].start,
                        mode: Mode::Normal,
                        register: None,
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
        if operator == Operator::Yank {
            let text = range
                .ranges
                .iter()
                .map(|r| view.text.slice(r.start.0, r.end.0))
                .collect::<Vec<_>>()
                .join(if matches!(shape, Shape::Block { .. }) {
                    "\n"
                } else {
                    ""
                });
            self.registers.yank(normalized_register(text, shape));
            state.previous_visual = view.selection.or(state.previous_visual);
            let cursor = if state.mode.is_visual() {
                range.ranges[0].start
            } else {
                view.cursor
            };
            return self.move_to(state, view, cursor, Mode::Normal, None);
        }
        if matches!(
            operator,
            Operator::Indent | Operator::Outdent | Operator::Format
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
        let mut prefix = 0;
        let case = matches!(
            operator,
            Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase
        );
        if !case && shape == Shape::Line {
            let r = &mut range.ranges[0];
            if operator == Operator::Change {
                // Keep the last line ending so the replacement remains a line.
                if r.end.0 > r.start.0 && view.text.byte_at(r.end.0 - 1) == Some(b'\n') {
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
            .map(|r| {
                let text = if case {
                    transform_case(&view.text.slice(r.start.0, r.end.0), operator)
                } else if operator == Operator::Change && shape == Shape::Line {
                    view.text
                        .slice(r.start.0, resolver.first_non_blank(r.start.0))
                        .into_owned()
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
        if operator == Operator::Change && matches!(shape, Shape::Block { .. }) {
            state.block_origin = Some(range.ranges[0].start);
            state.block_insert = range.ranges.iter().skip(1).map(|r| r.start).collect();
            let removed: Vec<_> = range
                .ranges
                .iter()
                .map(|r| AppliedEdit {
                    range: *r,
                    removed: String::new(),
                    inserted: String::new(),
                })
                .collect();
            transform_block_positions(&mut state.block_insert, &removed);
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
        let register = (!case).then_some(DeletedRegister {
            shape,
            numbered,
            separator_bytes: prefix,
        });
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
