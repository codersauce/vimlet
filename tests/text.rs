use std::cell::Cell;
use unicode_segmentation::UnicodeSegmentation;
use vimlet::*;

struct Chunks {
    text: String,
    starts: Vec<usize>,
}

impl Chunks {
    fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            starts: text.char_indices().map(|(p, _)| p).collect(),
        }
    }
}

impl TextSource for Chunks {
    fn len_bytes(&self) -> usize {
        self.text.len()
    }

    fn chunk_at(&self, byte: usize) -> (&str, usize) {
        if self.text.is_empty() {
            return ("", 0);
        }
        let i = self
            .starts
            .partition_point(|p| *p <= byte.min(self.text.len() - 1))
            .saturating_sub(1);
        let start = self.starts[i];
        let end = self.starts.get(i + 1).copied().unwrap_or(self.text.len());
        (&self.text[start..end], start)
    }
}

fn check_source(source: &dyn TextSource, text: &str) {
    let boundaries: Vec<_> = text
        .grapheme_indices(true)
        .map(|(p, _)| p)
        .chain(std::iter::once(text.len()))
        .collect();
    for pair in boundaries.windows(2) {
        assert_eq!(
            source.next_grapheme(pair[0]),
            pair[1],
            "next at {} in {text:?}",
            pair[0]
        );
        assert_eq!(
            source.prev_grapheme(pair[1]),
            pair[0],
            "prev at {} in {text:?}",
            pair[1]
        );
    }
    for (scalar, (byte, _)) in text.char_indices().enumerate() {
        assert_eq!(source.scalar_to_byte(scalar), byte);
        assert_eq!(source.byte_to_scalar(byte), scalar);
    }
    assert_eq!(source.slice(0, source.len_bytes()), text);
}

#[test]
fn graphemes_cross_every_possible_scalar_chunk_boundary() {
    for text in [
        "",
        "abc",
        "a👩‍💻z",
        "x🇧🇷🇦🇷y",
        "aहिz",
        "e\u{301}\u{302}x",
        "a\r\nb\nc",
        "\u{600}a",
        "각x",
    ] {
        check_source(&text, text);
        check_source(&Chunks::new(text), text);
    }
}

#[cfg(feature = "ropey")]
#[test]
fn rope_chunks_and_strings_agree_on_unicode_and_lines() {
    for text in [
        "a👩‍💻z\r\nb\n".repeat(1000),
        format!("{}e\u{301}{}", "x".repeat(1023), "z".repeat(1000)),
        "a\rb\u{85}c\u{2028}d\n".into(),
    ] {
        let rope = ropey::Rope::from_str(&text);
        check_source(&rope, &text);
        for (p, _) in text.char_indices() {
            assert_eq!(
                TextSource::line_index(&rope, p),
                TextSource::line_index(&text, p)
            );
            assert_eq!(TextSource::line_start(&rope, p), text.line_start(p));
            assert_eq!(TextSource::line_end(&rope, p), text.line_end(p));
        }
    }
}

#[test]
fn whitespace_objects_and_counts_are_adapter_independent() {
    let text = "one  two three";
    let chunked = Chunks::new(text);
    for source in [&text as &dyn TextSource, &chunked] {
        let resolver = MotionResolver::new(source, ByteOffset(3));
        let inner = resolver
            .text_object(TextObjectScope::Inner, TextObjectKind::Word, 1)
            .unwrap();
        assert_eq!(
            source.slice(inner.ranges[0].start.0, inner.ranges[0].end.0),
            "  "
        );
        let around = resolver
            .text_object(TextObjectScope::Around, TextObjectKind::Word, 1)
            .unwrap();
        assert_eq!(
            source.slice(around.ranges[0].start.0, around.ranges[0].end.0),
            "  two"
        );
    }
}

#[test]
fn opaque_elements_are_not_searchable_delimiters_or_find_targets() {
    let text = "a[not(a)word] (real)z";
    let spans = [TextRange::new(1, 13)];
    let view = DocumentView {
        text: &text,
        cursor: ByteOffset(0),
        version: DocumentVersion::default(),
        selection: None,
        opaque: &spans,
        geometry: &DEFAULT_GEOMETRY,
    };
    let resolver = MotionResolver::from_view(view);
    assert_eq!(
        resolver
            .resolve(
                Motion::Character {
                    kind: CharacterMotion::Find,
                    character: 'a'
                },
                1
            )
            .unwrap()
            .position,
        ByteOffset(17)
    );
    assert_eq!(
        resolver
            .resolve(Motion::MatchDelimiter, 1)
            .unwrap()
            .position,
        ByteOffset(19)
    );
    assert_eq!(resolver.next(1), 13);
    assert_eq!(resolver.previous(13), 1);
    assert_eq!(resolver.expand_opaque(TextRange::new(2, 5)), spans[0]);
}

#[test]
fn invalid_utf8_coordinates_are_rejected() {
    let text = "éx";
    let view = DocumentView::new(&text, DocumentVersion::default(), ByteOffset(1));
    assert!(!view.is_valid());
}

struct Counted<'a> {
    text: &'a str,
    reads: Cell<usize>,
}

impl TextSource for Counted<'_> {
    fn len_bytes(&self) -> usize {
        self.text.len()
    }

    fn chunk_at(&self, _: usize) -> (&str, usize) {
        self.reads.set(self.reads.get() + 1);
        (self.text, 0)
    }

    fn slice(&self, start: usize, end: usize) -> std::borrow::Cow<'_, str> {
        assert!(end - start < 100, "local motion flattened text");
        std::borrow::Cow::Borrowed(&self.text[start..end])
    }
}

#[test]
fn local_motion_does_not_materialize_a_document() {
    let text = "a".repeat(3 * 1024 * 1024);
    let source = Counted {
        text: &text,
        reads: Cell::new(0),
    };
    let mut p = text.len() / 2;
    for _ in 0..100 {
        let resolver = MotionResolver::new(&source, ByteOffset(p));
        p = resolver.resolve(Motion::Left, 1).unwrap().position.0;
    }
    assert!(source.reads.get() < 1000);
}
