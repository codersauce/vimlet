//! Inner and around selections for words, sentences, paragraphs and delimiters.
use super::{MotionResolver, ResolvedRange, Shape, TextObjectKind, TextObjectScope, TextRange};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SentenceKind {
    Text,
    Whitespace,
    Paragraph,
}

#[derive(Clone, Copy)]
struct SentenceUnit {
    start: usize,
    end: usize,
    kind: SentenceKind,
}

impl MotionResolver<'_> {
    /// Resolve the counted object at the cursor, returning `None` when no such object exists.
    pub fn text_object(
        &self,
        scope: TextObjectScope,
        kind: TextObjectKind,
        count: usize,
    ) -> Option<ResolvedRange> {
        let cursor = self.normal_cursor(self.cursor);
        if cursor != self.cursor {
            return Self { cursor, ..*self }.text_object(scope, kind, count);
        }
        match kind {
            TextObjectKind::Word => self.word_object(scope, false, count),
            TextObjectKind::BigWord => self.word_object(scope, true, count),
            TextObjectKind::Sentence => self.sentence_object(scope, count),
            TextObjectKind::Paragraph => self.paragraph_object(scope, count),
            TextObjectKind::Delimited { open, close } => {
                self.delimited_object(scope, open, close, count)
            }
            TextObjectKind::Quote(quote) => self.quote_object(scope, quote, count),
        }
    }

    fn word_object(
        &self,
        scope: TextObjectScope,
        big: bool,
        count: usize,
    ) -> Option<ResolvedRange> {
        if self.cursor >= self.text.len_bytes() {
            return None;
        }
        let mut start = self.cursor;
        let mut end = self.cursor;
        let initial = self.kind(start, big);
        let line_start = self.text.line_start(start);
        while start > line_start && self.kind(self.previous(start), big) == initial {
            start = self.previous(start);
        }
        let groups = if scope == TextObjectScope::Around {
            count.max(1)
        } else {
            0
        };
        if groups > 0 {
            for _ in 0..groups {
                if self.kind(end, big) == 0 {
                    while end < self.text.len_bytes() && self.kind(end, big) == 0 {
                        end = self.next(end);
                    }
                }
                let kind = self.kind(end, big);
                while end < self.text.len_bytes() && self.kind(end, big) == kind {
                    end = self.next(end);
                }
            }
            if initial != 0 {
                let before_spaces = end;
                while end < self.text.len_bytes()
                    && self
                        .head(end)
                        .is_some_and(|c| c.is_whitespace() && c != '\n' && c != '\r')
                {
                    end = self.next(end);
                }
                if end == before_spaces {
                    while start > line_start && self.kind(self.previous(start), big) == 0 {
                        start = self.previous(start);
                    }
                }
            }
        } else {
            for index in 0..count.max(1) {
                if index > 0 && matches!(self.head(end), Some('\n' | '\r')) {
                    while end < self.text.len_bytes() && self.kind(end, big) == 0 {
                        end = self.next(end);
                    }
                }
                let kind = self.kind(end, big);
                while end < self.text.len_bytes() && self.kind(end, big) == kind {
                    if matches!(self.head(end), Some('\n' | '\r')) {
                        break;
                    }
                    end = self.next(end);
                }
            }
        }
        (end >= start).then(|| ResolvedRange::character(start, end))
    }

    fn delimited_object(
        &self,
        scope: TextObjectScope,
        open: char,
        close: char,
        count: usize,
    ) -> Option<ResolvedRange> {
        let mut search = self.cursor;
        let mut pair = None;
        for index in 0..count.max(1) {
            let mut p = search;
            let mut depth = 0usize;
            if self.head(p) == Some(close) {
                if p == 0 {
                    return None;
                }
                p = self.previous(p);
            }
            let first = loop {
                match self.head(p) {
                    Some(c) if c == close => depth += 1,
                    Some(c) if c == open => {
                        if depth == 0 {
                            break p;
                        }
                        depth -= 1;
                    }
                    _ => {}
                }
                if p == 0 {
                    return None;
                }
                p = self.previous(p);
            };
            p = first;
            depth = 0;
            let last = loop {
                match self.head(p) {
                    Some(c) if c == open => depth += 1,
                    Some(c) if c == close => {
                        depth -= 1;
                        if depth == 0 {
                            break p;
                        }
                    }
                    _ => {}
                }
                let next = self.next(p);
                if next <= p || next >= self.text.len_bytes() {
                    return None;
                }
                p = next;
            };
            pair = Some((first, last));
            if first == 0 {
                if index + 1 < count.max(1) {
                    return None;
                }
                break;
            }
            search = self.previous(first);
        }
        let (start, end) = pair?;
        if end < self.cursor {
            return None;
        }
        Some(match scope {
            TextObjectScope::Inner => ResolvedRange::character(self.next(start), end),
            TextObjectScope::Around => ResolvedRange::character(start, self.next(end)),
        })
    }

    fn quote_object(
        &self,
        scope: TextObjectScope,
        quote: char,
        count: usize,
    ) -> Option<ResolvedRange> {
        let mut p = self.text.line_start(self.cursor);
        let end = self.text.line_end(p);
        let mut open = None;
        let mut backslashes = 0;
        while p < end {
            let c = self.head(p)?;
            if c == quote && backslashes % 2 == 0 {
                if let Some(first) = open.take() {
                    if self.cursor <= p {
                        if scope == TextObjectScope::Inner && count < 2 {
                            return Some(ResolvedRange::character(self.next(first), p));
                        }
                        let mut start = first;
                        let mut last = self.next(p);
                        if scope == TextObjectScope::Around {
                            while last < end && self.kind(last, false) == 0 {
                                last = self.next(last);
                            }
                            if last == self.next(p) {
                                while start > self.text.line_start(start)
                                    && self.kind(self.previous(start), false) == 0
                                {
                                    start = self.previous(start);
                                }
                            }
                        }
                        return Some(ResolvedRange::character(start, last));
                    }
                } else {
                    open = Some(p);
                }
            }
            if c == '\\' {
                backslashes += 1;
            } else {
                backslashes = 0;
            }
            p = self.next(p);
        }
        None
    }

    fn paragraph_object(&self, scope: TextObjectScope, count: usize) -> Option<ResolvedRange> {
        if self.text.is_empty() {
            return None;
        }
        let blank = |line| {
            self.text
                .slice(line, self.text.line_end(line))
                .chars()
                .all(char::is_whitespace)
        };
        let mut first = self.text.line_start(self.cursor);
        let kind = blank(first);
        while first > 0 {
            let prev = self.text.line_start(first - 1);
            if blank(prev) != kind {
                break;
            }
            first = prev;
        }
        let mut end = first;
        let mut current_kind = kind;
        let groups = if scope == TextObjectScope::Around {
            count.max(1) * 2
        } else {
            count.max(1)
        };
        for _ in 0..groups {
            while end < self.text.len_bytes() && blank(end) == current_kind {
                let next = self.text.next_line(end);
                if next <= end {
                    break;
                }
                end = next;
            }
            current_kind = !current_kind;
        }
        if scope == TextObjectScope::Around && end == self.text.len_bytes() && !kind {
            while first > 0 {
                let prev = self.text.line_start(first - 1);
                if !blank(prev) {
                    break;
                }
                first = prev;
            }
        }
        Some(ResolvedRange {
            ranges: vec![TextRange::new(first, end)],
            shape: Shape::Line,
        })
    }

    fn sentence_units(&self) -> Vec<SentenceUnit> {
        let mut units = Vec::new();
        let len = self.text.len_bytes();
        let mut start = 0;
        while start < len {
            if (start == 0 || self.text.byte_at(start - 1) == Some(b'\n'))
                && matches!(self.head(start), Some('\n' | '\r'))
            {
                let mut end = self.text.next_line(start);
                while end < len && self.text.line_end(end) == end {
                    let next = self.text.next_line(end);
                    if next <= end {
                        break;
                    }
                    end = next;
                }
                if end <= start {
                    break;
                }
                units.push(SentenceUnit {
                    start,
                    end,
                    kind: SentenceKind::Paragraph,
                });
                start = end;
                continue;
            }
            let mut end = start;
            while end < len {
                if end > start
                    && (end == 0 || self.text.byte_at(end - 1) == Some(b'\n'))
                    && matches!(self.head(end), Some('\n' | '\r'))
                {
                    break;
                }
                if matches!(self.head(end), Some('.' | '!' | '?')) {
                    let mut candidate = self.next(end);
                    while matches!(self.head(candidate), Some(')' | ']' | '"' | '\'')) {
                        candidate = self.next(candidate);
                    }
                    if candidate == len || self.head(candidate).is_some_and(char::is_whitespace) {
                        end = if matches!(self.head(candidate), Some('\n' | '\r')) {
                            self.next(candidate)
                        } else {
                            candidate
                        };
                        break;
                    }
                }
                end = self.next(end);
            }
            if end == start {
                break;
            }
            units.push(SentenceUnit {
                start,
                end,
                kind: SentenceKind::Text,
            });
            start = end;
            while start < len && self.head(start).is_some_and(char::is_whitespace) {
                if (start == 0 || self.text.byte_at(start - 1) == Some(b'\n'))
                    && matches!(self.head(start), Some('\n' | '\r'))
                {
                    break;
                }
                start = self.next(start);
            }
            if start > end {
                units.push(SentenceUnit {
                    start: end,
                    end: start,
                    kind: SentenceKind::Whitespace,
                });
            }
        }
        units
    }

    pub(super) fn sentence_target(&self, count: usize, backward: bool) -> usize {
        let units = self.sentence_units();
        let mut cursor = self.cursor;
        for _ in 0..count {
            if backward {
                let mut i = units.partition_point(|u| u.start < cursor);
                while i > 0 && units[i - 1].kind == SentenceKind::Whitespace {
                    i -= 1;
                }
                cursor = i.checked_sub(1).map_or(0, |i| units[i].start);
            } else {
                let mut i = units.partition_point(|u| u.start <= cursor);
                while units
                    .get(i)
                    .is_some_and(|u| u.kind == SentenceKind::Whitespace)
                {
                    i += 1;
                }
                cursor = units
                    .get(i)
                    .map_or_else(|| self.normal_cursor(self.text.len_bytes()), |u| u.start);
            }
        }
        cursor
    }

    fn sentence_object(&self, scope: TextObjectScope, count: usize) -> Option<ResolvedRange> {
        let units = self.sentence_units();
        let current = units
            .iter()
            .position(|u| u.start <= self.cursor && self.cursor < u.end)?;
        let mut first = current;
        let mut last = current;
        if scope == TextObjectScope::Inner {
            for _ in 1..count.max(1) {
                let Some(next) = units.get(last + 1) else {
                    break;
                };
                if next.kind == SentenceKind::Whitespace
                    && self.text.slice(next.start, next.end).contains('\n')
                    && units
                        .get(last + 2)
                        .is_some_and(|u| u.kind == SentenceKind::Paragraph)
                {
                    break;
                }
                if next.kind == SentenceKind::Paragraph
                    && units[current].kind != SentenceKind::Paragraph
                {
                    break;
                }
                last += 1;
            }
        } else if units[current].kind == SentenceKind::Whitespace {
            for index in 0..count.max(1) {
                if units.get(last + 1).is_some() {
                    last += 1;
                }
                if index + 1 < count && units.get(last + 1).is_some() {
                    last += 1;
                }
            }
        } else {
            for index in 0..count.max(1) {
                if index > 0 && units.get(last + 1).is_some() {
                    last += 1;
                }
                if units
                    .get(last + 1)
                    .is_some_and(|u| u.kind == SentenceKind::Whitespace)
                {
                    last += 1;
                }
            }
            if last + 1 == units.len()
                && first > 0
                && units[first - 1].kind == SentenceKind::Whitespace
            {
                first -= 1;
            }
        }
        let start = units[first].start;
        let mut end = units[last].end;
        if end > start && end == self.text.line_start(end) {
            if start <= self.first_non_blank(start) {
                return Some(ResolvedRange {
                    ranges: vec![TextRange::new(self.text.line_start(start), end)],
                    shape: Shape::Line,
                });
            }
            end = self.text.line_end(end - 1);
        }
        Some(ResolvedRange::character(start, end))
    }
}
