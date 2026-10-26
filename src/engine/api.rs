//! Public input, command and host execution protocol.
use crate::{
    ByteOffset, DocumentVersion, Motion, ResolvedRange, Selection, Shape, TextObjectKind,
    TextObjectScope, TextRange,
};

/// Editing mode reported to the host after each accepted operation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Interpret keys as commands and motion prefixes.
    #[default]
    Normal,
    /// Insert typed text at the cursor.
    Insert,
    /// Overwrite text and retain overwritten graphemes for Backspace.
    Replace,
    /// Extend an inclusive characterwise selection.
    Visual,
    /// Select complete logical lines.
    VisualLine,
    /// Select a rectangle in host display columns.
    VisualBlock,
}

impl Mode {
    /// Report whether the mode uses a Visual anchor and cursor.
    pub fn is_visual(self) -> bool {
        matches!(self, Self::Visual | Self::VisualLine | Self::VisualBlock)
    }
}

/// Editing modes that the host can present on this surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Allow continuous overwrite with `R`.
    pub replace: bool,
    /// Allow entry into characterwise and linewise Visual modes.
    pub visual: bool,
    /// Allow rectangular selections when Visual mode is enabled.
    pub visual_block: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            replace: true,
            visual: true,
            visual_block: true,
        }
    }
}

/// Terminal-independent key. Text represents an accepted paste/input-method event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// One decoded Unicode scalar from an ordinary keypress.
    Char(char),
    /// A Control chord identified by its letter, such as `Ctrl('r')`.
    Ctrl(char),
    /// Cancel pending input or leave the current editing mode.
    Escape,
    /// The Return key, interpreted according to the active mode.
    Enter,
    /// Remove text before the insertion point or recover overwritten text in Replace.
    Backspace,
    /// Delete at the cursor without advancing it.
    Delete,
    /// The Tab key, distinct from a sequence of spaces.
    Tab,
    /// Move toward the preceding grapheme on the line.
    Left,
    /// Move toward the following grapheme on the line.
    Right,
    /// Move toward the preceding logical line.
    Up,
    /// Move toward the following logical line.
    Down,
    /// Move to the start of the current logical line.
    Home,
    /// Move to the end of the current logical line.
    End,
    /// A host-specific key identifier retained verbatim for macro playback.
    Named(String),
    /// A batch of UTF-8 input from a paste or input method.
    Text(String),
}

/// Provenance of a key used to distinguish recording from playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOrigin {
    /// Input supplied directly by the user and eligible for macro recording.
    Typed,
    /// A recorded key returning through host dispatch during playback.
    Macro,
    /// Programmatically supplied input excluded from user-key recording.
    Injected,
}

/// How the host keymap interpreted the original input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// Interpret the key using Vimlet's built-in bindings.
    Default,
    /// Interpret a semantic Vim command selected by the host keymap.
    Command(Command),
    /// Dispatch a host-defined command through the execution protocol.
    Host(HostCommand),
    /// Give the surrounding application ownership of an unmapped key.
    Unhandled,
}

/// An original key together with its provenance and resolved binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    /// Original normalized key, preserved even if its binding changes.
    pub key: Key,
    /// Source of the input used to decide whether to record it.
    pub origin: InputOrigin,
    /// Resolution of the original key in the active keymap.
    pub binding: Binding,
}

impl Input {
    /// Create typed input that uses the built-in Vim binding.
    pub fn key(key: Key) -> Self {
        Self {
            key,
            origin: InputOrigin::Typed,
            binding: Binding::Default,
        }
    }
}

/// Keymap context for the next input on this surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingContext {
    /// Resolve keys using the Normal-mode command map.
    Normal,
    /// Resolve keys at an insertion point.
    Insert,
    /// Resolve keys during continuous overwrite.
    Replace,
    /// Resolve keys while extending or operating on a selection.
    Visual,
    /// Use the original key as a suffix, such as the character after `f`.
    Literal,
    /// Resolve the remainder of a compound command or operator target.
    OperatorPending,
    /// Wait for a host result before dispatching another key.
    AwaitingHost,
}

