//! The editor: every change to a [`Document`] goes through here and lands in
//! the undo/redo history.
//!
//! # Undo steps and coalescing
//!
//! Each user action is one undo step, with two ways to merge many small
//! changes into one step:
//!
//! - **Gestures.** Call [`Editor::begin_gesture`] on pointer-down and
//!   [`Editor::end_gesture`] on pointer-up. Every change in between (a drag
//!   emits many) becomes one step. [`Editor::cancel_gesture`] (`Esc`
//!   mid-drag) reverts them instead. A gesture that ends where it started
//!   leaves no step.
//! - **Consecutive tweaks.** Outside a gesture, repeated updates to the *same*
//!   target (one edit, or the instructions) merge into the latest step, so
//!   typing a hint or the instructions is one step rather than one per key.
//!   Anything else (an add, a delete, an update to a different target, undo,
//!   redo, a gesture, or [`Editor::break_coalescing`]) closes that step.
//!
//! Any new change drops the redo stack. A change that alters nothing records
//! nothing and keeps the redo stack.

use std::fmt;

use crate::document::{Document, Edit, EditId, normalize_hint};
use crate::geometry::{ImageSize, Region};
use crate::op::{Op, OpError};

/// One reversible change to the document.
#[derive(Debug, Clone, PartialEq)]
enum Change {
    Insert { index: usize, edit: Edit },
    Remove { index: usize, edit: Edit },
    Update { before: Edit, after: Edit },
    Instructions { before: String, after: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Edit(EditId),
    Instructions,
}

impl Change {
    /// The coalescing target, for updates only.
    fn target(&self) -> Option<Target> {
        match self {
            Self::Update { after, .. } => Some(Target::Edit(after.id())),
            Self::Instructions { .. } => Some(Target::Instructions),
            Self::Insert { .. } | Self::Remove { .. } => None,
        }
    }

    fn is_noop(&self) -> bool {
        match self {
            Self::Update { before, after } => before == after,
            Self::Instructions { before, after } => before == after,
            Self::Insert { .. } | Self::Remove { .. } => false,
        }
    }

    /// Folds a later change to the same target into this one.
    fn absorb(&mut self, later: Change) {
        match (self, later) {
            (Self::Update { after, .. }, Self::Update { after: a, .. }) => *after = a,
            (Self::Instructions { after, .. }, Self::Instructions { after: a, .. }) => {
                *after = a;
            }
            _ => unreachable!("absorb is only called for matching targets"),
        }
    }

    fn inverse(&self) -> Change {
        match self.clone() {
            Self::Insert { index, edit } => Self::Remove { index, edit },
            Self::Remove { index, edit } => Self::Insert { index, edit },
            Self::Update { before, after } => Self::Update {
                before: after,
                after: before,
            },
            Self::Instructions { before, after } => Self::Instructions {
                before: after,
                after: before,
            },
        }
    }

    fn apply(&self, doc: &mut Document) {
        match self {
            Self::Insert { index, edit } => doc.edits_mut().insert(*index, edit.clone()),
            Self::Remove { index, .. } => {
                doc.edits_mut().remove(*index);
            }
            Self::Update { after, .. } => {
                let i = doc
                    .index_of(after.id())
                    .expect("history refers to an edit in the document");
                doc.edits_mut()[i] = after.clone();
            }
            Self::Instructions { after, .. } => after.clone_into(doc.instructions_mut()),
        }
    }
}

/// A group of changes undone and redone together.
#[derive(Debug, Clone, Default)]
struct Step {
    changes: Vec<Change>,
    /// Only the newest undo step may be open; an open step absorbs the next
    /// update to the same target.
    open: bool,
}

impl Step {
    /// Appends `change`, merging it into the last change when both update the
    /// same target, and dropping the result if it became a no-op.
    fn push_merged(&mut self, change: Change) {
        let target = change.target();
        if let Some(last) = self.changes.last_mut()
            && target.is_some()
            && last.target() == target
        {
            last.absorb(change);
            if last.is_noop() {
                self.changes.pop();
            }
        } else {
            self.changes.push(change);
        }
    }
}

/// Owns a [`Document`] and its undo/redo history. All mutation goes through
/// the editor.
///
/// Loading a new image means a new `Editor`; history isn't carried over.
#[derive(Debug, Clone)]
pub struct Editor {
    doc: Document,
    undo: Vec<Step>,
    redo: Vec<Step>,
    gesture: Option<Step>,
}

impl Editor {
    /// An editor with an empty document for an image of `image_size`.
    pub fn new(image_size: ImageSize) -> Self {
        Self::from_document(Document::new(image_size))
    }

