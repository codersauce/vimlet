use super::ReplaceRecovery;
use super::{
    Action, ActiveChange, ByteOffset, Command, DocumentView, EditingState, Effect, EngineError,
    InsertEdit, InsertStyle, Key, Mode, OperationKind, PlannedEdit, Recipe, Replacement, Session,
    Step, Transaction,
};
use crate::{AppliedEdit, MotionResolver, TextRange, TextSource};

impl Session {
    pub(super) fn begin_insert(
        &mut self,
        state: &mut EditingState,
        command: Command,
        style: InsertStyle,
        view: DocumentView<'_>,
    ) -> Step {
        if style == InsertStyle::Replace && !state.capabilities.replace {
            return Step::Error(EngineError::UnsupportedMode);
        }
        let resolver = MotionResolver::from_view(view);
        let mode = if style == InsertStyle::Replace {
            Mode::Replace
        } else {
            Mode::Insert
        };
        let mut replacements = Vec::new();
        let cursor = match style {
            InsertStyle::Before | InsertStyle::Replace => view.cursor.0,
            InsertStyle::After => resolver.next_on_line(view.cursor.0),
            InsertStyle::FirstNonBlank => resolver.first_non_blank(view.cursor.0),
            InsertStyle::LineEnd => view.text.line_end(view.cursor.0),
            InsertStyle::OpenAbove => {
                let start = view.text.line_start(view.cursor.0);
                replacements.push(Replacement {
                    range: TextRange::new(start, start),
                    text: "\n".into(),
                });
                start
            }
            InsertStyle::OpenBelow => {
                let end = view.text.line_end(view.cursor.0);
                replacements.push(Replacement {
                    range: TextRange::new(end, end),
                    text: "\n".into(),
                });
                end + 1
            }
        };
        self.edit(
            state,
            command,
            view,
            PlannedEdit {
                replacements,
                cursor: ByteOffset(cursor),
                mode,
                register: None,
            },
        )
    }

    pub(super) fn insert_key(
        &mut self,
        state: &mut EditingState,
        key: Key,
        view: DocumentView<'_>,
    ) -> Step {
        match key {
            Key::Escape | Key::Ctrl('[') => self.finish_insert(state, view),
            Key::Char(c) => self.insert_action(state, InsertEdit::Text(c.to_string()), view),
            Key::Text(text) => self.insert_action(state, InsertEdit::Text(text), view),
            Key::Enter => self.insert_action(state, InsertEdit::Text("\n".into()), view),
            Key::Tab => self.insert_action(state, InsertEdit::Text("\t".into()), view),
            Key::Backspace | Key::Ctrl('h') => {
                self.insert_action(state, InsertEdit::Backspace, view)
            }
            Key::Delete => self.insert_action(state, InsertEdit::Delete, view),
            Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End => {
                state.replace_recovery.clear();
                let motion = match key {
                    Key::Left => crate::Motion::Left,
                    Key::Right => crate::Motion::Right,
                    Key::Up => crate::Motion::Up,
                    Key::Down => crate::Motion::Down,
                    Key::Home => crate::Motion::LineStart,
                    _ => crate::Motion::LineEnd,
                };
                let resolver = MotionResolver::from_view(view);
                let cursor = match key {
                    Key::Right => resolver.next_on_line(view.cursor.0),
                    Key::End => view.text.line_end(view.cursor.0),
                    _ => resolver
                        .resolve(motion, 1)
                        .map_or(view.cursor.0, |target| target.position.0),
                };
                self.request(
                    state,
                    view,
                    OperationKind::Edit {
                        replacements: Vec::new(),
                        cursor: ByteOffset(cursor),
                        selection: None,
                        transaction: Transaction::BreakInsert,
                    },
                    Effect::BreakInsert,
                )
            }
            _ => Step::Unhandled,
        }
    }

    pub(super) fn insert_action(
        &mut self,
        state: &mut EditingState,
        action: InsertEdit,
        view: DocumentView<'_>,
    ) -> Step {
        let resolver = MotionResolver::from_view(view);
        let p = view.cursor.0;
        let mut effect = Effect::Insert(action.clone());
        let replacement = match &action {
            InsertEdit::Host(command) => {
                state.begin_insert_edit();
                return self.request(
                    state,
                    view,
                    OperationKind::Host {
                        command: *command,
                        count: 1,
                    },
                    effect,
                );
            }
            InsertEdit::Text(text) => {
                let mut end = p;
                if state.mode == Mode::Replace {
                    use unicode_segmentation::UnicodeSegmentation;
                    for g in text.graphemes(true) {
                        if matches!(g, "\n" | "\r\n") {
                            break;
                        }
                        if !resolver.at_line_end(end) {
                            end = resolver.next(end);
                        }
                    }
                }
                Replacement {
                    range: TextRange::new(p, end),
                    text: text.clone(),
                }
            }
            InsertEdit::Backspace => {
                if p == 0 {
                    return Step::Consumed;
                }
                if state.mode == Mode::Replace {
                    if let Some(recovery) = state.replace_recovery.last() {
                        if p == recovery.start + recovery.inserted.len()
                            && view.text.slice(recovery.start, p) == recovery.inserted
                        {
                            effect = Effect::ReplaceBackspace;
                            Replacement {
                                range: TextRange::new(resolver.previous(p), p),
                                text: recovery.removed.clone(),
                            }
                        } else {
                            return self.move_to(
                                state,
                                view,
                                ByteOffset(resolver.previous(p)),
                                state.mode,
                                None,
                            );
                        }
                    } else {
                        return self.move_to(
                            state,
                            view,
                            ByteOffset(resolver.previous(p)),
                            state.mode,
                            None,
                        );
                    }
                } else {
                    Replacement {
                        range: TextRange::new(resolver.previous(p), p),
                        text: String::new(),
                    }
                }
            }
            InsertEdit::Delete => {
                if p == view.text.len_bytes() {
                    return Step::Consumed;
                }
                Replacement {
                    range: TextRange::new(p, resolver.next(p)),
                    text: String::new(),
                }
            }
        };
        let transaction = state.begin_insert_edit();
        let cursor = if matches!(action, InsertEdit::Backspace | InsertEdit::Delete) {
            replacement.range.start
        } else {
            ByteOffset(replacement.range.start.0 + replacement.text.len())
        };
        self.request(
            state,
            view,
            OperationKind::Edit {
                replacements: vec![replacement],
                cursor,
                selection: None,
                transaction,
            },
            effect,
        )
    }

