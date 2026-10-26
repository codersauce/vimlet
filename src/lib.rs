//! Headless Vim key handling and motion resolution over borrowed UTF-8 text.
//!
//! Vimlet adds Vim editing to an application that already owns its text buffer.
//! It interprets keys, manages modes, registers and repeat, and asks your application
//! to apply edits. It needs no terminal, async runtime or Red dependency. Strings
//! work out of the box; Ropey support is an optional feature.
//!
//! # Start here
//!
//! **[Quickstart: edit a string with Vim keys](guide::quickstart)** is a complete,
//! runnable program. It changes `one two three` into `hello hello three` with
//! `cwhello<Esc>w.`, including the host code that applies edits and acknowledges them.
//!
//! Then follow the **[host integration guide](guide::integration)** to connect your
//! document, undo history and event loop. The **[recipes](guide::recipes)** cover
//! custom mappings, macros, composer drafts and Ropey storage.
//!
//! | I want to… | Start with |
//! | --- | --- |
//! | Add modal editing to a buffer or input field | [`Session`], [`EditingState`] and the [quickstart](guide::quickstart) |
//! | Compute a motion destination or text-object range | [`MotionResolver`] and the [example below](#resolving-a-motion) |
//! | Connect my application's mutation and undo pipeline | [`HostOperation`], [`HostResult`] and the [integration guide](guide::integration) |
//! | Use my own storage or a Ropey buffer | [`TextSource`] and the [storage recipe](guide::recipes#text-storage-and-ropey) |
//!
//! # What Vimlet owns
//!
//! Keep one [`EditingState`] per editing surface and a [`Session`] for surfaces that
//! share registers, macro playback and dot repeat. Pass a borrowed [`DocumentView`]
//! with each input. The view does not transfer ownership of the buffer.
//!
//! | Vimlet | Your application |
//! | --- | --- |
//! | Key interpretation, counts and pending commands | Terminal events and custom keymaps |
//! | Modes, register contents, dot recipes and macro state | Text storage, cursor, selection and revisions |
//! | Motions, text objects and proposed edits | Applying edits, undo history and change notifications |
//! | Requests for host services | Search UI, syntax, clipboard transport and rendering |
//!
//! [`Session::handle`] returns a [`Step`]. For a host operation, apply or reject the
//! request and acknowledge it with [`Session::resume`]. Process the returned step too:
//! one key can produce several operations. Cursor and edit coordinates are UTF-8 bytes.
//!
//! # Resolving a motion
//!
//! The resolver also works independently of modes and key handling. It reads your
//! text and returns a destination or range without modifying the document.
//!
//! ```
//! use vimlet::{ByteOffset, Motion, MotionResolver};
//!
//! let text = "éclair";
//! let resolver = MotionResolver::new(&text, ByteOffset(0));
//! let destination = resolver.resolve(Motion::Right, 1).map(|target| target.position);
//! assert_eq!(destination, Some(ByteOffset(2)));
//! ```
//!
//! For inner/around words, quotes or delimiters, use [`MotionResolver::text_object`].
//! For operator targets, use [`MotionResolver::operator_range`] to account for Vim's
//! inclusive, exclusive and linewise range rules.
//!
//! # Compatibility
//!
//! Neovim decides behavior for the Vim commands implemented by this crate. The
//! reference suite compares text, byte cursor, mode and unnamed register against
//! Neovim 0.12.1. This is a modal editing engine, not an implementation of Neovim's
//! entire command language or plugin runtime. Host services and display behavior
//! depend on your adapter; the [integration guide](guide::integration#host-services)
//! explains that boundary.

#![warn(missing_docs)]

pub mod engine;
#[cfg(doc)]
pub mod guide;
pub mod motion;
pub mod registers;
pub mod text;
pub mod types;

pub use engine::{
    Action, AppliedEdit, Binding, BindingContext, Capabilities, Command, EditTarget, EditingState,
    EngineError, HistoryCommand, HostCommand, HostOperation, HostOutcome, HostResult, Input,
    InputOrigin, InsertStyle, Key, Lifecycle, Limits, MacroBinding, MacroPlayback, Mode,
    OperationKind, Operator, Replacement, Session, Step, TargetRequest, Transaction,
};
pub use motion::{
    CharacterMotion, Motion, MotionResolver, ResolvedRange, Target, TextObjectKind,
    TextObjectScope, WordClass, WordDestination,
};
pub use registers::{Register, Registers};
pub use text::TextSource;
pub use types::{
    ByteOffset, DocumentVersion, DocumentView, Geometry, Selection, Shape, TerminalGeometry,
    TextRange, DEFAULT_GEOMETRY,
};
