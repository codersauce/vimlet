use super::{
    Action, CharacterMotion, Command, EditTarget, EditingState, HistoryCommand, InsertStyle, Key,
    Mode, Operator, Pending, Shape, TargetRequest, MAX_COUNT,
};
use crate::{Motion, TextObjectKind, TextObjectScope};
use crate::{WordClass, WordDestination};

pub(super) enum Parsed {
    Command(Command),
    Pending,
    Consumed,
    Unhandled,
    Repeat(usize),
    Record(char),
    Play(char, usize),
    Escape,
}

fn command(action: Action, count: usize) -> Parsed {
    Parsed::Command(Command::new(action, count))
}

fn target(op: Option<Operator>, target: EditTarget, count: usize) -> Parsed {
    if let Some(operator) = op {
        return command(Action::Operate { operator, target }, count);
    }
    match target {
        EditTarget::Motion(motion) => command(Action::Move(motion), count),
        EditTarget::Host(request) => command(Action::Target(request), count),
        EditTarget::Object(scope, kind) => command(Action::SelectObject { scope, kind }, count),
        _ => Parsed::Consumed,
    }
}

pub(super) fn parse(state: &mut EditingState, key: &Key) -> Parsed {
    if key == &Key::Escape {
        state.pending = None;
        return Parsed::Escape;
    }
    let digit = match key {
        Key::Char(c) if c.is_ascii_digit() => c.to_digit(10).map(|n| n as usize),
        _ => None,
    };
    if matches!(state.pending, None | Some(Pending::Operator { .. })) {
        if let Some(d) = digit {
            if d != 0 || state.count.is_some() {
                state.count =
                    Some((state.count.unwrap_or(0).saturating_mul(10) + d).min(MAX_COUNT));
                return Parsed::Pending;
            }
        }
    }
    let explicit_count = state.count.take().unwrap_or(0);
    let count = explicit_count.max(1);
    if let Some(pending) = state.pending.take() {
        return parse_suffix(state, key, pending, explicit_count);
    }
    if let Some(parsed) = parse_target(state, key, None, explicit_count) {
        return parsed;
    }
    match key {
        Key::Char('d' | 'c' | 'y' | '>' | '<') => {
            let op = match key {
                Key::Char('d') => Operator::Delete,
                Key::Char('c') => Operator::Change,
                Key::Char('y') => Operator::Yank,
                Key::Char('>') => Operator::Indent,
                _ => Operator::Outdent,
            };
            if state.mode.is_visual() {
                command(
                    Action::Operate {
                        operator: op,
                        target: EditTarget::Selection,
                    },
                    count,
                )
            } else {
                state.pending = Some(Pending::Operator { op, count });
                Parsed::Pending
            }
        }
        Key::Char('i' | 'a' | 'I' | 'A' | 'o' | 'O' | 'R') => {
            if state.mode.is_visual() {
                match key {
                    Key::Char('o' | 'O') => return command(Action::SwapVisual, count),
                    Key::Char('I' | 'A') if state.mode == Mode::VisualBlock => {
                        return command(
                            Action::BlockInsert {
                                append: key == &Key::Char('A'),
                            },
                            count,
                        )
                    }
                    _ => {}
                }
            }
            let style = match key {
                Key::Char('i') => InsertStyle::Before,
                Key::Char('a') => InsertStyle::After,
                Key::Char('I') => InsertStyle::FirstNonBlank,
                Key::Char('A') => InsertStyle::LineEnd,
                Key::Char('o') => InsertStyle::OpenBelow,
                Key::Char('O') => InsertStyle::OpenAbove,
                _ => InsertStyle::Replace,
            };
            command(Action::Insert(style), count)
        }
        Key::Char('v') => command(Action::Visual(Shape::Character), count),
        Key::Char('V') => command(Action::Visual(Shape::Line), count),
        Key::Ctrl('v') => command(Action::Visual(Shape::Block { width: 1 }), count),
        Key::Char('x' | 'X') | Key::Delete => {
            if state.mode.is_visual() {
                command(
                    Action::Operate {
                        operator: Operator::Delete,
                        target: EditTarget::Selection,
                    },
                    count,
                )
            } else {
                command(
                    Action::DeleteChars {
                        backward: key == &Key::Char('X'),
                    },
                    count,
                )
            }
        }
        Key::Char('s') => command(
            Action::Operate {
                operator: Operator::Change,
                target: if state.mode.is_visual() {
                    EditTarget::Selection
                } else {
                    EditTarget::Motion(Motion::Right)
                },
            },
            count,
        ),
        Key::Char('S') => command(
            Action::Operate {
                operator: Operator::Change,
                target: EditTarget::Lines,
            },
            count,
        ),
        Key::Char('D' | 'C' | 'Y') => {
            let operator = match key {
                Key::Char('D') => Operator::Delete,
                Key::Char('C') => Operator::Change,
                _ => Operator::Yank,
            };
            command(
                Action::Operate {
                    operator,
                    target: EditTarget::Motion(Motion::LineEnd),
                },
                count,
            )
        }
        Key::Char('r') => {
            state.pending = Some(Pending::Replace { count });
            Parsed::Pending
        }
        Key::Char('p' | 'P') => command(
            Action::Paste {
                before: key == &Key::Char('P'),
            },
            count,
        ),
        Key::Char('~') => command(Action::ToggleCase, count),
        Key::Char('J') => command(Action::Join { spaces: true }, count),
        Key::Char('u' | 'U') if state.mode.is_visual() => command(
            Action::Operate {
                operator: if key == &Key::Char('u') {
                    Operator::Lowercase
                } else {
                    Operator::Uppercase
                },
                target: EditTarget::Selection,
            },
            count,
        ),
        Key::Char('u') => command(Action::History(HistoryCommand::Undo), count),
        Key::Char('U') => command(Action::History(HistoryCommand::UndoLine), count),
        Key::Ctrl('r') => command(Action::History(HistoryCommand::Redo), count),
        Key::Char('.') => Parsed::Repeat(explicit_count),
        Key::Char('q') => {
            state.pending = Some(Pending::Record);
            Parsed::Pending
        }
        Key::Char('@') => {
            state.pending = Some(Pending::Play { count });
            Parsed::Pending
        }
        _ => Parsed::Unhandled,
    }
}

