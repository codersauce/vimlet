use super::visual::{mode_shape, selection_range, shape_mode};
use super::{
    Action, ByteOffset, Command, DocumentView, EditTarget, EditingState, Effect, EngineError, Mode,
    OperationKind, Operator, ResolvedRange, Selection, Session, Shape, Step, TargetRequest,
    MAX_COUNT,
};
use crate::{Motion, MotionResolver};

impl Session {
    pub(super) fn move_to(
        &mut self,
        state: &mut EditingState,
        view: DocumentView<'_>,
        cursor: ByteOffset,
        mode: Mode,
        selection: Option<Selection>,
    ) -> Step {
        self.request(
            state,
            view,
            OperationKind::Move { cursor, selection },
            Effect::Move { mode },
        )
    }

    pub(super) fn execute(
        &mut self,
        state: &mut EditingState,
        mut command: Command,
        view: DocumentView<'_>,
    ) -> Step {
        command.count = command.count.min(MAX_COUNT);
        let n = command.count.max(1);
        let resolver = MotionResolver::from_view(view);
        if !matches!(command.action, Action::Move(Motion::Up | Motion::Down)) {
            state.preferred_column = None;
        }
        match command.action {
            Action::Repeat => self.repeat(state, command.count, view),
            Action::Move(motion) => self.move_cursor(state, view, motion, command.count),
            Action::Visual(shape) => {
                let mode = shape_mode(shape);
                if !state.capabilities.visual
                    || (mode == Mode::VisualBlock && !state.capabilities.visual_block)
                {
                    return Step::Error(EngineError::UnsupportedMode);
                }
                if mode == state.mode {
                    state.previous_visual = view.selection;
                    return self.move_to(state, view, view.cursor, Mode::Normal, None);
                }
                let selection = Selection {
                    anchor: view.selection.map_or(view.cursor, |s| s.anchor),
                    cursor: view.cursor,
                    shape,
                };
                self.move_to(state, view, view.cursor, mode, Some(selection))
            }
            Action::SwapVisual => {
                let Some(s) = view.selection else {
                    return Step::Consumed;
                };
                self.move_to(
                    state,
                    view,
                    s.anchor,
                    state.mode,
                    Some(Selection {
                        anchor: s.cursor,
                        cursor: s.anchor,
                        ..s
                    }),
                )
            }
            Action::Reselect => {
                let Some(s) = state.previous_visual else {
                    return Step::Consumed;
                };
                let s = Selection {
                    anchor: ByteOffset(resolver.normal_cursor(s.anchor.0)),
                    cursor: ByteOffset(resolver.normal_cursor(s.cursor.0)),
                    ..s
                };
                self.move_to(state, view, s.cursor, shape_mode(s.shape), Some(s))
            }
            Action::SelectObject { scope, kind } => {
                let Some(range) = resolver.text_object(scope, kind, n) else {
                    return Step::Consumed;
                };
                let Some(r) = range.ranges.first() else {
                    return Step::Consumed;
                };
                let selection = Selection {
                    anchor: r.start,
                    cursor: ByteOffset(if r.is_empty() {
                        r.start.0
                    } else {
                        resolver.previous(r.end.0)
                    }),
                    shape: range.shape,
                };
                self.move_to(
                    state,
                    view,
                    selection.cursor,
                    shape_mode(range.shape),
                    Some(selection),
                )
            }
            Action::Insert(style) => self.begin_insert(state, command, style, view),
            Action::Operate { target, operator } => {
                self.operate_on(state, view, command, target, operator)
            }
            Action::DeleteChars { backward } => {
                self.delete_characters(state, view, command, backward)
            }
            Action::ReplaceChar(_) | Action::ToggleCase => {
                self.transform_characters(state, view, command)
            }
            Action::Paste { before } => self.paste(state, command, before, view),
            Action::Join { spaces } => self.join(state, command, spaces, view),
            Action::History(history) => self.request(
                state,
                view,
                OperationKind::History {
                    command: history,
                    count: n,
                },
                Effect::History,
            ),
            Action::Target(request) | Action::SelectTarget(request) => {
                self.resolve_request(state, command, request, view)
            }
            Action::Host(host) => {
                let effect = if host.repeatable {
                    Effect::Command {
                        command,
                        mode: state.mode,
                        register: None,
                        visual: None,
                    }
                } else {
                    Effect::Move { mode: state.mode }
                };
                self.request(
                    state,
                    view,
                    OperationKind::Host {
                        command: host,
                        count: n,
                    },
                    effect,
                )
            }
            Action::BlockInsert { .. } => {
                let Some(selection) = view.selection else {
                    return Step::Consumed;
                };
                self.resolve_request(state, command, TargetRequest::Block { selection }, view)
            }
        }
    }

