//! Resolve Vim motions and text objects over a borrowed [`TextSource`].

use crate::{ByteOffset, DocumentView, Shape, TextRange, TextSource};

mod objects;

/// A search for a single character on the current logical line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterMotion {
    /// Land on the next occurrence (`f`).
    Find,
    /// Stop immediately before the next occurrence (`t`).
    Till,
    /// Land on the previous occurrence (`F`).
    FindBackward,
    /// Stop immediately after the previous occurrence (`T`).
    TillBackward,
}

/// How much of a text object's surroundings the selection includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectScope {
    /// Select the object's contents (`i` prefix).
    Inner,
    /// Include the object's delimiters or adjacent whitespace (`a` prefix).
    Around,
}

/// The unit to select around the current cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectKind {
    /// Select a word using Vimlet's word-character classes (`w`).
    Word,
    /// Select a run of non-whitespace characters (`W`).
    BigWord,
    /// Select a sentence (`s`).
    Sentence,
    /// Select a paragraph as a linewise range (`p`).
    Paragraph,
    /// Select text between a matching pair of delimiters, allowing nesting.
    Delimited {
        /// Opening delimiter.
        open: char,
        /// Matching closing delimiter.
        close: char,
    },
    /// Select text between two occurrences of the supplied quote character.
    Quote(char),
}

impl TextObjectKind {
    /// Decode the key after an inner or around prefix. Unknown keys return `None`.
    pub fn from_key(key: char) -> Option<Self> {
        Some(match key {
            'w' => Self::Word,
            'W' => Self::BigWord,
            's' => Self::Sentence,
            'p' => Self::Paragraph,
            '(' | ')' | 'b' => Self::Delimited {
                open: '(',
                close: ')',
            },
            '[' | ']' => Self::Delimited {
                open: '[',
                close: ']',
            },
            '{' | '}' | 'B' => Self::Delimited {
                open: '{',
                close: '}',
            },
            '"' | '\'' | '`' => Self::Quote(key),
            _ => return None,
        })
    }
}

/// Destination within the next or previous Vim word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordDestination {
    /// Move forward to the start of a word (`w` or `W`).
    NextStart,
    /// Move forward to the end of a word (`e` or `E`).
    NextEnd,
    /// Move backward to the start of a word (`b` or `B`).
    PreviousStart,
    /// Move backward to the end of a word (`ge` or `gE`).
    PreviousEnd,
}

/// Classification used by word motions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordClass {
    /// Distinguish letters, digits and underscores from punctuation.
    Word,
    /// Treat each run of non-whitespace characters as a single WORD.
    BigWord,
}

/// A cursor movement that can also supply the target for an operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// Move one grapheme or opaque span left on the current line (`h`).
    Left,
    /// Move one grapheme or opaque span right on the current line (`l`).
    Right,
    /// Move up a logical line using the host's display columns (`k`).
    Up,
    /// Move down a logical line using the host's display columns (`j`).
    Down,
    /// Move to byte zero of the current line (`0`).
    LineStart,
    /// Move to the first non-whitespace grapheme on the line (`^`).
    FirstNonBlank,
    /// Move to the final grapheme on the line (`$`).
    LineEnd,
    /// Move to a word boundary using the supplied classification.
    Word {
        /// Which end of the word to target and in which direction.
        destination: WordDestination,
        /// How the resolver groups characters into words.
        class: WordClass,
    },
    /// Move to the first line, or the one-based line given by the count (`gg`).
    FirstLine,
    /// Move to the final line, or the one-based line given by the count (`G`).
    LastLine,
    /// Use the count as a percentage of the document's logical lines.
    Percent,
    /// Move to a paragraph boundary (`{` or `}`).
    Paragraph {
        /// Search preceding boundaries when true.
        backward: bool,
    },
    /// Move to a sentence boundary (`(` or `)`).
    Sentence {
        /// Search preceding boundaries when true.
        backward: bool,
    },
    /// Find a character on the current line.
    Character {
        /// Search direction and whether to land on the character or adjacent to it.
        kind: CharacterMotion,
        /// Unicode scalar to find.
        character: char,
    },
    /// Repeat a character search, advancing past the preceding till-match when needed.
    RepeatCharacter {
        /// Search direction and landing rule after applying the repeat key.
        kind: CharacterMotion,
        /// Character remembered from the original search.
        character: char,
    },
    /// Move to the partner of a delimiter at or after the cursor (`%`).
    MatchDelimiter,
}

