#[path = "../examples/support/mod.rs"]
mod support;

use support::StringHost;
use vimlet::*;

fn run(text: &str, keys: &str) -> (StringHost, Session, EditingState) {
    let mut h = StringHost::new(text);
    let mut s = Session::new();
    let mut e = EditingState::default();
    h.keys(&mut s, &mut e, keys);
    (h, s, e)
}

#[test]
fn successful_change_and_insert_form_one_undo_group() {
    let (mut h, mut s, mut e) = run("one two", "cwhello<Esc>");
    assert_eq!(h.text, "hello two");
    assert_eq!(h.undo_groups, 1);
    h.keys(&mut s, &mut e, "u");
    assert_eq!(h.text, "one two");
    h.keys(&mut s, &mut e, "<C-r>");
    assert_eq!(h.text, "hello two");
}

#[test]
fn failed_target_never_runs_dot_insertion_tail() {
    let (mut h, mut s, mut e) = run("one x two", "cfxYES<Esc>$.");
    assert_eq!(h.text, "YES two");
    assert_eq!(e.mode(), Mode::Normal);
    assert!(s.has_last_change());
    h.keys(&mut s, &mut e, "0.");
    assert_eq!(h.text, "YES two");
}

#[test]
fn accepted_empty_insert_becomes_a_noop_repeat() {
    let (mut h, mut s, mut e) = run("abcdef", "xi<Esc>.");
    assert_eq!(h.text, "bcdef");
    h.keys(&mut s, &mut e, ".");
    assert_eq!(h.text, "bcdef");
}

#[test]
fn dot_uses_semantic_action_after_keymap_change() {
    let (mut h, mut s, mut e) = run("one two three", "dw");
    let input = Input {
        key: Key::Char('z'),
        origin: InputOrigin::Typed,
        binding: Binding::Command(Command::new(Action::Move(Motion::LineEnd), 1)),
    };
    let step = s.handle(&mut e, input, h.view());
    h.drive(&mut s, &mut e, step);
    h.keys(&mut s, &mut e, "0.");
    assert_eq!(h.text, "three");
}

#[test]
fn stale_operation_is_rejected_before_mutation() {
    let mut h = StringHost::new("abc");
    let mut s = Session::new();
    let mut e = EditingState::default();
    let Step::HostOperation(op) = s.handle(&mut e, Input::key(Key::Char('x')), h.view()) else {
        panic!()
    };
    h.text = "new draft".into();
    h.version.revision += 1;
    let result = h.execute(op);
    assert_eq!(result.outcome, HostOutcome::Rejected);
    assert_eq!(h.text, "new draft");
    assert_eq!(s.resume(&mut e, result, h.view()), Step::Consumed);
    assert!(!s.has_last_change());
}

#[test]
fn wrong_ack_does_not_consume_the_real_operation() {
    let mut h = StringHost::new("abc");
    let mut s = Session::new();
    let mut e = EditingState::default();
    let Step::HostOperation(op) = s.handle(&mut e, Input::key(Key::Char('x')), h.view()) else {
        panic!()
    };
    let result = h.execute(op);
    let mut wrong = result.clone();
    wrong.operation += 1;
    assert_eq!(
        s.resume(&mut e, wrong, h.view()),
        Step::Error(EngineError::UnexpectedResult)
    );
    assert_eq!(s.resume(&mut e, result, h.view()), Step::Consumed);
    assert_eq!(h.text, "bc");
}

#[test]
fn second_key_cannot_overtake_host_operation() {
    let h = StringHost::new("abc");
    let mut s = Session::new();
    let mut e = EditingState::default();
    assert!(matches!(
        s.handle(&mut e, Input::key(Key::Char('x')), h.view()),
        Step::HostOperation(_)
    ));
    assert_eq!(
        s.handle(&mut e, Input::key(Key::Char('i')), h.view()),
        Step::Error(EngineError::AwaitingHost)
    );
}

#[test]
fn host_adjusted_insert_is_what_dot_replays() {
    let mut h = StringHost::new("abc def");
    let mut s = Session::new();
    let mut e = EditingState::default();
    h.keys(&mut s, &mut e, "i");
    let Step::HostOperation(mut op) = s.handle(&mut e, Input::key(Key::Char('x')), h.view()) else {
        panic!()
    };
    if let OperationKind::Edit {
        replacements,
        cursor,
        ..
    } = &mut op.kind
    {
        replacements[0].text = "FILTERED".into();
        *cursor = ByteOffset(8);
    }
    let result = h.execute(op);
    s.resume(&mut e, result, h.view());
    h.keys(&mut s, &mut e, "<Esc>w.");
    assert_eq!(h.text, "FILTEREDabc FILTEREDdef");
}

