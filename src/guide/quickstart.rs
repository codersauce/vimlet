//! Build a small Vim editor over a plain Rust `String`.
//!
//! This program executes `cwhello<Esc>w.` on `one two three`. Vimlet interprets the
//! keys and tracks the mode; the program applies its requested edits. The result is
//! `hello hello three`, with the editor back in Normal mode.
//!
//! # Add Vimlet to an application
//!
//! You need Rust 1.90 or newer. Create an application:
//!
//! ```sh
//! cargo new vimlet-demo
//! cd vimlet-demo
//! ```
//!
//! Add this under `[dependencies]` in the application's `Cargo.toml`:
//!
//! ```toml
//! vimlet = "0.2"
//! ```
//!
//! The default feature set supports strings and custom text sources. No Ropey
//! dependency is needed.
//!
//! # The complete program
//!
//! Copy the following into `src/main.rs`. It uses only Vimlet and the standard library;
//! all the host code is included. The same source lives in `examples/quickstart.rs`
//! and runs as a doctest, so the example is checked against the public API.
//!
//! This small host handles edits and cursor/selection changes. It has no undo history
//! or external services and rejects those requests. It dispatches the built-in keys
//! used below; the [integration guide](super::integration) covers application shortcuts
//! and macro replay.
#![doc = concat!("\n```rust\n", include_str!("../../examples/quickstart.rs"), "\n```\n")]
//!
//! # Run it
//!
//! In your application directory:
//!
//! ```sh
//! cargo run
//! ```
//!
//! Output:
//!
//! ```text
//! hello hello three
//! ```
//!
//! From the Vimlet checkout, run the same program with:
//!
//! ```sh
//! cargo run --locked --example quickstart
//! ```
//!
//! # Follow one command
//!
//! | Input | Engine response | Host work |
//! | --- | --- | --- |
//! | `c` | `AwaitingInput` | Keep the pending command and wait for its target. |
//! | `w` | An edit removing `one` | Apply it and acknowledge it; Vimlet enters Insert mode. |
//! | `hello` | An insertion for each character | Apply each edit and advance the document revision. |
//! | Escape | An edit operation ending the insertion transaction | Acknowledge it; Vimlet returns to Normal mode. |
//! | `w` | A cursor move | Move to `two` and acknowledge the new cursor. |
//! | `.` | The saved change, resolved at the new cursor | Keep applying and acknowledging operations until repeat completes. |
//!
//! Read [`EditingState::mode`](crate::EditingState::mode) after processing the returned
//! steps to update your cursor shape or mode indicator. The final assertion verifies
//! both the edited text and the return to Normal mode.
//!
//! Next: [connect your application's document and event loop](super::integration),
//! then add the [recipes](super::recipes) your surface needs.