    pub(super) fn resolve_request(
        &mut self,
        state: &mut EditingState,
        command: Command,
        request: TargetRequest,
        view: DocumentView<'_>,
    ) -> Step {
        self.request(
            state,
            view,
            OperationKind::Resolve {
                request,
                count: command.count.max(1),
            },
            Effect::Resolve(command),
        )
    }

    fn move_cursor(
        &mut self,
        state: &mut EditingState,
        view: DocumentView<'_>,
        motion: Motion,
        count: usize,
    ) -> Step {
        let resolver = MotionResolver::from_view(view);
        if motion == Motion::LineEnd {
            state.preferred_column = Some(usize::MAX);
        }
        let Some(mut target) = resolver.resolve(motion, count) else {
            self.abort_input(state);
            return Step::Consumed;
        };
        if matches!(motion, Motion::Up | Motion::Down) {
            let column = *state
                .preferred_column
                .get_or_insert_with(|| resolver.column(view.cursor.0));
            target.position =
                ByteOffset(resolver.at_column(view.text.line_start(target.position.0), column));
        }
        let cursor = ByteOffset(resolver.normal_cursor(target.position.0));
        let selection = if state.mode.is_visual() {
            Some(Selection {
                anchor: view.selection.map_or(view.cursor, |s| s.anchor),
                cursor,
                shape: mode_shape(state.mode),
            })
        } else {
            None
        };
        self.move_to(state, view, cursor, state.mode, selection)
    }

    fn operate_on(
        &mut self,
        state: &mut EditingState,
        view: DocumentView<'_>,
        command: Command,
        target: EditTarget,
        operator: Operator,
    ) -> Step {
        let resolver = MotionResolver::from_view(view);
        let n = command.count.max(1);
        let range = match target {
            EditTarget::Motion(motion) => resolver.operator_range(motion, command.count, operator),
            EditTarget::Object(scope, kind) => resolver.text_object(scope, kind, n),
            EditTarget::Lines => {
                let line = view.text.line_index(view.cursor.0);
                Some(
                    resolver.lines(
                        view.cursor.0,
                        view.text
                            .line_offset(line + n - 1)
                            .min(view.text.len_bytes()),
                    ),
                )
            }
            EditTarget::Selection => {
                let Some(s) = view.selection else {
                    return Step::Consumed;
                };
                if matches!(s.shape, Shape::Block { .. }) {
                    return self.resolve_request(
                        state,
                        command,
                        TargetRequest::Block { selection: s },
                        view,
                    );
                }
                Some(selection_range(resolver, s))
            }
            EditTarget::Host(request) => {
                return self.resolve_request(state, command, request, view)
            }
        };
        let Some(range) = range else {
            self.abort_input(state);
            return Step::Consumed;
        };
        self.resolved_command(state, command, range, view)
    }

    fn delete_characters(
        &mut self,
        state: &mut EditingState,
        view: DocumentView<'_>,
        command: Command,
        backward: bool,
    ) -> Step {
        let resolver = MotionResolver::from_view(view);
        let n = command.count.max(1);
        let mut start = view.cursor.0;
        let mut end = start;
        let line_start = view.text.line_start(start);
        let line_end = view.text.line_end(start);
        for _ in 0..n {
            if backward {
                if start == line_start {
                    break;
                }
                start = resolver.previous(start);
            } else {
                if end >= line_end {
                    break;
                }
                end = resolver.next(end);
            }
        }
        if start == end {
            return Step::Consumed;
        }
        self.edit_operator(
            state,
            command,
            Operator::Delete,
            ResolvedRange::character(start, end),
            view,
        )
    }

    fn transform_characters(
        &mut self,
        state: &mut EditingState,
        view: DocumentView<'_>,
        command: Command,
    ) -> Step {
        let resolver = MotionResolver::from_view(view);
        let n = command.count.max(1);
        if state.mode.is_visual() {
            let Some(s) = view.selection else {
                return Step::Consumed;
            };
            if matches!(s.shape, Shape::Block { .. }) {
                return self.resolve_request(
                    state,
                    command,
                    TargetRequest::Block { selection: s },
                    view,
                );
            }
            return self.resolved_command(state, command, selection_range(resolver, s), view);
        }
        let mut end = view.cursor.0;
        for _ in 0..n {
            if end >= view.text.line_end(end) {
                if matches!(command.action, Action::ReplaceChar(_)) {
                    return Step::Consumed;
                }
                break;
            }
            end = resolver.next(end);
        }
        if end == view.cursor.0 {
            return Step::Consumed;
        }
        self.resolved_command(
            state,
            command,
            ResolvedRange::character(view.cursor.0, end),
            view,
        )
    }
}
