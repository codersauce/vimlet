//! Borrowed UTF-8 access for contiguous strings and chunked text.

use std::borrow::Cow;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

/// A document accessible as borrowed UTF-8 chunks.
///
/// Implement [`Self::len_bytes`] and [`Self::chunk_at`]. Chunks must cover the
/// document without gaps or overlaps and end on Unicode scalar boundaries.
/// Callers supply in-bounds byte offsets unless a method specifies otherwise.
pub trait TextSource {
    /// Return the length of the entire document in UTF-8 bytes.
    fn len_bytes(&self) -> usize;

    /// Borrow the chunk containing `byte` and return its absolute starting byte offset.
    /// At EOF, return the final chunk. For an empty document return an empty chunk at zero.
    fn chunk_at(&self, byte: usize) -> (&str, usize);

    /// Return whether the document contains zero bytes.
    fn is_empty(&self) -> bool {
        self.len_bytes() == 0
    }

    /// Read one raw UTF-8 byte, or return `None` at or beyond EOF.
    fn byte_at(&self, byte: usize) -> Option<u8> {
        if byte >= self.len_bytes() {
            return None;
        }
        let (s, base) = self.chunk_at(byte);
        s.as_bytes().get(byte - base).copied()
    }

    /// Test whether `byte` lies on a UTF-8 scalar boundary, including EOF.
    fn is_char_boundary(&self, byte: usize) -> bool {
        if byte > self.len_bytes() {
            return false;
        }
        let (s, start) = self.chunk_at(byte);
        s.is_char_boundary(byte - start)
    }

    /// Read the half-open byte range `start..end`.
    ///
    /// Both endpoints must be in bounds. The default borrows within a chunk
    /// and allocates when the range crosses chunk boundaries.
    ///
    /// # Panics
    ///
    /// The default implementation panics if the endpoints split a UTF-8 scalar
    /// or `start` is greater than `end`.
    fn slice(&self, start: usize, end: usize) -> Cow<'_, str> {
        let (s, base) = self.chunk_at(start);
        if end <= base + s.len() {
            return Cow::Borrowed(&s[start - base..end - base]);
        }
        let mut out = String::with_capacity(end - start);
        let mut pos = start;
        while pos < end {
            let (s, base) = self.chunk_at(pos);
            let stop = end.min(base + s.len());
            out.push_str(&s[pos - base..stop - base]);
            pos = stop;
        }
        Cow::Owned(out)
    }

    /// Read the Unicode scalar beginning at `byte`, or return `None` at EOF.
    /// Within the document, `byte` must be a UTF-8 scalar boundary.
    fn char_at(&self, byte: usize) -> Option<char> {
        if byte >= self.len_bytes() {
            return None;
        }
        let (s, base) = self.chunk_at(byte);
        s[byte - base..].chars().next()
    }

    /// Find the next extended grapheme boundary in bytes, stopping at EOF.
    fn next_grapheme(&self, byte: usize) -> usize {
        boundary(self, byte, true)
    }

    /// Find the preceding extended grapheme boundary in bytes, stopping at zero.
    fn prev_grapheme(&self, byte: usize) -> usize {
        boundary(self, byte, false)
    }

    /// Start of the logical line containing `byte`.
    fn line_start(&self, byte: usize) -> usize {
        let mut pos = byte;
        while pos > 0 {
            let (s, base) = self.chunk_at(pos - 1);
            if let Some(i) = s.as_bytes()[..pos - base].iter().rposition(|b| *b == b'\n') {
                return base + i + 1;
            }
            pos = base;
        }
        0
    }

    /// End before LF or CRLF, with no synthetic trailing newline.
    fn line_end(&self, byte: usize) -> usize {
        let mut pos = byte;
        let end = loop {
            if pos == self.len_bytes() {
                break pos;
            }
            let (s, base) = self.chunk_at(pos);
            if let Some(i) = s.as_bytes()[pos - base..].iter().position(|b| *b == b'\n') {
                break pos + i;
            }
            pos = base + s.len();
        };
        if end > 0 && self.byte_at(end - 1) == Some(b'\r') {
            end - 1
        } else {
            end
        }
    }

    /// Return the byte offset after the line terminator, or EOF on the final line.
    fn next_line(&self, byte: usize) -> usize {
        let end = self.line_end(byte);
        if self.char_at(end) == Some('\r') {
            (end + 2).min(self.len_bytes())
        } else if self.char_at(end) == Some('\n') {
            end + 1
        } else {
            end
        }
    }

    /// Count LF separators before `byte` to obtain its zero-based logical line index.
    fn line_index(&self, byte: usize) -> usize {
        let mut pos = 0;
        let mut count = 0;
        while pos < byte {
            let (s, base) = self.chunk_at(pos);
            let end = byte.min(base + s.len());
            count += s.as_bytes()[pos - base..end - base]
                .iter()
                .filter(|b| **b == b'\n')
                .count();
            pos = end;
        }
        count
    }

    /// Return the starting byte offset of a zero-based logical line, clamped to EOF.
    fn line_offset(&self, line: usize) -> usize {
        let mut pos = 0;
        for _ in 0..line {
            let next = self.next_line(pos);
            if next <= pos {
                break;
            }
            pos = next;
        }
        pos
    }

    /// Convert a zero-based Unicode scalar index to a byte offset, clamped to EOF.
    fn scalar_to_byte(&self, scalar: usize) -> usize {
        let mut pos = 0;
        for _ in 0..scalar {
            match self.char_at(pos) {
                Some(c) => pos += c.len_utf8(),
                None => break,
            }
        }
        pos
    }

    /// Count Unicode scalars before an in-bounds UTF-8 byte boundary.
    fn byte_to_scalar(&self, byte: usize) -> usize {
        let mut pos = 0;
        let mut count = 0;
        while pos < byte {
            let (s, base) = self.chunk_at(pos);
            let end = byte.min(base + s.len());
            count += s[pos - base..end - base].chars().count();
            pos = end;
        }
        count
    }
}

