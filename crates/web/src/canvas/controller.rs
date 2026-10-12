//! The canvas interaction state machine: current tool and operation, the
//! selected edit, and the pointer drag in progress.
//!
//! It is pure Rust (no browser types), so the whole pointer flow is
//! unit-tested natively. The web glue in [`super`] only converts pointer
//! events to image pixels and calls [`Controller::pointer_down`],
//! [`Controller::pointer_move`] and [`Controller::pointer_up`].
//!
//! Every document change goes through the core [`Editor`]:
//!
//! - A finished rectangle or lasso is one [`Editor::add_edit`], so one undo
//!   step.
//! - Every other drag (move source, translate, scale, rotate) is wrapped in
//!   [`Editor::begin_gesture`] / [`Editor::end_gesture`], so the whole drag
//!   is one undo step; [`Controller::cancel`] (`Esc`) and a
//!   `pointercancel` revert it with [`Editor::cancel_gesture`].
//! - Picking an operation, toggling the mirror axis and typing a hint go
//!   through [`Editor::set_op`] and [`Editor::set_hint`].
//!
//! Hit-testing only looks at source regions (see [`geom::hit_test`]), so a
//! ghost is never selectable and a new region never starts from one.
//!
//! # Driving it from other UI (#11 history panel, #13 shortcuts)
//!
//! The app holds one `Controller` in a signal (see [`super::CanvasState`]).
//! Shortcuts map onto [`Controller::set_tool`] (`V`/`R`/`L`),
//! [`Controller::choose_op`] (`T`/`S`/`M`/`O`/`H`; `M` again toggles the
//! axis), [`Controller::cancel`] (`Esc`), [`Controller::delete_selected`]
//! (`Delete`), and [`Controller::undo`] / [`Controller::redo`]. A history
//! row click is [`Controller::select`].

use paste2prompt_core::{
    Anchor, Edit, EditError, EditId, Editor, ImageSize, MirrorAxis, Op, Region,
};

use super::geom::{self, Px};

/// Hit radius of a handle, in screen pixels.
pub const HANDLE_RADIUS: f64 = 10.0;
/// Distance of the rotate handle above the region's bounding box, in screen
/// pixels.
pub const ROTATE_HANDLE_OFFSET: f64 = 28.0;
/// A press that moves less than this (screen pixels) is a click.
pub const CLICK_SLOP: f64 = 4.0;
/// Minimum spacing between captured lasso points, in screen pixels.
pub const LASSO_STEP: f64 = 2.0;
/// Lasso simplification tolerance, in screen pixels.
pub const LASSO_EPSILON: f64 = 1.5;
/// Rotation snap step with Shift held, in degrees.
pub const ROTATE_SNAP: f64 = 15.0;

/// The pointer tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tool {
    /// `V`: select an edit; dragging moves its **source** region.
    Select,
    /// `R`: draw a rectangle region.
    #[default]
    Rect,
    /// `L`: draw a freehand lasso region.
    Lasso,
}

impl Tool {
    /// Every tool, in toolbar order.
    pub const ALL: [Self; 3] = [Self::Select, Self::Rect, Self::Lasso];

    /// Button label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Rect => "Rectangle",
            Self::Lasso => "Lasso",
        }
    }

    /// The single-key shortcut from the spec.
    pub fn key(self) -> char {
        match self {
            Self::Select => 'V',
            Self::Rect => 'R',
            Self::Lasso => 'L',
        }
    }
}

/// The kind of operation, without parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OpKind {
    /// `T`
    #[default]
    Translate,
    /// `S`
    Scale,
    /// `M`
    Mirror,
    /// `O`
    Rotate,
    /// `H`
    Hint,
}

impl OpKind {
    /// Every op, in toolbar order.
    pub const ALL: [Self; 5] = [
        Self::Translate,
        Self::Scale,
        Self::Mirror,
        Self::Rotate,
        Self::Hint,
    ];

    /// The kind of `op`.
    pub fn of(op: &Op) -> Self {
        match op {
            Op::Translate { .. } => Self::Translate,
            Op::Scale { .. } => Self::Scale,
            Op::Mirror { .. } => Self::Mirror,
            Op::Rotate { .. } => Self::Rotate,
            Op::Hint => Self::Hint,
        }
    }

    /// The identity-ish starting op of this kind, before any handle drag.
    pub fn default_op(self, axis: MirrorAxis) -> Op {
        match self {
            Self::Translate => Op::Translate { dx: 0.0, dy: 0.0 },
            Self::Scale => Op::uniform_scale(1.0),
            Self::Mirror => Op::Mirror { axis },
            Self::Rotate => Op::Rotate {
                degrees: 0.0,
                anchor: Anchor::Center,
            },
            Self::Hint => Op::Hint,
        }
    }

    /// Button label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Translate => "Translate",
            Self::Scale => "Scale",
            Self::Mirror => "Mirror",
            Self::Rotate => "Rotate",
            Self::Hint => "Hint",
        }
    }

    /// The single-key shortcut from the spec.
    pub fn key(self) -> char {
        match self {
            Self::Translate => 'T',
            Self::Scale => 'S',
            Self::Mirror => 'M',
            Self::Rotate => 'O',
            Self::Hint => 'H',
        }
    }
}

/// A pointer sample in image pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pointer {
    /// Position in image pixels (may be outside the image).
    pub pos: Px,
    /// Shift held: keep aspect ratio (scale) or snap to 15° (rotate).
    pub shift: bool,
}

#[cfg(test)]
impl Pointer {
    /// A sample without modifiers.
    pub fn at(x: f64, y: f64) -> Self {
        Self {
            pos: (x, y),
            shift: false,
        }
    }

    /// The same sample with Shift held.
    pub fn with_shift(self) -> Self {
        Self {
            shift: true,
            ..self
        }
    }
}

/// A draggable handle on the selected edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// Scale corner, clockwise from top-left (`0..4`).
    Corner(usize),
    /// Rotation handle.
    Rotate,
}

/// A region being drawn, for the view to preview.
#[derive(Debug, Clone, PartialEq)]
pub enum Preview {
    /// Rectangle from one corner to the other.
    Rect(Px, Px),
    /// Lasso path so far (open).
    Lasso(Vec<Px>),
}

/// What the pointer would do here, for the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    /// Nothing loaded, or nothing to do.
    Default,
    /// Draw a new region.
    Crosshair,
    /// Select or move a region / drag a translate destination.
    Move,
    /// Over a scale corner.
    Resize,
    /// Over the rotate handle.
    Rotate,
}