/// The destination of a motion and the rules for using it with an operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// Destination in UTF-8 bytes from the start of the document.
    pub position: ByteOffset,
    /// Whether an operator includes the grapheme at the far end of the motion.
    pub inclusive: bool,
    /// Whether an operator acts on the full logical lines between the endpoints.
    pub linewise: bool,
}

/// The byte ranges and selection shape produced by resolving an edit target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRange {
    /// Half-open UTF-8 byte ranges in document order. A block can span several ranges.
    pub ranges: Vec<TextRange>,
    /// Shape to retain when yanking, deleting or putting the resolved text.
    pub shape: Shape,
}

impl ResolvedRange {
    /// Construct a characterwise range from inclusive start and exclusive end byte offsets.
    pub fn character(start: usize, end: usize) -> Self {
        Self {
            ranges: vec![TextRange::new(start, end)],
            shape: Shape::Character,
        }
    }
}

/// Resolve motions against a borrowed document without changing its text or cursor.
#[derive(Clone, Copy)]
pub struct MotionResolver<'a> {
    pub(crate) text: &'a dyn TextSource,
    pub(crate) cursor: usize,
    pub(crate) opaque: &'a [TextRange],
    geometry: &'a dyn crate::Geometry,
}

impl<'a> MotionResolver<'a> {
    /// Start at a grapheme boundary using the default terminal geometry and no opaque spans.
    pub fn new(text: &'a dyn TextSource, cursor: ByteOffset) -> Self {
        Self {
            text,
            cursor: cursor.0,
            opaque: &[],
            geometry: &crate::DEFAULT_GEOMETRY,
        }
    }

    /// Use the supplied cursor, opaque spans and geometry. The view must be valid.
    pub fn from_view(view: DocumentView<'a>) -> Self {
        Self {
            text: view.text,
            cursor: view.cursor.0,
            opaque: view.opaque,
            geometry: view.geometry,
        }
    }

    pub(crate) fn span(&self, p: usize) -> Option<TextRange> {
        let index = self.opaque.partition_point(|s| s.end.0 <= p);
        self.opaque.get(index).copied().filter(|s| s.start.0 <= p)
    }

    /// Return the byte offset after the grapheme or opaque span at `p`, stopping at EOF.
    pub fn next(&self, p: usize) -> usize {
        self.span(p)
            .map_or_else(|| self.text.next_grapheme(p), |s| s.end.0)
    }

    /// Return the byte offset before `p`, treating each opaque span as a single unit.
    pub fn previous(&self, p: usize) -> usize {
        let previous = self.text.prev_grapheme(p);
        self.span(previous).map_or(previous, |s| s.start.0)
    }

    pub(crate) fn head(&self, p: usize) -> Option<char> {
        if self.span(p).is_some() {
            Some('\u{fffc}')
        } else {
            self.text.char_at(p)
        }
    }

    pub(crate) fn kind(&self, p: usize, big: bool) -> u8 {
        match self.head(p) {
            None => 0,
            Some(c) if c.is_whitespace() => 0,
            Some(c) if big || c.is_alphanumeric() || c == '_' => 1,
            Some(c) if c.is_ascii_punctuation() => 2,
            _ => 3,
        }
    }

    /// Find the first non-whitespace byte on the line containing `p`, or its end if blank.
    pub fn first_non_blank(&self, p: usize) -> usize {
        let mut p = self.text.line_start(p);
        let end = self.text.line_end(p);
        while p < end && self.head(p).is_some_and(char::is_whitespace) {
            p = self.next(p);
        }
        p
    }

    /// Clamp a Normal-mode cursor without allowing it inside an opaque element.
    pub fn normal_cursor(&self, p: usize) -> usize {
        // Saved Visual endpoints can outlive edits that change a UTF-8 boundary.
        let mut p = p.min(self.text.len_bytes());
        while !self.text.is_char_boundary(p) {
            p -= 1;
        }
        if p > 0 {
            let previous = self.text.prev_grapheme(p);
            if self.text.next_grapheme(previous) > p {
                p = previous;
            }
        }
        let p = self.span(p).map_or(p, |s| s.start.0);
        if p < self.text.len_bytes() && !matches!(self.head(p), Some('\n' | '\r')) {
            return p;
        }
        if p == 0 {
            return 0;
        }
        let previous = self.previous(p);
        if matches!(self.head(previous), Some('\n' | '\r')) {
            p
        } else {
            previous
        }
    }

