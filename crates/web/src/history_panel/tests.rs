//! The history panel's controller calls, driven natively: the same calls
//! the panel's buttons, drag and instructions box make.

use paste2prompt_core::{Document, EditId, ImageSize};

use super::rules::{Step, reorder_index, rows, step_index};
use crate::canvas::{Controller, Pointer, Tool};

fn loaded() -> Controller {
    let mut c = Controller::new();
    c.load(ImageSize::new(200, 100).unwrap());
    c
}

/// Draws a 30 x 30 rectangle with its top-left at `x`, 10 and returns its id.
fn rect_at(c: &mut Controller, x: f64) -> EditId {
    c.select(None);
    c.set_tool(Tool::Rect);
    c.pointer_down(Pointer::at(x, 10.0));
    c.pointer_move(Pointer::at(x + 15.0, 25.0));
    c.pointer_up(Pointer::at(x + 30.0, 40.0));
    c.selected().expect("new rect is selected")
}

fn three(c: &mut Controller) -> [EditId; 3] {
    [rect_at(c, 10.0), rect_at(c, 60.0), rect_at(c, 110.0)]
}

fn order(c: &Controller) -> Vec<EditId> {
    doc(c).edits().iter().map(|e| e.id()).collect()
}

fn doc(c: &Controller) -> &Document {
    c.editor().unwrap().document()
}

/// Simulates the panel's drag: rows 20px tall, pointer moving to each `ys`.
fn drag_row(c: &mut Controller, id: EditId, ys: &[f64]) {
    assert!(c.begin_reorder());
    for &y in ys {
        let from = doc(c).index_of(id).unwrap();
        let mids: Vec<f64> = (0..doc(c).edits().len())
            .map(|i| 10.0 + 20.0 * i as f64)
            .collect();
        c.reorder_to(id, reorder_index(&mids, from, y)).unwrap();
    }
    c.end_reorder();
}

#[test]
fn up_and_down_buttons_are_one_undo_step_each() {
    let mut c = loaded();
    let [a, b, d] = three(&mut c);
    let to = step_index(2, 3, Step::Up).unwrap();
    c.move_edit(d, to).unwrap();
    assert_eq!(order(&c), [a, d, b]);
    let to = step_index(1, 3, Step::Up).unwrap();
    c.move_edit(d, to).unwrap();
    assert_eq!(order(&c), [d, a, b]);
    assert!(c.undo());
    assert_eq!(order(&c), [a, d, b]);
    assert!(c.undo());
    assert_eq!(order(&c), [a, b, d]);
    assert!(c.redo());
    assert_eq!(order(&c), [a, d, b]);
}

#[test]
fn a_drag_reorder_is_one_undo_step() {
    let mut c = loaded();
    let [a, b, d] = three(&mut c);
    // Drag the first row down past both others, in small steps.
    drag_row(&mut c, a, &[15.0, 25.0, 35.0, 45.0, 55.0]);
    assert_eq!(order(&c), [b, d, a]);
    assert!(c.undo());
    assert_eq!(order(&c), [a, b, d]);
    assert!(c.redo());
    assert_eq!(order(&c), [b, d, a]);
}

#[test]
fn a_drag_reorder_back_to_the_start_records_nothing() {
    let mut c = loaded();
    let [a, b, d] = three(&mut c);
    let before = c.editor().unwrap().clone();
    drag_row(&mut c, a, &[45.0, 5.0]);
    assert_eq!(order(&c), [a, b, d]);
    assert!(c.undo(), "the last add is still the top step");
    assert_eq!(order(&c), [a, b]);
    assert!(c.redo());
    assert_eq!(doc(&c), before.document());
}

