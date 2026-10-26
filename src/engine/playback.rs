//! Repeat recipes and bounded macro dispatch.
use super::{
    Binding, DocumentVersion, DocumentView, EditingState, EngineError, Input, InputOrigin,
    MacroFrame, Session, Step,
};

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
        if self.frames.len() >= self.limits.macro_depth {
            self.cancel_playback();
            return Step::Error(EngineError::PlaybackLimit);
        }
        let keys = register.playback();
        if keys.is_empty() {
            return Step::Consumed;
        }
        if self.frames.is_empty() {
            self.macro_steps = 0;
            self.macro_document = Some(view.version.id);
        }
        self.last_macro = Some(name);
        self.frames.push(MacroFrame {
            keys,
            index: 0,
            repeats: count.max(1),
        });
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
        loop {
            let Some(frame) = self.frames.last_mut() else {
                self.macro_document = None;
                return Step::Consumed;
            };
            if frame.index == frame.keys.len() {
                if frame.repeats > 1 {
                    frame.repeats -= 1;
                    frame.index = 0;
                } else {
                    self.frames.pop();
                    continue;
                }
            }
            if self.macro_steps >= self.limits.macro_steps {
                self.cancel_playback();
                return Step::Error(EngineError::PlaybackLimit);
            }
            self.macro_steps += 1;
            let key = frame.keys[frame.index].clone();
            frame.index += 1;
            return Step::Replay(Input {
                key,
                origin: InputOrigin::Macro,
                binding: Binding::Default,
            });
        }
    }
}