fn parse_target(
    state: &mut EditingState,
    key: &Key,
    op: Option<Operator>,
    count: usize,
) -> Option<Parsed> {
    let n = count.max(1);
    let motion = match key {
        Key::Char('h') | Key::Left | Key::Backspace => Motion::Left,
        Key::Char('l') | Key::Right | Key::Char(' ') => Motion::Right,
        Key::Char('j') | Key::Down => Motion::Down,
        Key::Char('k') | Key::Up => Motion::Up,
        Key::Char('0') | Key::Home => Motion::LineStart,
        Key::Char('^') => Motion::FirstNonBlank,
        Key::Char('$') | Key::End => Motion::LineEnd,
        Key::Char('w' | 'W' | 'b' | 'B' | 'e' | 'E') => {
            let Key::Char(c) = key else { unreachable!() };
            Motion::Word {
                destination: match c {
                    'b' | 'B' => WordDestination::PreviousStart,
                    'e' | 'E' => WordDestination::NextEnd,
                    _ => WordDestination::NextStart,
                },
                class: if c.is_uppercase() {
                    WordClass::BigWord
                } else {
                    WordClass::Word
                },
            }
        }
        Key::Char('G') => Motion::LastLine,
        Key::Char('%') => {
            if count > 0 {
                Motion::Percent
            } else {
                Motion::MatchDelimiter
            }
        }
        Key::Char('{' | '}') => Motion::Paragraph {
            backward: key == &Key::Char('{'),
        },
        Key::Char('(' | ')') => Motion::Sentence {
            backward: key == &Key::Char('('),
        },
        Key::Char('f' | 'F' | 't' | 'T') => {
            let kind = match key {
                Key::Char('f') => CharacterMotion::Find,
                Key::Char('F') => CharacterMotion::FindBackward,
                Key::Char('t') => CharacterMotion::Till,
                _ => CharacterMotion::TillBackward,
            };
            state.pending = Some(Pending::Character { op, kind, count: n });
            return Some(Parsed::Pending);
        }
        Key::Char(';' | ',') => {
            let Some((mut kind, character)) = state.last_character else {
                return Some(Parsed::Consumed);
            };
            if key == &Key::Char(',') {
                kind = match kind {
                    CharacterMotion::Find => CharacterMotion::FindBackward,
                    CharacterMotion::FindBackward => CharacterMotion::Find,
                    CharacterMotion::Till => CharacterMotion::TillBackward,
                    CharacterMotion::TillBackward => CharacterMotion::Till,
                };
            }
            Motion::RepeatCharacter { kind, character }
        }
        Key::Char('g') => {
            state.pending = Some(Pending::G { op, count });
            return Some(Parsed::Pending);
        }
        Key::Char('i' | 'a') if op.is_some() || state.mode.is_visual() => {
            state.pending = Some(Pending::Object {
                op,
                scope: if key == &Key::Char('i') {
                    TextObjectScope::Inner
                } else {
                    TextObjectScope::Around
                },
                count: n,
            });
            return Some(Parsed::Pending);
        }
        Key::Char('/' | '?' | 'n' | 'N') => {
            return Some(target(
                op,
                EditTarget::Host(TargetRequest::Search {
                    backward: matches!(key, Key::Char('?' | 'N')),
                    repeat: matches!(key, Key::Char('n' | 'N')),
                }),
                n,
            ))
        }
        Key::Char('\'' | '`') => {
            state.pending = Some(Pending::Mark {
                op,
                linewise: key == &Key::Char('\''),
                count: n,
            });
            return Some(Parsed::Pending);
        }
        _ => return None,
    };
    Some(target(op, EditTarget::Motion(motion), count))
}

