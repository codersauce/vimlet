//! Connect Vimlet to your document, event loop and application services.
//!
//! The [quickstart](super::quickstart) shows a synchronous string host. This guide
//! explains the contracts to preserve when replacing it with your application's
//! storage, mutation pipeline and services.
//!
//! # Keep state at the right scope
//!
//! Use one [`EditingState`](crate::EditingState) per editing surface. It stores the
//! mode, pending counts and operators, and any outstanding host operation. Recreating
//! it for every key loses compound commands such as `2dw`.
//!
//! A [`Session`](crate::Session) holds registers, macro state and the last successful
//! change for dot. Share a session when those should be shared between surfaces;
//! create separate sessions when they should be independent. Dispatch each outstanding
//! operation back to the surface that requested it.
//!
//! Your application retains its document, cursor, Visual selection and undo history.
//! Build a fresh [`DocumentView`](crate::DocumentView) for every call to `handle` or
//! `resume`. It borrows the current text and describes that exact revision.
//!
//! # Dispatch input and drain the returned steps
//!
//! Normalize an application event into a [`Key`](crate::Key), then call
//! [`Session::handle`](crate::Session::handle) with the input and current view.
//! [`Input::key`](crate::Input::key) uses Vimlet's built-in bindings. For a custom
//! keymap, consult [`EditingState::binding_context`](crate::EditingState::binding_context)
//! before assigning the input's binding; see the [mapping recipe](super::recipes#custom-keymaps).
//!
//! ```text
//! key + current document view
//!             |
//!       Session::handle
//!             |
//!       Step::HostOperation ----> check version, apply or reject
//!             ^                              |
//!             |                        HostResult + fresh view
//!             |                              |
//!             +----------------------- Session::resume
//!
//! Stop when the step needs input, is consumed/unhandled, or reports an error.
//! Macro replay returns input to the application's normal dispatch path.
//! ```
//!
//! Both `handle` and `resume` return [`Step`](crate::Step). Handle every variant:
//!
//! | Step | Host responsibility |
//! | --- | --- |
//! | `HostOperation` | Execute or reject the request, then call `resume` and process its return value. |
//! | `AwaitingInput` | Keep the pending state and wait for the next key in the command. |
//! | `Consumed` | Finish this input. During macro playback, request the next macro key. |
//! | `Unhandled` | Route the original input to application shortcuts. |
//! | `Replay` | Send the recorded input through ordinary application dispatch, preserving its origin. |
//! | `Error` | Report or handle the error; do not count the command as a successful edit. |
//!
//! A single dot-repeat command can generate several host operations. Stopping after
//! the first acknowledgement leaves the remaining change unapplied. A prefix can also
//! span macro keys: after a replayed key completes, request the next key even when the
//! interpreter returned `AwaitingInput`.
//!
//! Do not send another key to a surface with an outstanding host operation. If a
//! search prompt or plugin call is asynchronous, retain the operation and acknowledge
//! it when the service finishes. Queue later input or explicitly cancel the pending
//! operation with a lifecycle transition; a cancelled operation's eventual result
//! must not be applied or resumed.
//!
//! # Apply edits and acknowledge actual results
//!
//! Route [`OperationKind::Edit`](crate::OperationKind::Edit) through the application's
//! normal mutation pipeline. This is where undo storage, marks, change notifications,
//! language-server synchronization and rendering should observe the change.
//!
//! 1. Check [`HostOperation::is_current`](crate::HostOperation::is_current) immediately
//!    before mutation. Check again after any asynchronous work.
//! 2. Capture the text that will be removed. Every replacement range refers to the
//!    **pre-operation revision**, so apply the replacements from end to start.
//! 3. Update the document revision if content changed, then set the actual cursor and
//!    selection. A cursor move alone need not advance the content revision.
//! 4. Build [`HostResult`](crate::HostResult), echoing the operation's `id` and
//!    `expected` version as `operation` and `before`. Report the actual resulting
//!    version, cursor and selection in both the result and the fresh view.
//! 5. Call `resume` and process the next step.
//!
//! [`HostOutcome::Applied`](crate::HostOutcome::Applied) contains the actual
//! [`AppliedEdit`](crate::AppliedEdit) values, sorted and disjoint in original-document
//! order. Report host filtering or range expansion accurately: register contents and
//! repeat depend on the removed and inserted text. Set `changed` to whether content
//! changed; a true value requires a strictly newer revision.
//!
//! For [`OperationKind::Move`](crate::OperationKind::Move), update the cursor and
//! selection and acknowledge with no edits and `changed: false`. Moves also require
//! acknowledgement because they can complete a mode transition.
//!
//! # Coordinate and revision rules
//!
//! [`ByteOffset`](crate::ByteOffset) counts UTF-8 bytes from the start of the document.
//! For example, moving right across `é` advances by two bytes. It does not count
//! Unicode scalars, graphemes or display columns. Cursor and selection positions must
//! lie on grapheme boundaries; EOF is a valid coordinate.
//!
//! Edit ranges are half-open (`start..end`). Visual selection anchors and cursors
//! are inclusive and can run in either direction. Keep the selection's
//! [`Shape`](crate::Shape) when reporting it back to the engine.
//!
//! Give each document a stable [`DocumentVersion::id`](crate::DocumentVersion::id).
//! Its revision must advance for content changes made anywhere in the application,
//! including external edits and undo. Restoring older text must not restore an older
//! revision number. A new draft or document should get a new identity.
//!
//! If an operation is stale, leave the document untouched and acknowledge it with
//! [`HostOutcome::Rejected`](crate::HostOutcome::Rejected). Keep `before` equal to the
//! operation's expected version and report the current version as `after`. If the
//! surface has already switched documents, cancel its pending operation through
//! [`Session::lifecycle`](crate::Session::lifecycle) and discard the old continuation.
//!
//! # Group undo transactions
//!
//! Vimlet describes undo boundaries; the host stores and restores history.
//! [`Transaction`](crate::Transaction) accompanies every edit operation:
//!
//! | Phase | Host action |
//! | --- | --- |
//! | `Atomic` | Record a complete command as one undo group. |
//! | `Begin` | Open a group for a change or Insert/Replace session. |
//! | `Continue` | Append to the current group. |
//! | `End` | Close the group when leaving Insert/Replace. |
//! | `BreakInsert` | Close the group while retaining an insertion-point cursor. |
//!
//! An edit with no replacements can still open or close a transaction. The deletion
//! and typed replacement in `cwhello<Esc>` belong to the same undo group.
//! [`OperationKind::History`](crate::OperationKind::History) asks the host to execute
//! undo, redo or line undo. Report the resulting document revision and surface state
//! through the same acknowledgement protocol.
//!
//! # Host services
//!
//! Some commands need information or policies that belong to the application:
//!
//! | Operation | Host implementation |
//! | --- | --- |
//! | `Resolve` | Search, marks, syntax targets, display geometry or block projection. |
//! | `Transform` | Indentation, outdentation or formatting over resolved ranges. |
//! | `History` | Undo, redo and line undo. |
//! | `Host` | An application command identified by a stable numeric ID. |
//!
//! Resolve requests produce [`HostOutcome::Target`](crate::HostOutcome::Target) with
//! ranges against the requested revision. A target lookup does not mutate the document.
//! To repeat a saved search or syntax query at a new cursor, return
//! [`HostOutcome::RememberedTarget`](crate::HostOutcome::RememberedTarget) and retain
//! the query needed to interpret its replay request. See the
//! [search recipe](super::recipes#search-and-dot-repeat).
//!
//! Reject unsupported or cancelled operations. Do not acknowledge a service as applied
//! when no implementation ran. A service can finish asynchronously; the revision
//! check still belongs immediately before mutation or accepting its resolved ranges.
//!
//! # Finish or replace an editing context
//!
//! Use [`Lifecycle`](crate::Lifecycle) to distinguish cancelling a prefix, leaving
//! modal interaction, switching documents, and starting or replacing a draft.
//! [`Session::lifecycle`](crate::Session::lifecycle) clears transient state and queued
//! continuations while retaining session registers and the last successful change.
//!
//! Finish an active Insert/Replace transaction by dispatching Escape and completing
//! its host operations before changing contexts. Lifecycle calls do not mutate or
//! close host undo history. For draft reset behavior, see the
//! [composer recipe](super::recipes#composer-drafts).
//!
//! # Troubleshooting
//!
//! | Symptom | Check |
//! | --- | --- |
//! | `cw` or a count never completes | Reuse the same editing state between keys. |
//! | Text changes but the mode does not | Acknowledge the operation and read the mode after `resume`. |
//! | Dot applies only part of the change | Drain every step returned by `resume`. |
//! | `AwaitingHost` | A new key arrived before the previous operation was acknowledged. |
//! | `InvalidDocument` | Check byte offsets, grapheme boundaries and sorted, disjoint opaque spans. |
//! | `UnexpectedResult` | Check the operation token and that the acknowledgement matches the outstanding operation kind. |
//! | `StaleResult` | Compare the result's versions, cursor and selection with the fresh view; advance the revision for content changes. |
//! | Unicode cursor movement is wrong | Convert host character indices to UTF-8 byte offsets at the adapter boundary. |
//! | Undo splits one insertion into many steps | Honor `Begin`/`Continue`/`End` rather than creating a group per character. |
//!
//! The executable `examples/support/mod.rs` host demonstrates history and macro
//! dispatch. `examples/support/rope.rs` demonstrates converting byte ranges for rope
//! mutation. Both are small example adapters; your application's services stay in
//! its own host implementation.