/// Action to apply to a resolved motion, text object or selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    /// Remove the target and write its contents to a register.
    Delete,
    /// Remove the target and enter Insert after the host accepts the edit.
    Change,
    /// Copy the target into the register bank.
    Yank,
    /// Convert the selected text to lowercase.
    Lowercase,
    /// Convert the selected text to uppercase.
    Uppercase,
    /// Invert the case of each selected letter.
    ToggleCase,
    /// Ask the host to increase indentation over the target.
    Indent,
    /// Ask the host to decrease indentation over the target.
    Outdent,
    /// Ask the host to toggle language-specific comments over the target.
    Comment,
    /// Ask the host to format the target using its formatting policy.
    Format,
}

/// Where insertion begins and whether it overwrites existing text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertStyle {
    /// Insert immediately before the current grapheme (`i`).
    Before,
    /// Insert after the current grapheme (`a`).
    After,
    /// Insert at the first nonblank position of the line (`I`).
    FirstNonBlank,
    /// Append at the logical line end (`A`).
    LineEnd,
    /// Open a new logical line above the cursor (`O`).
    OpenAbove,
    /// Open a new logical line below the cursor (`o`).
    OpenBelow,
    /// Begin continuous overwrite at the cursor (`R`).
    Replace,
}

/// Undo operation to execute through host-owned history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryCommand {
    /// Restore an earlier host undo group (`u`).
    Undo,
    /// Reapply an undone group (Ctrl-R).
    Redo,
    /// Undo changes to the line using Neovim's `U` semantics.
    UndoLine,
}

/// Host-defined command identifier, not an application enum or callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCommand {
    /// Stable identifier interpreted by the application.
    pub id: u64,
    /// Whether an accepted host edit can be requested again by dot.
    /// In an active insertion, report the actual edits to retain this command in its repeat sequence.
    pub repeatable: bool,
}

/// Target whose query, geometry or syntax information belongs to the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRequest {
    /// Resolve an interactive or repeated search through the host.
    Search {
        /// Use the backward or reverse search direction when true.
        backward: bool,
        /// Reuse the previous host search when true.
        repeat: bool,
    },
    /// Look up a host-maintained mark as an operator or movement target.
    Mark {
        /// Register-like name of the mark.
        name: char,
        /// Resolve to the marked logical line when true.
        linewise: bool,
    },
    /// Resolve a target using the host's view or layout.
    Geometry {
        /// Host-defined layout target identifier.
        id: u64,
    },
    /// Resolve a target using the host's syntax services.
    Syntax {
        /// Host-defined syntax target identifier.
        id: u64,
    },
    /// Project a rectangular selection onto individual logical lines.
    Block {
        /// Inclusive anchor and cursor of the block to project.
        selection: Selection,
    },
}

/// Source of the ranges consumed by a Vim operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditTarget {
    /// Resolve a motion relative to the current cursor.
    Motion(Motion),
    /// Resolve the specified inner or around text object.
    Object(TextObjectScope, TextObjectKind),
    /// Use complete logical lines starting at the cursor's line.
    Lines,
    /// Use the shape and endpoints of the active Visual selection.
    Selection,
    /// Suspend interpretation while the host resolves the target.
    Host(TargetRequest),
}

/// Semantic command retained for repeat independently of key mappings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Replay the last successful change. An explicit count replaces its original count.
    Repeat,
    /// Extend Visual selection to the entire range returned by a host text object.
    SelectTarget(TargetRequest),
    /// Select a text object using the requested boundaries.
    SelectObject {
        /// Whether to include adjacent whitespace or delimiters.
        scope: TextObjectScope,
        /// Object family to resolve at the cursor.
        kind: TextObjectKind,
    },
    /// Move the cursor, extending the anchor when Visual mode is active.
    Move(Motion),
    /// Resolve a target and then apply an editing operator to it.
    Operate {
        /// Operation to perform once the target is known.
        operator: Operator,
        /// Motion, object, selection or host service that defines the range.
        target: EditTarget,
    },
    /// Begin an Insert or Replace transaction at the requested position.
    Insert(InsertStyle),
    /// Replace selected graphemes with the supplied character (`r`).
    ReplaceChar(char),
    /// Delete graphemes on the current line (`x` or `X`).
    DeleteChars {
        /// Delete preceding graphemes when true.
        backward: bool,
    },
    /// Put the unnamed register using its recorded selection shape.
    Paste {
        /// Insert before the cursor or line when true (`P`).
        before: bool,
    },
    /// Toggle case at the cursor or over the selection (`~`).
    ToggleCase,
    /// Join logical lines, including those covered by a Visual selection.
    Join {
        /// Use Vim's whitespace adjustment for `J`, or preserve spacing for `gJ`.
        spaces: bool,
    },
    /// Enter, switch or toggle the requested Visual shape.
    Visual(Shape),
    /// Exchange the active cursor and selection anchor (`o`).
    SwapVisual,
    /// Restore the previous Visual selection (`gv`).
    Reselect,
    /// Request a change to host-owned undo history.
    History(HistoryCommand),
    /// Move or extend a selection to a host-resolved destination.
    Target(TargetRequest),
    /// Dispatch a host-defined semantic action.
    Host(HostCommand),
    /// Insert on the rows covered by a Visual block.
    BlockInsert {
        /// Append at the far edge of the block when true (`A`).
        append: bool,
    },
}

