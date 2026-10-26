//! Interpret Vim input and mode changes against a host-owned document.
//!
//! The host checks the document revision, executes each operation and reports
//! the resulting edits, cursor and selection.
mod api;
mod edits;
mod execute;
mod insert;
mod parser;
mod playback;
mod put;
mod visual;

use crate::{
    ByteOffset, CharacterMotion, DocumentVersion, DocumentView, Register, Registers, ResolvedRange,
    Selection, Shape,
};
pub use api::{
    Action, AppliedEdit, Binding, BindingContext, Capabilities, Command, EditTarget, EngineError,
    HistoryCommand, HostCommand, HostOperation, HostOutcome, HostResult, Input, InputOrigin,
    InsertStyle, Key, Lifecycle, Mode, OperationKind, Operator, Replacement, Step, TargetRequest,
    Transaction,
};
pub use playback::MacroPlayback;
use std::collections::VecDeque;

const MAX_COUNT: usize = 100_000;

#[derive(Debug, Clone)]
enum Pending {
    Operator {
        op: Operator,
        count: usize,
    },
    Bracket {
        prefix: char,
        op: Operator,
        count: usize,
    },
    G {
        op: Option<Operator>,
        count: usize,
    },
    Character {
        op: Option<Operator>,
        kind: CharacterMotion,
        count: usize,
    },
    Object {
        op: Option<Operator>,
        scope: crate::TextObjectScope,
        count: usize,
    },
    Replace {
        count: usize,
    },
    Record,
    Play {
        count: usize,
    },
    Mark {
        op: Option<Operator>,
        linewise: bool,
        count: usize,
    },
}

#[derive(Debug, Clone)]
struct BlockInsertion {
    offset: ByteOffset,
    padding: usize,
}

#[derive(Debug, Clone)]
enum InsertEdit {
    Host(HostCommand),
    Text(String),
    Backspace,
    Delete,
}

#[derive(Debug, Clone)]
struct Recipe {
    command: Command,
    tail: Vec<InsertEdit>,
    visual: Option<VisualExtent>,
}

#[derive(Debug, Clone, Copy)]
struct VisualExtent {
    shape: Shape,
    lines: usize,
    columns: usize,
}

#[derive(Debug, Clone)]
struct ActiveChange {
    recipe: Recipe,
}

#[derive(Debug, Clone)]
struct ReplaceRecovery {
    start: usize,
    inserted: String,
    removed: String,
}

/// Register contents come from the host's actual removal. A final-line delete
/// also removes its preceding separator, which belongs outside the register.
#[derive(Debug, Clone, Copy)]
struct DeletedRegister {
    shape: Shape,
    numbered: bool,
    separator_bytes: usize,
    ends_on_empty_line: bool,
}

struct PlannedEdit {
    replacements: Vec<Replacement>,
    cursor: ByteOffset,
    mode: Mode,
    register: Option<DeletedRegister>,
}

#[derive(Debug, Clone)]
enum Effect {
    Command {
        command: Command,
        mode: Mode,
        register: Option<DeletedRegister>,
        visual: Option<VisualExtent>,
    },
    Move {
        mode: Mode,
    },
    Insert(InsertEdit),
    ReplaceBackspace,
    BreakInsert,
    EndInsert,
    Resolve(Command),
    History,
}

#[derive(Debug, Clone)]
struct Awaiting {
    operation: u64,
    expected: DocumentVersion,
    effect: Effect,
}

/// A macro binding interpreted for a host that dispatches its own recorded keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroBinding {
    /// A prefix was consumed and the next key names the register.
    AwaitingRegister,
    /// Start recording into this register. Uppercase appends.
    Record(char),
    /// Dispatch the named register the requested number of times.
    Play {
        /// The typed name, including `@` for the last played register.
        register: char,
        /// Repetition count, at least one.
        count: usize,
    },
    /// Escape or an invalid suffix ended the pending binding.
    Cancelled,
}