fn parse_suffix(
    state: &mut EditingState,
    key: &Key,
    pending: Pending,
    explicit_count: usize,
) -> Parsed {
    let count = explicit_count.max(1);
    match pending {
        Pending::Record => match key {
            Key::Char(c) => Parsed::Record(*c),
            _ => Parsed::Consumed,
        },
        Pending::Play { count } => match key {
            Key::Char(c) => Parsed::Play(*c, count),
            _ => Parsed::Consumed,
        },
        Pending::Replace { count } => match key {
            Key::Char(c) => command(Action::ReplaceChar(*c), count),
            Key::Enter => command(Action::ReplaceChar('\n'), count),
            _ => Parsed::Consumed,
        },
        Pending::Character { op, kind, count } => match key {
            Key::Char(c) => {
                state.last_character = Some((kind, *c));
                target(
                    op,
                    EditTarget::Motion(Motion::Character {
                        kind,
                        character: *c,
                    }),
                    count,
                )
            }
            _ => Parsed::Consumed,
        },
        Pending::Object { op, scope, count } => match key {
            Key::Char(c) => TextObjectKind::from_key(*c).map_or(Parsed::Consumed, |kind| {
                target(op, EditTarget::Object(scope, kind), count)
            }),
            _ => Parsed::Consumed,
        },
        Pending::Mark {
            op,
            linewise,
            count,
        } => match key {
            Key::Char(name) => target(
                op,
                EditTarget::Host(TargetRequest::Mark {
                    name: *name,
                    linewise,
                }),
                count,
            ),
            _ => Parsed::Consumed,
        },
        Pending::G { op, count } => match key {
            Key::Char('g') => target(op, EditTarget::Motion(Motion::FirstLine), count),
            Key::Char('e' | 'E') => target(
                op,
                EditTarget::Motion(Motion::Word {
                    destination: WordDestination::PreviousEnd,
                    class: if key == &Key::Char('E') {
                        WordClass::BigWord
                    } else {
                        WordClass::Word
                    },
                }),
                count,
            ),
            Key::Char('u' | 'U' | '~') if op.is_none() => {
                let op = match key {
                    Key::Char('u') => Operator::Lowercase,
                    Key::Char('U') => Operator::Uppercase,
                    _ => Operator::ToggleCase,
                };
                if state.mode.is_visual() {
                    command(
                        Action::Operate {
                            operator: op,
                            target: EditTarget::Selection,
                        },
                        count,
                    )
                } else {
                    state.pending = Some(Pending::Operator { op, count });
                    Parsed::Pending
                }
            }
            Key::Char('J') if op.is_none() => command(Action::Join { spaces: false }, count),
            Key::Char('v') if op.is_none() => command(Action::Reselect, count),
            Key::Char('q') if op.is_none() => {
                state.pending = Some(Pending::Operator {
                    op: Operator::Format,
                    count,
                });
                Parsed::Pending
            }
            _ => Parsed::Consumed,
        },
        Pending::Operator {
            op,
            count: operator_count,
        } => {
            let count = count.saturating_mul(operator_count).min(MAX_COUNT);
            let doubled = matches!(
                (op, key),
                (Operator::Delete, Key::Char('d'))
                    | (Operator::Change, Key::Char('c'))
                    | (Operator::Yank, Key::Char('y'))
                    | (Operator::Lowercase, Key::Char('u'))
                    | (Operator::Uppercase, Key::Char('U'))
                    | (Operator::ToggleCase, Key::Char('~'))
                    | (Operator::Indent, Key::Char('>'))
                    | (Operator::Outdent, Key::Char('<'))
                    | (Operator::Format, Key::Char('q'))
            );
            if doubled {
                return target(Some(op), EditTarget::Lines, count);
            }
            let count = if operator_count == 1
                && explicit_count == 0
                && matches!(key, Key::Char('G' | '%'))
            {
                0
            } else {
                count
            };
            parse_target(state, key, Some(op), count).unwrap_or(Parsed::Consumed)
        }
    }
}
