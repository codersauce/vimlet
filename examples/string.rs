mod support;

use vimlet::{EditingState, Session};

fn main() {
    let mut host = support::StringHost::new("one two three");
    let mut session = Session::new();
    let mut state = EditingState::default();
    host.keys(&mut session, &mut state, "cwhello<Esc>w.");
    println!(
        "{} (mode: {:?}, cursor: {})",
        host.text,
        state.mode(),
        host.cursor.0
    );
}
