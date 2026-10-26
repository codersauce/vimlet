use vimlet::{
    AppliedEdit, ByteOffset, DocumentVersion, DocumentView, EditingState, HostOutcome, HostResult,
    Input, Key, Mode, OperationKind, Session, Step,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut text = String::from("one two three");
    let mut cursor = ByteOffset(0);
    let mut selection = None;
    let mut version = DocumentVersion { id: 1, revision: 0 };
    let mut session = Session::new();
    let mut state = EditingState::default(); // Start in Normal mode.

    let keys =
        "cwhello"
            .chars()
            .map(Key::Char)
            .chain([Key::Escape, Key::Char('w'), Key::Char('.')]);

    for key in keys {
        let mut view = DocumentView::new(&text, version, cursor);
        view.selection = selection;
        let mut step = session.handle(&mut state, Input::key(key), view);

        loop {
            let operation = match step {
                Step::HostOperation(operation) => operation,
                Step::Consumed | Step::AwaitingInput => break,
                Step::Error(error) => return Err(error.into()),
                _ => return Err("this example only dispatches built-in editing keys".into()),
            };

            let outcome = if !operation.is_current(version) {
                HostOutcome::Rejected
            } else {
                match operation.kind {
                    OperationKind::Move {
                        cursor: next_cursor,
                        selection: next_selection,
                    } => {
                        cursor = next_cursor;
                        selection = next_selection;
                        HostOutcome::Applied {
                            edits: Vec::new(),
                            changed: false,
                        }
                    }
                    OperationKind::Edit {
                        replacements,
                        cursor: next_cursor,
                        selection: next_selection,
                        transaction: _, // This small host has no undo history.
                    } => {
                        // Capture removed text before applying any replacement.
                        let edits: Vec<_> = replacements
                            .iter()
                            .map(|replacement| AppliedEdit {
                                range: replacement.range,
                                removed: text[replacement.range.as_range()].into(),
                                inserted: replacement.text.clone(),
                            })
                            .collect();
                        let changed = edits.iter().any(|edit| edit.removed != edit.inserted);
                        // All ranges refer to the old text; start at the end.
                        for replacement in replacements.iter().rev() {
                            text.replace_range(replacement.range.as_range(), &replacement.text);
                        }
                        if changed {
                            version.revision += 1;
                        }
                        cursor = next_cursor;
                        selection = next_selection;
                        HostOutcome::Applied { edits, changed }
                    }
                    _ => HostOutcome::Rejected,
                }
            };

            let result = HostResult {
                operation: operation.id,
                before: operation.expected,
                after: version,
                cursor,
                selection,
                outcome,
            };
            let mut view = DocumentView::new(&text, version, cursor);
            view.selection = selection;
            // Dot can produce several operations from one key.
            step = session.resume(&mut state, result, view);
        }
    }

    assert_eq!(text, "hello hello three");
    assert_eq!(state.mode(), Mode::Normal);
    println!("{text}");
    Ok(())
}