/// Modal state and pending commands for one editing surface.
///
/// Keep the same state until the host acknowledges any outstanding operation.
#[derive(Debug)]
pub struct EditingState {
    mode: Mode,
    capabilities: Capabilities,
    pending: Option<Pending>,
    count: Option<usize>,
    awaiting: Option<Awaiting>,
    last_character: Option<(CharacterMotion, char)>,
    preferred_column: Option<usize>,
    previous_visual: Option<Selection>,
    active: Option<ActiveChange>,
    replay_tail: Option<VecDeque<InsertEdit>>,
    replace_recovery: Vec<ReplaceRecovery>,
    block_insert: Vec<BlockInsertion>,
    block_origin: Option<ByteOffset>,
    document: Option<u64>,
}

impl Default for EditingState {
    fn default() -> Self {
        Self::new(Mode::Normal, Capabilities::default())
    }
}

impl EditingState {
    /// Create a surface in the requested mode, falling back to Normal if it lacks that capability.
    pub fn new(mode: Mode, capabilities: Capabilities) -> Self {
        let mode = if (mode == Mode::Replace && !capabilities.replace)
            || (mode.is_visual() && !capabilities.visual)
            || (mode == Mode::VisualBlock && !capabilities.visual_block)
        {
            Mode::Normal
        } else {
            mode
        };
        Self {
            mode,
            capabilities,
            pending: None,
            count: None,
            awaiting: None,
            last_character: None,
            preferred_column: None,
            previous_visual: None,
            active: None,
            replay_tail: None,
            replace_recovery: Vec::new(),
            block_insert: Vec::new(),
            block_origin: None,
            document: None,
        }
    }

    /// Return the current mode, including transitions acknowledged by the host.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Report whether a prefix, count or host acknowledgement is outstanding.
    pub fn has_pending_input(&self) -> bool {
        self.pending.is_some() || self.count.is_some() || self.awaiting.is_some()
    }

    /// Supply a count already consumed by a host keymap. Zero clears the count.
    pub fn set_count(&mut self, count: usize) {
        self.count = (count > 0).then_some(count.min(MAX_COUNT));
    }

    /// Start a mapped operator and wait for its motion or text object.
    ///
    /// Zero denotes an omitted count. Further count digits multiply a supplied count.
    /// Call on an idle Normal surface.
    pub fn begin_operator(&mut self, operator: Operator, count: usize) {
        self.count = None;
        self.pending = Some(Pending::Operator {
            op: operator,
            count: count.min(MAX_COUNT),
        });
    }

    /// Report whether the next key names a macro recording or playback register.
    pub fn awaiting_macro_register(&self) -> bool {
        matches!(self.pending, Some(Pending::Record | Pending::Play { .. }))
    }

    /// Interpret a Normal-mode macro key for an external playback dispatcher.
    ///
    /// Supply any mapped count with the prefix. Unrelated input leaves the surface untouched.
    pub fn macro_binding(&mut self, key: &Key, count: usize) -> Option<MacroBinding> {
        if !self.awaiting_macro_register() {
            if self.mode != Mode::Normal
                || self.has_pending_input()
                || !matches!(key, Key::Char('q' | '@'))
            {
                return None;
            }
            self.set_count(count);
        }
        Some(match parser::parse(self, key) {
            parser::Parsed::Pending => MacroBinding::AwaitingRegister,
            parser::Parsed::Record(name) => MacroBinding::Record(name),
            parser::Parsed::Play(register, count) => MacroBinding::Play { register, count },
            _ => MacroBinding::Cancelled,
        })
    }

    /// Report whether an operator or its motion has a count that changes a percentage target.
    pub fn has_motion_count(&self) -> bool {
        self.count.is_some() || matches!(self.pending, Some(Pending::Operator { count: 1.., .. }))
    }