// `&str` is itself a sized adapter so it can be passed through `&dyn TextSource`.
impl TextSource for &str {
    fn len_bytes(&self) -> usize {
        self.len()
    }

    fn chunk_at(&self, _: usize) -> (&str, usize) {
        (self, 0)
    }
}

impl TextSource for String {
    fn len_bytes(&self) -> usize {
        self.len()
    }

    fn chunk_at(&self, _: usize) -> (&str, usize) {
        (self.as_str(), 0)
    }
}

fn boundary<T: TextSource + ?Sized>(text: &T, byte: usize, forward: bool) -> usize {
    let len = text.len_bytes();
    if (forward && byte >= len) || (!forward && byte == 0) {
        return byte.min(len);
    }
    let mut cursor = GraphemeCursor::new(byte, len, true);
    let (mut chunk, mut start) = text.chunk_at(if forward { byte } else { byte - 1 });
    loop {
        let result = if forward {
            cursor.next_boundary(chunk, start)
        } else {
            cursor.prev_boundary(chunk, start)
        };
        match result {
            Ok(Some(p)) => return p,
            Ok(None) => return if forward { len } else { 0 },
            Err(GraphemeIncomplete::NextChunk) => {
                (chunk, start) = text.chunk_at(start + chunk.len());
            }
            Err(GraphemeIncomplete::PrevChunk) => {
                (chunk, start) = text.chunk_at(start - 1);
            }
            Err(GraphemeIncomplete::PreContext(end)) => {
                let (context, base) = text.chunk_at(end - 1);
                cursor.provide_context(&context[..end - base], base);
            }
            Err(GraphemeIncomplete::InvalidOffset) => {
                panic!("TextSource returned an invalid chunk")
            }
        }
    }
}

/// Ropey integration is absent from builds without the `ropey` feature.
#[cfg(feature = "ropey")]
impl TextSource for ropey::Rope {
    fn len_bytes(&self) -> usize {
        self.len_bytes()
    }

    fn chunk_at(&self, byte: usize) -> (&str, usize) {
        let (s, start, _, _) = self.chunk_at_byte(byte);
        (s, start)
    }

    fn scalar_to_byte(&self, scalar: usize) -> usize {
        self.char_to_byte(scalar.min(self.len_chars()))
    }

    fn byte_to_scalar(&self, byte: usize) -> usize {
        self.byte_to_char(byte)
    }
}