#[derive(Debug, Clone)]
enum Drag {
    DrawRect {
        start: Px,
        current: Px,
    },
    Lasso {
        points: Vec<Px>,
    },
    MoveSource {
        id: EditId,
        start: Px,
        region0: Region,
    },
    Translate {
        id: EditId,
        start: Px,
        /// The translate at drag start, normalized. The drag adds the
        /// pointer's offset to it, so the destination follows the pointer
        /// from where it was rather than jumping to the drag start.
        d0: (f64, f64),
    },
    Scale {
        id: EditId,
        anchor: Anchor,
        anchor_px: Px,
        corner0: Px,
    },
    Rotate {
        id: EditId,
        anchor: Anchor,
        anchor_px: Px,
        degrees0: f64,
        start_angle: f64,
    },
}

impl Drag {
    /// Whether this drag runs inside an editor gesture.
    fn is_gesture(&self) -> bool {
        !matches!(self, Self::DrawRect { .. } | Self::Lasso { .. })
    }
}

#[derive(Debug, Clone)]
struct Session {
    editor: Editor,
    selected: Option<EditId>,
    drag: Option<Drag>,
    /// Where the pointer went down, image pixels.
    press: Px,
    /// Whether the pointer has moved [`CLICK_SLOP`] from [`Self::press`]
    /// in this drag. Until it has, a gesture drag changes nothing, so a
    /// click (or hand jitter) on a region or handle is never an edit.
    past_slop: bool,
}

