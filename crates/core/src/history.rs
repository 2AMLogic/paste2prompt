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
//!   Consecutive [`Editor::move_edit`] calls on the same edit inside a
//!   gesture fold into one move, so a drag-reorder that ends where it started
//!   also leaves no step.
//! - **Typing.** Outside a gesture, only text typing coalesces: consecutive
//!   [`Editor::set_hint`] calls on the *same* edit, or consecutive
//!   [`Editor::set_instructions`] calls, merge into the latest step, so typing
//!   is one step rather than one per key. Anything else (an add, a delete, a
//!   move, [`Editor::set_op`], [`Editor::set_region`], a hint on a different edit,
//!   undo, redo, a gesture, or [`Editor::break_coalescing`]) closes that step.
//!
//! Outside a gesture, every [`Editor::set_op`], [`Editor::set_region`] and
//! [`Editor::move_edit`] is its own step, so discrete actions such as pressing `M` to switch the mirror
//! axis, or switching op kind, undo one press at a time.
//!
//! Any new change drops the redo stack. A change that alters nothing records
//! nothing and keeps the redo stack.
//!
//! [`Editor::undo`] during an open gesture cancels the gesture (like
//! [`Editor::cancel_gesture`]) instead of committing it, so the redo stack is
//! kept and no earlier step is undone.

use std::fmt;

use crate::document::{Document, Edit, EditId, normalize_hint};
use crate::geometry::{ImageSize, Region};
use crate::op::{Op, OpError};

/// One reversible change to the document.
#[derive(Debug, Clone, PartialEq)]
enum Change {
    Insert {
        index: usize,
        edit: Edit,
    },
    Remove {
        index: usize,
        edit: Edit,
    },
    Update {
        before: Edit,
        after: Edit,
    },
    Instructions {
        before: String,
        after: String,
    },
    /// Moves edit `id` from index `from` to index `to` (its index afterwards).
    Move {
        id: EditId,
        from: usize,
        to: usize,
    },
}

/// What a change updates, for merging changes within one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Edit(EditId),
    Instructions,
    /// An edit's position. Kept apart from [`Target::Edit`] so a move never
    /// merges with an update of the same edit.
    Order(EditId),
}

/// What an open step keeps absorbing outside a gesture: text typing only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Typing {
    Hint(EditId),
    Instructions,
}

impl Change {
    /// The merge target, for updates only.
    fn target(&self) -> Option<Target> {
        match self {
            Self::Update { after, .. } => Some(Target::Edit(after.id())),
            Self::Instructions { .. } => Some(Target::Instructions),
            Self::Move { id, .. } => Some(Target::Order(*id)),
            Self::Insert { .. } | Self::Remove { .. } => None,
        }
    }

    fn is_noop(&self) -> bool {
        match self {
            Self::Update { before, after } => before == after,
            Self::Instructions { before, after } => before == after,
            Self::Move { from, to, .. } => from == to,
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
            // Consecutive moves of one edit compose: from -> mid -> to.
            (Self::Move { to, .. }, Self::Move { to: t, .. }) => *to = t,
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
            Self::Move { id, from, to } => Self::Move {
                id,
                from: to,
                to: from,
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
            Self::Move { id, from, to } => {
                let edits = doc.edits_mut();
                debug_assert_eq!(
                    edits[*from].id(),
                    *id,
                    "history refers to the edit at `from`"
                );
                let edit = edits.remove(*from);
                edits.insert(*to, edit);
            }
        }
    }
}

/// A group of changes undone and redone together.
#[derive(Debug, Clone, Default)]
struct Step {
    changes: Vec<Change>,
    /// Only the newest undo step may be open; an open step absorbs the next
    /// typing change of the same kind.
    open: Option<Typing>,
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
        let id = self.doc.alloc_id().ok_or(EditError::IdsExhausted)?;
        let edit = Edit::new(id, region, op, hint);
        let index = self.doc.edits().len();
        self.commit(Change::Insert { index, edit }, None);
        Ok(id)
    }

    /// Replaces an edit's source region (the `V`-tool drag, which re-picks
    /// the original pixels the edit refers to). The op is unchanged.
    ///
    /// Outside a gesture each call is its own undo step.
    pub fn set_region(&mut self, id: EditId, region: Region) -> Result<(), EditError> {
        self.update(id, None, |e| *e.region_mut() = region)
    }

