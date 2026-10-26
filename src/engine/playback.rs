//! Repeat recipes and bounded macro dispatch.
use super::{
    Binding, DocumentVersion, DocumentView, EditingState, EngineError, Input, InputOrigin, Limits,
    Session, Step,
};

#[derive(Debug)]
struct MacroFrame<K> {
    keys: Vec<K>,
    index: usize,
    repeats: usize,
}

/// Bounded macro playback for hosts that dispatch their own key type.
///
/// Push a frame for each register invocation, then dispatch one key at a time.
/// Keep the stack while waiting for a host action, and discard it if the target
/// document changed during the wait. Counts and nested frames share one key budget.
#[derive(Debug)]
pub struct MacroPlayback<K> {
    frames: Vec<MacroFrame<K>>,
    steps: usize,
    pub(super) limits: Limits,
}

impl<K> Default for MacroPlayback<K> {
    fn default() -> Self {
        Self::with_limits(Limits::default())
    }
}

impl<K> MacroPlayback<K> {
    /// Start an empty stack with the given depth and step limits.
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            frames: Vec::new(),
            steps: 0,
            limits,
        }
    }

    /// Return whether an invocation is active, including during dispatch of its final key.
    pub fn is_playing(&self) -> bool {
        !self.frames.is_empty()
    }

    /// Discard every frame and its remaining repetitions.
    pub fn cancel(&mut self) {
        self.frames.clear();
        self.steps = 0;
    }

    /// Schedule keys ahead of the caller's remaining input. A zero count plays once.
    ///
    /// # Errors
    /// Exceeding the nesting limit cancels all frames and returns [`EngineError::PlaybackLimit`].
    pub fn push(&mut self, keys: Vec<K>, count: usize) -> Result<(), EngineError> {
        if self.frames.len() >= self.limits.macro_depth {
            self.cancel();
            return Err(EngineError::PlaybackLimit);
        }
        if !keys.is_empty() {
            self.frames.push(MacroFrame {
                keys,
                index: 0,
                repeats: count.max(1),
            });
        }
        Ok(())
    }
}

impl<K: Clone> MacroPlayback<K> {
    /// Take one key for host dispatch. Poll again after its action has completed.
    ///
    /// # Errors
    /// Exceeding the step limit cancels all frames and returns [`EngineError::PlaybackLimit`].
    pub fn next_key(&mut self) -> Result<Option<K>, EngineError> {
        loop {
            let Some(frame) = self.frames.last_mut() else {
                self.steps = 0;
                return Ok(None);
            };
            if frame.index == frame.keys.len() {
                if frame.repeats == 1 {
                    self.frames.pop();
                    continue;
                }
                frame.repeats -= 1;
                frame.index = 0;
            }
            if self.steps >= self.limits.macro_steps {
                self.cancel();
                return Err(EngineError::PlaybackLimit);
            }
            self.steps += 1;
            let key = frame.keys[frame.index].clone();
            frame.index += 1;
            return Ok(Some(key));
        }
    }
}

impl Session {
    pub(super) fn repeat(
        &mut self,
        state: &mut EditingState,
        count: usize,
        view: DocumentView<'_>,
    ) -> Step {
        let Some(mut recipe) = self.last_change.clone() else {
            return Step::Consumed;
        };
        if count > 0 {
            recipe.command.count = count;
        }
        state.replay_tail = Some(recipe.tail.into());
        if let Some(extent) = recipe.visual {
            return self.repeat_visual(state, recipe.command, extent, view);
        }
        let step = self.execute(state, recipe.command, view);
        if !matches!(step, Step::HostOperation(_)) {
            self.abort_input(state);
        }
        step
    }

    pub(super) fn play(
        &mut self,
        state: &EditingState,
        name: char,
        count: usize,
        view: DocumentView<'_>,
    ) -> Step {
        let name = if name == '@' {
            let Some(name) = self.last_macro else {
                return Step::Consumed;
            };
            name
        } else {
            name
        };
        let Some(register) = self.registers.get(name) else {
            self.cancel_playback();
            return Step::Consumed;
        };
        let keys = register.playback();
        if keys.is_empty() {
            return Step::Consumed;
        }
        if !self.playback.is_playing() {
            self.macro_document = Some(view.version.id);
        }
        if let Err(error) = self.playback.push(keys, count) {
            self.macro_document = None;
            return Step::Error(error);
        }
        self.last_macro = Some(name);
        self.next_macro_input(state, view.version)
    }

    /// Take the next recorded key for host dispatch after the previous key completes.
    pub fn next_macro_input(&mut self, state: &EditingState, version: DocumentVersion) -> Step {
        if state.awaiting.is_some() {
            return Step::Error(EngineError::AwaitingHost);
        }
        if self.macro_document.is_some_and(|id| id != version.id) {
            self.cancel_playback();
            return Step::Consumed;
        }
        match self.playback.next_key() {
            Ok(Some(key)) => Step::Replay(Input {
                key,
                origin: InputOrigin::Macro,
                binding: Binding::Default,
            }),
            Ok(None) => {
                self.macro_document = None;
                Step::Consumed
            }
            Err(error) => {
                self.macro_document = None;
                Step::Error(error)
            }
        }
    }
}
