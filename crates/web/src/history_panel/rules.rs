//! Pure rules for the history panel: the rows it lists and the reorder
//! index math. No browser types here, so `cargo test` covers them natively.

use paste2prompt_core::{CaptionLine, Document, EditId, caption_lines};

/// One history row, in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: EditId,
    /// 1-based badge number, as on the canvas and in the caption.
    pub number: usize,
    /// The op's name, e.g. `translate`.
    pub op: &'static str,
    /// What the edit does, from the caption row, e.g. `move right 15%, up 4%`.
    pub summary: String,
    pub selected: bool,
}

/// The rows for `doc`. The summary is the caption's description column, so
/// the panel reads exactly like the exported caption.
pub fn rows(doc: &Document, selected: Option<EditId>) -> Vec<Row> {
    let lines = caption_lines(doc);
    let descriptions = lines.into_iter().filter_map(|l| match l {
        CaptionLine::Edit(e) => Some(e),
        _ => None,
    });
    doc.edits()
        .iter()
        .zip(descriptions)
        .map(|(edit, line)| Row {
            id: edit.id(),
            number: line.number,
            op: line.op,
            summary: line.description,
            selected: selected == Some(edit.id()),
        })
        .collect()
}

/// Direction of an up/down button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Up,
    Down,
}

/// Where the up/down button moves the edit at `index` in a list of `len`,
/// or `None` at the end of the list (the button is disabled).
pub fn step_index(index: usize, len: usize, step: Step) -> Option<usize> {
    match step {
        Step::Up => index.checked_sub(1),
        Step::Down => (index + 1 < len).then_some(index + 1),
    }
}

/// The index a dragged row lands at: the number of other rows whose
/// vertical midpoint lies above the pointer. `mids` are the rows' midpoints
/// in current order (the dragged row included, at `from`); `y` is the
/// pointer, in the same coordinates.
///
/// The dragged row's own midpoint is ignored, so the result is stable while
/// the pointer stays over that row, and a row only swaps once the pointer
/// passes its neighbour's middle.
pub fn reorder_index(mids: &[f64], from: usize, y: f64) -> usize {
    mids.iter()
        .enumerate()
        .filter(|&(i, &m)| i != from && m < y)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use paste2prompt_core::{Editor, ImageSize, MirrorAxis, Op, Region};

    #[test]
    fn step_index_stops_at_the_ends() {
        assert_eq!(step_index(0, 3, Step::Up), None);
        assert_eq!(step_index(1, 3, Step::Up), Some(0));
        assert_eq!(step_index(1, 3, Step::Down), Some(2));
        assert_eq!(step_index(2, 3, Step::Down), None);
        assert_eq!(step_index(0, 1, Step::Down), None);
    }

    #[test]
    fn reorder_index_counts_other_rows_above_the_pointer() {
        // Three 20px rows: midpoints 10, 30, 50.
        let mids = [10.0, 30.0, 50.0];
        // Dragging the first row: stays until the pointer passes 30.
        assert_eq!(reorder_index(&mids, 0, 5.0), 0);
        assert_eq!(reorder_index(&mids, 0, 29.0), 0);
        assert_eq!(reorder_index(&mids, 0, 31.0), 1);
        assert_eq!(reorder_index(&mids, 0, 51.0), 2);
        assert_eq!(reorder_index(&mids, 0, 1e9), 2);
        // Dragging the last row up.
        assert_eq!(reorder_index(&mids, 2, 45.0), 2);
        assert_eq!(reorder_index(&mids, 2, 29.0), 1);
        assert_eq!(reorder_index(&mids, 2, -5.0), 0);
        // A middle row over its own midpoint stays put.
        assert_eq!(reorder_index(&mids, 1, 30.0), 1);
        assert_eq!(reorder_index(&[], 0, 3.0), 0);
    }

    #[test]
    fn rows_follow_document_order_with_caption_summaries() {
        let size = ImageSize::new(100, 100).unwrap();
        let mut ed = Editor::new(size);
        let a = ed
            .add_edit(
                Region::rect_from_pixels(size, (0.0, 0.0), (10.0, 10.0)).unwrap(),
                Op::Mirror {
                    axis: MirrorAxis::Horizontal,
                },
                None,
            )
            .unwrap();
        let b = ed
            .add_edit(
                Region::rect_from_pixels(size, (20.0, 20.0), (40.0, 40.0)).unwrap(),
                Op::Hint,
                Some("make it red".into()),
            )
            .unwrap();
        ed.move_edit(b, 0).unwrap();

        let rows = rows(ed.document(), Some(a));
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].id, rows[0].number, rows[0].op), (b, 1, "hint"));
        assert!(rows[0].summary.contains("make it red"));
        assert!(!rows[0].selected);
        assert_eq!((rows[1].id, rows[1].number, rows[1].op), (a, 2, "mirror"));
        assert!(rows[1].selected);
        assert!(!rows[1].summary.is_empty());
    }
}
