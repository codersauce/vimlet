//! Keep counted and nested macro edits in one host undo transaction.
//!
//! Command interpretation and dot recipes retain their usual boundaries. Only the host's
//! transaction markers are joined, with an empty End operation after playback finishes.
use super::{
    DocumentView, EditingState, Effect, Input, Mode, OperationKind, Session, Step, Transaction,
};

#[derive(Debug, Clone)]
pub(super) enum MacroContinuation {
    Input(Input),
    Step(Step),
    Request { kind: OperationKind, effect: Effect },
}

impl Session {
    pub(super) fn finish_macro_step(
        &mut self,
        state: &mut EditingState,
        step: Step,
        view: DocumentView<'_>,
    ) -> Step {
        if state.macro_transaction
            && state.mode == Mode::Normal
            && step == Step::Consumed
            && state.awaiting.is_none()
            && view.selection.is_none()
        {
            // Joining an Atomic/End marker must not leave its insertion-point cursor in Normal.
            let cursor = super::ByteOffset(
                crate::MotionResolver::from_view(view).normal_cursor(view.cursor.0),
            );
            if cursor != view.cursor {
                return self.move_to(state, view, cursor, Mode::Normal, None);
            }
        }
        if state.macro_transaction
            && !self.playback.has_remaining_keys()
            && state.awaiting.is_none()
            && !matches!(state.mode, Mode::Insert | Mode::Replace)
            && !matches!(step, Step::Replay(_))
            && view.is_valid()
            && state.document == Some(view.version.id)
        {
            return self.finish_macro_transaction(state, view, MacroContinuation::Step(step));
        }
        step
    }

    pub(super) fn finish_macro_transaction(
        &mut self,
        state: &mut EditingState,
        view: DocumentView<'_>,
        continuation: MacroContinuation,
    ) -> Step {
        state.macro_transaction = false;
        self.request(
            state,
            view,
            OperationKind::Edit {
                replacements: Vec::new(),
                cursor: view.cursor,
                selection: view.selection,
                transaction: Transaction::End,
            },
            Effect::FinishMacro(Box::new(continuation)),
        )
    }
}