/// Tool, operation, selection and drag state over one image's [`Editor`].
#[derive(Debug, Clone)]
pub struct Controller {
    tool: Tool,
    op_kind: OpKind,
    mirror_axis: MirrorAxis,
    /// Screen pixels per image pixel, for screen-sized tolerances.
    view_scale: f64,
    session: Option<Session>,
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller {
    /// A controller with no image loaded.
    pub fn new() -> Self {
        Self {
            tool: Tool::default(),
            op_kind: OpKind::default(),
            mirror_axis: MirrorAxis::Horizontal,
            view_scale: 1.0,
            session: None,
        }
    }

    /// Starts a new session for an image of `size`: a fresh editor with
    /// empty history. The tool and operation choice carry over.
    pub fn load(&mut self, size: ImageSize) {
        self.session = Some(Session {
            editor: Editor::new(size),
            selected: None,
            drag: None,
            press: (0.0, 0.0),
            past_slop: false,
        });
    }

    /// Ends the session (no image).
    pub fn unload(&mut self) {
        self.session = None;
    }

    /// The editor, when an image is loaded. Read-only: change the document
    /// through the controller so selection and drags stay consistent.
    pub fn editor(&self) -> Option<&Editor> {
        self.session.as_ref().map(|s| &s.editor)
    }

    /// The current tool.
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Switches tool, cancelling any drag in progress.
    pub fn set_tool(&mut self, tool: Tool) {
        self.cancel_drag();
        self.tool = tool;
    }

    /// The current operation: the op for new regions, and the op of the
    /// selected edit when there is one.
    pub fn op_kind(&self) -> OpKind {
        self.op_kind
    }

    /// The mirror axis new mirror ops get.
    pub fn mirror_axis(&self) -> MirrorAxis {
        self.mirror_axis
    }

    /// Picks an operation. With an edit selected, its op becomes this kind
    /// (one undo step; a no-op if it already is). Picking mirror again
    /// toggles the axis, as the `M` key does.
    pub fn choose_op(&mut self, kind: OpKind) -> Result<(), EditError> {
        if kind == OpKind::Mirror && self.op_kind == OpKind::Mirror {
            return self.toggle_mirror_axis();
        }
        self.op_kind = kind;
        let axis = self.mirror_axis;
        let Some((id, op)) = self.selected_edit().map(|e| (e.id(), *e.op())) else {
            return Ok(());
        };
        if OpKind::of(&op) == kind {
            return Ok(());
        }
        self.apply_op(id, kind.default_op(axis))
    }

    /// Flips the mirror axis, and the selected edit's axis if it is a mirror.
    pub fn toggle_mirror_axis(&mut self) -> Result<(), EditError> {
        self.mirror_axis = self.mirror_axis.toggled();
        let axis = self.mirror_axis;
        match self
            .selected_edit()
            .filter(|e| matches!(e.op(), Op::Mirror { .. }))
            .map(Edit::id)
        {
            Some(id) => self.apply_op(id, Op::Mirror { axis }),
            None => Ok(()),
        }
    }

    fn apply_op(&mut self, id: EditId, op: Op) -> Result<(), EditError> {
        self.cancel_drag();
        let Some(s) = &mut self.session else {
            return Ok(());
        };
        // Each op pick is its own undo step, never merged with the next
        // tweak to the same edit.
        s.editor.break_coalescing();
        let result = s.editor.set_op(id, op);
        s.editor.break_coalescing();
        result
    }

    /// The selected edit's id.
    pub fn selected(&self) -> Option<EditId> {
        self.session.as_ref().and_then(|s| s.selected)
    }

    /// The selected edit.
    pub fn selected_edit(&self) -> Option<&Edit> {
        let s = self.session.as_ref()?;
        s.editor.document().edit(s.selected?)
    }

    /// Selects an edit (or nothing). The current operation follows the
    /// selected edit's op, so the matching handles show. An unknown id
    /// selects nothing.
    pub fn select(&mut self, id: Option<EditId>) {
        self.cancel_drag();
        let Some(s) = &mut self.session else { return };
        s.selected = id.filter(|&id| s.editor.document().edit(id).is_some());
        self.sync_op_to_selection();
    }

    fn sync_op_to_selection(&mut self) {
        let Some(op) = self.selected_edit().map(|e| *e.op()) else {
            return;
        };
        self.op_kind = OpKind::of(&op);
        if let Op::Mirror { axis } = op {
            self.mirror_axis = axis;
        }
    }

    /// Sets the selected edit's hint text (empty clears it). Consecutive
    /// calls coalesce into one undo step until [`Self::end_hint_edit`].
    pub fn set_hint(&mut self, text: &str) -> Result<(), EditError> {
        let Some(id) = self.selected() else {
            return Ok(());
        };
        let s = self.session.as_mut().expect("selection implies a session");
        s.editor.set_hint(id, Some(text.to_owned()))
    }

    /// Call when the hint field gains or loses focus, so one typing session
    /// is one undo step of its own.
    pub fn end_hint_edit(&mut self) {
        if let Some(s) = &mut self.session {
            s.editor.break_coalescing();
        }
    }

    /// Deletes the selected edit. Returns whether anything was deleted.
    pub fn delete_selected(&mut self) -> bool {
        self.cancel_drag();
        let Some(s) = &mut self.session else {
            return false;
        };
        let Some(id) = s.selected.take() else {
            return false;
        };
        s.editor.delete_edit(id).is_ok()
    }

    /// Reverts and drops the drag in progress. Returns whether there was one.
    fn cancel_drag(&mut self) -> bool {
        let Some(s) = &mut self.session else {
            return false;
        };
        let Some(drag) = s.drag.take() else {
            return false;
        };
        if drag.is_gesture() {
            s.editor.cancel_gesture();
        }
        true
    }

    /// Tells the controller how big an image pixel is on screen, so click
    /// slop, handle radii and lasso tolerance stay constant in screen
    /// pixels.
    pub fn set_view_scale(&mut self, screen_per_image_px: f64) {
        if screen_per_image_px.is_finite() && screen_per_image_px > 0.0 {
            self.view_scale = screen_per_image_px;
        }
    }

    /// Converts a screen-pixel length to image pixels.
    fn screen(&self, len: f64) -> f64 {
        len / self.view_scale
    }

    /// The region being drawn, if any.
    pub fn preview(&self) -> Option<Preview> {
        match self.session.as_ref()?.drag.as_ref()? {
            Drag::DrawRect { start, current } => Some(Preview::Rect(*start, *current)),
            Drag::Lasso { points } => Some(Preview::Lasso(points.clone())),
            _ => None,
        }
    }

    /// The selected edit's handles in image pixels: four scale corners for
    /// a scale, one rotate handle for a rotate, none otherwise.
    pub fn handles(&self) -> Vec<(Handle, Px)> {
        let (Some(s), Some(edit)) = (self.session.as_ref(), self.selected_edit()) else {
            return Vec::new();
        };
        let size = s.editor.document().image_size();
        let region = edit.region();
        match *edit.op() {
            Op::Scale { sx, sy, anchor } => {
                let a = size.to_pixels(anchor.resolve(region));
                geom::bounds_corners(region, size)
                    .iter()
                    .enumerate()
                    .map(|(i, &c)| (Handle::Corner(i), geom::scaled_point(a, c, sx, sy)))
                    .collect()
            }
            Op::Rotate { degrees, anchor } => {
                let a = size.to_pixels(anchor.resolve(region));
                let top = size.to_pixels(region.bounds().min()).1;
                let offset = self.screen(ROTATE_HANDLE_OFFSET);
                vec![(Handle::Rotate, geom::rotate_handle(a, top, degrees, offset))]
            }
            _ => Vec::new(),
        }
    }

    fn handle_at(&self, p: Px) -> Option<Handle> {
        let r = self.screen(HANDLE_RADIUS);
        self.handles()
            .into_iter()
            .filter(|&(_, h)| geom::dist(h, p) <= r)
            .min_by(|a, b| geom::dist(a.1, p).total_cmp(&geom::dist(b.1, p)))
            .map(|(h, _)| h)
    }

    /// Whether `p` is inside the selected edit's source region and that
    /// edit is a translate, so a drag here sets its destination.
    fn on_selected_translate(&self, p: Px) -> bool {
        let (Some(s), Some(edit)) = (self.session.as_ref(), self.selected_edit()) else {
            return false;
        };
        matches!(edit.op(), Op::Translate { .. })
            && geom::region_contains(edit.region(), s.editor.document().image_size(), p)
    }

    /// The cursor to show at `p`.
    pub fn cursor_at(&self, p: Px) -> Cursor {
        let Some(s) = &self.session else {
            return Cursor::Default;
        };
        match self.handle_at(p) {
            Some(Handle::Corner(_)) => return Cursor::Resize,
            Some(Handle::Rotate) => return Cursor::Rotate,
            None => {}
        }
        match self.tool {
            Tool::Select => {
                if geom::hit_test(s.editor.document(), p).is_some() {
                    Cursor::Move
                } else {
                    Cursor::Default
                }
            }
            Tool::Rect | Tool::Lasso => {
                if self.on_selected_translate(p) {
                    Cursor::Move
                } else {
                    Cursor::Crosshair
                }
            }
        }
    }

    /// Pointer pressed at `p` (image pixels). Returns whether a drag
    /// started, i.e. whether the view should capture the pointer.
    pub fn pointer_down(&mut self, p: Pointer) -> bool {
        if self.session.as_ref().is_none_or(|s| s.drag.is_some()) {
            return false;
        }
        let Some(drag) = self.drag_for(p.pos) else {
            return false;
        };
        let s = self.session.as_mut().expect("checked above");
        if drag.is_gesture() {
            s.editor.begin_gesture();
        }
        s.drag = Some(drag);
        s.press = p.pos;
        s.past_slop = false;
        true
    }

    /// Decides what a press at `p` starts. Handles of the selected edit come
    /// first, then the tool's own behavior. `None`: the press only changed
    /// the selection.
    fn drag_for(&mut self, p: Px) -> Option<Drag> {
        if let Some(handle) = self.handle_at(p) {
            let s = self.session.as_ref().expect("handles imply a session");
            let size = s.editor.document().image_size();
            let edit = self.selected_edit().expect("handles imply a selection");
            let id = edit.id();
            let region = edit.region();
            match (handle, *edit.op()) {
                (Handle::Corner(i), Op::Scale { anchor, .. }) => {
                    return Some(Drag::Scale {
                        id,
                        anchor,
                        anchor_px: size.to_pixels(anchor.resolve(region)),
                        corner0: geom::bounds_corners(region, size)[i],
                    });
                }
                (Handle::Rotate, Op::Rotate { degrees, anchor }) => {
                    let anchor_px = size.to_pixels(anchor.resolve(region));
                    return Some(Drag::Rotate {
                        id,
                        anchor,
                        anchor_px,
                        degrees0: degrees,
                        start_angle: geom::rotation_degrees(anchor_px, p),
                    });
                }
                _ => {}
            }
        }
        Some(match self.tool {
            Tool::Select => {
                // Source regions only: a ghost under the pointer selects
                // nothing, and pressing on empty space just deselects.
                let hit = geom::hit_test(self.editor().expect("session").document(), p);
                self.select(hit);
                let edit = self.editor()?.document().edit(hit?)?;
                Drag::MoveSource {
                    id: edit.id(),
                    start: p,
                    // Every move recomputes from the region at drag start,
                    // so clamping at the image edge never accumulates drift.
                    region0: edit.region().clone(),
                }
            }
            Tool::Rect | Tool::Lasso if self.on_selected_translate(p) => {
                let edit = self.selected_edit().expect("translate implies a selection");
                let Op::Translate { dx, dy } = *edit.op() else {
                    unreachable!("on_selected_translate checked the op")
                };
                Drag::Translate {
                    id: edit.id(),
                    start: p,
                    d0: (dx, dy),
                }
            }
            Tool::Rect => Drag::DrawRect {
                start: p,
                current: p,
            },
            Tool::Lasso => Drag::Lasso { points: vec![p] },
        })
    }

    /// Pointer moved to `p` while pressed.
    pub fn pointer_move(&mut self, p: Pointer) {
        let step = self.screen(LASSO_STEP);
        let slop = self.screen(CLICK_SLOP);
        let tool = self.tool;
        let Some(s) = &mut self.session else { return };
        let size = s.editor.document().image_size();
        let Some(drag) = &mut s.drag else { return };
        let pos = p.pos;
        if drag.is_gesture() && !s.past_slop {
            if geom::dist(s.press, pos) < slop {
                return;
            }
            s.past_slop = true;
        }
        let result = match drag {
            Drag::DrawRect { current, .. } => {
                *current = pos;
                Ok(())
            }
            Drag::Lasso { points } => {
                if points.last().is_none_or(|&l| geom::dist(l, pos) >= step) {
                    points.push(pos);
                }
                Ok(())
            }
            Drag::MoveSource { id, start, region0 } => {
                debug_assert_eq!(tool, Tool::Select);
                let (dx, dy) = size.delta_to_normalized(pos.0 - start.0, pos.1 - start.1);
                s.editor.set_region(*id, region0.shifted(dx, dy))
            }
            Drag::Translate { id, start, d0 } => {
                let (dx, dy) = size.delta_to_normalized(pos.0 - start.0, pos.1 - start.1);
                // Recomputed from the drag start every move, like rotate's
                // `degrees0`, so clamping never accumulates drift. The core
                // accepts at most a full image width/height; the pointer can
                // go further over the canvas margin.
                let (dx, dy) = ((d0.0 + dx).clamp(-1.0, 1.0), (d0.1 + dy).clamp(-1.0, 1.0));
                s.editor.set_op(*id, Op::Translate { dx, dy })
            }
            Drag::Scale {
                id,
                anchor,
                anchor_px,
                corner0,
            } => {
                let (sx, sy) = geom::scale_factors(*anchor_px, *corner0, pos, p.shift);
                s.editor.set_op(
                    *id,
                    Op::Scale {
                        sx,
                        sy,
                        anchor: *anchor,
                    },
                )
            }
            Drag::Rotate {
                id,
                anchor,
                anchor_px,
                degrees0,
                start_angle,
            } => {
                let turned = geom::rotation_degrees(*anchor_px, pos) - *start_angle;
                let mut degrees = geom::normalize_degrees(*degrees0 + turned);
                if p.shift {
                    degrees = geom::snap_degrees(degrees, ROTATE_SNAP);
                }
                s.editor.set_op(
                    *id,
                    Op::Rotate {
                        degrees,
                        anchor: *anchor,
                    },
                )
            }
        };
        // The edit vanished or the op was rejected: drop the drag rather
        // than leave a half-applied gesture open.
        if result.is_err() {
            self.cancel_drag();
        }
    }

    /// Pointer released at `p`: finishes the drag. A rectangle or lasso
    /// becomes a new edit (selected); a press that barely moved is a click,
    /// which selects the source region under it (never a ghost).
    pub fn pointer_up(&mut self, p: Pointer) {
        self.pointer_move(p);
        let slop = self.screen(CLICK_SLOP);
        let epsilon = self.screen(LASSO_EPSILON);
        let axis = self.mirror_axis;
        let op = self.op_kind.default_op(axis);
        let Some(s) = &mut self.session else { return };
        let Some(drag) = s.drag.take() else { return };
        let size = s.editor.document().image_size();
        let region = match drag {
            Drag::DrawRect { start, current } => {
                if geom::dist(start, current) < slop {
                    Err(start)
                } else {
                    Region::rect_from_pixels(size, start, current).map_err(|_| start)
                }
            }
            Drag::Lasso { points } => {
                let start = points[0];
                let simplified = geom::simplify_closed(&points, epsilon);
                let spread = simplified
                    .iter()
                    .map(|&q| geom::dist(start, q))
                    .fold(0.0, f64::max);
                if spread < slop {
                    Err(start)
                } else {
                    Region::polygon_from_pixels(size, simplified).map_err(|_| start)
                }
            }
            _ => {
                s.editor.end_gesture();
                return;
            }
        };
        match region {
            Ok(region) => {
                // add_edit only rejects an invalid op; defaults are valid.
                if let Ok(id) = s.editor.add_edit(region, op, None) {
                    s.selected = Some(id);
                }
            }
            Err(click) => {
                let hit = geom::hit_test(s.editor.document(), click);
                self.select(hit);
            }
        }
    }

    /// The browser cancelled the pointer (`pointercancel`, lost capture):
    /// revert the drag.
    pub fn pointer_cancel(&mut self) {
        self.cancel_drag();
    }
}

/// Undo, redo, `Esc` and drag state: driven by the history UI (#11) and the
/// keyboard shortcuts (#13), which land after this module.
#[allow(
    dead_code,
    reason = "API for the undo UI (#11) and keyboard shortcuts (#13); tests cover it"
)]
impl Controller {
    /// Whether [`Self::undo`] would do anything.
    pub fn can_undo(&self) -> bool {
        self.editor().is_some_and(Editor::can_undo)
    }