    /// Return the zero-based display column at byte offset `p` using the host's geometry.
    pub fn column(&self, p: usize) -> usize {
        let mut start = self.text.line_start(p);
        let mut col = 0;
        while start < p {
            let next = self.next(start);
            col += self.geometry.width(&self.text.slice(start, next), col);
            start = next;
        }
        col
    }

    /// Map a display column on the line beginning at `start` to a Normal-mode byte offset.
    pub fn at_column(&self, start: usize, col: usize) -> usize {
        let end = self.text.line_end(start);
        let mut p = start;
        let mut column = 0;
        while column < col && p < end {
            let next = self.next(p);
            let width = self.geometry.width(&self.text.slice(p, next), column);
            if column + width > col {
                break;
            }
            column += width;
            p = next;
        }
        self.normal_cursor(p)
    }

    /// Resolve a motion from the current cursor. Zero represents an omitted prefix count.
    /// A character search or delimiter match can return `None` when there is no target.
    pub fn resolve(&self, motion: Motion, count: usize) -> Option<Target> {
        let n = count.max(1);
        let mut p = self.cursor;
        let mut inclusive = false;
        let mut linewise = false;
        match motion {
            Motion::Left => {
                for _ in 0..n {
                    if p == 0 {
                        break;
                    }
                    let previous = self.previous(p);
                    if matches!(self.head(previous), Some('\n' | '\r')) {
                        break;
                    }
                    p = previous;
                }
            }
            Motion::Right => {
                for _ in 0..n {
                    if matches!(self.head(p), None | Some('\n' | '\r')) {
                        break;
                    }
                    let next = self.next(p);
                    if next >= self.text.len_bytes() || matches!(self.head(next), Some('\n' | '\r'))
                    {
                        break;
                    }
                    p = next;
                }
            }
            Motion::Up | Motion::Down => {
                let col = self.column(p);
                let mut line = self.text.line_start(p);
                for _ in 0..n {
                    if motion == Motion::Up {
                        if line == 0 {
                            break;
                        }
                        line = self.text.line_start(line - 1);
                    } else {
                        let next = self.text.next_line(line);
                        if next <= line || next >= self.text.len_bytes() {
                            break;
                        }
                        line = next;
                    }
                }
                p = self.at_column(line, col);
                linewise = true;
            }
            Motion::LineStart => p = self.text.line_start(p),
            Motion::FirstNonBlank => p = self.first_non_blank(p),
            Motion::LineEnd => {
                for _ in 1..n {
                    let next = self.text.next_line(p);
                    if next >= self.text.len_bytes() {
                        break;
                    }
                    p = next;
                }
                p = self.normal_cursor(self.text.line_end(p));
                inclusive = true;
            }
            Motion::Word { destination, class } => {
                let backward = matches!(
                    destination,
                    WordDestination::PreviousStart | WordDestination::PreviousEnd
                );
                let end = matches!(
                    destination,
                    WordDestination::NextEnd | WordDestination::PreviousEnd
                );
                let big = class == WordClass::BigWord;
                p = self.word(n, backward, end, big);
                inclusive = end;
            }
            Motion::FirstLine | Motion::LastLine | Motion::Percent => {
                let last = if self.text.is_empty() {
                    0
                } else {
                    self.text
                        .line_index(self.text.prev_grapheme(self.text.len_bytes()))
                };
                let line = match motion {
                    Motion::FirstLine => count.saturating_sub(1),
                    Motion::LastLine if count == 0 => last,
                    Motion::LastLine => count - 1,
                    _ => ((last + 1) * count.min(100))
                        .div_ceil(100)
                        .saturating_sub(1),
                };
                p = self.first_non_blank(self.text.line_offset(line.min(last)));
                linewise = true;
            }
            Motion::Character { kind, character } => {
                let backward = matches!(
                    kind,
                    CharacterMotion::FindBackward | CharacterMotion::TillBackward
                );
                let mut remaining = n;
                let start = self.text.line_start(p);
                let end = self.text.line_end(p);
                loop {
                    if backward {
                        if p <= start {
                            return None;
                        }
                        p = self.previous(p);
                    } else {
                        p = self.next(p);
                        if p >= end {
                            return None;
                        }
                    }
                    if self.span(p).is_none() && self.head(p) == Some(character) {
                        remaining -= 1;
                        if remaining == 0 {
                            break;
                        }
                    }
                }
                match kind {
                    CharacterMotion::Till => p = self.previous(p),
                    CharacterMotion::TillBackward => p = self.next(p),
                    _ => {}
                }
                inclusive = !backward;
            }
            Motion::RepeatCharacter { kind, character } => {
                let mut resolver = *self;
                let motion = Motion::Character { kind, character };
                if matches!(kind, CharacterMotion::Till | CharacterMotion::TillBackward)
                    && resolver
                        .resolve(motion, 1)
                        .is_some_and(|t| t.position.0 == self.cursor)
                {
                    resolver.cursor = if kind == CharacterMotion::Till {
                        self.next(self.cursor)
                    } else {
                        self.previous(self.cursor)
                    };
                }
                return resolver.resolve(motion, count);
            }
            Motion::MatchDelimiter => {
                p = self.match_delimiter()?;
                inclusive = true;
            }
            Motion::Paragraph { backward } => {
                p = self.paragraph_target(n, backward);
                inclusive = p == self.normal_cursor(self.text.len_bytes()) && !backward;
            }
            Motion::Sentence { backward } => {
                p = self.sentence_target(n, backward);
                inclusive = p == self.normal_cursor(self.text.len_bytes()) && !backward;
            }
        }
        Some(Target {
            position: ByteOffset(p),
            inclusive,
            linewise,
        })
    }

