//! Minimal Ropey mutation adapter. History and application services intentionally
//! remain host responsibilities, just as they do with the string adapter.
use vimlet::{
    AppliedEdit, ByteOffset, DocumentVersion, DocumentView, EditingState, HostOperation,
    HostOutcome, HostResult, Input, MotionResolver, OperationKind, Selection, Session, Step,
    TextSource, Transaction, DEFAULT_GEOMETRY,
};

pub struct RopeHost {
    pub text: ropey::Rope,
    pub cursor: ByteOffset,
    pub selection: Option<Selection>,
    pub version: DocumentVersion,
}

impl RopeHost {
    pub fn new(text: &str) -> Self {
        Self {
            text: ropey::Rope::from_str(text),
            cursor: ByteOffset(0),
            selection: None,
            version: DocumentVersion { id: 1, revision: 0 },
        }
    }

    pub fn view(&self) -> DocumentView<'_> {
        DocumentView {
            text: &self.text,
            cursor: self.cursor,
            selection: self.selection,
            version: self.version,
            opaque: &[],
            geometry: &DEFAULT_GEOMETRY,
        }
    }

    pub fn execute(&mut self, op: HostOperation) -> HostResult {
        let before = self.version;
        let outcome = if !op.is_current(before) {
            HostOutcome::Rejected
        } else {
            match op.kind {
                OperationKind::Move { cursor, selection } => {
                    self.cursor = cursor;
                    self.selection = selection;
                    HostOutcome::Applied {
                        edits: Vec::new(),
                        changed: false,
                    }
                }
                OperationKind::Edit {
                    replacements,
                    cursor,
                    selection,
                    transaction,
                } => {
                    let edits: Vec<_> = replacements
                        .iter()
                        .map(|r| AppliedEdit {
                            range: r.range,
                            removed: TextSource::slice(&self.text, r.range.start.0, r.range.end.0)
                                .into_owned(),
                            inserted: r.text.clone(),
                        })
                        .collect();
                    let changed = edits.iter().any(|e| e.removed != e.inserted);
                    for r in replacements.into_iter().rev() {
                        let start = self.text.byte_to_char(r.range.start.0);
                        let end = self.text.byte_to_char(r.range.end.0);
                        self.text.remove(start..end);
                        self.text.insert(start, &r.text);
                    }
                    self.cursor = ByteOffset(cursor.0.min(self.text.len_bytes()));
                    self.selection = selection;
                    if matches!(transaction, Transaction::Atomic | Transaction::End) {
                        self.cursor = ByteOffset(
                            MotionResolver::from_view(self.view()).normal_cursor(self.cursor.0),
                        );
                    }
                    if changed {
                        self.version.revision += 1;
                    }
                    HostOutcome::Applied { edits, changed }
                }
                _ => HostOutcome::Rejected,
            }
        };
        HostResult {
            operation: op.id,
            before: op.expected,
            after: self.version,
            cursor: self.cursor,
            selection: self.selection,
            outcome,
        }
    }

    pub fn keys(&mut self, session: &mut Session, state: &mut EditingState, keys: &str) {
        for key in super::parse_keys(keys) {
            let mut step = session.handle(state, Input::key(key), self.view());
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
                    Step::Error(error) => panic!("{error:?}"),
                    _ => break,
                };
            }
        }
    }
}