    /// Whether [`Self::redo`] would do anything.
    pub fn can_redo(&self) -> bool {
        self.editor().is_some_and(Editor::can_redo)
    }

    /// Undoes one step. During a drag (on the canvas, or a history-panel
    /// reorder) it only cancels (reverts) the drag and undoes no earlier
    /// step, matching [`Editor::undo`] during a gesture.
    pub fn undo(&mut self) -> bool {
        if self.cancel_in_progress() {
            self.revalidate_selection();
            return true;
        }
        let done = self.session.as_mut().is_some_and(|s| s.editor.undo());
        self.revalidate_selection();
        done
    }

    /// Redoes one step. During a drag (on the canvas, or a history-panel
    /// reorder) it only cancels (reverts) the drag and redoes nothing, like
    /// [`Self::undo`]: one key press never both throws away the drag and
    /// changes history. ([`Editor::redo`] alone would instead commit an open
    /// reorder gesture, clearing the redo stack.)
    pub fn redo(&mut self) -> bool {
        if self.cancel_in_progress() {
            self.revalidate_selection();
            return true;
        }
        let done = self.session.as_mut().is_some_and(|s| s.editor.redo());
        self.revalidate_selection();
        done
    }

    /// Cancels a canvas drag or an open history-panel reorder gesture.
    /// Returns whether either was in progress.
    fn cancel_in_progress(&mut self) -> bool {
        if self.cancel_drag() {
            return true;
        }
        match &mut self.session {
            Some(s) if s.editor.in_gesture() => {
                s.editor.cancel_gesture();
                true
            }
            _ => false,
        }
    }

