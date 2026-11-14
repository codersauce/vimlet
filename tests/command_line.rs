use serde::Deserialize;
use vimlet::{ByteOffset, CommandLine, Key, PromptKind, PromptOutcome};

#[derive(Deserialize)]
struct Case {
    kind: String,
    keys: Vec<String>,
    #[serde(default)]
    initial: String,
    text: Option<String>,
    caret: Option<usize>,
    outcome: Option<String>,
}

#[test]
fn command_line_matches_neovim_fixtures() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("command_line.json")).unwrap();
    for case in cases {
        let kind = match case.kind.as_str() {
            "Ex" => PromptKind::Ex,
            "SearchForward" => PromptKind::SearchForward,
            "SearchBackward" => PromptKind::SearchBackward,
            "Field" => PromptKind::Field,
            _ => unreachable!(),
        };
        let mut line = CommandLine::new(kind, case.initial).unwrap();
        let mut outcome = None;
        for key in &case.keys {
            let key = match key.as_str() {
                "Left" => Key::Left,
                "Backspace" => Key::Backspace,
                "Delete" => Key::Delete,
                "Enter" => Key::Enter,
                "Escape" => Key::Escape,
                "Ctrl-u" => Key::Ctrl('u'),
                "Ctrl-w" => Key::Ctrl('w'),
                key if key.starts_with("Paste:") => Key::Text(key[6..].into()),
                ch => Key::Char(ch.parse().unwrap()),
            };
            outcome = match line.handle(key) {
                PromptOutcome::Accepted(text) => Some(text),
                PromptOutcome::Cancelled => Some("cancelled".into()),
                _ => None,
            };
        }
        assert_eq!(outcome, case.outcome, "{:?}", case.keys);
        if let Some(text) = case.text {
            assert_eq!(line.text(), text, "{:?}", case.keys);
            assert_eq!(Some(line.caret().0), case.caret, "{:?}", case.keys);
        }
    }
}

#[test]
fn rejected_replacements_and_quoted_keys_cannot_submit() {
    let mut line = CommandLine::new(PromptKind::Ex, "e\u{301}").unwrap();
    assert!(line.replace(line.text().into(), ByteOffset(1)).is_err());
    assert!(line.replace("one\ntwo".into(), ByteOffset(0)).is_err());
    assert_eq!(line.handle(Key::Ctrl('v')), PromptOutcome::Consumed);
    assert_eq!(line.handle(Key::Enter), PromptOutcome::Consumed);
    assert_eq!(
        line.handle(Key::Text("x\u{3}\ny".into())),
        PromptOutcome::Edited
    );
    assert_eq!(
        line.handle(Key::Enter),
        PromptOutcome::Accepted("e\u{301}x\u{3}".into())
    );
    assert_eq!(line.handle(Key::Enter), PromptOutcome::Unhandled);
}
