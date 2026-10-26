//! Headless Vim key handling and motion resolution over borrowed UTF-8 text.
//!
//! Keep an [`EditingState`] for each editing surface. A [`Session`] interprets
//! input, tracks registers and repeat history, and returns [`Step`]s. The host
//! owns the document and acknowledges operations with [`Session::resume`].
//! Cursor positions and edit ranges use UTF-8 byte offsets in a particular
//! document revision. Enable the `ropey` feature for a Ropey text source.
//!
//! # Examples
//!
//! Use [`MotionResolver`] directly when all you need is a destination or range.
//!
//! ```
//! use vimlet::{ByteOffset, Motion, MotionResolver};
//!
//! let text = "éclair";
//! let resolver = MotionResolver::new(&text, ByteOffset(0));
//! let destination = resolver.resolve(Motion::Right, 1).map(|target| target.position);
//! assert_eq!(destination, Some(ByteOffset(2)));
//! ```

#![warn(missing_docs)]

pub mod engine;

pub mod motion;

pub mod registers;

pub mod text;

pub mod types;

pub use engine::{
    Action, AppliedEdit, Binding, BindingContext, Capabilities, Command, EditTarget, EditingState,
    EngineError, HistoryCommand, HostCommand, HostOperation, HostOutcome, HostResult, Input,
    InputOrigin, InsertStyle, Key, Lifecycle, Limits, Mode, OperationKind, Operator, Replacement,
    Session, Step, TargetRequest, Transaction,
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