    /// An editor over an existing document (e.g. parsed from JSON), with empty
    /// history.
    pub fn from_document(doc: Document) -> Self {
        Self {
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            gesture: None,
        }
    }

    /// The current document.
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Consumes the editor, returning the document. An open gesture is kept.
    pub fn into_document(self) -> Document {
        self.doc
    }

    /// Appends a new edit and returns its id. The region must already be in
    /// original-image coordinates; there is no way to build one from another
    /// edit's result.
    pub fn add_edit(
        &mut self,
        region: Region,
        op: Op,
        hint: Option<String>,
    ) -> Result<EditId, EditError> {
        op.validate()?;
        let id = self.doc.alloc_id();
        let edit = Edit::new(id, region, op, hint);
        let index = self.doc.edits().len();
        self.commit(Change::Insert { index, edit });
        Ok(id)
    }

    /// Replaces an edit's source region (the `V`-tool drag, which re-picks
    /// the original pixels the edit refers to). The op is unchanged.
    pub fn set_region(&mut self, id: EditId, region: Region) -> Result<(), EditError> {
        self.update(id, |e| *e.region_mut() = region)
    }

    /// Replaces an edit's op, e.g. a translate's `dx`/`dy` while dragging the
    /// destination, or a different kind of op altogether.
    pub fn set_op(&mut self, id: EditId, op: Op) -> Result<(), EditError> {
        op.validate()?;
        self.update(id, |e| *e.op_mut() = op)
    }

    /// Sets or clears an edit's hint text. An empty string clears it.
    pub fn set_hint(&mut self, id: EditId, hint: Option<String>) -> Result<(), EditError> {
        self.update(id, |e| *e.hint_mut() = normalize_hint(hint))
    }

    /// Deletes an edit, returning it. Undo restores it at the same position
    /// with the same id.
    pub fn delete_edit(&mut self, id: EditId) -> Result<Edit, EditError> {
        let index = self.doc.index_of(id).ok_or(EditError::UnknownEdit(id))?;
        let edit = self.doc.edits()[index].clone();
        self.commit(Change::Remove {
            index,
            edit: edit.clone(),
        });
        Ok(edit)
    }

    /// Replaces the image-level instructions.
    pub fn set_instructions(&mut self, text: impl Into<String>) {
        let change = Change::Instructions {
            before: self.doc.instructions().to_owned(),
            after: text.into(),
        };
        self.commit(change);
    }

    /// Starts a gesture: every change until [`end_gesture`](Self::end_gesture)
    /// is one undo step. Ends any gesture already open.
    pub fn begin_gesture(&mut self) {
        self.end_gesture();
        self.break_coalescing();
        self.gesture = Some(Step::default());
    }

    /// Ends the open gesture, recording it as one undo step if it changed
    /// anything. Does nothing without an open gesture.
    pub fn end_gesture(&mut self) {
        if let Some(step) = self.gesture.take()
            && !step.changes.is_empty()
        {
            self.redo.clear();
            self.break_coalescing();
            self.undo.push(step);
        }
    }

    /// Reverts every change made since [`begin_gesture`](Self::begin_gesture)
    /// and discards the gesture. The redo stack is kept. Does nothing without
    /// an open gesture.
    pub fn cancel_gesture(&mut self) {
        if let Some(step) = self.gesture.take() {
            for c in step.changes.iter().rev() {
                c.inverse().apply(&mut self.doc);
            }
        }
    }

    /// Whether a gesture is open.
    pub fn in_gesture(&self) -> bool {
        self.gesture.is_some()
    }