#[test]
fn host_adjusted_delete_populates_actual_register() {
    let mut h = StringHost::new("abc def");
    let mut s = Session::new();
    let mut e = EditingState::default();
    let Step::HostOperation(mut op) = s.handle(&mut e, Input::key(Key::Char('x')), h.view()) else {
        panic!()
    };
    if let OperationKind::Edit { replacements, .. } = &mut op.kind {
        replacements[0].range.end = ByteOffset(3);
    }
    let result = h.execute(op);
    s.resume(&mut e, result, h.view());
    assert_eq!(s.registers().get('"').unwrap().text(), "abc");
}

#[test]
fn pending_invalid_suffix_is_consumed() {
    let (mut h, mut s, mut e) = run("abc", "d");
    assert_eq!(h.key(&mut s, &mut e, Key::Enter), Step::Consumed);
    assert!(!e.has_pending_input());
    assert_eq!(h.text, "abc");
}

#[test]
fn literal_target_bypasses_host_binding() {
    let (mut h, mut s, mut e) = run("abc", "f");
    let step = s.handle(
        &mut e,
        Input {
            key: Key::Char('c'),
            origin: InputOrigin::Typed,
            binding: Binding::Host(HostCommand {
                id: 99,
                repeatable: false,
            }),
        },
        h.view(),
    );
    h.drive(&mut s, &mut e, step);
    assert_eq!(h.cursor, ByteOffset(2));
}

#[test]
fn capability_rejection_does_not_half_enter_mode() {
    let mut h = StringHost::new("abc");
    let mut s = Session::new();
    let mut e = EditingState::new(
        Mode::Normal,
        Capabilities {
            replace: false,
            visual: false,
            visual_block: false,
        },
    );
    for key in ['v', 'V', 'R'] {
        assert_eq!(
            h.key(&mut s, &mut e, Key::Char(key)),
            Step::Error(EngineError::UnsupportedMode)
        );
        assert_eq!(e.mode(), Mode::Normal);
        assert!(!e.has_pending_input());
    }
}

#[test]
fn reset_preserves_registers_but_stops_playback_and_pending() {
    let (mut h, mut s, mut e) = run("abc", "qaxq");
    h.keys(&mut s, &mut e, "d2");
    s.lifecycle(&mut e, Lifecycle::NewDraft);
    assert_eq!(e.mode(), Mode::Insert);
    assert!(!e.has_pending_input());
    assert!(!s.is_playing());
    assert!(s.registers().get('a').is_some());
    assert!(s.has_last_change());
}

#[test]
fn macro_is_routed_through_host_and_can_suspend() {
    let mut h = StringHost::new("abc");
    let mut s = Session::new();
    s.registers_mut().set(
        'a',
        Register::from_keys(vec![Key::Char('!'), Key::Char('x')]),
    );
    let mut e = EditingState::default();
    h.keys(&mut s, &mut e, "@");
    let Step::Replay(mut input) = s.handle(&mut e, Input::key(Key::Char('a')), h.view()) else {
        panic!()
    };
    assert_eq!(input.key, Key::Char('!'));
    input.binding = Binding::Host(HostCommand {
        id: 42,
        repeatable: false,
    });
    let Step::HostOperation(op) = s.handle(&mut e, input, h.view()) else {
        panic!()
    };
    assert_eq!(
        s.next_macro_input(&e, h.version),
        Step::Error(EngineError::AwaitingHost)
    );
    let result = HostResult {
        operation: op.id,
        before: h.version,
        after: h.version,
        cursor: h.cursor,
        selection: None,
        outcome: HostOutcome::Applied {
            edits: Vec::new(),
            changed: false,
        },
    };
    assert_eq!(s.resume(&mut e, result, h.view()), Step::Consumed);
    let step = s.next_macro_input(&e, h.version);
    h.drive(&mut s, &mut e, step);
    assert_eq!(h.text, "bc");
}

#[test]
fn submission_cancels_macro_tail() {
    let mut h = StringHost::new("abc");
    let mut s = Session::new();
    let mut e = EditingState::default();
    s.registers_mut()
        .set('a', Register::from_keys(vec![Key::Enter, Key::Char('x')]));
    h.keys(&mut s, &mut e, "@");
    let Step::Replay(input) = s.handle(&mut e, Input::key(Key::Char('a')), h.view()) else {
        panic!()
    };
    assert_eq!(input.key, Key::Enter);
    s.lifecycle(&mut e, Lifecycle::NewDraft);
    h.text.clear();
    h.cursor = ByteOffset(0);
    h.version.revision += 1;
    assert_eq!(s.next_macro_input(&e, h.version), Step::Consumed);
    assert!(h.text.is_empty());
}