    /// Return the prefix for a host-defined motion or text-object suffix.
    pub fn target_prefix(&self) -> Option<char> {
        match self.pending {
            Some(Pending::Object {
                scope: crate::TextObjectScope::Inner,
                ..
            }) => Some('i'),
            Some(Pending::Object { .. }) => Some('a'),
            Some(Pending::Bracket { prefix, .. }) => Some(prefix),
            Some(Pending::G { .. }) => Some('g'),
            _ => None,
        }
    }

    /// Return the semantic command behind the outstanding host edit, including dot playback.
    ///
    /// Hosts can use the action to apply language-specific indentation when opening or pasting lines.
    pub fn pending_edit_command(&self) -> Option<&Command> {
        match &self.awaiting.as_ref()?.effect {
            Effect::Command { command, .. } => Some(command),
            _ => None,
        }
    }

    /// Return the last character search for host commands that share `;` and `,` history.
    pub fn last_character_motion(&self) -> Option<(CharacterMotion, char)> {
        self.last_character
    }

    /// Synchronize a host-initiated mode change on the same document.
    ///
    /// Discard transient input and keep the document's character search and Visual selection.
    /// Finish any pending edit or undo group before switching modes.
    pub fn sync_mode(&mut self, mode: Mode) {
        if self.mode != mode {
            *self = Self {
                document: self.document,
                last_character: self.last_character,
                previous_visual: self.previous_visual,
                ..Self::new(mode, self.capabilities)
            };
        }
    }

    /// Return the character motion awaiting a literal target, if any.
    pub fn pending_character_motion(&self) -> Option<CharacterMotion> {
        match self.pending {
            Some(Pending::Character { kind, .. }) => Some(kind),
            _ => None,
        }
    }

    /// Tell the host whether Escape belongs to this editing surface.
    pub fn owns_escape(&self) -> bool {
        self.mode != Mode::Normal || self.has_pending_input()
    }

    /// Select the keymap context for the next key, including literal suffixes.
    pub fn binding_context(&self) -> BindingContext {
        if self.awaiting.is_some() {
            return BindingContext::AwaitingHost;
        }
        match self.pending {
            Some(
                Pending::Character { .. }
                | Pending::Replace { .. }
                | Pending::Record
                | Pending::Play { .. }
                | Pending::Object { .. }
                | Pending::Mark { .. },
            ) => BindingContext::Literal,
            Some(_) => BindingContext::OperatorPending,
            None => match self.mode {
                Mode::Normal => BindingContext::Normal,
                Mode::Insert => BindingContext::Insert,
                Mode::Replace => BindingContext::Replace,
                _ => BindingContext::Visual,
            },
        }
    }
}

/// Bounds for recording and dispatching macros in a session.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum number of simultaneously nested macro frames.
    pub macro_depth: usize,
    /// Maximum replayed keys across a single playback and its nested calls.
    pub macro_steps: usize,
    /// Maximum original keys retained in one recording.
    pub recorded_keys: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            macro_depth: 100,
            macro_steps: 100_000,
            recorded_keys: 10_000,
        }
    }
}

/// Repeat history, macro playback and registers shared across editing surfaces.
#[derive(Debug, Default)]
pub struct Session {
    registers: Registers,
    last_change: Option<Recipe>,
    recording: Option<(char, Vec<Key>)>,
    last_macro: Option<char>,
    playback: MacroPlayback<Key>,
    macro_document: Option<u64>,
    next_operation: u64,
}