    /// Closes the latest undo step so the next tweak starts a new one (e.g.
    /// when a text field loses focus).
    pub fn break_coalescing(&mut self) {
        if let Some(top) = self.undo.last_mut() {
            top.open = false;
        }
    }

    /// Whether [`undo`](Self::undo) would do anything.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.gesture.as_ref().is_some_and(|g| !g.changes.is_empty())
    }

    /// Whether [`redo`](Self::redo) would do anything.
    pub fn can_redo(&self) -> bool {
        // Redo ends an open gesture first, and a gesture that changed
        // anything clears the redo stack when it ends.
        !self.redo.is_empty() && self.gesture.as_ref().is_none_or(|g| g.changes.is_empty())
    }

    /// Undoes the latest step, ending any open gesture first. Returns `false`
    /// if there was nothing to undo.
    pub fn undo(&mut self) -> bool {
        self.end_gesture();
        let Some(mut step) = self.undo.pop() else {
            return false;
        };
        for c in step.changes.iter().rev() {
            c.inverse().apply(&mut self.doc);
        }
        step.open = false;
        self.redo.push(step);
        self.break_coalescing();
        true
    }

    /// Redoes the latest undone step, ending any open gesture first. Returns
    /// `false` if there was nothing to redo.
    pub fn redo(&mut self) -> bool {
        self.end_gesture();
        let Some(step) = self.redo.pop() else {
            return false;
        };
        for c in &step.changes {
            c.apply(&mut self.doc);
        }
        self.break_coalescing();
        self.undo.push(step);
        true
    }

    fn update(&mut self, id: EditId, f: impl FnOnce(&mut Edit)) -> Result<(), EditError> {
        let before = self.doc.edit(id).ok_or(EditError::UnknownEdit(id))?.clone();
        let mut after = before.clone();
        f(&mut after);
        self.commit(Change::Update { before, after });
        Ok(())
    }

    /// Applies `change` to the document and records it.
    fn commit(&mut self, change: Change) {
        if change.is_noop() {
            return;
        }
        change.apply(&mut self.doc);
        if let Some(gesture) = &mut self.gesture {
            gesture.push_merged(change);
            return;
        }
        self.redo.clear();
        let target = change.target();
        if let Some(top) = self.undo.last_mut()
            && top.open
            && target.is_some()
            && top.changes.len() == 1
            && top.changes[0].target() == target
        {
            top.push_merged(change);
            if top.changes.is_empty() {
                self.undo.pop();
            }
            return;
        }
        self.break_coalescing();
        self.undo.push(Step {
            changes: vec![change],
            open: target.is_some(),
        });
    }
}

/// Why an editor call was rejected. A rejected call changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// No edit with this id is in the document.
    UnknownEdit(EditId),
    /// The op's parameters are invalid.
    InvalidOp(OpError),
}

impl From<OpError> for EditError {
    fn from(e: OpError) -> Self {
        Self::InvalidOp(e)
    }
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEdit(id) => write!(f, "no edit with id {id}"),
            Self::InvalidOp(e) => write!(f, "invalid operation: {e}"),
        }
    }
}