    /// Replaces an edit's op, e.g. a translate's `dx`/`dy` while dragging the
    /// destination, or a different kind of op altogether.
    ///
    /// Outside a gesture each call is its own undo step (e.g. each `M` press).
    pub fn set_op(&mut self, id: EditId, op: Op) -> Result<(), EditError> {
        op.validate()?;
        self.update(id, None, |e| *e.op_mut() = op)
    }

    /// Sets or clears an edit's hint text. An empty string clears it.
    ///
    /// Outside a gesture, consecutive calls for the same edit are one undo
    /// step (typing).
    pub fn set_hint(&mut self, id: EditId, hint: Option<String>) -> Result<(), EditError> {
        self.update(id, Some(Typing::Hint(id)), |e| {
            *e.hint_mut() = normalize_hint(hint);
        })
    }

    /// Deletes an edit, returning it. Undo restores it at the same position
    /// with the same id.
    pub fn delete_edit(&mut self, id: EditId) -> Result<Edit, EditError> {
        let index = self.doc.index_of(id).ok_or(EditError::UnknownEdit(id))?;
        let edit = self.doc.edits()[index].clone();
        self.commit(
            Change::Remove {
                index,
                edit: edit.clone(),
            },
            None,
        );
        Ok(edit)
    }

    /// Moves an edit so it ends up at `index` (0-based) in the list, shifting
    /// the edits in between. Badge numbers ([`Document::number_of`]) follow
    /// the new order.
    ///
    /// `index` must be less than the number of edits. Moving an edit to the
    /// index it already has records nothing. Outside a gesture each move is
    /// its own undo step and never coalesces with typing; inside a gesture
    /// (a drag-reorder) the whole drag is one step.
    pub fn move_edit(&mut self, id: EditId, index: usize) -> Result<(), EditError> {
        let from = self.doc.index_of(id).ok_or(EditError::UnknownEdit(id))?;
        let len = self.doc.edits().len();
        if index >= len {
            return Err(EditError::IndexOutOfRange { index, len });
        }
        self.commit(
            Change::Move {
                id,
                from,
                to: index,
            },
            None,
        );
        Ok(())
    }

    /// Replaces the image-level instructions.
    ///
    /// Outside a gesture, consecutive calls are one undo step (typing).
    pub fn set_instructions(&mut self, text: impl Into<String>) {
        let change = Change::Instructions {
            before: self.doc.instructions().to_owned(),
            after: text.into(),
        };
        self.commit(change, Some(Typing::Instructions));
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
            top.open = None;
        }
    }

    /// Whether [`undo`](Self::undo) would do anything. During a gesture, that
    /// is whether the gesture has changed anything.
    pub fn can_undo(&self) -> bool {
        match &self.gesture {
            Some(g) => !g.changes.is_empty(),
            None => !self.undo.is_empty(),
        }
    }

    /// Whether [`redo`](Self::redo) would do anything.
    pub fn can_redo(&self) -> bool {
        // Redo ends an open gesture first, and a gesture that changed
        // anything clears the redo stack when it ends.
        !self.redo.is_empty() && self.gesture.as_ref().is_none_or(|g| g.changes.is_empty())
    }

    /// Undoes the latest step. Returns `false` if there was nothing to undo.
    ///
    /// During an open gesture this cancels the gesture instead, exactly like
    /// [`cancel_gesture`](Self::cancel_gesture): its changes are reverted, the
    /// redo stack is kept, and no earlier step is undone (even when the
    /// gesture has no net change). It returns whether the gesture had changed
    /// anything.
    pub fn undo(&mut self) -> bool {
        if let Some(gesture) = &self.gesture {
            let changed = !gesture.changes.is_empty();
            self.cancel_gesture();
            return changed;
        }
        let Some(mut step) = self.undo.pop() else {
            return false;
        };
        for c in step.changes.iter().rev() {
            c.inverse().apply(&mut self.doc);
        }
        step.open = None;
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

    fn update(
        &mut self,
        id: EditId,
        typing: Option<Typing>,
        f: impl FnOnce(&mut Edit),
    ) -> Result<(), EditError> {
        let before = self.doc.edit(id).ok_or(EditError::UnknownEdit(id))?.clone();
        let mut after = before.clone();
        f(&mut after);
        self.commit(Change::Update { before, after }, typing);
        Ok(())
    }

    /// Applies `change` to the document and records it. Outside a gesture, a
    /// `typing` change merges into an open step of the same kind; anything
    /// else (`None`) is a fresh step.
    fn commit(&mut self, change: Change, typing: Option<Typing>) {
        if change.is_noop() {
            return;
        }
        change.apply(&mut self.doc);
        if let Some(gesture) = &mut self.gesture {
            gesture.push_merged(change);
            return;
        }
        self.redo.clear();
        if let Some(top) = self.undo.last_mut()
            && typing.is_some()
            && top.open == typing
            && top.changes.len() == 1
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
            open: typing,
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
    /// The document has used every edit id (2^53 of them) and can't add more.
    IdsExhausted,
    /// A position past the end of the edit list.
    IndexOutOfRange {
        /// The requested 0-based position.
        index: usize,
        /// How many edits the document has.
        len: usize,
    },
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
            Self::IdsExhausted => f.write_str("no edit ids left in this document"),
            Self::IndexOutOfRange { index, len } => {
                write!(f, "index {index} is out of range for {len} edits")
            }
        }
    }
}