    fn word(&self, count: usize, backward: bool, end: bool, big: bool) -> usize {
        let len = self.text.len_bytes();
        let mut p = self.cursor;
        for _ in 0..count {
            if backward {
                if p == 0 {
                    break;
                }
                if end && self.kind(p, big) != 0 {
                    let k = self.kind(p, big);
                    while p > 0 && self.kind(self.previous(p), big) == k {
                        p = self.previous(p);
                    }
                }
                if p == 0 {
                    break;
                }
                p = self.previous(p);
                while p > 0 && self.kind(p, big) == 0 {
                    p = self.previous(p);
                }
                if !end {
                    let k = self.kind(p, big);
                    while p > 0 && self.kind(self.previous(p), big) == k {
                        p = self.previous(p);
                    }
                }
            } else {
                let k = self.kind(p, big);
                if end {
                    let next = self.next(p);
                    if next < len && k != 0 && self.kind(next, big) == k {
                        p = next;
                    } else {
                        p = next;
                        while p < len && self.kind(p, big) == 0 {
                            p = self.next(p);
                        }
                    }
                    if p < len {
                        let k = self.kind(p, big);
                        while self.next(p) < len && self.kind(self.next(p), big) == k {
                            p = self.next(p);
                        }
                    }
                } else {
                    // An empty line is a word boundary. Do not skip over it.
                    if p < len
                        && (p == 0 || self.text.byte_at(p - 1) == Some(b'\n'))
                        && matches!(self.head(p), Some('\n' | '\r') | None)
                    {
                        p = self.next(p);
                    } else {
                        while p < len && self.kind(p, big) == k {
                            p = self.next(p);
                            if k == 0 && (p == 0 || self.text.byte_at(p - 1) == Some(b'\n')) {
                                break;
                            }
                        }
                    }
                    while p < len && self.kind(p, big) == 0 {
                        if (p == 0 || self.text.byte_at(p - 1) == Some(b'\n'))
                            && matches!(self.head(p), Some('\n' | '\r') | None)
                        {
                            break;
                        }
                        p = self.next(p);
                    }
                }
            }
        }
        p.min(len)
    }