impl std::error::Error for EditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidOp(e) => Some(e),
            Self::UnknownEdit(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point;

    fn editor() -> Editor {
        Editor::new(ImageSize::new(800, 600).unwrap())
    }

    fn region(x: f64) -> Region {
        Region::rect(Point::new(x, 0.1), Point::new(x + 0.2, 0.3)).unwrap()
    }

    fn translate(dx: f64) -> Op {
        Op::Translate { dx, dy: 0.0 }
    }

    fn ids(ed: &Editor) -> Vec<EditId> {
        ed.document().edits().iter().map(Edit::id).collect()
    }

    #[test]
    fn add_undo_redo() {
        let mut ed = editor();
        assert!(!ed.can_undo() && !ed.can_redo());
        let a = ed.add_edit(region(0.1), translate(0.1), None).unwrap();
        let b = ed
            .add_edit(region(0.5), Op::Hint, Some("red".into()))
            .unwrap();
        assert_eq!(ids(&ed), [a, b]);

        assert!(ed.undo());
        assert_eq!(ids(&ed), [a]);
        assert!(ed.undo());
        assert!(ed.document().edits().is_empty());
        assert!(!ed.undo());

        assert!(ed.redo());
        assert!(ed.redo());
        assert_eq!(ids(&ed), [a, b]);
        assert_eq!(ed.document().edit(b).unwrap().hint(), Some("red"));
        assert!(!ed.redo());
    }

    #[test]
    fn new_action_drops_redo() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.1), None).unwrap();
        ed.add_edit(region(0.5), Op::Hint, None).unwrap();
        ed.undo();
        assert!(ed.can_redo());
        let c = ed.add_edit(region(0.3), Op::Hint, None).unwrap();
        assert!(!ed.can_redo());
        assert!(!ed.redo());
        assert_eq!(ids(&ed), [a, c]);
    }

    #[test]
    fn noop_change_records_nothing_and_keeps_redo() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.1), None).unwrap();
        ed.set_instructions("x");
        ed.undo();
        ed.set_op(a, translate(0.1)).unwrap();
        ed.set_hint(a, Some(String::new())).unwrap();
        assert!(ed.can_redo());
        ed.undo();
        assert!(ed.document().edits().is_empty());
    }

    #[test]
    fn drag_gesture_is_one_undo_step() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.0), None).unwrap();
        ed.begin_gesture();
        assert!(ed.in_gesture());
        for i in 1..=20 {
            ed.set_op(a, translate(f64::from(i) * 0.01)).unwrap();
        }
        ed.end_gesture();
        assert_eq!(*ed.document().edit(a).unwrap().op(), translate(0.2));

        assert!(ed.undo());
        assert_eq!(*ed.document().edit(a).unwrap().op(), translate(0.0));
        assert!(ed.redo());
        assert_eq!(*ed.document().edit(a).unwrap().op(), translate(0.2));
        // Exactly two steps: the add and the drag.
        assert!(ed.undo() && ed.undo() && !ed.undo());
    }

    #[test]
    fn two_drags_are_two_steps() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.0), None).unwrap();
        for target in [0.1, 0.2] {
            ed.begin_gesture();
            ed.set_op(a, translate(target / 2.0)).unwrap();
            ed.set_op(a, translate(target)).unwrap();
            ed.end_gesture();
        }
        ed.undo();
        assert_eq!(*ed.document().edit(a).unwrap().op(), translate(0.1));
    }

    #[test]
    fn gesture_back_to_start_leaves_no_step() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.0), None).unwrap();
        ed.begin_gesture();
        ed.set_region(a, region(0.4)).unwrap();
        ed.set_region(a, region(0.1)).unwrap();
        ed.end_gesture();
        ed.undo();
        assert!(ed.document().edits().is_empty());
    }

    #[test]
    fn gesture_spanning_targets_is_one_step() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.0), None).unwrap();
        ed.begin_gesture();
        ed.set_region(a, region(0.3)).unwrap();
        ed.set_hint(a, Some("blue".into())).unwrap();
        ed.set_region(a, region(0.4)).unwrap();
        ed.end_gesture();
        ed.undo();
        let e = ed.document().edit(a).unwrap();
        assert_eq!(*e.region(), region(0.1));
        assert_eq!(e.hint(), None);
    }

    #[test]
    fn cancel_gesture_reverts_and_keeps_redo() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.0), None).unwrap();
        ed.set_instructions("x");
        ed.undo();
        ed.begin_gesture();
        ed.set_op(a, translate(0.3)).unwrap();
        ed.cancel_gesture();
        assert!(!ed.in_gesture());
        assert_eq!(*ed.document().edit(a).unwrap().op(), translate(0.0));
        assert!(ed.redo());
        assert_eq!(ed.document().instructions(), "x");
    }

    #[test]
    fn undo_ends_open_gesture() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.0), None).unwrap();
        ed.begin_gesture();
        ed.set_op(a, translate(0.3)).unwrap();
        assert!(ed.undo());
        assert!(!ed.in_gesture());
        assert_eq!(*ed.document().edit(a).unwrap().op(), translate(0.0));
        assert_eq!(ids(&ed), [a]);
    }

    #[test]
    fn consecutive_tweaks_of_same_target_coalesce() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), Op::Hint, None).unwrap();
        for text in ["m", "ma", "mak", "make it red"] {
            ed.set_hint(a, Some(text.into())).unwrap();
        }
        ed.undo();
        assert_eq!(ed.document().edit(a).unwrap().hint(), None);
        ed.undo();
        assert!(ed.document().edits().is_empty());
    }

    #[test]
    fn coalescing_breaks_on_other_target_or_explicit_break() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), Op::Hint, None).unwrap();
        let b = ed.add_edit(region(0.5), Op::Hint, None).unwrap();
        ed.set_hint(a, Some("one".into())).unwrap();
        ed.set_hint(b, Some("two".into())).unwrap();
        ed.set_hint(a, Some("one!".into())).unwrap();
        ed.break_coalescing();
        ed.set_hint(a, Some("one!!".into())).unwrap();

        ed.undo();
        assert_eq!(ed.document().edit(a).unwrap().hint(), Some("one!"));
        ed.undo();
        assert_eq!(ed.document().edit(a).unwrap().hint(), Some("one"));
        ed.undo();
        assert_eq!(ed.document().edit(b).unwrap().hint(), None);
    }

    #[test]
    fn tweak_after_undo_starts_new_step() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), Op::Hint, None).unwrap();
        ed.set_hint(a, Some("one".into())).unwrap();
        ed.add_edit(region(0.5), Op::Hint, None).unwrap();
        ed.undo();
        ed.set_hint(a, Some("two".into())).unwrap();
        ed.undo();
        assert_eq!(ed.document().edit(a).unwrap().hint(), Some("one"));
    }

    #[test]
    fn instructions_coalesce_and_undo() {
        let mut ed = editor();
        ed.set_instructions("t");
        ed.set_instructions("tighten");
        assert!(ed.undo());
        assert_eq!(ed.document().instructions(), "");
        assert!(!ed.undo());
        assert!(ed.redo());
        assert_eq!(ed.document().instructions(), "tighten");
    }

    #[test]
    fn delete_restores_position_and_id() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), Op::Hint, None).unwrap();
        let b = ed.add_edit(region(0.3), Op::Hint, None).unwrap();
        let c = ed.add_edit(region(0.5), Op::Hint, None).unwrap();
        let removed = ed.delete_edit(b).unwrap();
        assert_eq!(removed.id(), b);
        assert_eq!(ids(&ed), [a, c]);
        assert_eq!(ed.document().number_of(c), Some(2));
        ed.undo();
        assert_eq!(ids(&ed), [a, b, c]);
        ed.redo();
        assert_eq!(ids(&ed), [a, c]);
    }

    #[test]
    fn ids_are_never_reused() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), Op::Hint, None).unwrap();
        ed.undo();
        let b = ed.add_edit(region(0.1), Op::Hint, None).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn rejected_calls_change_nothing() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.0), None).unwrap();
        assert_eq!(
            ed.set_op(a, Op::uniform_scale(-1.0)),
            Err(EditError::InvalidOp(OpError::NonPositiveScale))
        );
        assert_eq!(
            ed.add_edit(region(0.1), translate(f64::NAN), None),
            Err(EditError::InvalidOp(OpError::NotFinite))
        );
        ed.delete_edit(a).unwrap();
        assert_eq!(ed.delete_edit(a), Err(EditError::UnknownEdit(a)));
        assert_eq!(
            ed.set_region(a, region(0.2)),
            Err(EditError::UnknownEdit(a))
        );
        ed.undo();
        assert_eq!(ids(&ed), [a]);
        assert!(ed.undo() && !ed.undo());
    }

    #[test]
    fn editor_state_round_trips_through_json() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.25), None).unwrap();
        ed.set_hint(a, Some("and make it blue".into())).unwrap();
        ed.set_instructions("Overall: tighten");
        let doc = Document::from_json(&ed.document().to_json()).unwrap();
        assert_eq!(&doc, ed.document());
        // A reloaded editor keeps allocating fresh ids.
        let mut ed2 = Editor::from_document(doc);
        let b = ed2.add_edit(region(0.5), Op::Hint, None).unwrap();
        assert_ne!(a, b);
    }
}
