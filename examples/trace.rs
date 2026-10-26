//! JSON-lines driver for the development-only Neovim comparison runner.
mod support;

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead};
use vimlet::{BindingContext, ByteOffset, EditingState, Mode, Session, Shape};

#[derive(Deserialize)]
struct Case {
    text: String,
    keys: String,
    #[serde(default)]
    cursor: usize,
}

#[derive(Serialize)]
struct Observation {
    text: String,
    cursor: usize,
    mode: String,
    register: String,
    register_type: String,
}

fn main() {
    for line in io::stdin().lock().lines() {
        let case: Case = serde_json::from_str(&line.unwrap()).unwrap();
        let mut host = support::StringHost::new(&case.text);
        host.cursor = ByteOffset(case.cursor);
        let mut session = Session::new();
        let mut state = EditingState::default();
        host.keys(&mut session, &mut state, &case.keys);
        let mode = match state.mode() {
            Mode::Normal => {
                if state.binding_context() == BindingContext::OperatorPending {
                    "no"
                } else {
                    "n"
                }
            }
            Mode::Insert => "i",
            Mode::Replace => "R",
            Mode::Visual => "v",
            Mode::VisualLine => "V",
            Mode::VisualBlock => "\u{16}",
        };
        let (register, register_type) =
            session
                .registers()
                .get('"')
                .map_or((String::new(), String::new()), |r| {
                    (
                        r.text().to_owned(),
                        match r.shape() {
                            Shape::Character => "v".into(),
                            Shape::Line => "V".into(),
                            Shape::Block { width } => format!("\u{16}{width}"),
                        },
                    )
                });
        println!(
            "{}",
            serde_json::to_string(&Observation {
                text: host.text,
                cursor: host.cursor.0,
                mode: mode.into(),
                register,
                register_type
            })
            .unwrap()
        );
    }
}
