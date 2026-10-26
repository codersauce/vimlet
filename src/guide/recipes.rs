//! Adapt Vimlet to keymaps, macros, composer drafts and different text storage.
//!
//! These recipes assume the dispatch and acknowledgement loop from the
//! [integration guide](super::integration).
//!
//! # Custom keymaps
//!
//! Query [`EditingState::binding_context`](crate::EditingState::binding_context) before
//! resolving each key. Keep the original key and change its
//! [`Binding`](crate::Binding) to describe the mapping. This preserves the input that
//! macro recording must remember.
//!
//! For example, map `L` to one rightward motion in Normal and operator-pending contexts:
//!
//! ```
//! use vimlet::{Action, Binding, BindingContext, Command, EditingState, Input, Key, Motion};
//!
//! fn mapped_input(state: &EditingState, key: Key) -> Input {
//!     let mut input = Input::key(key);
//!     if input.key == Key::Char('L')
//!         && matches!(
//!             state.binding_context(),
//!             BindingContext::Normal | BindingContext::OperatorPending
//!         )
//!     {
//!         input.binding = Binding::Command(Command::new(Action::Move(Motion::Right), 0));
//!     }
//!     input
//! }
//!
//! let state = EditingState::default();
//! assert!(matches!(mapped_input(&state, Key::Char('L')).binding, Binding::Command(_)));
//! ```
//!
//! A command count of zero means no explicit count. A mapped motion can complete
//! a pending operator. In `Literal` context, preserve suffixes such as the character
//! after `f`; the same `L` must still name the character to find. Use `Binding::Host`
//! for application commands that participate in the host protocol, or
//! `Binding::Unhandled` to give an application shortcut ownership of the key.
//!
//! # Macro recording and playback
//!
//! Built-in `q` and `@` commands use the session's register bank. Recording retains
//! original typed keys; playback returns them as [`Step::Replay`](crate::Step::Replay).
//! Resolve each replayed input through the application's current keymap, retaining
//! [`InputOrigin::Macro`](crate::InputOrigin::Macro). Calling `Input::key` to recreate
//! it would incorrectly mark it as typed input.
//!
//! After the replayed key and its host operations finish, call
//! [`Session::next_macro_input`](crate::Session::next_macro_input). Keep requesting keys
//! while playback is active, including after a consumed prefix. Suspend this loop while
//! a host operation is outstanding, and resume it after acknowledgement. Use an
//! iterative event loop so nested macros do not recurse through the application stack.
//!
//! [`Limits`](crate::Limits) bounds recording length, macro depth and dispatched keys.
//! Report limit errors and use [`Session::cancel_playback`](crate::Session::cancel_playback)
//! when application interaction interrupts playback. If a host operation is outstanding,
//! also cancel that surface's continuation with a lifecycle transition before accepting
//! replacement input.
//!
//! # Search and dot repeat
//!
//! A search-based operator asks the host to resolve a
//! [`TargetRequest::Search`](crate::TargetRequest::Search). Open the application's
//! prompt, resolve the query in the requested revision, and return the range. For an
//! operator ending at a search match, [`MotionResolver::exclusive_range`](crate::MotionResolver::exclusive_range)
//! applies the exclusive endpoint and linewise conversion rules to the match position.
//!
//! When dot should repeat that query, return
//! [`HostOutcome::RememberedTarget`](crate::HostOutcome::RememberedTarget). Retain the
//! query behind a replay request that your adapter can recognize. For example, a
//! host-defined syntax target ID can identify a saved query. When Vimlet requests it
//! again, resolve it from the new cursor; do not reuse old byte ranges or substitute
//! an unrelated search typed after the original change. The host owns the saved query
//! and its lifetime; Vimlet retains the replay request.
//!
//! # Composer drafts
//!
//! A composer can start in Insert mode and disable modes it cannot display:
//!
//! ```
//! use vimlet::{Capabilities, EditingState, Lifecycle, Mode, Session};
//!
//! let mut session = Session::new();
//! let mut state = EditingState::new(
//!     Mode::Insert,
//!     Capabilities { replace: false, visual: true, visual_block: false },
//! );
//! assert_eq!(state.mode(), Mode::Insert);
//!
//! // At a completed editing boundary, invalidate old pending draft state.
//! session.lifecycle(&mut state, Lifecycle::NewDraft);
//! assert_eq!(state.mode(), Mode::Insert);
//! ```
//!
//! Before submitting or replacing an active insertion, dispatch Escape and finish
//! its host operations to close the current undo group. Then assign the fresh draft
//! its own document identity, reset the host cursor and selection, and call
//! `lifecycle` with `NewDraft` or `ReplaceDraft`. The session's registers and last
//! successful change survive; pending surface work does not.
//!
//! Use [`EditingState::owns_escape`](crate::EditingState::owns_escape) to decide whether
//! Escape belongs to Vimlet before using it to dismiss application UI. Represent an
//! attachment or placeholder with a sorted, nonempty range in
//! [`DocumentView::opaque`](crate::DocumentView::opaque) to make it one motion unit.
//! Recompute those ranges after host edits, keeping them disjoint and on grapheme
//! boundaries.
//!
//! # Text storage and Ropey
//!
//! [`String`] and `&str` implement [`TextSource`](crate::TextSource) with no features
//! enabled. For another buffer type, implement `len_bytes` and `chunk_at`. Chunks must
//! cover the text without gaps or overlaps, end on Unicode scalar boundaries, and
//! report their absolute starting byte offsets. The trait supplies default navigation
//! methods; a storage adapter can override them where it has a more efficient index.
//!
//! If your application already uses Ropey, enable the adapter:
//!
//! ```toml
//! [dependencies]
//! vimlet = { version = "0.1", features = ["ropey"] }
//! ropey = "1.6.1"
//! ```
//!
//! The feature implements `TextSource` for `ropey::Rope`. It lets Vimlet read chunks
//! without first copying the whole document into a string. Ropey remains the host's
//! storage; Vimlet still proposes byte-range edits for the host to apply.
//!
//! ```
//! # #[cfg(feature = "ropey")]
//! # {
//! use ropey::Rope;
//! use vimlet::{ByteOffset, DocumentVersion, DocumentView, Motion, MotionResolver};
//!
//! let rope = Rope::from_str("éclair");
//! let version = DocumentVersion { id: 1, revision: 0 };
//! let view = DocumentView::new(&rope, version, ByteOffset(0));
//! let destination = MotionResolver::from_view(view).resolve(Motion::Right, 1);
//! assert_eq!(destination.map(|target| target.position), Some(ByteOffset(2)));
//! # }
//! ```
//!
//! Convert replacement byte endpoints with `Rope::byte_to_char` before calling Ropey's
//! character-indexed `remove` and `insert` methods. Apply replacements from end to start
//! just as with a string. Run `cargo run --features ropey --example rope` to see the
//! mutation adapter in `examples/support/rope.rs`.
//!
//! # Display geometry
//!
//! [`DocumentView::new`](crate::DocumentView::new) uses terminal cell widths with
//! eight-column tabs. Set the view's geometry to a [`TerminalGeometry`](crate::TerminalGeometry)
//! with your tab stop, or implement [`Geometry`](crate::Geometry) for another display.
//! Vimlet passes a grapheme or a complete opaque span plus its starting display column
//! to the geometry's `width` method. Block selections also require the host to project
//! their rectangle into byte ranges when it receives `TargetRequest::Block`.