#[test]
fn undo_mid_reorder_reverts_it_and_later_moves_are_ignored() {
    let mut c = loaded();
    let [a, b, d] = three(&mut c);
    assert!(c.begin_reorder());
    c.reorder_to(a, 2).unwrap();
    assert_eq!(order(&c), [b, d, a]);
    c.undo();
    assert_eq!(order(&c), [a, b, d]);
    // The rest of that drag must not land as separate undo steps.
    c.reorder_to(a, 1).unwrap();
    c.end_reorder();
    assert_eq!(order(&c), [a, b, d]);
}

#[test]
fn cancel_reorder_restores_the_order() {
    let mut c = loaded();
    let [a, b, d] = three(&mut c);
    assert!(c.begin_reorder());
    c.reorder_to(d, 0).unwrap();
    c.cancel_reorder();
    assert_eq!(order(&c), [a, b, d]);
    assert!(c.undo());
    assert_eq!(order(&c), [a, b]);
}

#[test]
fn delete_from_the_panel_clears_its_selection_and_undoes() {
    let mut c = loaded();
    let [a, b, d] = three(&mut c);
    c.select(Some(b));
    assert!(c.delete_edit(b));
    assert!(!c.delete_edit(b), "already gone");
    assert_eq!(order(&c), [a, d]);
    assert_eq!(c.selected(), None);
    // Deleting another edit keeps the selection.
    c.select(Some(a));
    assert!(c.delete_edit(d));
    assert_eq!(c.selected(), Some(a));
    assert!(c.undo());
    assert!(c.undo());
    assert_eq!(order(&c), [a, b, d]);
}

#[test]
fn row_click_selects_and_rows_track_the_selection() {
    let mut c = loaded();
    let [a, b, _] = three(&mut c);
    c.select(Some(a));
    let r = rows(doc(&c), c.selected());
    assert!(r[0].selected && !r[1].selected);
    c.select(Some(b));
    let r = rows(doc(&c), c.selected());
    assert!(!r[0].selected && r[1].selected);
}

#[test]
fn typing_instructions_coalesces_until_blur() {
    let mut c = loaded();
    rect_at(&mut c, 10.0);
    c.end_instructions_edit(); // focus
    for text in ["t", "ti", "tig", "tight"] {
        c.set_instructions(text);
    }
    c.end_instructions_edit(); // blur
    c.end_instructions_edit(); // focus again
    c.set_instructions("tighter");
    assert_eq!(c.instructions(), "tighter");
    assert!(c.undo());
    assert_eq!(c.instructions(), "tight");
    assert!(c.undo());
    assert_eq!(c.instructions(), "");
    assert_eq!(doc(&c).edits().len(), 1, "typing undid as its own steps");
}

#[test]
fn undo_and_redo_restore_edits_order_and_instructions_exactly() {
    let mut c = loaded();
    let [a, b, d] = three(&mut c);
    c.set_instructions("tighten the spacing");
    c.end_instructions_edit();
    c.move_edit(d, 0).unwrap();
    drag_row(&mut c, a, &[55.0]);
    c.delete_edit(b);
    c.set_instructions("tighten the spacing\nkeep the logo");
    c.end_instructions_edit();

    // Snapshot every state, walk all the way back, then all the way forward.
    let mut states = vec![doc(&c).clone()];
    while c.can_undo() {
        assert!(c.undo());
        states.push(doc(&c).clone());
    }
    assert!(doc(&c).edits().is_empty());
    assert_eq!(c.instructions(), "");
    for expected in states.iter().rev().skip(1) {
        assert!(c.redo());
        assert_eq!(doc(&c), expected);
    }
    assert!(!c.can_redo());
    assert_eq!(order(&c), [d, a]);
    assert_eq!(c.instructions(), "tighten the spacing\nkeep the logo");
}

#[test]
fn panel_calls_without_an_image_do_nothing() {
    let mut c = Controller::new();
    assert!(!c.begin_reorder());
    c.end_reorder();
    c.cancel_reorder();
    c.set_instructions("x");
    c.end_instructions_edit();
    assert_eq!(c.instructions(), "");
    assert!(c.editor().is_none());
}