/// An action and its prefix count. Zero signifies an omitted count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// Editing intent to execute or retain for dot.
    pub action: Action,
    /// Prefix count, with zero preserving the meaning of an omitted count.
    pub count: usize,
}

impl Command {
    /// Combine an editing intent with its explicit or omitted prefix count.
    pub fn new(action: Action, count: usize) -> Self {
        Self { action, count }
    }
}

/// One UTF-8 replacement against the pre-operation document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    /// Half-open UTF-8 byte range in the original document revision.
    pub range: TextRange,
    /// UTF-8 replacement text. An empty string removes the range.
    pub text: String,
}

/// Grouping instructions for host-owned undo. Begin/Continue/End encompass one
/// Insert/Replace session. Atomic is a whole command; an End can have no edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transaction {
    /// Treat this command as a complete undo group.
    Atomic,
    /// Open a group for the upcoming Insert or Replace session.
    Begin,
    /// Add edits to the currently open group.
    Continue,
    /// Close the current undo group while keeping an insertion-point cursor.
    BreakInsert,
    /// Close the current group when leaving Insert or Replace.
    End,
}

/// Work to execute through the host mutation or service pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationKind {
    /// Apply document changes and update the editing surface.
    Edit {
        /// Disjoint changes against the same original revision, applied from end to start.
        replacements: Vec<Replacement>,
        /// Requested UTF-8 byte offset in the resulting document.
        cursor: ByteOffset,
        /// Requested selection after the edit, or none to clear it.
        selection: Option<Selection>,
        /// How this step participates in host undo history.
        transaction: Transaction,
    },
    /// Update cursor and selection without replacing text.
    Move {
        /// Destination UTF-8 byte offset in the current document.
        cursor: ByteOffset,
        /// Visual endpoints to display, or none to clear the selection.
        selection: Option<Selection>,
    },
    /// Ask the host for a range before completing a command.
    Resolve {
        /// Service and target whose coordinates the host must resolve.
        request: TargetRequest,
        /// Number of target occurrences or repetitions requested.
        count: usize,
    },
    /// Traverse or restore host-owned history.
    History {
        /// Kind of undo or redo to perform.
        command: HistoryCommand,
        /// Number of history operations requested.
        count: usize,
    },
    /// Services such as formatting/indentation retain host-specific policy.
    Transform {
        /// Host transform to apply, such as indentation or formatting.
        operator: Operator,
        /// Resolved ranges and register shape of the target.
        range: ResolvedRange,
        /// Command count accompanying the transform.
        count: usize,
    },
    /// Execute an application-defined command through its normal dispatch path.
    Host {
        /// Stable identifier and repeat policy of the application command.
        command: HostCommand,
        /// Command count to pass to the host.
        count: usize,
    },
}

/// A single outstanding operation. Check `expected` immediately before any
/// mutation, including after async work. Return Rejected on a stale revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOperation {
    /// Token the host must echo in its acknowledgement.
    pub id: u64,
    /// Document identity and revision required immediately before execution.
    pub expected: DocumentVersion,
    /// Host work to perform if the document is current.
    pub kind: OperationKind,
}

impl HostOperation {
    /// Check the document identity and revision immediately before applying the operation.
    pub fn is_current(&self, version: DocumentVersion) -> bool {
        self.expected == version
    }
}