    /// Resolve an operator motion, including `cw`, end-of-line `dw`, and
    /// Vim's exclusive-motion-to-column-zero rule.
    pub fn operator_range(
        &self,
        motion: Motion,
        count: usize,
        operator: crate::Operator,
    ) -> Option<ResolvedRange> {
        let change = operator == crate::Operator::Change;
        if motion == Motion::Right {
            let mut end = self.cursor;
            for _ in 0..count.max(1) {
                if matches!(self.head(end), None | Some('\n' | '\r')) {
                    break;
                }
                end = self.next(end);
            }
            return (end > self.cursor || change)
                .then(|| ResolvedRange::character(self.cursor, end));
        }
        let target = if let Motion::Word {
            destination: WordDestination::NextStart,
            class,
        } = motion
        {
            let big = class == WordClass::BigWord;
            if change && self.kind(self.cursor, big) != 0 {
                // For cw the character under the cursor can already be the first counted word end.
                let next = self.next(self.cursor);
                let at_end = self.kind(next, big) != self.kind(self.cursor, big);
                let remaining = count.max(1) - usize::from(at_end);
                let end = self.word(remaining, false, true, big);
                return Some(ResolvedRange::character(self.cursor, self.next(end)));
            }
            let mut last = *self;
            last.cursor = self.word(count.max(1) - 1, false, false, big);
            let mut end = last.word(1, false, false, big);
            let line_end = self.text.line_end(last.cursor);
            // Only the final step stops at EOL. Starting on an empty line includes its separator.
            if end >= line_end
                && self.text.line_start(last.cursor) > self.cursor
                && self.cursor <= self.first_non_blank(self.cursor)
            {
                return Some(self.lines(self.cursor, last.cursor));
            }
            if end > line_end {
                end = if self.text.line_start(last.cursor) == line_end {
                    self.text.next_line(last.cursor)
                } else {
                    line_end
                };
            }
            if change && end == self.cursor {
                return Some(ResolvedRange::character(end, end));
            }
            Target {
                position: ByteOffset(end),
                inclusive: false,
                linewise: false,
            }
        } else {
            self.resolve(motion, count)?
        };
        if target.linewise {
            if matches!(motion, Motion::Up | Motion::Down)
                && self.text.line_start(target.position.0) == self.text.line_start(self.cursor)
            {
                return None;
            }
            return Some(self.lines(self.cursor, target.position.0));
        }
        let (mut start, mut end) = (
            self.cursor.min(target.position.0),
            self.cursor.max(target.position.0),
        );
        if target.inclusive {
            end = self.next(end);
        }
        if !target.inclusive && end > start && self.text.line_start(end) == end {
            if start <= self.first_non_blank(start) {
                start = self.text.line_start(start);
                return Some(ResolvedRange {
                    ranges: vec![TextRange::new(start, end)],
                    shape: Shape::Line,
                });
            }
            end = self.text.line_end(end - 1);
        }
        (start < end).then(|| ResolvedRange::character(start, end))
    }

    /// Cover the complete lines containing byte offsets `a` and `b`, including intervening lines.
    pub fn lines(&self, a: usize, b: usize) -> ResolvedRange {
        ResolvedRange {
            ranges: vec![TextRange::new(
                self.text.line_start(a.min(b)),
                self.text.next_line(a.max(b)),
            )],
            shape: Shape::Line,
        }
    }

    /// Extend a range to cover every opaque span it intersects.
    pub fn expand_opaque(&self, range: TextRange) -> TextRange {
        let mut range = range;
        for span in self.opaque {
            if span.start < range.end && range.start < span.end {
                range.start = range.start.min(span.start);
                range.end = range.end.max(span.end);
            }
        }
        range
    }

    fn match_delimiter(&self) -> Option<usize> {
        let mut p = self.cursor;
        let end = self.text.line_end(p);
        while p < end && !self.head(p).is_some_and(|c| "()[]{}".contains(c)) {
            p = self.next(p);
        }
        let c = self.head(p)?;
        let (open, close, backward) = match c {
            '(' => ('(', ')', false),
            ')' => ('(', ')', true),
            '[' => ('[', ']', false),
            ']' => ('[', ']', true),
            '{' => ('{', '}', false),
            '}' => ('{', '}', true),
            _ => return None,
        };
        let mut depth = 1;
        loop {
            if backward {
                if p == 0 {
                    return None;
                }
                p = self.previous(p);
            } else {
                p = self.next(p);
                if p >= self.text.len_bytes() {
                    return None;
                }
            }
            let c = self.head(p)?;
            if c == if backward { close } else { open } {
                depth += 1;
            }
            if c == if backward { open } else { close } {
                depth -= 1;
                if depth == 0 {
                    return Some(p);
                }
            }
        }
    }

    fn paragraph_target(&self, count: usize, backward: bool) -> usize {
        let mut line = self.text.line_start(self.cursor);
        for _ in 0..count {
            let mut skip_empty = self.text.line_end(line) == line;
            loop {
                if backward {
                    if line == 0 {
                        return 0;
                    }
                    line = self.text.line_start(line - 1);
                } else {
                    let next = self.text.next_line(line);
                    if next <= line || next >= self.text.len_bytes() {
                        return self.normal_cursor(self.text.len_bytes());
                    }
                    line = next;
                }
                let empty = self.text.line_end(line) == line;
                if skip_empty && empty {
                    continue;
                }
                skip_empty = false;
                if empty {
                    break;
                }
            }
        }
        line
    }
}
