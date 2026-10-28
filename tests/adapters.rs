#![cfg(feature = "ropey")]
#[path = "../examples/support/mod.rs"]
mod support;

use vimlet::*;

#[test]
fn the_same_command_streams_work_over_borrowed_strings_and_ropes() {
    let cases: serde_json::Value = serde_json::from_str(include_str!("conformance.json")).unwrap();
    let mut compared = 0;
    for case in cases.as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        let keys = case["keys"].as_str().unwrap();
        // The tiny rope example has no undo or block-layout service. These contracts
        // are covered by the string host and service-specific tests.
        if keys.contains("<C-v>")
            || keys.contains("<C-r>")
            || keys.contains("xU")
            || keys.contains("<Esc>u")
            || case["requires_history"].as_bool() == Some(true)
        {
            continue;
        }
        let cursor = ByteOffset(case["cursor"].as_u64().unwrap_or(0) as usize);
        let (mut a, mut b) = (
            support::StringHost::new(text),
            support::rope::RopeHost::new(text),
        );
        a.cursor = cursor;
        b.cursor = cursor;
        let (mut sa, mut sb) = (Session::new(), Session::new());
        let (mut ea, mut eb) = (EditingState::default(), EditingState::default());
        a.keys(&mut sa, &mut ea, keys);
        b.keys(&mut sb, &mut eb, keys);
        assert_eq!(a.text, b.text.to_string(), "{case}");
        assert_eq!(a.cursor, b.cursor, "{case}");
        assert_eq!(ea.mode(), eb.mode(), "{case}");
        assert_eq!(sa.registers().get('"'), sb.registers().get('"'), "{case}");
        compared += 1;
    }
    assert!(compared > 150);
}