impl std::error::Error for EditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidOp(e) => Some(e),
            Self::UnknownEdit(_) | Self::IdsExhausted | Self::IndexOutOfRange { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point;
    use crate::op::{Anchor, MirrorAxis};

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
    fn undo_during_gesture_cancels_it() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.0), None).unwrap();
        ed.begin_gesture();
        ed.set_op(a, translate(0.3)).unwrap();
        assert!(ed.can_undo());
        assert!(ed.undo());
        assert!(!ed.in_gesture());
        assert_eq!(*ed.document().edit(a).unwrap().op(), translate(0.0));
        assert_eq!(ids(&ed), [a]);
        // The cancelled drag left no step: the next undo removes the add.
        assert!(ed.undo());
        assert!(ed.document().edits().is_empty());
        assert!(!ed.undo());
    }

    #[test]
    fn undo_during_gesture_keeps_redo() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.0), None).unwrap();
        ed.set_instructions("x");
        ed.undo();
        assert!(ed.can_redo());
        ed.begin_gesture();
        ed.set_op(a, translate(0.3)).unwrap();
        assert!(ed.undo());
        assert_eq!(*ed.document().edit(a).unwrap().op(), translate(0.0));
        // The instructions change is still redoable, and redo doesn't bring
        // the drag back.
        assert!(ed.can_redo());
        assert!(ed.redo());
        assert_eq!(ed.document().instructions(), "x");
        assert_eq!(*ed.document().edit(a).unwrap().op(), translate(0.0));
        assert!(!ed.redo());
    }

    #[test]
    fn undo_during_gesture_with_no_net_change_keeps_previous_step() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.0), None).unwrap();
        ed.begin_gesture();
        ed.set_region(a, region(0.4)).unwrap();
        ed.set_region(a, region(0.1)).unwrap();
        assert!(!ed.can_undo());
        // Nothing to revert, and the add of the edit being dragged survives.
        assert!(!ed.undo());
        assert!(!ed.in_gesture());
        assert_eq!(ids(&ed), [a]);

        // Same for a gesture that hasn't changed anything yet.
        ed.begin_gesture();
        assert!(!ed.undo());
        assert_eq!(ids(&ed), [a]);
        assert!(ed.set_region(a, region(0.3)).is_ok());

        // Outside a gesture, undo works as usual.
        assert!(ed.undo());
        assert_eq!(*ed.document().edit(a).unwrap().region(), region(0.1));
        assert!(ed.undo());
        assert!(ed.document().edits().is_empty());
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
    fn hint_typing_then_op_change_undoes_only_the_op() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.1), None).unwrap();
        for text in ["m", "make", "make it red"] {
            ed.set_hint(a, Some(text.into())).unwrap();
        }
        let mirror = Op::Mirror {
            axis: MirrorAxis::Horizontal,
        };
        ed.set_op(a, mirror).unwrap();

        assert!(ed.undo());
        let e = ed.document().edit(a).unwrap();
        assert_eq!(*e.op(), translate(0.1));
        assert_eq!(e.hint(), Some("make it red"));

        // The typing is one step of its own.
        assert!(ed.undo());
        assert_eq!(ed.document().edit(a).unwrap().hint(), None);
        assert!(ed.undo());
        assert!(ed.document().edits().is_empty());
    }

    #[test]
    fn op_change_then_hint_typing_are_separate_steps() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.1), None).unwrap();
        ed.set_op(a, Op::Hint).unwrap();
        ed.set_hint(a, Some("r".into())).unwrap();
        ed.set_hint(a, Some("red".into())).unwrap();
        assert!(ed.undo());
        let e = ed.document().edit(a).unwrap();
        assert_eq!(*e.op(), Op::Hint);
        assert_eq!(e.hint(), None);
        assert!(ed.undo());
        assert_eq!(*ed.document().edit(a).unwrap().op(), translate(0.1));
    }

    #[test]
    fn each_mirror_toggle_is_its_own_step() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.1), None).unwrap();
        // `M` three times: mirror horizontally, then switch axis twice.
        let mut axis = MirrorAxis::Horizontal;
        for _ in 0..3 {
            ed.set_op(a, Op::Mirror { axis }).unwrap();
            axis = axis.toggled();
        }
        let op = |ed: &Editor| *ed.document().edit(a).unwrap().op();
        assert_eq!(
            op(&ed),
            Op::Mirror {
                axis: MirrorAxis::Horizontal
            }
        );
        assert!(ed.undo());
        assert_eq!(
            op(&ed),
            Op::Mirror {
                axis: MirrorAxis::Vertical
            }
        );
        assert!(ed.undo());
        assert_eq!(
            op(&ed),
            Op::Mirror {
                axis: MirrorAxis::Horizontal
            }
        );
        assert!(ed.undo());
        assert_eq!(op(&ed), translate(0.1));
        assert!(ed.undo());
        assert!(ed.document().edits().is_empty());
    }

    #[test]
    fn set_region_outside_gesture_is_one_step_each() {
        let mut ed = editor();
        let a = ed.add_edit(region(0.1), translate(0.1), None).unwrap();
        ed.set_region(a, region(0.3)).unwrap();
        ed.set_region(a, region(0.5)).unwrap();
        assert!(ed.undo());
        assert_eq!(*ed.document().edit(a).unwrap().region(), region(0.3));
        assert!(ed.undo());
        assert_eq!(*ed.document().edit(a).unwrap().region(), region(0.1));
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
    fn out_of_range_anchor_is_rejected_by_editor() {
        let mut ed = editor();
        let off = Op::Rotate {
            degrees: 15.0,
            anchor: Anchor::Point(Point::new(-50.0, 9e9)),
        };
        assert_eq!(
            ed.add_edit(region(0.1), off, None),
            Err(EditError::InvalidOp(OpError::AnchorOutOfRange))
        );
        assert!(!ed.can_undo());
        let a = ed.add_edit(region(0.1), translate(0.0), None).unwrap();
        let off = Op::Scale {
            sx: 2.0,
            sy: 2.0,
            anchor: Anchor::Point(Point::new(0.5, 1.2)),
        };
        assert_eq!(
            ed.set_op(a, off),
            Err(EditError::InvalidOp(OpError::AnchorOutOfRange))
        );
        assert_eq!(*ed.document().edit(a).unwrap().op(), translate(0.0));
        // A clamped anchor is accepted.
        let clamped = Op::Scale {
            sx: 2.0,
            sy: 2.0,
            anchor: Anchor::Point(Point::new(0.5, 1.2).clamped()),
        };
        assert!(ed.set_op(a, clamped).is_ok());
    }

    #[test]
    fn add_edit_reports_exhausted_ids_instead_of_overflowing() {
        let json = editor().document().to_json().replacen(
            r#""next_id":1"#,
            r#""next_id":9007199254740992"#,
            1,
        );
        let mut ed = Editor::from_document(Document::from_json(&json).unwrap());
        assert_eq!(
            ed.add_edit(region(0.1), Op::Hint, None),
            Err(EditError::IdsExhausted)
        );
        assert!(ed.document().edits().is_empty() && !ed.can_undo());
    }

    /// An editor with four hint edits, in order `[a, b, c, d]`.
    fn four() -> (Editor, [EditId; 4]) {
        let mut ed = editor();
        let ids = [0.1, 0.3, 0.5, 0.7].map(|x| ed.add_edit(region(x), Op::Hint, None).unwrap());
        (ed, ids)
    }

    #[test]
    fn move_edit_up_down_first_last() {
        let (mut ed, [a, b, c, d]) = four();
        // Up one.
        ed.move_edit(c, 1).unwrap();
        assert_eq!(ids(&ed), [a, c, b, d]);
        // Down one.
        ed.move_edit(c, 2).unwrap();
        assert_eq!(ids(&ed), [a, b, c, d]);
        // To first.
        ed.move_edit(d, 0).unwrap();
        assert_eq!(ids(&ed), [d, a, b, c]);
        // To last.
        ed.move_edit(d, 3).unwrap();
        assert_eq!(ids(&ed), [a, b, c, d]);
        ed.move_edit(a, 3).unwrap();
        assert_eq!(ids(&ed), [b, c, d, a]);
    }

    #[test]
    fn move_edit_renumbers_badges() {
        let (mut ed, [a, b, c, d]) = four();
        ed.move_edit(d, 0).unwrap();
        let doc = ed.document();
        assert_eq!(
            [a, b, c, d].map(|id| doc.number_of(id)),
            [Some(2), Some(3), Some(4), Some(1)]
        );
        ed.undo();
        let doc = ed.document();
        assert_eq!(
            [a, b, c, d].map(|id| doc.number_of(id)),
            [Some(1), Some(2), Some(3), Some(4)]
        );
    }

    #[test]
    fn move_edit_undo_redo() {
        let (mut ed, [a, b, c, d]) = four();
        ed.move_edit(a, 2).unwrap();
        assert_eq!(ids(&ed), [b, c, a, d]);
        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, b, c, d]);
        assert!(ed.redo());
        assert_eq!(ids(&ed), [b, c, a, d]);
        assert!(!ed.redo());
        // The edit itself is untouched by the move.
        assert_eq!(*ed.document().edit(a).unwrap().region(), region(0.1));
    }

    #[test]
    fn each_move_is_its_own_step() {
        let (mut ed, [a, b, c, d]) = four();
        ed.move_edit(d, 2).unwrap();
        ed.move_edit(d, 1).unwrap();
        ed.move_edit(d, 0).unwrap();
        assert_eq!(ids(&ed), [d, a, b, c]);
        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, d, b, c]);
        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, b, d, c]);
        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, b, c, d]);
    }

    #[test]
    fn move_breaks_typing_coalescing() {
        let (mut ed, [a, b, ..]) = four();
        ed.set_hint(a, Some("r".into())).unwrap();
        ed.move_edit(a, 1).unwrap();
        ed.set_hint(a, Some("red".into())).unwrap();
        assert!(ed.undo());
        assert_eq!(ed.document().edit(a).unwrap().hint(), Some("r"));
        assert_eq!(ids(&ed)[..2], [b, a]);
        assert!(ed.undo());
        assert_eq!(ids(&ed)[..2], [a, b]);
        assert_eq!(ed.document().edit(a).unwrap().hint(), Some("r"));
    }

    #[test]
    fn move_between_other_steps_undoes_in_order() {
        let (mut ed, [a, b, c, d]) = four();
        ed.set_op(b, translate(0.1)).unwrap();
        ed.move_edit(b, 3).unwrap();
        ed.delete_edit(c).unwrap();
        assert_eq!(ids(&ed), [a, d, b]);

        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, c, d, b]);
        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, b, c, d]);
        assert_eq!(*ed.document().edit(b).unwrap().op(), translate(0.1));
        assert!(ed.undo());
        assert_eq!(*ed.document().edit(b).unwrap().op(), Op::Hint);

        assert!(ed.redo() && ed.redo() && ed.redo());
        assert_eq!(ids(&ed), [a, d, b]);
        assert_eq!(*ed.document().edit(b).unwrap().op(), translate(0.1));
    }

    #[test]
    fn move_after_undo_drops_redo() {
        let (mut ed, [a, b, c, _]) = four();
        ed.set_instructions("x");
        ed.undo();
        ed.move_edit(c, 0).unwrap();
        assert!(!ed.can_redo());
        assert_eq!(ids(&ed)[..3], [c, a, b]);
    }

    #[test]
    fn move_to_same_index_records_nothing_and_keeps_redo() {
        let (mut ed, [a, b, c, d]) = four();
        ed.set_instructions("x");
        ed.undo();
        assert_eq!(ed.move_edit(b, 1), Ok(()));
        assert_eq!(ids(&ed), [a, b, c, d]);
        assert!(ed.can_redo());
        // The next undo removes the last add, not a phantom move.
        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, b, c]);
    }

    #[test]
    fn move_edit_rejects_bad_id_or_index() {
        let (mut ed, [a, b, c, d]) = four();
        assert_eq!(
            ed.move_edit(a, 4),
            Err(EditError::IndexOutOfRange { index: 4, len: 4 })
        );
        assert_eq!(
            ed.move_edit(a, usize::MAX),
            Err(EditError::IndexOutOfRange {
                index: usize::MAX,
                len: 4
            })
        );
        ed.delete_edit(d).unwrap();
        assert_eq!(ed.move_edit(d, 0), Err(EditError::UnknownEdit(d)));
        assert_eq!(ids(&ed), [a, b, c]);
        // Neither error left a step: undo restores the delete, then the adds.
        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, b, c, d]);
        assert!(ed.undo() && ed.undo() && ed.undo() && ed.undo() && !ed.undo());

        let mut empty = editor();
        assert_eq!(empty.move_edit(a, 0), Err(EditError::UnknownEdit(a)));
        assert!(!empty.can_undo());
    }

    #[test]
    fn move_errors_display() {
        let e = EditError::IndexOutOfRange { index: 5, len: 3 };
        assert_eq!(e.to_string(), "index 5 is out of range for 3 edits");
    }

    #[test]
    fn drag_reorder_gesture_is_one_step() {
        let (mut ed, [a, b, c, d]) = four();
        ed.begin_gesture();
        ed.move_edit(a, 1).unwrap();
        ed.move_edit(a, 2).unwrap();
        ed.move_edit(a, 3).unwrap();
        ed.end_gesture();
        assert_eq!(ids(&ed), [b, c, d, a]);
        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, b, c, d]);
        assert!(ed.redo());
        assert_eq!(ids(&ed), [b, c, d, a]);
        // Exactly the four adds and the drag.
        for _ in 0..5 {
            assert!(ed.undo());
        }
        assert!(!ed.undo());
    }

    #[test]
    fn drag_reorder_back_to_start_leaves_no_step() {
        let (mut ed, [a, b, c, d]) = four();
        ed.begin_gesture();
        ed.move_edit(b, 3).unwrap();
        ed.move_edit(b, 0).unwrap();
        ed.move_edit(b, 1).unwrap();
        ed.end_gesture();
        assert_eq!(ids(&ed), [a, b, c, d]);
        // The undo removes the last add, not a no-op drag.
        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, b, c]);
    }

    #[test]
    fn gesture_mixing_moves_and_updates_undoes_cleanly() {
        let (mut ed, [a, b, c, d]) = four();
        ed.begin_gesture();
        ed.move_edit(a, 2).unwrap();
        ed.set_region(a, region(0.9)).unwrap();
        ed.move_edit(d, 0).unwrap();
        ed.move_edit(a, 1).unwrap();
        ed.end_gesture();
        assert_eq!(ids(&ed), [d, a, b, c]);
        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, b, c, d]);
        assert_eq!(*ed.document().edit(a).unwrap().region(), region(0.1));
        assert!(ed.redo());
        assert_eq!(ids(&ed), [d, a, b, c]);
        assert_eq!(*ed.document().edit(a).unwrap().region(), region(0.9));
    }

    #[test]
    fn cancelled_drag_reorder_restores_order() {
        let (mut ed, [a, b, c, d]) = four();
        ed.begin_gesture();
        ed.move_edit(c, 0).unwrap();
        ed.move_edit(a, 3).unwrap();
        ed.cancel_gesture();
        assert_eq!(ids(&ed), [a, b, c, d]);
        assert!(ed.undo());
        assert_eq!(ids(&ed), [a, b, c]);
    }

    #[test]
    fn moved_order_round_trips_through_json() {
        let (mut ed, [a, b, c, d]) = four();
        ed.move_edit(c, 0).unwrap();
        let doc = Document::from_json(&ed.document().to_json()).unwrap();
        assert_eq!(&doc, ed.document());
        let order: Vec<EditId> = doc.edits().iter().map(Edit::id).collect();
        assert_eq!(order, [c, a, b, d]);
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