    fn revalidate_selection(&mut self) {
        if let Some(s) = &mut self.session
            && let Some(id) = s.selected
            && s.editor.document().edit(id).is_none()
        {
            s.selected = None;
        }
        self.sync_op_to_selection();
    }

    /// `Esc`: cancels the drag in progress (reverting it), or else clears
    /// the selection. Returns whether it did anything.
    pub fn cancel(&mut self) -> bool {
        if self.cancel_drag() {
            return true;
        }
        if self.selected().is_some() {
            self.select(None);
            return true;
        }
        false
    }

    /// Whether a pointer drag is in progress.
    pub fn dragging(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.drag.is_some())
    }
}

/// History panel (#11): delete and reorder edits by id, a drag-reorder as
/// one gesture, and the image-level instructions.
impl Controller {
    /// Deletes the edit `id` (one undo step), clearing the selection if it
    /// was selected. Returns whether anything was deleted.
    pub fn delete_edit(&mut self, id: EditId) -> bool {
        self.cancel_drag();
        let Some(s) = &mut self.session else {
            return false;
        };
        if s.selected == Some(id) {
            s.selected = None;
        }
        s.editor.delete_edit(id).is_ok()
    }

    /// Moves the edit `id` to `index` in the list (one undo step), e.g. for
    /// the up and down buttons.
    pub fn move_edit(&mut self, id: EditId, index: usize) -> Result<(), EditError> {
        self.cancel_drag();
        let Some(s) = &mut self.session else {
            return Ok(());
        };
        s.editor.break_coalescing();
        s.editor.move_edit(id, index)
    }

    /// Starts a drag-reorder: every [`Self::reorder_to`] until
    /// [`Self::end_reorder`] is one undo step. Returns whether it started.
    pub fn begin_reorder(&mut self) -> bool {
        self.cancel_drag();
        let Some(s) = &mut self.session else {
            return false;
        };
        s.editor.begin_gesture();
        true
    }

    /// Moves `id` to `index` during a drag-reorder. Ignored once the
    /// reorder's gesture is gone (an undo mid-drag cancels it), so later
    /// moves of the same drag never land as separate undo steps.
    pub fn reorder_to(&mut self, id: EditId, index: usize) -> Result<(), EditError> {
        match &mut self.session {
            Some(s) if s.drag.is_none() && s.editor.in_gesture() => s.editor.move_edit(id, index),
            _ => Ok(()),
        }
    }

    /// Ends a drag-reorder, recording it as one undo step if the order
    /// changed.
    pub fn end_reorder(&mut self) {
        if let Some(s) = &mut self.session
            && s.drag.is_none()
        {
            s.editor.end_gesture();
        }
    }

    /// Abandons a drag-reorder (`pointercancel`), restoring the order it
    /// started from.
    pub fn cancel_reorder(&mut self) {
        if let Some(s) = &mut self.session
            && s.drag.is_none()
        {
            s.editor.cancel_gesture();
        }
    }

    /// The image-level instructions, or `""` with no image.
    pub fn instructions(&self) -> &str {
        self.editor().map_or("", |e| e.document().instructions())
    }

    /// Replaces the image-level instructions. Consecutive calls coalesce
    /// into one undo step until [`Self::end_instructions_edit`].
    pub fn set_instructions(&mut self, text: &str) {
        if let Some(s) = &mut self.session {
            s.editor.set_instructions(text);
        }
    }