#[test]
fn recursive_macro_hits_a_bound() {
    let mut h = StringHost::new("abc");
    let mut s = Session::with_limits(Limits {
        macro_depth: 4,
        macro_steps: 30,
        recorded_keys: 10,
    });
    let mut e = EditingState::default();
    s.registers_mut()
        .set('a', Register::from_text("@a", Shape::Character));
    h.keys(&mut s, &mut e, "@");
    assert_eq!(
        h.key(&mut s, &mut e, Key::Char('a')),
        Step::Error(EngineError::PlaybackLimit)
    );
    assert!(!s.is_playing());
}

#[test]
fn uppercase_macro_appends_and_numeric_register_is_shared() {
    let (mut h, mut s, mut e) = run("abcdefgh", "qaxqqAlq@a");
    assert_eq!(h.text, "bdefgh");
    assert_eq!(h.cursor, ByteOffset(2));
    s.registers_mut()
        .set('0', Register::from_text("x", Shape::Character));
    h.keys(&mut s, &mut e, "@0");
    assert_eq!(h.text, "bdfgh");
    h.keys(&mut s, &mut e, "yy");
    assert_eq!(s.registers().get('0').unwrap().text(), "bdfgh\n");
}

#[test]
fn replace_backspace_recovers_original_and_repeats() {
    let (mut h, mut s, mut e) = run("abcdef ghijkl", "RXY<BS>Z<Esc>w.");
    assert_eq!(h.text, "XZcdef XZijkl");
    assert_eq!(e.mode(), Mode::Normal);
    h.keys(&mut s, &mut e, "u");
    assert_eq!(h.text, "XZcdef ghijkl");
}

#[test]
fn visual_block_change_uses_host_geometry() {
    let (h, _, e) = run("abcd\nefgh\nijkl", "<C-v>jlcX<Esc>");
    assert_eq!(h.text, "Xcd\nXgh\nijkl");
    assert_eq!(e.mode(), Mode::Normal);
}

#[test]
fn same_session_shares_dot_but_not_pending_mode() {
    let (mut h, mut s, mut first) = run("abc", "x");
    let mut second = EditingState::default();
    let mut other = StringHost::new("xyz");
    other.version.id = 2;
    other.keys(&mut s, &mut second, ".");
    assert_eq!(other.text, "yz");
    h.keys(&mut s, &mut first, "d");
    assert!(first.has_pending_input());
    assert!(!second.has_pending_input());
}

#[test]
fn line_undo_is_a_distinct_host_request() {
    let h = StringHost::new("abc");
    let mut s = Session::new();
    let mut e = EditingState::default();
    let Step::HostOperation(op) = s.handle(&mut e, Input::key(Key::Char('U')), h.view()) else {
        panic!()
    };
    assert!(matches!(
        op.kind,
        OperationKind::History {
            command: HistoryCommand::UndoLine,
            ..
        }
    ));
}

#[test]
fn mapped_search_target_completes_an_operator_and_replays_at_the_new_cursor() {
    let (mut host, mut session, mut state) = run("one two three", "d");
    let search = TargetRequest::Search {
        backward: false,
        repeat: false,
    };
    let step = session.handle(
        &mut state,
        Input {
            key: Key::Char('z'),
            origin: InputOrigin::Typed,
            binding: Binding::Command(Command::new(Action::Target(search), 1)),
        },
        host.view(),
    );
    let Step::HostOperation(operation) = step else {
        panic!("expected a search request")
    };
    assert!(matches!(operation.kind,OperationKind::Resolve{request,..} if request==search));
    let remembered = TargetRequest::Syntax { id: 7 };
    let result = HostResult {
        operation: operation.id,
        before: host.version,
        after: host.version,
        cursor: host.cursor,
        selection: None,
        outcome: HostOutcome::RememberedTarget {
            range: ResolvedRange::character(0, 4),
            replay: remembered,
        },
    };
    let step = session.resume(&mut state, result, host.view());
    host.drive(&mut session, &mut state, step);
    assert_eq!(host.text, "two three");
    let Step::HostOperation(operation) =
        session.handle(&mut state, Input::key(Key::Char('.')), host.view())
    else {
        panic!("expected a fresh resolution")
    };
    assert!(matches!(operation.kind,OperationKind::Resolve{request,..} if request==remembered));
    assert_eq!(operation.expected, host.version);
}

#[test]
fn moving_in_insert_starts_a_separate_undo_group() {
    let (mut host, mut session, mut state) = run("abc", "iab<Left>X<Esc>u");
    assert_eq!(host.text, "ababc");
    host.keys(&mut session, &mut state, "u");
    assert_eq!(host.text, "abc");
}

