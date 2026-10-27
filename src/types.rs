//! Coordinates and host document views. All offsets are UTF-8 bytes.

use crate::TextSource;
use unicode_width::UnicodeWidthStr;

/// Host display widths for logical-line motion and block selections.
///
/// The resolver passes one grapheme or one entire opaque span at a time.
pub trait Geometry {
    /// Return the number of display cells occupied at zero-based display `column`.
    fn width(&self, grapheme: &str, column: usize) -> usize;
}

/// Neovim-like default terminal cells and eight-column tabs.
pub struct TerminalGeometry {
    /// Columns between tab stops. Zero is treated as one.
    pub tabstop: usize,
}

impl Geometry for TerminalGeometry {
    fn width(&self, grapheme: &str, column: usize) -> usize {
        if grapheme == "\t" {
            let tab = self.tabstop.max(1);
            tab - column % tab
        } else {
            UnicodeWidthStr::width(grapheme).max(1)
        }
    }
}

/// Default terminal geometry with eight-column tab stops.
pub static DEFAULT_GEOMETRY: TerminalGeometry = TerminalGeometry { tabstop: 8 };

/// A zero-based UTF-8 byte offset from the beginning of a document.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteOffset(
    /// Number of bytes preceding this position.
    pub usize,
);

/// A half-open range in one document revision.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TextRange {
    /// Inclusive UTF-8 byte offset.
    pub start: ByteOffset,
    /// Exclusive UTF-8 byte offset in the same revision.
    pub end: ByteOffset,
}

impl TextRange {
    /// Construct a half-open byte range. The caller supplies ordered endpoints.
    pub const fn new(start: usize, end: usize) -> Self {
        Self {
            start: ByteOffset(start),
            end: ByteOffset(end),
        }
    }

    /// Return whether the endpoints coincide.
    pub const fn is_empty(self) -> bool {
        self.start.0 == self.end.0
    }

    /// Expose the byte offsets as a standard half-open Rust range.
    pub fn as_range(self) -> std::ops::Range<usize> {
        self.start.0..self.end.0
    }
}

/// Shape retained in selections and registers.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Preserve the selected graphemes, including any intervening line breaks.
    #[default]
    Character,
    /// Preserve whole logical lines for linewise operations.
    Line,
    /// Width in host display cells. Geometry is supplied by the host.
    Block {
        /// Horizontal extent of the block, measured in host display cells.
        width: usize,
    },
}

/// Cursor and anchor are inclusive in Visual mode.
/// An endpoint may occupy a logical line's end, including EOF on a trailing empty line.
/// Display positions beyond that line end are not represented by these byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Inclusive UTF-8 byte offset where the selection began.
    pub anchor: ByteOffset,
    /// Inclusive UTF-8 byte offset of the active end, which may precede the anchor.
    pub cursor: ByteOffset,
    /// How to interpret the two endpoints when resolving an operation.
    pub shape: Shape,
}

/// A host identity and monotonically advancing revision.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentVersion {
    /// Host-assigned document identity, shared by all revisions of that document.
    pub id: u64,
    /// Host revision used to reject operations issued before a content change.
    pub revision: u64,
}

/// Immutable input for one engine step. Opaque spans must be sorted,
/// non-overlapping, nonempty, and on grapheme boundaries.
#[derive(Clone, Copy)]
pub struct DocumentView<'a> {
    /// Borrowed UTF-8 text for this revision.
    pub text: &'a dyn TextSource,
    /// Identity and revision of the supplied text.
    pub version: DocumentVersion,
    /// Current UTF-8 byte offset on a grapheme boundary. EOF is valid.
    pub cursor: ByteOffset,
    /// Active Visual selection in this revision, if present.
    pub selection: Option<Selection>,
    /// Sorted, non-overlapping byte ranges the resolver traverses as indivisible units.
    pub opaque: &'a [TextRange],
    /// Host display widths used by vertical movement and block selections.
    pub geometry: &'a dyn Geometry,
}

impl<'a> DocumentView<'a> {
    /// Borrow a document with default terminal geometry, no selection and no opaque spans.
    pub fn new(text: &'a dyn TextSource, version: DocumentVersion, cursor: ByteOffset) -> Self {
        Self {
            text,
            version,
            cursor,
            selection: None,
            opaque: &[],
            geometry: &DEFAULT_GEOMETRY,
        }
    }

    /// Checks bounds and span ordering before the engine touches host data.
    pub fn is_valid(self) -> bool {
        let valid = |p: ByteOffset| {
            p.0 <= self.text.len_bytes()
                && self.text.is_char_boundary(p.0)
                && (p.0 == 0 || self.text.next_grapheme(self.text.prev_grapheme(p.0)) == p.0)
        };
        valid(self.cursor)
            && self
                .selection
                .is_none_or(|s| valid(s.anchor) && valid(s.cursor))
            && self
                .opaque
                .iter()
                .all(|r| valid(r.start) && valid(r.end) && r.start < r.end)
            && self.opaque.windows(2).all(|r| r[0].end <= r[1].start)
    }
}
