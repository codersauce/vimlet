//! Visual selection shapes and repeat extents.
use super::{
    ByteOffset, Command, DocumentView, EditingState, Mode, ResolvedRange, Selection, Session,
    Shape, Step, TargetRequest, VisualExtent,
};
use crate::MotionResolver;

impl Session {
    pub(super) fn swap_block_corners(
        &mut self,
        state: &mut EditingState,
        view: DocumentView<'_>,
    ) -> Step {
        let Some(selection) = view.selection else {
            return Step::Consumed;
        };
        let resolver = MotionResolver::from_view(view);
        let (anchor_start, anchor_end) = resolver.column_span(selection.anchor.0);
        let (cursor_start, cursor_end) = resolver.column_span(selection.cursor.0);
        let left = anchor_start.min(cursor_start);
        let right = anchor_end.max(cursor_end);
        let anchor_line = view.text.line_start(selection.anchor.0);
        let cursor_line = view.text.line_start(selection.cursor.0);
        let mut anchor = resolver.at_visual_column(anchor_line, left);
        let mut cursor = resolver.at_visual_column(cursor_line, right);
        let mut column = right;
        // A corner can cover several cells in a tab or wide grapheme. Neovim tries
        // the right edge first, then the left if it would leave the cursor in place.
        if cursor == selection.cursor.0 {
            anchor = resolver.at_visual_column(anchor_line, right);
            cursor = resolver.at_visual_column(cursor_line, left);
            column = left;
        }
        state.preferred_column = Some(column);
        let selection = Selection {
            anchor: ByteOffset(anchor),
            cursor: ByteOffset(cursor),
            ..selection
        };
        self.move_to(state, view, selection.cursor, state.mode, Some(selection))
    }

    pub(super) fn repeat_visual(
        &mut self,
        state: &mut EditingState,
        command: Command,
        extent: VisualExtent,
        view: DocumentView<'_>,
    ) -> Step {
        let resolver = MotionResolver::from_view(view);
        let line = view.text.line_index(view.cursor.0);
        let last = view.text.line_offset(line + extent.lines.saturating_sub(1));
        let end = if matches!(extent.shape, Shape::Block { .. }) {
            resolver.at_column(
                last,
                resolver.column(view.cursor.0) + extent.columns.saturating_sub(1),
            )
        } else if extent.lines <= 1 {
            // Neovim's cursor sits on the final display cell of a tab.
            let (start, end) = resolver.column_span(view.cursor.0);
            let column = if view.text.byte_at(view.cursor.0) == Some(b'\t') {
                end
            } else {
                start
            };
            resolver.at_visual_column(last, column + extent.columns.saturating_sub(1))
        } else {
            resolver.at_column(last, extent.columns.saturating_sub(1))
        };
        let selection = Selection {
            anchor: view.cursor,
            cursor: ByteOffset(end),
            shape: extent.shape,
        };
        if matches!(extent.shape, Shape::Block { .. }) {
            return self.resolve_request(state, command, TargetRequest::Block { selection }, view);
        }
        self.resolved_command(state, command, selection_range(resolver, selection), view)
    }
}

pub(super) fn mode_shape(mode: Mode) -> Shape {
    match mode {
        Mode::VisualLine => Shape::Line,
        Mode::VisualBlock => Shape::Block { width: 1 },
        _ => Shape::Character,
    }
}

pub(super) fn shape_mode(shape: Shape) -> Mode {
    match shape {
        Shape::Line => Mode::VisualLine,
        Shape::Block { .. } => Mode::VisualBlock,
        _ => Mode::Visual,
    }
}

pub(super) fn selection_range(resolver: MotionResolver<'_>, selection: Selection) -> ResolvedRange {
    if selection.shape == Shape::Line {
        return resolver.lines(selection.anchor.0, selection.cursor.0);
    }
    ResolvedRange::character(
        selection.anchor.0.min(selection.cursor.0),
        resolver.next(selection.anchor.0.max(selection.cursor.0)),
    )
}

pub(super) fn visual_extent(view: DocumentView<'_>) -> Option<VisualExtent> {
    let s = view.selection?;
    let resolver = MotionResolver::from_view(view);
    let first = s.anchor.0.min(s.cursor.0);
    let last = s.anchor.0.max(s.cursor.0);
    let lines = view.text.line_index(last) - view.text.line_index(first) + 1;
    let columns = if matches!(s.shape, Shape::Block { .. }) {
        resolver.column(first).abs_diff(resolver.column(last)) + 1
    } else if lines == 1 {
        resolver.column_span(last).1 - resolver.column(first) + 1
    } else {
        resolver.column_span(last).1 + 1
    };
    Some(VisualExtent {
        shape: s.shape,
        lines,
        columns,
    })
}