#[test]
fn suspended_dot_tail_stays_with_its_surface() {
    let (mut host, mut session, mut state) = run("abc def", "cwX<Esc>w");
    let step = session.handle(&mut state, Input::key(Key::Char('.')), host.view());
    let mut other = StringHost::new("other");
    other.version.id = 2;
    let mut other_state = EditingState::default();
    other.keys(&mut session, &mut other_state, "l");
    host.drive(&mut session, &mut state, step);
    assert_eq!(host.text, "X X");
    assert_eq!(state.mode(), Mode::Normal);
    assert_eq!(other.text, "other");
}

#[test]
fn backspace_in_replace_mode_restores_one_grapheme_from_batched_input() {
    let (mut host, mut session, mut state) = run("aβcd", "R");
    host.key(&mut session, &mut state, Key::Text("X👩‍💻".into()));
    host.key(&mut session, &mut state, Key::Backspace);
    assert_eq!(host.text, "Xβcd");
    host.key(&mut session, &mut state, Key::Backspace);
    assert_eq!(host.text, "aβcd");
}

#[test]
fn host_macro_keys_share_one_budget_across_counts_and_nested_calls() {
    let mut playback = MacroPlayback::with_limits(Limits {
        macro_steps: 4,
        ..Limits::default()
    });
    playback.push(vec![1, 2], 2).unwrap();
    assert_eq!(playback.next_key().unwrap(), Some(1));
    playback.push(vec![3], 2).unwrap();
    assert_eq!(playback.next_key().unwrap(), Some(3));
    assert_eq!(playback.next_key().unwrap(), Some(3));
    assert_eq!(playback.next_key().unwrap(), Some(2));
    assert_eq!(playback.next_key(), Err(EngineError::PlaybackLimit));
    assert!(!playback.is_playing());
    playback.push(vec![4], 1).unwrap();
    assert_eq!(playback.next_key().unwrap(), Some(4));
    assert_eq!(playback.next_key().unwrap(), None);
}

#[test]
fn host_recorded_keys_append_and_play_through_the_shared_register() {
    let mut session = Session::new();
    let mut state = EditingState::default();
    assert_eq!(
        state.macro_binding(&Key::Char('@'), 3),
        Some(MacroBinding::AwaitingRegister)
    );
    assert_eq!(
        state.macro_binding(&Key::Char('a'), 0),
        Some(MacroBinding::Play {
            register: 'a',
            count: 3
        })
    );
    session.start_recording('a').unwrap();
    session.record_key(&Key::Named("<A-x>".into())).unwrap();
    session.finish_recording();
    session.start_recording('A').unwrap();
    session.record_key(&Key::Text("λ".into())).unwrap();
    session.finish_recording();
    let keys = vec![Key::Named("<A-x>".into()), Key::Text("λ".into())];
    assert_eq!(session.macro_keys('a'), Some(keys.clone()));
    assert_eq!(session.macro_keys('@'), Some(keys));
}

#[test]
fn externally_committed_change_replaces_the_previous_dot_recipe() {
    let (host, mut session, mut state) = run("abc", "x");
    session.record_host_change(17);
    let input = Input::key(Key::Char('.'));
    state.set_count(2);
    let Step::HostOperation(op) = session.handle(&mut state, input, host.view()) else {
        panic!("expected an extension request")
    };
    assert_eq!(
        op.kind,
        OperationKind::Host {
            command: HostCommand {
                id: 17,
                repeatable: true
            },
            count: 2
        }
    );
}

#[test]
fn dot_replays_a_host_insert_edit_in_the_same_insertion() {
    let (mut host, mut session, mut state) = run("abc", "iX");
    let command = HostCommand {
        id: 11,
        repeatable: true,
    };
    let mut input = Input::key(Key::Ctrl('w'));
    input.binding = Binding::Host(command);
    let Step::HostOperation(request) = session.handle(&mut state, input, host.view()) else {
        panic!("expected host edit")
    };
    let before = host.version;
    host.text.remove(0);
    host.cursor = ByteOffset(0);
    host.version.revision += 1;
    let result = HostResult {
        operation: request.id,
        before,
        after: host.version,
        cursor: host.cursor,
        selection: None,
        outcome: HostOutcome::Applied {
            edits: vec![AppliedEdit {
                range: TextRange::new(0, 1),
                removed: "X".into(),
                inserted: String::new(),
            }],
            changed: true,
        },
    };
    assert_eq!(
        session.resume(&mut state, result, host.view()),
        Step::Consumed
    );
    host.keys(&mut session, &mut state, "<Esc>");
    let mut step = session.handle(&mut state, Input::key(Key::Char('.')), host.view());
    while let Step::HostOperation(request) = step {
        if request.kind == (OperationKind::Host { command, count: 1 }) {
            assert_eq!(host.text, "Xabc");
            return;
        }
        let result = host.execute(request);
        step = session.resume(&mut state, result, host.view());
    }
    panic!("dot ended before the host edit");
}