    /// Call when the instructions box gains or loses focus, so one typing
    /// session is one undo step of its own.
    pub fn end_instructions_edit(&mut self) {
        if let Some(s) = &mut self.session {
            s.editor.break_coalescing();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 200 x 100 image shown at 1:1, so screen and image pixels agree.
    fn loaded() -> Controller {
        let mut c = Controller::new();
        c.load(ImageSize::new(200, 100).unwrap());
        c
    }

    fn drag(c: &mut Controller, from: Pointer, to: Pointer) {
        c.pointer_down(from);
        let steps = 8;
        for i in 1..steps {
            let t = f64::from(i) / f64::from(steps);
            c.pointer_move(Pointer {
                pos: (
                    from.pos.0 + (to.pos.0 - from.pos.0) * t,
                    from.pos.1 + (to.pos.1 - from.pos.1) * t,
                ),
                shift: to.shift,
            });
        }
        c.pointer_up(to);
    }

    fn click(c: &mut Controller, at: Pointer) {
        c.pointer_down(at);
        c.pointer_up(at);
    }

    fn edits(c: &Controller) -> &[Edit] {
        c.editor().unwrap().document().edits()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    /// Draws a 20,20 - 60,60 rectangle with the current op and returns its id.
    fn rect(c: &mut Controller) -> EditId {
        c.set_tool(Tool::Rect);
        drag(c, Pointer::at(20.0, 20.0), Pointer::at(60.0, 60.0));
        c.selected().expect("new rect is selected")
    }

    #[test]
    fn nothing_happens_without_an_image() {
        let mut c = Controller::new();
        assert!(!c.pointer_down(Pointer::at(1.0, 1.0)));
        c.pointer_up(Pointer::at(50.0, 50.0));
        assert!(c.editor().is_none());
        assert!(!c.undo());
    }

    #[test]
    fn rect_drag_creates_one_selected_edit_and_one_undo_step() {
        let mut c = loaded();
        let id = rect(&mut c);
        assert_eq!(edits(&c).len(), 1);
        let e = &edits(&c)[0];
        assert_eq!(e.id(), id);
        let size = c.editor().unwrap().document().image_size();
        assert_eq!(
            e.region(),
            &Region::rect_from_pixels(size, (20.0, 20.0), (60.0, 60.0)).unwrap()
        );
        assert_eq!(OpKind::of(e.op()), OpKind::Translate);
        assert!(c.undo());
        assert!(edits(&c).is_empty());
        assert_eq!(c.selected(), None);
        assert!(!c.can_undo());
        assert!(c.redo());
        assert_eq!(edits(&c).len(), 1);
    }

    #[test]
    fn rect_uses_the_current_op() {
        let mut c = loaded();
        c.choose_op(OpKind::Rotate).unwrap();
        rect(&mut c);
        assert_eq!(OpKind::of(edits(&c)[0].op()), OpKind::Rotate);
    }

    #[test]
    fn a_click_does_not_create_a_region() {
        let mut c = loaded();
        click(&mut c, Pointer::at(50.0, 50.0));
        drag(&mut c, Pointer::at(50.0, 50.0), Pointer::at(52.0, 51.0));
        assert!(edits(&c).is_empty());
        assert!(!c.can_undo());
    }

    #[test]
    fn rect_preview_tracks_the_pointer() {
        let mut c = loaded();
        c.pointer_down(Pointer::at(10.0, 10.0));
        c.pointer_move(Pointer::at(30.0, 40.0));
        assert_eq!(c.preview(), Some(Preview::Rect((10.0, 10.0), (30.0, 40.0))));
        c.pointer_up(Pointer::at(30.0, 40.0));
        assert_eq!(c.preview(), None);
    }

    #[test]
    fn lasso_closes_and_simplifies() {
        let mut c = loaded();
        c.set_tool(Tool::Lasso);
        c.pointer_down(Pointer::at(20.0, 20.0));
        // Trace a square with many collinear samples.
        let mut path = Vec::new();
        for i in 1..=40 {
            path.push((20.0 + f64::from(i), 20.0));
        }
        for i in 1..=40 {
            path.push((60.0, 20.0 + f64::from(i) * 0.5));
        }
        for i in 1..=40 {
            path.push((60.0 - f64::from(i), 40.0));
        }
        for &(x, y) in &path {
            c.pointer_move(Pointer::at(x, y));
        }
        match c.preview() {
            Some(Preview::Lasso(points)) => assert!(points.len() > 20),
            other => panic!("expected a lasso preview, got {other:?}"),
        }
        // Release without returning to the start: the path is closed.
        c.pointer_up(Pointer::at(20.0, 40.0));
        assert_eq!(edits(&c).len(), 1);
        let Region::Polygon(poly) = edits(&c)[0].region() else {
            panic!("lasso makes a polygon");
        };
        assert_eq!(poly.points().len(), 4, "{:?}", poly.points());
        assert!(c.selected().is_some());
    }

    #[test]
    fn tiny_lasso_is_a_click() {
        let mut c = loaded();
        let id = rect(&mut c);
        c.select(None);
        c.set_tool(Tool::Lasso);
        drag(&mut c, Pointer::at(30.0, 30.0), Pointer::at(31.0, 31.0));
        assert_eq!(edits(&c).len(), 1);
        assert_eq!(c.selected(), Some(id));
    }

    #[test]
    fn clicking_a_region_selects_it_and_empty_space_deselects() {
        let mut c = loaded();
        c.choose_op(OpKind::Hint).unwrap();
        let id = rect(&mut c);
        c.select(None);
        click(&mut c, Pointer::at(40.0, 40.0));
        assert_eq!(c.selected(), Some(id));
        click(&mut c, Pointer::at(150.0, 80.0));
        assert_eq!(c.selected(), None);
        c.set_tool(Tool::Select);
        click(&mut c, Pointer::at(40.0, 40.0));
        assert_eq!(c.selected(), Some(id));
        click(&mut c, Pointer::at(150.0, 80.0));
        assert_eq!(c.selected(), None);
    }

    #[test]
    fn selecting_follows_the_edits_op() {
        let mut c = loaded();
        c.choose_op(OpKind::Scale).unwrap();
        let id = rect(&mut c);
        c.choose_op(OpKind::Hint).unwrap();
        assert_eq!(OpKind::of(edits(&c)[0].op()), OpKind::Hint);
        c.select(None);
        c.choose_op(OpKind::Rotate).unwrap();
        c.select(Some(id));
        assert_eq!(c.op_kind(), OpKind::Hint);
    }

    #[test]
    fn translate_drag_sets_destination_and_ghost_follows() {
        let mut c = loaded();
        let id = rect(&mut c);
        let before = c.editor().unwrap().can_undo();
        assert!(before);
        drag(&mut c, Pointer::at(40.0, 40.0), Pointer::at(140.0, 60.0));
        let e = &edits(&c)[0];
        let Op::Translate { dx, dy } = *e.op() else {
            panic!("still a translate")
        };
        assert!(close(dx, 0.5) && close(dy, 0.2), "{dx} {dy}");
        // The source region did not move.
        let size = c.editor().unwrap().document().image_size();
        assert_eq!(e.region().bounds().min(), size.to_normalized(20.0, 20.0));
        // Ghost center is under the pointer's offset.
        let g = e.ghost(size).unwrap();
        let gc = size.to_pixels(g.center());
        assert!(close(gc.0, 140.0) && close(gc.1, 60.0));
        // The whole drag is one undo step.
        assert!(c.undo());
        assert_eq!(edits(&c)[0].op(), &Op::Translate { dx: 0.0, dy: 0.0 });
        assert_eq!(c.selected(), Some(id));
        assert!(c.undo());
        assert!(edits(&c).is_empty());
    }

    #[test]
    fn translate_past_the_image_clamps_instead_of_cancelling() {
        let mut c = loaded();
        rect(&mut c);
        // The canvas margin lets the pointer leave the 200 x 100 image; a
        // translate is at most a full image width/height (core validates
        // -1..=1), so the drag clamps there and stays live.
        c.pointer_down(Pointer::at(40.0, 40.0));
        c.pointer_move(Pointer::at(500.0, -300.0));
        assert!(c.dragging());
        c.pointer_up(Pointer::at(500.0, -300.0));
        assert_eq!(edits(&c)[0].op(), &Op::Translate { dx: 1.0, dy: -1.0 });
    }

    #[test]
    fn clicking_a_ghost_selects_nothing_and_cannot_start_a_drag_on_it() {
        let mut c = loaded();
        let id = rect(&mut c);
        drag(&mut c, Pointer::at(40.0, 40.0), Pointer::at(140.0, 40.0));
        // The ghost now covers x 120..160; the source is still 20..60.
        for tool in Tool::ALL {
            c.set_tool(tool);
            c.select(None);
            click(&mut c, Pointer::at(140.0, 40.0));
            assert_eq!(c.selected(), None, "{tool:?}");
        }
        // With the edit selected, dragging on its ghost does not move the
        // translate (the drag draws a new region on original pixels).
        c.set_tool(Tool::Rect);
        c.select(Some(id));
        let op_before = *edits(&c)[0].op();
        drag(&mut c, Pointer::at(130.0, 30.0), Pointer::at(150.0, 50.0));
        assert_eq!(edits(&c)[0].op(), &op_before);
        assert_eq!(edits(&c).len(), 2);
        // Select tool on the ghost: nothing selected, nothing moved.
        c.set_tool(Tool::Select);
        let region_before = edits(&c)[0].region().clone();
        c.select(None);
        assert!(
            !c.pointer_down(Pointer::at(125.0, 25.0)),
            "no drag starts on a ghost"
        );
        drag(&mut c, Pointer::at(125.0, 25.0), Pointer::at(180.0, 90.0));
        assert_eq!(edits(&c)[0].region(), &region_before);
        assert_eq!(c.selected(), None);
        // And a Select drag on empty space draws nothing.
        assert_eq!(edits(&c).len(), 2);
    }

    #[test]
    fn select_tool_drag_moves_the_source_not_the_op() {
        let mut c = loaded();
        let id = rect(&mut c);
        drag(&mut c, Pointer::at(40.0, 40.0), Pointer::at(80.0, 40.0));
        let op = *edits(&c)[0].op();
        c.set_tool(Tool::Select);
        c.select(None);
        drag(&mut c, Pointer::at(30.0, 30.0), Pointer::at(130.0, 50.0));
        assert_eq!(c.selected(), Some(id));
        let e = &edits(&c)[0];
        assert_eq!(e.op(), &op);
        let size = c.editor().unwrap().document().image_size();
        let min = size.to_pixels(e.region().bounds().min());
        assert!(close(min.0, 120.0) && close(min.1, 40.0), "{min:?}");
        // One gesture, one step.
        assert!(c.undo());
        let min = size.to_pixels(edits(&c)[0].region().bounds().min());
        assert!(close(min.0, 20.0) && close(min.1, 20.0));
    }

    #[test]
    fn scale_corner_drag_and_shift_keeps_aspect() {
        let mut c = loaded();
        c.choose_op(OpKind::Scale).unwrap();
        rect(&mut c);
        // Corner 2 (bottom-right) of a 20..60 box about center 40,40.
        let handles = c.handles();
        assert_eq!(handles.len(), 4);
        assert_eq!(handles[2], (Handle::Corner(2), (60.0, 60.0)));
        drag(&mut c, Pointer::at(60.0, 60.0), Pointer::at(80.0, 50.0));
        let Op::Scale { sx, sy, anchor } = *edits(&c)[0].op() else {
            panic!("scale")
        };
        assert!(close(sx, 2.0) && close(sy, 0.5), "{sx} {sy}");
        assert_eq!(anchor, Anchor::Center);
        // The handle followed the pointer.
        assert_eq!(c.handles()[2].1, (80.0, 50.0));
        // Shift: uniform.
        drag(
            &mut c,
            Pointer::at(80.0, 50.0),
            Pointer::at(70.0, 90.0).with_shift(),
        );
        let Op::Scale { sx, sy, .. } = *edits(&c)[0].op() else {
            panic!("scale")
        };
        assert!(close(sx, sy), "{sx} {sy}");
        // Two drags, two steps, plus the add.
        assert!(c.undo());
        assert!(c.undo());
        assert_eq!(edits(&c)[0].op(), &Op::uniform_scale(1.0));
    }

    #[test]
    fn rotate_handle_drag_and_shift_snaps() {
        let mut c = loaded();
        c.choose_op(OpKind::Rotate).unwrap();
        rect(&mut c);
        let (h, pos) = c.handles()[0];
        assert_eq!(h, Handle::Rotate);
        // Center 40,40; box top 20; handle 28 px above that.
        assert!(close(pos.0, 40.0) && close(pos.1, 20.0 - ROTATE_HANDLE_OFFSET));
        // Drag to the right of the anchor: a quarter turn clockwise.
        drag(&mut c, Pointer::at(pos.0, pos.1), Pointer::at(90.0, 40.0));
        let Op::Rotate { degrees, .. } = *edits(&c)[0].op() else {
            panic!("rotate")
        };
        assert!(close(degrees, 90.0), "{degrees}");
        // The handle rode along.
        let pos = c.handles()[0].1;
        assert!(close(pos.0, 40.0 + 20.0 + ROTATE_HANDLE_OFFSET) && close(pos.1, 40.0));
        // Shift snaps to 15°: aim at ~100°.
        let target = geom::rotate_handle((40.0, 40.0), 20.0, 101.0, ROTATE_HANDLE_OFFSET);
        drag(
            &mut c,
            Pointer::at(pos.0, pos.1),
            Pointer {
                pos: target,
                shift: true,
            },
        );
        let Op::Rotate { degrees, .. } = *edits(&c)[0].op() else {
            panic!("rotate")
        };
        assert!(close(degrees, 105.0), "{degrees}");
    }

    #[test]
    fn mirror_toggle_flips_axis_one_step_each() {
        let mut c = loaded();
        c.choose_op(OpKind::Mirror).unwrap();
        rect(&mut c);
        assert_eq!(
            edits(&c)[0].op(),
            &Op::Mirror {
                axis: MirrorAxis::Horizontal
            }
        );
        c.choose_op(OpKind::Mirror).unwrap();
        assert_eq!(
            edits(&c)[0].op(),
            &Op::Mirror {
                axis: MirrorAxis::Vertical
            }
        );
        c.toggle_mirror_axis().unwrap();
        assert_eq!(c.mirror_axis(), MirrorAxis::Horizontal);
        assert!(c.undo());
        assert_eq!(
            edits(&c)[0].op(),
            &Op::Mirror {
                axis: MirrorAxis::Vertical
            }
        );
        assert_eq!(c.mirror_axis(), MirrorAxis::Vertical);
    }

    #[test]
    fn hint_typing_is_one_step() {
        let mut c = loaded();
        c.choose_op(OpKind::Hint).unwrap();
        rect(&mut c);
        c.end_hint_edit();
        for text in ["m", "ma", "mak", "make it red"] {
            c.set_hint(text).unwrap();
        }
        c.end_hint_edit();
        assert_eq!(edits(&c)[0].hint(), Some("make it red"));
        assert!(c.undo());
        assert_eq!(edits(&c)[0].hint(), None);
        assert_eq!(edits(&c).len(), 1);
    }

    #[test]
    fn delete_selected_and_undo_restores() {
        let mut c = loaded();
        let id = rect(&mut c);
        assert!(c.delete_selected());
        assert!(edits(&c).is_empty());
        assert!(!c.delete_selected());
        assert!(c.undo());
        assert_eq!(edits(&c)[0].id(), id);
    }

    #[test]
    fn cancel_reverts_a_drag_in_progress() {
        let mut c = loaded();
        rect(&mut c);
        c.pointer_down(Pointer::at(40.0, 40.0));
        c.pointer_move(Pointer::at(120.0, 40.0));
        assert_ne!(edits(&c)[0].op(), &Op::Translate { dx: 0.0, dy: 0.0 });
        assert!(c.cancel());
        assert_eq!(edits(&c)[0].op(), &Op::Translate { dx: 0.0, dy: 0.0 });
        assert!(!c.dragging());
        // Esc again clears the selection, then does nothing.
        assert!(c.cancel());
        assert_eq!(c.selected(), None);
        assert!(!c.cancel());
        // pointercancel reverts too.
        c.set_tool(Tool::Select);
        c.pointer_down(Pointer::at(40.0, 40.0));
        c.pointer_move(Pointer::at(90.0, 40.0));
        c.pointer_cancel();
        let size = c.editor().unwrap().document().image_size();
        let min = size.to_pixels(edits(&c)[0].region().bounds().min());
        assert!(close(min.0, 20.0));
        // Only the add is in history.
        assert!(c.undo());
        assert!(!c.can_undo());
    }

    #[test]
    fn tolerances_scale_with_the_view() {
        let mut c = loaded();
        c.choose_op(OpKind::Scale).unwrap();
        rect(&mut c);
        // At 4 screen px per image px a 3 image-px miss is 12 screen px:
        // outside the 10 px handle radius.
        c.set_view_scale(4.0);
        assert_eq!(c.cursor_at((63.0, 60.0)), Cursor::Crosshair);
        assert_eq!(c.cursor_at((62.0, 60.0)), Cursor::Resize);
        // And a 2 image-px drag is 8 screen px: a real drag, not a click.
        c.select(None);
        drag(&mut c, Pointer::at(100.0, 70.0), Pointer::at(102.0, 72.0));
        assert_eq!(edits(&c).len(), 2);
    }

    #[test]
    fn load_starts_a_fresh_session_keeping_tool_and_op() {
        let mut c = loaded();
        c.set_tool(Tool::Lasso);
        c.choose_op(OpKind::Hint).unwrap();
        c.set_tool(Tool::Rect);
        rect(&mut c);
        c.load(ImageSize::new(50, 50).unwrap());
        assert!(edits(&c).is_empty());
        assert!(!c.can_undo());
        assert_eq!(c.tool(), Tool::Rect);
        assert_eq!(c.op_kind(), OpKind::Hint);
    }

    #[test]
    fn undo_mid_drag_cancels_the_drag_first() {
        let mut c = loaded();
        rect(&mut c);
        c.pointer_down(Pointer::at(40.0, 40.0));
        c.pointer_move(Pointer::at(120.0, 40.0));
        assert!(c.dragging());
        assert!(c.undo());
        assert!(!c.dragging());
        // Only the drag was undone: the rect, the step before it, survives.
        assert_eq!(edits(&c).len(), 1);
        assert_eq!(edits(&c)[0].op(), &Op::Translate { dx: 0.0, dy: 0.0 });
        assert!(c.can_undo());
        assert!(!c.can_redo());
        // The next undo removes the rect.
        assert!(c.undo());
        assert!(edits(&c).is_empty());
    }

    #[test]
    fn redo_mid_drag_only_cancels_the_drag() {
        let mut c = loaded();
        let a = rect(&mut c);
        drag(&mut c, Pointer::at(120.0, 20.0), Pointer::at(160.0, 60.0));
        assert_eq!(edits(&c).len(), 2);
        assert!(c.undo());
        assert_eq!(edits(&c).len(), 1);
        assert!(c.can_redo());
        // Drag the remaining rect's translate, then redo mid-drag.
        c.select(Some(a));
        c.pointer_down(Pointer::at(40.0, 40.0));
        c.pointer_move(Pointer::at(120.0, 40.0));
        assert!(c.redo());
        assert!(!c.dragging());
        // The drag was reverted and nothing was redone.
        assert_eq!(edits(&c).len(), 1);
        assert_eq!(edits(&c)[0].op(), &Op::Translate { dx: 0.0, dy: 0.0 });
        assert!(c.can_redo());
        // Redo again restores the second rect.
        assert!(c.redo());
        assert_eq!(edits(&c).len(), 2);
    }

    #[test]
    fn clicking_the_selected_translates_source_keeps_its_destination() {
        let mut c = loaded();
        let id = rect(&mut c);
        drag(&mut c, Pointer::at(40.0, 40.0), Pointer::at(140.0, 60.0));
        let before = *edits(&c)[0].op();
        assert_eq!(before, Op::Translate { dx: 0.5, dy: 0.2 });
        // Undo depth, counted on a copy: the add plus the translate drag.
        let steps = |c: &Controller| {
            let mut c = c.clone();
            let mut n = 0;
            while c.undo() {
                n += 1;
            }
            n
        };
        let steps_before = steps(&c);
        assert_eq!(steps_before, 2);
        // A plain click inside the source, and a jittery one under the slop.
        click(&mut c, Pointer::at(40.0, 40.0));
        assert_eq!(edits(&c)[0].op(), &before);
        drag(&mut c, Pointer::at(40.0, 40.0), Pointer::at(42.0, 41.0));
        assert_eq!(edits(&c)[0].op(), &before);
        assert_eq!(steps(&c), steps_before, "a click records no undo step");
        assert_eq!(c.selected(), Some(id));
        // A real drag moves the destination relative to where it was.
        drag(&mut c, Pointer::at(40.0, 40.0), Pointer::at(60.0, 30.0));
        let Op::Translate { dx, dy } = *edits(&c)[0].op() else {
            panic!("still a translate")
        };
        assert!(close(dx, 0.6) && close(dy, 0.1), "{dx} {dy}");
        assert_eq!(steps(&c), steps_before + 1);
        // And that drag undoes back to the old destination.
        assert!(c.undo());
        assert_eq!(edits(&c)[0].op(), &before);
    }
}
