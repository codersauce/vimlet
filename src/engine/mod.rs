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
use std::collections::VecDeque;

const MAX_COUNT: usize = 100_000;

#[derive(Debug, Clone)]
enum Pending {
    Operator {
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
enum InsertEdit {
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
    block_insert: Vec<ByteOffset>,
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

#[derive(Debug)]
struct MacroFrame {
    keys: Vec<Key>,
    index: usize,
    repeats: usize,
}

/// Repeat history, macro playback and registers shared across editing surfaces.
#[derive(Debug, Default)]
pub struct Session {
    registers: Registers,
    last_change: Option<Recipe>,
    recording: Option<(char, Vec<Key>)>,
    last_macro: Option<char>,
    frames: Vec<MacroFrame>,
    macro_document: Option<u64>,
    macro_steps: usize,

    next_operation: u64,
    limits: Limits,
}

impl Session {
    /// Start a session with an empty register bank and the default macro limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start an empty session with caller-selected recording and playback limits.
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            limits,
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

    /// Report whether playback frames remain on the dispatch stack.
    pub fn is_playing(&self) -> bool {
        !self.frames.is_empty()
    }

    /// Report whether dot has a completed change to repeat.
    pub fn has_last_change(&self) -> bool {
        self.last_change.is_some()
    }

    /// Discard queued macro input, including remaining nested frames.
    pub fn cancel_playback(&mut self) {
        self.frames.clear();
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
        {
            if let Some((name, keys)) = self.recording.take() {
                self.registers.set(name, Register::from_keys(keys));
                return Step::Consumed;
            }
        }
        if input.origin == InputOrigin::Typed {
            if let Some((_, keys)) = &mut self.recording {
                if keys.len() >= self.limits.recorded_keys {
                    self.recording = None;
                    return Step::Error(EngineError::RecordingLimit);
                }
                keys.push(input.key.clone());
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
            parser::Parsed::Repeat(count) => self.repeat(state, count, view),
            parser::Parsed::Record(name) => {
                if !name.is_ascii_alphanumeric() {
                    return Step::Error(EngineError::InvalidRegister);
                }
                if input.origin == InputOrigin::Macro {
                    return Step::Consumed;
                }
                self.recording = Some((name, Vec::new()));
                Step::Consumed
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
                }) = register
                {
                    let mut text = edits
                        .iter()
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
                    if !text.is_empty() {
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
                if state.mode == Mode::Replace {
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

fn transform_block_positions(positions: &mut [ByteOffset], edits: &[AppliedEdit]) {
    for p in positions {
        let mut delta = 0isize;
        for e in edits {
            if e.range.end <= *p {
                delta += e.inserted.len() as isize - (e.range.end.0 - e.range.start.0) as isize;
            }
        }
        p.0 = p.0.saturating_add_signed(delta);
    }
}
