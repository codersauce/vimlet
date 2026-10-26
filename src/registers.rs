//! One bank for text and recorded input, including automatic Vim registers.
use crate::{Key, Shape};
use std::collections::BTreeMap;

/// Text or recorded input stored with the selection shape used by put.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    text: String,
    shape: Shape,
    keys: Option<Vec<Key>>,
}

impl Register {
    /// Store text with its put shape. Macro playback decodes the text as keystrokes.
    pub fn from_text(text: impl Into<String>, shape: Shape) -> Self {
        Self {
            text: text.into(),
            shape,
            keys: None,
        }
    }

    /// Store the original keys so later playback preserves special keys and paste events.
    pub fn from_keys(keys: Vec<Key>) -> Self {
        let text = keys.iter().map(key_text).collect();
        Self {
            text,
            shape: Shape::Character,
            keys: Some(keys),
        }
    }

    /// Text stored in the register, including line endings for linewise values.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Original selection shape for a put.
    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// Keep recorded special keys intact when dispatching a macro.
    pub fn playback(&self) -> Vec<Key> {
        self.keys
            .clone()
            .unwrap_or_else(|| self.text.chars().map(text_key).collect())
    }
}

/// Vim registers shared by the editing surfaces attached to one session.
#[derive(Debug, Default, Clone)]
pub struct Registers {
    values: BTreeMap<char, Register>,
    unnamed: Option<char>,
    unnamed_revision: u64,
}

impl Registers {
    /// Return a change counter for clipboard synchronization, including repeated identical yanks.
    pub fn unnamed_revision(&self) -> u64 {
        self.unnamed_revision
    }

    /// Look up a register by name, following the unnamed alias and folding uppercase to lowercase.
    pub fn get(&self, name: char) -> Option<&Register> {
        let name = if name == '"' {
            self.unnamed.unwrap_or('"')
        } else {
            name.to_ascii_lowercase()
        };
        self.values.get(&name)
    }

    /// Iterate over stored registers in name order.
    pub fn iter(&self) -> impl Iterator<Item = (char, &Register)> {
        self.values.iter().map(|(k, v)| (*k, v))
    }

    /// Write a register, appending to the lowercase register for an uppercase name.
    /// Returns `false` for a name this bank cannot store.
    pub fn set(&mut self, name: char, mut value: Register) -> bool {
        if !name.is_ascii_alphanumeric() && !matches!(name, '"' | '-' | '+' | '*') {
            return false;
        }
        let lower = name.to_ascii_lowercase();
        if name.is_ascii_uppercase() {
            if let Some(old) = self.values.get(&lower) {
                let mut keys = old.playback();
                keys.extend(value.playback());
                let shape = if old.shape == Shape::Line || value.shape == Shape::Line {
                    Shape::Line
                } else {
                    value.shape
                };
                let mut text = old.text.clone();
                text.push_str(&value.text);
                value = Register {
                    text,
                    shape,
                    keys: Some(keys),
                };
            }
        }
        self.values.insert(lower, value);
        if name == '"' {
            self.unnamed = Some('"');
        }
        if self.unnamed == Some(lower) {
            self.unnamed_revision = self.unnamed_revision.wrapping_add(1);
        }
        true
    }

    pub(crate) fn yank(&mut self, value: Register) {
        self.unnamed_revision = self.unnamed_revision.wrapping_add(1);
        self.values.insert('0', value);
        self.unnamed = Some('0');
    }

    pub(crate) fn delete(&mut self, value: Register, numbered: bool) {
        self.unnamed_revision = self.unnamed_revision.wrapping_add(1);
        if numbered || value.shape != Shape::Character || value.text.contains('\n') {
            for n in (2..=9).rev() {
                let previous = char::from(b'0' + n - 1);
                let name = char::from(b'0' + n);
                if let Some(value) = self.values.remove(&previous) {
                    self.values.insert(name, value);
                } else {
                    self.values.remove(&name);
                }
            }
            self.values.insert('1', value);
            self.unnamed = Some('1');
        } else {
            self.values.insert('-', value);
            self.unnamed = Some('-');
        }
    }
}

fn text_key(c: char) -> Key {
    match c {
        '\u{1b}' => Key::Escape,
        '\n' | '\r' => Key::Enter,
        '\u{8}' | '\u{7f}' => Key::Backspace,
        '\t' => Key::Tab,
        c if c < ' ' => Key::Ctrl(char::from_u32(c as u32 + 96).unwrap_or(c)),
        _ => Key::Char(c),
    }
}

fn key_text(key: &Key) -> String {
    match key {
        Key::Char(c) => c.to_string(),
        Key::Text(s) | Key::Named(s) => s.clone(),
        Key::Escape => "\u{1b}".into(),
        Key::Enter => "\n".into(),
        Key::Backspace => "\u{8}".into(),
        Key::Tab => "\t".into(),
        Key::Ctrl(c) => char::from_u32((*c as u32) & 31).unwrap_or('\0').to_string(),
        // Non-text keys retain their exact typed representation in `keys`.
        other => format!("<{other:?}>"),
    }
}