    pub(super) fn finish_insert(
        &mut self,
        state: &mut EditingState,
        view: DocumentView<'_>,
    ) -> Step {
        let resolver = MotionResolver::from_view(view);
        let mut replacements = Vec::new();
        let mut cursor = view.cursor.0;
        if let Some(active) = &state.active {
            // Counted i/a/R repeat the inserted sequence; an operator count changes
            // the target instead. Backspace recipes are replayed during dot.
            let text = inserted_text(&active.recipe.tail);
            if matches!(active.recipe.command.action, Action::Insert(_))
                && active.recipe.command.count > 1
                && !text.is_empty()
            {
                let text = if matches!(
                    active.recipe.command.action,
                    Action::Insert(InsertStyle::OpenAbove | InsertStyle::OpenBelow)
                ) {
                    format!("\n{text}").repeat(active.recipe.command.count - 1)
                } else {
                    text.repeat(active.recipe.command.count - 1)
                };
                let mut end = cursor;
                if state.mode == Mode::Replace {
                    use unicode_segmentation::UnicodeSegmentation;
                    for _ in text.graphemes(true) {
                        if end >= view.text.line_end(end) {
                            break;
                        }
                        end = resolver.next(end);
                    }
                }
                replacements.push(Replacement {
                    range: TextRange::new(cursor, end),
                    text: text.clone(),
                });
                cursor += text.len();
            }
            if !text.is_empty() {
                for p in &state.block_insert {
                    replacements.push(Replacement {
                        range: TextRange::new(
                            p.offset.0,
                            p.offset.0 + usize::from(p.tab_tail.is_some()),
                        ),
                        text: format!(
                            "{}{text}{}",
                            " ".repeat(p.padding),
                            " ".repeat(p.tab_tail.unwrap_or(0))
                        ),
                    });
                }
            }
        }
        replacements.sort_by_key(|r| r.range.start);
        if cursor == view.cursor.0 {
            if cursor > view.text.line_start(cursor) {
                cursor = resolver.previous(cursor);
            }
        } else {
            if let Some(r) = replacements.iter().find(|r| r.range.start == view.cursor) {
                cursor = r.range.start.0 + r.text.prev_grapheme(r.text.len());
            }
        }
        if let Some(origin) = state.block_origin {
            cursor = origin.0;
        }
        self.request(
            state,
            view,
            OperationKind::Edit {
                replacements,
                cursor: ByteOffset(cursor),
                selection: None,
                transaction: Transaction::End,
            },
            Effect::EndInsert,
        )
    }
}

fn inserted_text(tail: &[InsertEdit]) -> String {
    let mut text = String::new();
    for edit in tail {
        match edit {
            InsertEdit::Text(s) => text.push_str(s),
            InsertEdit::Backspace => {
                text.truncate(text.prev_grapheme(text.len()));
            }
            InsertEdit::Delete | InsertEdit::Host(_) => {}
        }
    }
    text
}

impl EditingState {
    fn begin_insert_edit(&mut self) -> Transaction {
        if self.active.is_none() {
            self.active = Some(ActiveChange {
                recipe: Recipe {
                    command: Command::new(
                        Action::Insert(if self.mode == Mode::Replace {
                            InsertStyle::Replace
                        } else {
                            InsertStyle::Before
                        }),
                        1,
                    ),
                    tail: Vec::new(),
                    visual: None,
                },
            });
            Transaction::Begin
        } else {
            Transaction::Continue
        }
    }

    /// Each Backspace unwinds one accepted grapheme, even when the host
    /// acknowledges an entire paste as a single replacement.
    pub(super) fn remember_overwrites(&mut self, edits: &[AppliedEdit]) {
        use unicode_segmentation::UnicodeSegmentation;
        let mut delta = 0isize;
        for edit in edits {
            let start = edit.range.start.0.saturating_add_signed(delta);
            let mut removed = edit.removed.graphemes(true);
            let mut inserted = edit.inserted.grapheme_indices(true).peekable();
            while let Some((offset, grapheme)) = inserted.next() {
                let original = if inserted.peek().is_none() {
                    // An opaque host element may have expanded the deleted range.
                    removed.as_str().to_owned()
                } else {
                    removed.next().unwrap_or_default().to_owned()
                };
                self.replace_recovery.push(ReplaceRecovery {
                    start: start + offset,
                    inserted: grapheme.to_owned(),
                    removed: original,
                });
            }
            delta += edit.inserted.len() as isize - edit.removed.len() as isize;
        }
    }
}
