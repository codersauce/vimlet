//! Edit a one-line prompt with Neovim's Command-line bindings.

use crate::{ByteOffset, Key, TextSource};
use unicode_segmentation::UnicodeSegmentation;

/// The purpose of a command-line draft.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PromptKind {
    /// An Ex command opened with `:`.
    #[default]
    Ex,
    /// A forward search opened with `/`.
    SearchForward,
    /// A backward search opened with `?`.
    SearchBackward,
    /// A general input field. Backspace on an empty draft keeps it open.
    Field,
}

/// Direction of a history or completion request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptDirection {
    /// Older history or the previous completion.
    Previous,
    /// Newer history or the next completion.
    Next,
}

/// The result of dispatching a key to a prompt.
#[derive(Debug, PartialEq, Eq)]
pub enum PromptOutcome {
    /// The text changed. Hosts can refresh dependent previews.
    Edited,
    /// Only the insertion point changed.
    Moved,
    /// The key was handled without changing the draft.
    Consumed,
    /// The host should navigate its history using the current draft and caret.
    History(PromptDirection),
    /// The host should navigate its completion candidates.
    Complete(PromptDirection),
    /// The user accepted this exact draft, which may be empty.
    Accepted(String),
    /// The user cancelled the prompt.
    Cancelled,
    /// The host may handle the key.
    Unhandled,
}

/// A draft or insertion point violates the single-line editor's invariants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidPromptDraft;

impl std::fmt::Display for InvalidPromptDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("expected one line and a caret on a UTF-8 grapheme boundary")
    }
}

impl std::error::Error for InvalidPromptDraft {}

/// An owned, single-line draft with a UTF-8 byte caret.
///
/// The host renders the prompt prefix separately and handles history, completion,
/// acceptance and cancellation. Use a new instance when opening another prompt.
#[derive(Clone, Default)]
pub struct CommandLine {
    kind: PromptKind,
    text: String,
    caret: usize,
    closed: bool,
    skip_operand: bool,
}

impl std::fmt::Debug for CommandLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandLine")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl CommandLine {
    /// Create an empty draft for the given prompt.
    pub fn empty(kind: PromptKind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }

    /// Open a draft with the caret at the end.
    ///
    /// # Errors
    ///
    /// Returns an error if the text contains a line break.
    pub fn new(kind: PromptKind, text: impl Into<String>) -> Result<Self, InvalidPromptDraft> {
        let mut line = Self::empty(kind);
        let text = text.into();
        let end = ByteOffset(text.len());
        line.replace(text, end)?;
        Ok(line)
    }

    /// Return the editable text, excluding any prefix or label.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Return the zero-based UTF-8 byte offset of the insertion point.
    pub fn caret(&self) -> ByteOffset {
        ByteOffset(self.caret)
    }

    /// Replace the active draft, for example after recalling history.
    ///
    /// For asynchronous replies the caller should check that the same prompt,
    /// draft and caret are still active before calling this method.
    ///
    /// # Errors
    ///
    /// Rejects closed prompts, line breaks and caret positions inside graphemes.
    pub fn replace(&mut self, text: String, caret: ByteOffset) -> Result<(), InvalidPromptDraft> {
        if self.closed
            || text.contains(['\r', '\n'])
            || !(caret.0 == text.len()
                || text.grapheme_indices(true).any(|(byte, _)| byte == caret.0))
        {
            return Err(InvalidPromptDraft);
        }
        self.text = text;
        self.caret = caret.0;
        self.skip_operand = false;
        Ok(())
    }

    /// Dispatch one normalized key. A text batch inserts at most its first line.
    ///
    /// Accept and cancel close the prompt. Further keys are unhandled.
    pub fn handle(&mut self, key: Key) -> PromptOutcome {
        use PromptOutcome::{Accepted, Cancelled, Complete, Consumed, History, Unhandled};

        if self.closed {
            return Unhandled;
        }
        if std::mem::take(&mut self.skip_operand) {
            return Consumed;
        }
        let previous = self.text.as_str().prev_grapheme(self.caret);
        let next = self.text.as_str().next_grapheme(self.caret);
        match key {
            Key::Char(ch) if !ch.is_control() => {
                self.edit(self.caret, self.caret, ch.encode_utf8(&mut [0; 4]))
            }
            Key::Text(text) => {
                let first = text.split(['\r', '\n']).next().unwrap_or_default();
                self.edit(self.caret, self.caret, first)
            }
            Key::Left => self.move_to(previous),
            Key::Right => self.move_to(next),
            Key::Home | Key::Ctrl('b') => self.move_to(0),
            Key::End | Key::Ctrl('e') => self.move_to(self.text.len()),
            Key::Backspace | Key::Ctrl('h')
                if self.text.is_empty() && self.kind != PromptKind::Field =>
            {
                self.closed = true;
                Cancelled
            }
            Key::Backspace | Key::Ctrl('h') => self.edit(previous, self.caret, ""),
            Key::Delete if self.caret == self.text.len() => self.edit(previous, self.caret, ""),
            Key::Delete => self.edit(self.caret, next, ""),
            Key::Ctrl('u') => self.edit(0, self.caret, ""),
            Key::Ctrl('w') => self.edit(self.previous_word(), self.caret, ""),
            Key::Ctrl('v' | 'q' | 'r') => {
                self.skip_operand = true;
                Consumed
            }
            Key::Enter | Key::Ctrl('j' | 'm') => {
                self.closed = true;
                Accepted(self.text.clone())
            }
            Key::Escape | Key::Ctrl('c' | '[') => {
                self.closed = true;
                self.caret = 0;
                self.text.clear();
                Cancelled
            }
            Key::Up => History(PromptDirection::Previous),
            Key::Down => History(PromptDirection::Next),
            Key::Tab => Complete(PromptDirection::Next),
            Key::Named(name) if name == "BackTab" => Complete(PromptDirection::Previous),
            _ => Unhandled,
        }
    }

    fn move_to(&mut self, caret: usize) -> PromptOutcome {
        if self.caret == caret {
            return PromptOutcome::Consumed;
        }
        self.caret = caret;
        PromptOutcome::Moved
    }

    fn edit(&mut self, start: usize, end: usize, replacement: &str) -> PromptOutcome {
        if start == end && replacement.is_empty() {
            return PromptOutcome::Consumed;
        }
        self.text.replace_range(start..end, replacement);
        let end = start + replacement.len();
        // Inserting or removing text can join clusters on either side of the edit.
        self.caret = self
            .text
            .grapheme_indices(true)
            .find_map(|(byte, _)| (byte >= end).then_some(byte))
            .unwrap_or(self.text.len());
        PromptOutcome::Edited
    }

    fn previous_word(&self) -> usize {
        let mut graphemes = self.text[..self.caret].grapheme_indices(true).rev();
        let class = |grapheme: &str| {
            let first = grapheme.chars().next().expect("nonempty grapheme");
            if first.is_whitespace() {
                0
            } else if first.is_alphanumeric() || first == '_' {
                1
            } else {
                2
            }
        };
        let Some((start, first)) = graphemes.find(|(_, g)| class(g) != 0) else {
            return 0;
        };
        graphemes
            .take_while(|(_, g)| class(g) == class(first))
            .last()
            .map_or(start, |(byte, _)| byte)
    }
}
