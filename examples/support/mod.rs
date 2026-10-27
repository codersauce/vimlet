//! A deliberately small host, shared by examples and contract tests. Applications
//! keep their own storage, layout, undo and service implementations.
#![allow(dead_code)]
use vimlet::{
    AppliedEdit, ByteOffset, DocumentVersion, DocumentView, EditingState, HistoryCommand,
    HostOperation, HostOutcome, HostResult, Input, Key, MotionResolver, OperationKind, Selection,
    Session, Step, TargetRequest, TextRange, TextSource, Transaction, DEFAULT_GEOMETRY,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    text: String,
    cursor: ByteOffset,
    selection: Option<Selection>,
}

pub struct StringHost {
    pub text: String,
    pub cursor: ByteOffset,
    pub selection: Option<Selection>,
    pub version: DocumentVersion,
    pub opaque: Vec<TextRange>,
    pub undo_groups: usize,
    history: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    transaction: Option<Snapshot>,
    line_before: Option<(usize, String)>,
}

impl StringHost {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            cursor: ByteOffset(0),
            selection: None,
            version: DocumentVersion { id: 1, revision: 0 },
            opaque: Vec::new(),
            undo_groups: 0,
            history: Vec::new(),
            redo: Vec::new(),
            transaction: None,
            line_before: None,
        }
    }

    pub fn view(&self) -> DocumentView<'_> {
        DocumentView {
            text: &self.text,
            version: self.version,
            cursor: self.cursor,
            selection: self.selection,
            opaque: &self.opaque,
            geometry: &DEFAULT_GEOMETRY,
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            selection: self.selection,
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.text = s.text;
        self.cursor = s.cursor;
        self.selection = s.selection;
    }

    pub fn execute(&mut self, op: HostOperation) -> HostResult {
        let before = self.version;
        if !op.is_current(before) {
            return HostResult {
                operation: op.id,
                before: op.expected,
                after: before,
                cursor: self.cursor,
                selection: self.selection,
                outcome: HostOutcome::Rejected,
            };
        }
        let snapshot = self.snapshot();
        let mut outcome = HostOutcome::Applied {
            edits: Vec::new(),
            changed: false,
        };
        match op.kind {
            OperationKind::Move { cursor, selection } => {
                self.cursor = cursor;
                self.selection = selection;
            }
            OperationKind::Edit {
                mut replacements,
                cursor,
                selection,
                transaction,
            } => {
                if replacements.iter().any(|r| {
                    r.range.end.0 > self.text.len()
                        || r.range.start > r.range.end
                        || !self.text.is_char_boundary(r.range.start.0)
                        || !self.text.is_char_boundary(r.range.end.0)
                }) || replacements
                    .windows(2)
                    .any(|r| r[0].range.end > r[1].range.start)
                {
                    outcome = HostOutcome::Rejected;
                } else {
                    if transaction == Transaction::Begin {
                        self.transaction = Some(snapshot.clone());
                    }
                    let edits: Vec<_> = replacements
                        .iter()
                        .map(|r| AppliedEdit {
                            range: r.range,
                            removed: self.text[r.range.as_range()].into(),
                            inserted: r.text.clone(),
                        })
                        .collect();
                    let changed = edits.iter().any(|e| e.removed != e.inserted);
                    if changed {
                        let line = self.text.line_start(self.cursor.0);
                        if self.line_before.as_ref().is_none_or(|(p, _)| *p != line) {
                            self.line_before =
                                Some((line, self.text[line..self.text.line_end(line)].into()));
                        }
                    }
                    for r in replacements.drain(..).rev() {
                        self.text.replace_range(r.range.as_range(), &r.text);
                    }
                    self.cursor = ByteOffset(cursor.0.min(self.text.len()));
                    self.selection = selection;
                    if matches!(transaction, Transaction::Atomic | Transaction::End) {
                        self.cursor = ByteOffset(
                            MotionResolver::from_view(self.view()).normal_cursor(self.cursor.0),
                        );
                    }
                    if changed {
                        self.version.revision += 1;
                        self.redo.clear();
                    }
                    if transaction == Transaction::Atomic && changed {
                        self.history.push(snapshot.clone());
                        self.undo_groups += 1;
                    }
                    if matches!(transaction, Transaction::End | Transaction::BreakInsert) {
                        if let Some(start) = self.transaction.take() {
                            if start.text != self.text {
                                self.history.push(start);
                                self.undo_groups += 1;
                            }
                        }
                    }
                    outcome = HostOutcome::Applied { edits, changed };
                }
            }
            OperationKind::History { command, count } => {
                match command {
                    HistoryCommand::Undo => {
                        for _ in 0..count {
                            if let Some(s) = self.history.pop() {
                                self.redo.push(self.snapshot());
                                self.restore(s);
                            }
                        }
                    }
                    HistoryCommand::Redo => {
                        for _ in 0..count {
                            if let Some(s) = self.redo.pop() {
                                self.history.push(self.snapshot());
                                self.restore(s);
                            }
                        }
                    }
                    HistoryCommand::UndoLine => {
                        if let Some((p, text)) = self.line_before.take() {
                            if p <= self.text.len() {
                                let end = self.text.line_end(p);
                                let current = self.text[p..end].to_string();
                                self.history.push(self.snapshot());
                                self.text.replace_range(p..end, &text);
                                self.line_before = Some((p, current));
                                self.cursor = ByteOffset(p);
                            }
                        }
                    }
                }
                let changed = self.text != snapshot.text;
                if changed {
                    self.version.revision += 1;
                }
                outcome = HostOutcome::Applied {
                    edits: Vec::new(),
                    changed,
                };
            }
            OperationKind::Resolve {
                request: TargetRequest::Block { selection },
                ..
            } => {
                outcome = HostOutcome::Target(
                    MotionResolver::from_view(self.view()).block_range(selection),
                );
            }
            _ => outcome = HostOutcome::Rejected,
        }
        HostResult {
            operation: op.id,
            before,
            after: self.version,
            cursor: self.cursor,
            selection: self.selection,
            outcome,
        }
    }

    pub fn drive(
        &mut self,
        session: &mut Session,
        state: &mut EditingState,
        mut step: Step,
    ) -> Step {
        loop {
            step = match step {
                Step::HostOperation(op) => {
                    let result = self.execute(op);
                    session.resume(state, result, self.view())
                }
                Step::Replay(input) => session.handle(state, input, self.view()),
                Step::Consumed | Step::AwaitingInput if session.is_playing() => {
                    session.next_macro_input(state, self.version)
                }
                other => return other,
            };
        }
    }

    pub fn key(&mut self, session: &mut Session, state: &mut EditingState, key: Key) -> Step {
        let step = session.handle(state, Input::key(key), self.view());
        self.drive(session, state, step)
    }

    pub fn keys(&mut self, session: &mut Session, state: &mut EditingState, keys: &str) {
        for key in parse_keys(keys) {
            let result = self.key(session, state, key);
            assert!(!matches!(result, Step::Error(_)), "{result:?}");
        }
    }
}

pub fn parse_keys(s: &str) -> Vec<Key> {
    let mut result = Vec::new();
    let mut s = s;
    while !s.is_empty() {
        let found = [
            ("<Esc>", Key::Escape),
            ("<CR>", Key::Enter),
            ("<BS>", Key::Backspace),
            ("<Del>", Key::Delete),
            ("<Tab>", Key::Tab),
            ("<C-r>", Key::Ctrl('r')),
            ("<C-v>", Key::Ctrl('v')),
            ("<Left>", Key::Left),
            ("<Right>", Key::Right),
        ];
        if let Some((prefix, key)) = found.into_iter().find(|(prefix, _)| s.starts_with(prefix)) {
            result.push(key);
            s = &s[prefix.len()..];
        } else {
            let c = s.chars().next().unwrap();
            result.push(Key::Char(c));
            s = &s[c.len_utf8()..];
        }
    }
    result
}

#[cfg(feature = "ropey")]
pub mod rope;