impl Session {
    /// Start a session with an empty register bank and the default macro limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start an empty session with caller-selected recording and playback limits.
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            playback: MacroPlayback::with_limits(limits),
            ..Self::default()
        }
    }

    /// Borrow the register bank shared by editing surfaces using this session.
    pub fn registers(&self) -> &Registers {
        &self.registers
    }

    /// Access the register bank to synchronize host register or clipboard contents.
    pub fn registers_mut(&mut self) -> &mut Registers {
        &mut self.registers
    }

    /// Return the destination register while a macro is being recorded.
    pub fn recording(&self) -> Option<char> {
        self.recording.as_ref().map(|r| r.0)
    }

    /// Start recording original keys into an alphanumeric register. Uppercase appends on finish.
    ///
    /// # Errors
    /// Returns [`EngineError::InvalidRegister`] when the register cannot hold a macro.
    pub fn start_recording(&mut self, name: char) -> Result<(), EngineError> {
        if !name.is_ascii_alphanumeric() {
            return Err(EngineError::InvalidRegister);
        }
        self.recording = Some((name, Vec::new()));
        Ok(())
    }

    /// Record one original key dispatched outside the interpreter.
    ///
    /// Use [`InputOrigin::Injected`] if subsequently feeding the same key to [`Self::handle`].
    ///
    /// # Errors
    /// Exceeding the recording limit discards the unfinished recording.
    pub fn record_key(&mut self, key: &Key) -> Result<(), EngineError> {
        if let Some((_, keys)) = &mut self.recording {
            if keys.len() >= self.playback.limits.recorded_keys {
                self.recording = None;
                return Err(EngineError::RecordingLimit);
            }
            keys.push(key.clone());
        }
        Ok(())
    }

    /// Commit the recording to the shared register bank and return its lowercase name.
    pub fn finish_recording(&mut self) -> Option<char> {
        let (name, keys) = self.recording.take()?;
        self.registers.set(name, Register::from_keys(keys));
        Some(name.to_ascii_lowercase())
    }

    /// Report whether playback frames remain on the dispatch stack.
    pub fn is_playing(&self) -> bool {
        self.playback.is_playing()
    }

    /// Remember a host command after its independently dispatched edit commits.
    ///
    /// Dot emits an [`OperationKind::Host`] request with this identifier and the repeat count.
    pub fn record_host_change(&mut self, id: u64) {
        self.last_change = Some(Recipe {
            command: Command::new(
                Action::Host(HostCommand {
                    id,
                    repeatable: true,
                }),
                1,
            ),
            tail: Vec::new(),
            visual: None,
        });
    }

    /// Forget the dot recipe after a host edit that cannot be repeated at another cursor.
    pub fn clear_last_change(&mut self) {
        self.last_change = None;
    }

    /// Report whether dot has a completed change to repeat.
    pub fn has_last_change(&self) -> bool {
        self.last_change.is_some()
    }

    /// Discard queued macro input, including remaining nested frames.
    pub fn cancel_playback(&mut self) {
        self.playback.cancel();
        self.macro_document = None;
    }

    fn abort_input(&mut self, state: &mut EditingState) {
        state.replay_tail = None;
        self.cancel_playback();
    }

    /// Reset a surface intentionally. Finish an active host undo transaction
    /// through Escape before switching documents; this call invalidates queued
    /// continuations and never mutates host history on its own.
    pub fn lifecycle(&mut self, state: &mut EditingState, event: Lifecycle) {
        state.pending = None;
        state.count = None;
        state.awaiting = None;
        self.abort_input(state);
        if event == Lifecycle::CancelPending {
            return;
        }
        state.active = None;
        state.replace_recovery.clear();
        state.block_insert.clear();
        state.block_origin = None;
        state.preferred_column = None;
        state.mode = if matches!(event, Lifecycle::NewDraft | Lifecycle::ReplaceDraft) {
            Mode::Insert
        } else {
            Mode::Normal
        };
        if matches!(
            event,
            Lifecycle::SwitchDocument | Lifecycle::NewDraft | Lifecycle::ReplaceDraft
        ) {
            state.document = None;
            state.previous_visual = None;
            state.last_character = None;
        }
    }

    /// Interpret one input against the host's current document view.
    /// Pass any returned host operation to the host and acknowledge its result with [`Self::resume`].
    pub fn handle(
        &mut self,
        state: &mut EditingState,
        input: Input,
        view: DocumentView<'_>,
    ) -> Step {
        if !view.is_valid() {
            return Step::Error(EngineError::InvalidDocument);
        }
        if state.awaiting.is_some() {
            return Step::Error(EngineError::AwaitingHost);
        }
        if state.document.is_some_and(|id| id != view.version.id) {
            self.lifecycle(state, Lifecycle::SwitchDocument);
        }
        state.document = Some(view.version.id);
        if input.key == Key::Char('q')
            && state.mode == Mode::Normal
            && !state.has_pending_input()
            && input.origin == InputOrigin::Typed
            && self.finish_recording().is_some()
        {
            return Step::Consumed;
        }
        if input.origin == InputOrigin::Typed {
            if let Err(error) = self.record_key(&input.key) {
                return Step::Error(error);
            }
        }
        if input.key == Key::Escape {
            if input.origin == InputOrigin::Typed {
                self.abort_input(state);
            }
            if state.pending.is_some() {
                state.pending = None;
                state.count = None;
                return Step::Consumed;
            }
        }
        if matches!(state.mode, Mode::Insert | Mode::Replace) {
            if let Binding::Host(command) = input.binding {
                if command.repeatable {
                    return self.insert_action(state, InsertEdit::Host(command), view);
                }
                return self.execute(state, Command::new(Action::Host(command), 1), view);
            }
            if let Binding::Command(command) = input.binding {
                return self.execute(state, command, view);
            }
            if input.binding == Binding::Unhandled {
                return Step::Unhandled;
            }
            return self.insert_key(state, input.key, view);
        }
        if let Binding::Command(Command {
            action: Action::Target(request),
            ..
        }) = input.binding
        {
            let scoped = match state.pending {
                Some(Pending::Object { op, count, .. } | Pending::G { op, count }) => {
                    Some((op, count))
                }
                Some(Pending::Bracket { op, count, .. }) => Some((Some(op), count)),
                _ => None,
            };
            if let Some((operator, count)) = scoped {
                let object = matches!(state.pending, Some(Pending::Object { .. }));
                state.pending = None;
                let action = match operator {
                    Some(operator) => Action::Operate {
                        operator,
                        target: EditTarget::Host(request),
                    },
                    None if object => Action::SelectTarget(request),
                    None => Action::Target(request),
                };
                return self.execute(state, Command::new(action, count), view);
            }
        }
        // Motion mappings complete an operator. Literal suffixes (e.g. the
        // character after f) are deliberately interpreted as the original key.
        if let (Some(Pending::Operator { op, count }), Binding::Command(mapped)) =
            (&state.pending, &input.binding)
        {
            let target = match mapped.action {
                Action::Move(motion) => Some(EditTarget::Motion(motion)),
                Action::Target(request) => Some(EditTarget::Host(request)),
                Action::SelectObject { scope, kind } => Some(EditTarget::Object(scope, kind)),
                _ => None,
            };
            if let Some(target) = target {
                let operator = *op;
                let count = count
                    .max(&1)
                    .saturating_mul(state.count.take().unwrap_or(1))
                    .saturating_mul(mapped.count.max(1));
                state.pending = None;
                return self.execute(
                    state,
                    Command::new(Action::Operate { operator, target }, count),
                    view,
                );
            }
        }
        if state.pending.is_none() {
            match input.binding {
                Binding::Command(mut command) => {
                    if let Some(count) = state.count.take() {
                        command.count = count.saturating_mul(command.count.max(1));
                    }
                    return self.execute(state, command, view);
                }
                Binding::Host(command) => {
                    let count = state.count.take().unwrap_or(1);
                    return self.execute(state, Command::new(Action::Host(command), count), view);
                }
                Binding::Unhandled => return Step::Unhandled,
                Binding::Default => {}
            }
        }
        match parser::parse(state, &input.key) {
            parser::Parsed::Command(command) => self.execute(state, command, view),
            parser::Parsed::Pending => Step::AwaitingInput,
            parser::Parsed::Consumed => {
                self.abort_input(state);
                Step::Consumed
            }
            parser::Parsed::Unhandled => Step::Unhandled,
            parser::Parsed::Record(name) => {
                if !name.is_ascii_alphanumeric() {
                    return Step::Error(EngineError::InvalidRegister);
                }
                if input.origin == InputOrigin::Macro {
                    return Step::Consumed;
                }
                self.start_recording(name)
                    .map_or_else(Step::Error, |()| Step::Consumed)
            }
            parser::Parsed::Play(name, count) => self.play(state, name, count, view),
            parser::Parsed::Escape => {
                state.count = None;
                if input.origin == InputOrigin::Typed {
                    self.abort_input(state);
                }
                if state.mode.is_visual() {
                    state.previous_visual = view.selection;
                    return self.move_to(state, view, view.cursor, Mode::Normal, None);
                }
                Step::Consumed
            }
        }
    }

    fn request(
        &mut self,
        state: &mut EditingState,
        view: DocumentView<'_>,
        kind: OperationKind,
        effect: Effect,
    ) -> Step {
        self.next_operation = self.next_operation.wrapping_add(1);
        let operation = HostOperation {
            id: self.next_operation,
            expected: view.version,
            kind,
        };
        state.awaiting = Some(Awaiting {
            operation: operation.id,
            expected: operation.expected,
            effect,
        });
        Step::HostOperation(operation)
    }

    /// Acknowledge an operation after host-side revision checking and execution.
    /// Wrong operation IDs do not consume the outstanding operation.
    pub fn resume(
        &mut self,
        state: &mut EditingState,
        result: HostResult,
        view: DocumentView<'_>,
    ) -> Step {
        let Some(awaiting) = state.awaiting.as_ref() else {
            return Step::Error(EngineError::UnexpectedResult);
        };
        if result.operation != awaiting.operation {
            return Step::Error(EngineError::UnexpectedResult);
        }
        let Some(awaiting) = state.awaiting.take() else {
            return Step::Error(EngineError::UnexpectedResult);
        };
        if result.before != awaiting.expected
            || result.after != view.version
            || result.cursor != view.cursor
            || result.selection != view.selection
            || !view.is_valid()
            || result.after.id != result.before.id
        {
            self.abort_input(state);
            state.active = None;
            return Step::Error(EngineError::StaleResult);
        }
        if result.outcome == HostOutcome::Rejected {
            self.abort_input(state);
            return Step::Consumed;
        }
        if let Effect::Resolve(command) = awaiting.effect {
            let (command, range) = match result.outcome {
                HostOutcome::Target(range) => (command, range),
                HostOutcome::RememberedTarget { range, replay } => {
                    let action = match command.action {
                        Action::Operate { operator, .. } => Action::Operate {
                            operator,
                            target: EditTarget::Host(replay),
                        },
                        Action::Target(_) => Action::Target(replay),
                        Action::SelectTarget(_) => Action::SelectTarget(replay),
                        _ => return Step::Error(EngineError::UnexpectedResult),
                    };
                    (Command::new(action, command.count), range)
                }
                _ => return Step::Error(EngineError::UnexpectedResult),
            };
            if result.after != result.before {
                self.abort_input(state);
                return Step::Error(EngineError::StaleResult);
            }
            return self.resolved_command(state, command, range, view);
        }
        let HostOutcome::Applied { edits, changed } = result.outcome else {
            return Step::Error(EngineError::UnexpectedResult);
        };
        if changed && result.after.revision <= result.before.revision {
            self.abort_input(state);
            return Step::Error(EngineError::StaleResult);
        }
        match awaiting.effect {
            Effect::Move { mode } => state.mode = mode,
            Effect::BreakInsert => {
                if let Some(active) = state.active.take() {
                    self.last_change = Some(active.recipe);
                }
                state.replace_recovery.clear();
            }
            Effect::History => {
                state.replace_recovery.clear();
                state.active = None;
            }
            Effect::Command {
                command,
                mode,
                register,
                visual,
            } => {
                if let Some(DeletedRegister {
                    shape,
                    numbered,
                    separator_bytes: prefix,
                    ends_on_empty_line,
                }) = register
                {
                    let removed_rows = match (shape, visual) {
                        (Shape::Block { .. }, Some(extent)) => extent.lines,
                        _ => edits.len(),
                    };
                    let mut text = edits
                        .iter()
                        .take(removed_rows)
                        .map(|e| e.removed.as_str())
                        .collect::<Vec<_>>()
                        .join(if matches!(shape, Shape::Block { .. }) {
                            "\n"
                        } else {
                            ""
                        });
                    if prefix > 0 && text.is_char_boundary(prefix) {
                        text.drain(..prefix);
                    }
                    if ends_on_empty_line {
                        text.push('\n');
                    }
                    if !text.is_empty() || shape == Shape::Line && !edits.is_empty() {
                        self.registers
                            .delete(normalized_register(text, shape), numbered);
                    }
                }
                state.mode = mode;
                let recipe = Recipe {
                    command,
                    tail: Vec::new(),
                    visual,
                };
                if matches!(mode, Mode::Insert | Mode::Replace) {
                    state.active = Some(ActiveChange { recipe });
                    state.replace_recovery.clear();
                } else if (changed || !edits.is_empty()) && state.replay_tail.is_none() {
                    self.last_change = Some(recipe);
                }
            }
            Effect::Insert(edit) => {
                let host_edit = matches!(edit, InsertEdit::Host(_));
                if let Some(active) = &mut state.active {
                    if !edits.is_empty() {
                        let edit = match edit {
                            InsertEdit::Text(_) => InsertEdit::Text(
                                edits.iter().map(|e| e.inserted.as_str()).collect(),
                            ),
                            other => other,
                        };
                        active.recipe.tail.push(edit);
                    }
                }
                if host_edit {
                    state.replace_recovery.clear();
                } else if state.mode == Mode::Replace {
                    state.remember_overwrites(&edits);
                }
                transform_block_positions(&mut state.block_insert, &edits);
            }
            Effect::ReplaceBackspace => {
                state.replace_recovery.pop();
                if let Some(active) = &mut state.active {
                    if changed {
                        active.recipe.tail.push(InsertEdit::Backspace);
                    }
                }
                transform_block_positions(&mut state.block_insert, &edits);
            }
            Effect::EndInsert => {
                state.mode = Mode::Normal;
                state.replace_recovery.clear();
                state.block_insert.clear();
                state.block_origin = None;
                if let Some(active) = state.active.take() {
                    if state.replay_tail.is_none() {
                        // In Neovim even an empty i<Esc> replaces the previous dot command.
                        self.last_change = Some(active.recipe);
                    }
                }
                state.replay_tail = None;
            }
            Effect::Resolve(_) => unreachable!(),
        }
        if state.replay_tail.is_some() && matches!(state.mode, Mode::Insert | Mode::Replace) {
            if let Some(edit) = state.replay_tail.as_mut().and_then(VecDeque::pop_front) {
                return self.insert_action(state, edit, view);
            }
            return self.finish_insert(state, view);
        }
        state.replay_tail = None;
        Step::Consumed
    }
}

fn normalized_register(mut text: String, shape: Shape) -> Register {
    if shape == Shape::Line && !text.ends_with('\n') {
        text.push('\n');
    }
    Register::from_text(text, shape)
}

fn transform_block_positions(positions: &mut [BlockInsertion], edits: &[AppliedEdit]) {
    for p in positions {
        let mut delta = 0isize;
        for e in edits {
            if e.range.end <= p.offset {
                delta += e.inserted.len() as isize - (e.range.end.0 - e.range.start.0) as isize;
            }
        }
        p.offset.0 = p.offset.0.saturating_add_signed(delta);
    }
}