/// Report actual edits, including host filtering and range expansion. Ranges
/// are in the pre-operation document, sorted and disjoint. Removed text makes
/// register/repeat recovery independent of a host's storage implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedEdit {
    /// Actual half-open byte range replaced in the pre-operation document.
    pub range: TextRange,
    /// Original UTF-8 text removed by the host, including any range expansion.
    pub removed: String,
    /// UTF-8 text accepted by the host after filtering or normalization.
    pub inserted: String,
}

/// The outcome observed by the host when it executes an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostOutcome {
    /// Acknowledge an accepted edit, cursor move or history command.
    Applied {
        /// Actual changes in original-document order. A move can leave this empty.
        edits: Vec<AppliedEdit>,
        /// Whether the host changed content. A content change requires a newer revision.
        changed: bool,
    },
    /// Return ranges resolved against the requested document revision.
    Target(ResolvedRange),
    /// Remember a host-resolved search or syntax target for dot. The host
    /// retains the query and resolves it at the new cursor when requested.
    RememberedTarget {
        /// Target to apply in the current revision.
        range: ResolvedRange,
        /// Request to resolve afresh when dot repeats the command.
        replay: TargetRequest,
    },
    /// Decline the operation and cancel its pending replay.
    Rejected,
}

/// Acknowledgement of an operation against the supplied document revisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostResult {
    /// Token copied from the outstanding host operation.
    pub operation: u64,
    /// Document identity and revision against which the request was issued.
    pub before: DocumentVersion,
    /// Current document identity and revision after execution.
    pub after: DocumentVersion,
    /// Actual UTF-8 byte cursor offset in the resulting document.
    pub cursor: ByteOffset,
    /// Actual resulting Visual selection, if any.
    pub selection: Option<Selection>,
    /// Accepted changes, a resolved target or a rejection.
    pub outcome: HostOutcome,
}

/// Result of interpreting input or acknowledging host work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The input belongs to Vimlet and this step is complete.
    Consumed,
    /// A prefix was consumed. Dispatch the next key in the updated binding context.
    AwaitingInput,
    /// The host may route this input to its application shortcuts.
    Unhandled,
    /// Execute this request and acknowledge it before sending another key.
    HostOperation(HostOperation),
    /// Route this input through the application's ordinary dispatch before
    /// calling handle again. Obtain the next key with `next_macro_input`.
    Replay(Input),
    /// Interpretation or acknowledgement failed with the supplied reason.
    Error(EngineError),
}

/// A failed input or host protocol precondition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineError {
    /// A coordinate, opaque span or resolved target failed validation.
    InvalidDocument,
    /// Another input arrived while this surface was waiting for host execution.
    AwaitingHost,
    /// The acknowledgement token or outcome does not match the outstanding request.
    UnexpectedResult,
    /// The document or resulting cursor and selection disagree with the acknowledgement.
    StaleResult,
    /// The requested mode is disabled for this surface.
    UnsupportedMode,
    /// Playback exceeded its nesting depth or dispatched-key budget.
    PlaybackLimit,
    /// The current recording reached its configured key limit.
    RecordingLimit,
    /// The supplied name cannot designate a macro recording register.
    InvalidRegister,
}

/// Explicit lifetime transitions; registers and the last successful change stay
/// session-wide while surface pending state and playback are cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// Clear prefixes, outstanding continuations and playback while retaining the mode.
    CancelPending,
    /// End modal interaction and return the surface to Normal.
    LeaveMode,
    /// Discard document-local editing state before focusing another document.
    SwitchDocument,
    /// Clear transient commands when restoring host history.
    RestoreHistory,
    /// Invalidate old draft coordinates and start the replacement in Insert.
    ReplaceDraft,
    /// Begin a fresh draft in Insert while retaining session registers.
    NewDraft,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidDocument => "invalid document coordinates or ranges",
            Self::AwaitingHost => "an operation is waiting for the host",
            Self::UnexpectedResult => "host result does not match the outstanding operation",
            Self::StaleResult => "document changed before the operation completed",
            Self::UnsupportedMode => "editing surface does not support the requested mode",
            Self::PlaybackLimit => "macro playback exceeded its step or nesting limit",
            Self::RecordingLimit => "macro recording exceeded its key limit",
            Self::InvalidRegister => "invalid macro register name",
        })
    }
}

impl std::error::Error for EngineError {}
