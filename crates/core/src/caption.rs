//! The caption block printed under the exported image (spec §Output → Image
//! layout), and the same text for "Copy caption as text".
//!
//! The caption is what carries the instructions to the model: chat apps
//! re-encode images, so nothing but visible text survives. It is therefore
//! plain English, in a fixed shape:
//!
//! ```text
//! paste2prompt edit list (user-authored)
//! Coordinates are % of image width/height. Edits refer to the original image.
//! #1  translate  region (12%,30%)-(40%,52%)                         →  move right 15%, up 4%
//! #2  rotate     region (60%,10%)-(80%,25%)                         →  rotate 90° clockwise about center (70%,18%)
//! #3  hint       lasso around (50%,70%) within (40%,60%)-(60%,80%)  →  "make this button red"
//! Overall: tighten the spacing between the cards
//! ```
//!
//! [`caption_lines`] returns structured [`CaptionLine`]s so the PNG renderer
//! can lay the edit rows out in columns with its own font metrics;
//! [`Caption::to_plain_text`] pads the same columns with spaces.
//!
//! Formatting rules:
//! - Coordinates and offsets are whole percentages of image width (x) and
//!   height (y); angles are whole degrees. Scale factors keep up to two
//!   decimals. A change too small to show at that precision reads "less
//!   than 1%" (or "less than 1°"), never as a no-op.
//! - A rectangle region is its top-left and bottom-right corners. A lasso
//!   region is its centroid plus its bounding box, both of its drawn curve
//!   ([`Region::outline`]), since a list of control points means nothing
//!   to a reader. A self-intersecting lasso, whose area
//!   centroid can fall outside the shape, prints its bounding-box center.
//! - Scale and rotate always print their pivot's position: `about center
//!   (x%,y%)` for a rectangle, `about bounding-box center (x%,y%)` for a
//!   lasso (so the pivot isn't confused with the printed centroid), or
//!   `about point (x%,y%)`. A mirror names its axis line, e.g. `across x=30%`.
//! - A rotation names its direction (clockwise or counter-clockwise) and is
//!   reduced to the equivalent turn of at most 180°.
//! - Hints are quoted, with runs of whitespace (including line breaks)
//!   collapsed to one space so each edit stays on one line. Double quotes
//!   inside a hint become single quotes.
//! - A named region prints its name, quoted like a hint, in place of the
//!   word `region` or `lasso`: `"logo" (12%,30%)-(40%,52%)`. A label edit
//!   (a name and nothing else) reads `→  named "logo"`. When any region has
//!   a name, a third header line ([`NAMES_HEADER`]) says the names can be
//!   used to refer to the regions.
//! - Image-level instructions follow as `Overall: …` only when they contain
//!   text. Each non-blank line of a multi-line instruction becomes its own
//!   caption line, indented under the first. Every Unicode line break (LF,
//!   CR, VT, FF, NEL, U+2028, U+2029) splits lines.

use std::fmt;

use crate::document::{Document, Edit};
use crate::geometry::{Point, Rect, Region};
use crate::op::{Anchor, MirrorAxis, Op};

/// The fixed two-line header every caption starts with.
pub const CAPTION_HEADER: [&str; 2] = [
    "paste2prompt edit list (user-authored)",
    "Coordinates are % of image width/height. Edits refer to the original image.",
];

/// The header line added after [`CAPTION_HEADER`] when at least one region
/// has a name. Captions without names keep the two-line header.
pub const NAMES_HEADER: &str =
    "Named regions can be referred to by name in the instructions and in your reply.";

/// Prefix of the image-level instructions line. A renderer laying out
/// [`CaptionLine::Overall`] itself draws this before the text and indents
/// [`CaptionLine::OverallContinued`] by its width.
pub const OVERALL_PREFIX: &str = "Overall: ";

/// Separator between an edit row's region column and its description column.
pub const ARROW: &str = "→";

/// The largest magnitude [`pct`] prints, in percent. [`Op::validate`] keeps
/// stored offsets far below this; the clamp only makes sure no `f64` can
/// overflow the formatting.
const PCT_LIMIT: f64 = 1_000_000.0;

/// One line of the caption block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptionLine {
    /// One of the two fixed [`CAPTION_HEADER`] lines, or [`NAMES_HEADER`].
    Header(&'static str),
    /// One edit, split into columns.
    Edit(EditLine),
    /// The first line of the image-level instructions, without the
    /// `Overall: ` prefix (see [`CaptionLine::text`]).
    Overall(String),
    /// A further line of multi-line instructions. Rendered indented to line up
    /// with the text after `Overall: `.
    OverallContinued(String),
}

impl CaptionLine {
    /// This line as unpadded text: an edit row's columns are joined with
    /// single spaces, and the `Overall: ` prefix or continuation indent is
    /// included. Use [`Caption::to_plain_text`] for column-aligned output.
    pub fn text(&self) -> String {
        match self {
            Self::Header(h) => (*h).to_owned(),
            Self::Edit(e) => format!(
                "{} {} {} {ARROW} {}",
                e.label(),
                e.op,
                e.region,
                e.description
            ),
            Self::Overall(t) => format!("{OVERALL_PREFIX}{t}"),
            Self::OverallContinued(t) => format!("{}{t}", continuation_indent()),
        }
    }
}

/// The columns of one edit row: `#number  op  region  →  description`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditLine {
    /// The 1-based edit number, matching the badge on the image (see
    /// [`Document::number_of`]).
    pub number: usize,
    /// The op's name ([`Op::name`]): `translate`, `scale`, `mirror`, `rotate`
    /// or `hint`. An op with a hint attached keeps its own name.
    pub op: &'static str,
    /// Where the region is, e.g. `region (12%,30%)-(40%,52%)` or
    /// `lasso around (50%,70%) within (40%,60%)-(60%,80%)`. A named region
    /// has its quoted name in place of `region` or `lasso`:
    /// `"logo" (12%,30%)-(40%,52%)`.
    pub region: String,
    /// What to do, in words, e.g. `move right 15%, up 4%`, with any hint
    /// quoted at the end.
    pub description: String,
}

impl EditLine {
    /// The number column as printed, e.g. `#3`.
    pub fn label(&self) -> String {
        format!("#{}", self.number)
    }

    fn from_edit(number: usize, edit: &Edit) -> Self {
        Self {
            number,
            op: edit.op().name(),
            region: region_column(edit),
            description: describe_edit(edit),
        }
    }
}

/// The caption for a document: the header, one row per edit in order, then
/// the image-level instructions if there are any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caption {
    lines: Vec<CaptionLine>,
}

impl Caption {
    /// Builds the caption for `doc`.
    pub fn from_document(doc: &Document) -> Self {
        let mut lines: Vec<CaptionLine> = CAPTION_HEADER
            .iter()
            .map(|h| CaptionLine::Header(h))
            .collect();
        if doc.edits().iter().any(|e| e.name().is_some()) {
            lines.push(CaptionLine::Header(NAMES_HEADER));
        }
        lines.extend(
            doc.edits()
                .iter()
                .enumerate()
                .map(|(i, e)| CaptionLine::Edit(EditLine::from_edit(i + 1, e))),
        );
        let mut instructions = instruction_lines(doc.instructions());
        if let Some(first) = instructions.next() {
            lines.push(CaptionLine::Overall(first.to_owned()));
            lines.extend(instructions.map(|l| CaptionLine::OverallContinued(l.to_owned())));
        }
        Self { lines }
    }

    /// The structured lines, in order.
    pub fn lines(&self) -> &[CaptionLine] {
        &self.lines
    }

    /// Consumes the caption, returning its lines.
    pub fn into_lines(self) -> Vec<CaptionLine> {
        self.lines
    }

    /// The caption as plain text for the clipboard, one caption line per text
    /// line (no trailing newline). Edit rows are padded with spaces so the
    /// op, region and description columns line up in a monospace font.
    pub fn to_plain_text(&self) -> String {
        let edits = || {
            self.lines.iter().filter_map(|l| match l {
                CaptionLine::Edit(e) => Some(e),
                _ => None,
            })
        };
        let width = |f: fn(&EditLine) -> usize| edits().map(f).max().unwrap_or(0);
        let label_w = width(|e| e.label().chars().count());
        let op_w = width(|e| e.op.chars().count());
        let region_w = width(|e| e.region.chars().count());

        let rows: Vec<String> = self
            .lines
            .iter()
            .map(|l| match l {
                CaptionLine::Edit(e) => format!(
                    "{:<label_w$}  {:<op_w$}  {:<region_w$}  {ARROW}  {}",
                    e.label(),
                    e.op,
                    e.region,
                    e.description
                ),
                other => other.text(),
            })
            .collect();
        rows.join("\n")
    }
}

impl fmt::Display for Caption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_plain_text())
    }
}

/// The structured caption lines for `doc`. Shorthand for
/// `Caption::from_document(doc).into_lines()`.
pub fn caption_lines(doc: &Document) -> Vec<CaptionLine> {
    Caption::from_document(doc).into_lines()
}

fn continuation_indent() -> String {
    " ".repeat(OVERALL_PREFIX.chars().count())
}

/// Every character a clipboard target or text renderer may treat as a line
/// break: LF, CR, VT, FF, NEL, and the Unicode line and paragraph separators.
const LINE_BREAKS: [char; 7] = [
    '\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}',
];

/// The non-blank lines of the instructions, trimmed. Any of [`LINE_BREAKS`]
/// ends a line (so `\r\n` is one break plus an empty line, which is dropped).
fn instruction_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split(LINE_BREAKS)
        .map(str::trim)
        .filter(|l| !l.is_empty())
}

/// A normalized value as a whole percentage, clamped to `±PCT_LIMIT` (NaN
/// prints as `0`). `-0` prints as `0`.
fn pct(v: f64) -> i64 {
    // Regions are within 0..=1 and ops are validated, so the clamp never
    // bites in practice. It keeps the result far from `i64::MIN`, so callers
    // can negate it or take its absolute value without overflow.
    let r = (v * 100.0).round().clamp(-PCT_LIMIT, PCT_LIMIT) as i64;
    if r == 0 { 0 } else { r }
}

fn point(p: Point) -> String {
    format!("({}%,{}%)", pct(p.x), pct(p.y))
}

fn rect(r: &Rect) -> String {
    format!("{}-{}", point(r.min()), point(r.max()))
}

fn describe_region(region: &Region) -> String {
    match region {
        Region::Rect(r) => format!("region {}", rect(r)),
        Region::Polygon(_) => {
            let bounds = region.bounds();
            format!(
                "lasso around {} within {}",
                point(centroid(&region.outline(), &bounds)),
                rect(&bounds)
            )
        }
    }
}

/// The area centroid of a lasso's flattened outline (where its pixels are
/// centered), which for a lopsided lasso differs from the bounding-box
/// center.
///
/// Falls back to the bounding-box center when the signed-area formula gives
/// a point that isn't finite or lies outside `bounds`. That happens for a
/// self-intersecting lasso (a freehand figure-eight): its lobes' signed areas
/// partly cancel, so the formula divides by a small net area and lands off
/// the shape, even off the image.
fn centroid(pts: &[Point], bounds: &Rect) -> Point {
    let n = pts.len();
    let (mut a2, mut cx, mut cy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (p, q) = (pts[i], pts[(i + 1) % n]);
        let cross = p.x * q.y - q.x * p.y;
        a2 += cross;
        cx += (p.x + q.x) * cross;
        cy += (p.y + q.y) * cross;
    }
    if a2.abs() < 1e-12 {
        // A sliver, or a self-intersecting lasso whose lobes nearly cancel,
        // has almost no signed area; use the bounding-box center rather than
        // divide by ~zero.
        return bounds.center();
    }
    let c = Point::new(cx / (3.0 * a2), cy / (3.0 * a2));
    if within(c, bounds) {
        c
    } else {
        bounds.center()
    }
}

/// Whether `p` is finite and inside `r`, allowing for float error at the
/// edges (a lasso's true centroid can't be outside its bounding box).
fn within(p: Point, r: &Rect) -> bool {
    const EPS: f64 = 1e-9;
    let (min, max) = (r.min(), r.max());
    p.x.is_finite()
        && p.y.is_finite()
        && (min.x - EPS..=max.x + EPS).contains(&p.x)
        && (min.y - EPS..=max.y + EPS).contains(&p.y)
}

/// The region column, with a named region's quoted name in place of its
/// leading `region` / `lasso` word.
fn region_column(edit: &Edit) -> String {
    let region = describe_region(edit.region());
    match (edit.name(), region.split_once(' ')) {
        (Some(name), Some((_, rest))) => format!("{} {rest}", quote_hint(name)),
        _ => region,
    }
}

fn describe_edit(edit: &Edit) -> String {
    let hint = edit.hint().map(quote_hint).filter(|h| h.len() > 2);
    match (edit.op(), hint) {
        (Op::Label, hint) => {
            let name = edit.name().map_or("(no name)".to_owned(), quote_hint);
            match hint {
                Some(h) => format!("named {name}; also {h}"),
                None => format!("named {name}"),
            }
        }
        (Op::Hint, Some(h)) => h,
        (Op::Hint, None) => "(no hint text)".to_owned(),
        (op, Some(h)) => format!("{}; also {h}", describe_op(op, edit.region())),
        (op, None) => describe_op(op, edit.region()),
    }
}

/// Quotes a hint or a name, collapsing whitespace so it stays on one line. Double
/// quotes inside the hint become single quotes, so the hint can't appear to
/// end early and the closing quote is always the last one on the row.
fn quote_hint(hint: &str) -> String {
    let words: Vec<&str> = hint.split_whitespace().collect();
    format!("\"{}\"", words.join(" ").replace('"', "'"))
}

fn describe_op(op: &Op, region: &Region) -> String {
    match *op {
        Op::Translate { dx, dy } => describe_translate(dx, dy),
        Op::Scale { sx, sy, anchor } => describe_scale(sx, sy, about(anchor, region)),
        Op::Mirror { axis } => {
            // The flip is about the bounding-box center (`Op::Mirror`); name
            // the line so a lasso's printed centroid isn't taken for it.
            let c = region.center();
            match axis {
                MirrorAxis::Horizontal => {
                    format!(
                        "mirror horizontally across x={}% (flip left to right)",
                        pct(c.x)
                    )
                }
                MirrorAxis::Vertical => {
                    format!(
                        "mirror vertically across y={}% (flip top to bottom)",
                        pct(c.y)
                    )
                }
            }
        }
        Op::Rotate { degrees, anchor } => describe_rotate(degrees, &about(anchor, region)),
        Op::Hint | Op::Label => String::new(),
    }
}

fn describe_translate(dx: f64, dy: f64) -> String {
    let (x, y) = (pct(dx), pct(dy));
    let mut parts = Vec::new();
    if x != 0 {
        let dir = if x > 0 { "right" } else { "left" };
        parts.push(format!("{dir} {}%", x.unsigned_abs()));
    }
    if y != 0 {
        let dir = if y > 0 { "down" } else { "up" };
        parts.push(format!("{dir} {}%", y.unsigned_abs()));
    }
    if parts.is_empty() {
        "move less than 1% (about in place)".to_owned()
    } else {
        format!("move {}", parts.join(", "))
    }
}

fn describe_scale(sx: f64, sy: f64, about: String) -> String {
    let (x, y) = (factor(sx), factor(sy));
    let amount = if x == y {
        x.phrase()
    } else {
        format!("width {}, height {}", x.phrase(), y.phrase())
    };
    format!("scale {amount} {about}")
}

fn describe_rotate(degrees: f64, about: &str) -> String {
    // Reduce to the equivalent turn in (-180, 180], so a drag that wound past
    // a full turn reads as the turn the model should actually make.
    let mut d = degrees.rem_euclid(360.0);
    if d > 180.0 {
        d -= 360.0;
    }
    let whole = d.round() as i64;
    match whole {
        0 => format!("rotate less than 1° {about}"),
        180 | -180 => format!("rotate 180° (half turn) {about}"),
        w if w > 0 => format!("rotate {w}° clockwise {about}"),
        w => format!("rotate {}° counter-clockwise {about}", -w),
    }
}

/// The pivot phrase for scale and rotate, with the pivot's position always
/// printed. [`Anchor::Center`] is the region's bounding-box center; for a
/// lasso that's named outright, since the row's printed point is the
/// lasso's centroid, which is usually somewhere else.
fn about(anchor: Anchor, region: &Region) -> String {
    match (anchor, region) {
        (Anchor::Point(p), _) => format!("about point {}", point(p)),
        (Anchor::Center, Region::Rect(_)) => format!("about center {}", point(region.center())),
        (Anchor::Center, Region::Polygon(_)) => {
            format!("about bounding-box center {}", point(region.center()))
        }
    }
}

/// One axis of a scale, as printed.
#[derive(Debug, PartialEq, Eq)]
enum Factor {
    /// The factor with up to two decimals and no trailing zeros (`1.5`, `2`,
    /// `0.75`, `<0.01`). Exactly 1 prints as `1`.
    Times(String),
    /// Within half a percent of 1 but not exactly 1: `"larger"` or
    /// `"smaller"`. Printing `×1` would read as a no-op, so this says "less
    /// than 1% larger/smaller", like translate's and rotate's "less than".
    Slight(&'static str),
}

impl Factor {
    fn phrase(&self) -> String {
        match self {
            Self::Times(f) => format!("×{f}"),
            Self::Slight(dir) => format!("less than 1% {dir}"),
        }
    }
}

fn factor(v: f64) -> Factor {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    match s {
        "0" => Factor::Times("<0.01".to_owned()),
        "1" if v > 1.0 => Factor::Slight("larger"),
        "1" if v < 1.0 => Factor::Slight("smaller"),
        s => Factor::Times(s.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::ImageSize;
    use crate::history::Editor;

    fn editor() -> Editor {
        Editor::new(ImageSize::new(1600, 900).unwrap())
    }

    fn rect_region(x1: f64, y1: f64, x2: f64, y2: f64) -> Region {
        Region::rect(Point::new(x1, y1), Point::new(x2, y2)).unwrap()
    }

    fn text(ed: &Editor) -> String {
        Caption::from_document(ed.document()).to_plain_text()
    }

    #[test]
    fn empty_document_is_just_the_header() {
        let ed = editor();
        insta::assert_snapshot!(text(&ed), @r"
        paste2prompt edit list (user-authored)
        Coordinates are % of image width/height. Edits refer to the original image.
        ");
        assert_eq!(
            caption_lines(ed.document()),
            vec![
                CaptionLine::Header(CAPTION_HEADER[0]),
                CaptionLine::Header(CAPTION_HEADER[1]),
            ]
        );
        assert!(CAPTION_HEADER[0].starts_with(crate::APP_NAME));
    }

    #[test]
    fn spec_example() {
        let mut ed = editor();
        ed.add_edit(
            rect_region(0.12, 0.30, 0.40, 0.52),
            Op::Translate {
                dx: 0.15,
                dy: -0.04,
            },
            None,
        )
        .unwrap();
        ed.add_edit(
            rect_region(0.60, 0.10, 0.80, 0.25),
            Op::Rotate {
                degrees: 90.0,
                anchor: Anchor::Center,
            },
            None,
        )
        .unwrap();
        ed.add_edit(
            Region::polygon([
                Point::new(0.40, 0.60),
                Point::new(0.60, 0.60),
                Point::new(0.60, 0.80),
                Point::new(0.40, 0.80),
            ])
            .unwrap(),
            Op::Hint,
            Some("make this button red".into()),
        )
        .unwrap();
        ed.set_instructions("tighten the spacing between the cards");
        insta::assert_snapshot!(text(&ed), @r#"
        paste2prompt edit list (user-authored)
        Coordinates are % of image width/height. Edits refer to the original image.
        #1  translate  region (12%,30%)-(40%,52%)                         →  move right 15%, up 4%
        #2  rotate     region (60%,10%)-(80%,25%)                         →  rotate 90° clockwise about center (70%,18%)
        #3  hint       lasso around (50%,70%) within (38%,57%)-(63%,83%)  →  "make this button red"
        Overall: tighten the spacing between the cards
        "#);
    }

    #[test]
    fn structured_lines_carry_columns() {
        let mut ed = editor();
        ed.add_edit(
            rect_region(0.1, 0.2, 0.3, 0.4),
            Op::Mirror {
                axis: MirrorAxis::Vertical,
            },
            None,
        )
        .unwrap();
        ed.set_instructions("one\ntwo");
        let lines = caption_lines(ed.document());
        assert_eq!(
            lines[2],
            CaptionLine::Edit(EditLine {
                number: 1,
                op: "mirror",
                region: "region (10%,20%)-(30%,40%)".into(),
                description: "mirror vertically across y=30% (flip top to bottom)".into(),
            })
        );
        assert_eq!(lines[3], CaptionLine::Overall("one".into()));
        assert_eq!(lines[4], CaptionLine::OverallContinued("two".into()));
        assert_eq!(
            lines[2].text(),
            "#1 mirror region (10%,20%)-(30%,40%) → mirror vertically across y=30% (flip top to bottom)"
        );
        assert_eq!(lines[3].text(), "Overall: one");
        assert_eq!(lines[4].text(), "         two");
    }

    #[test]
    fn every_op() {
        let mut ed = editor();
        let r = || rect_region(0.2, 0.2, 0.4, 0.3);
        let ops = [
            Op::Translate { dx: -0.2, dy: 0.0 },
            Op::Translate { dx: 0.0, dy: 0.333 },
            Op::Translate {
                dx: 0.004,
                dy: -0.004,
            },
            Op::uniform_scale(1.5),
            Op::uniform_scale(2.0),
            Op::Scale {
                sx: 1.25,
                sy: 0.75,
                anchor: Anchor::Point(Point::new(0.1, 0.9)),
            },
            Op::Mirror {
                axis: MirrorAxis::Horizontal,
            },
            Op::Mirror {
                axis: MirrorAxis::Vertical,
            },
            Op::Rotate {
                degrees: 15.4,
                anchor: Anchor::Center,
            },
            Op::Rotate {
                degrees: -45.0,
                anchor: Anchor::Point(Point::new(0.5, 0.5)),
            },
            Op::Rotate {
                degrees: 270.0,
                anchor: Anchor::Center,
            },
            Op::Rotate {
                degrees: 180.0,
                anchor: Anchor::Center,
            },
        ];
        for op in ops {
            ed.add_edit(r(), op, None).unwrap();
        }
        insta::assert_snapshot!(text(&ed), @r"
        paste2prompt edit list (user-authored)
        Coordinates are % of image width/height. Edits refer to the original image.
        #1   translate  region (20%,20%)-(40%,30%)  →  move left 20%
        #2   translate  region (20%,20%)-(40%,30%)  →  move down 33%
        #3   translate  region (20%,20%)-(40%,30%)  →  move less than 1% (about in place)
        #4   scale      region (20%,20%)-(40%,30%)  →  scale ×1.5 about center (30%,25%)
        #5   scale      region (20%,20%)-(40%,30%)  →  scale ×2 about center (30%,25%)
        #6   scale      region (20%,20%)-(40%,30%)  →  scale width ×1.25, height ×0.75 about point (10%,90%)
        #7   mirror     region (20%,20%)-(40%,30%)  →  mirror horizontally across x=30% (flip left to right)
        #8   mirror     region (20%,20%)-(40%,30%)  →  mirror vertically across y=25% (flip top to bottom)
        #9   rotate     region (20%,20%)-(40%,30%)  →  rotate 15° clockwise about center (30%,25%)
        #10  rotate     region (20%,20%)-(40%,30%)  →  rotate 45° counter-clockwise about point (50%,50%)
        #11  rotate     region (20%,20%)-(40%,30%)  →  rotate 90° counter-clockwise about center (30%,25%)
        #12  rotate     region (20%,20%)-(40%,30%)  →  rotate 180° (half turn) about center (30%,25%)
        ");
    }

    #[test]
    fn lasso_uses_centroid_and_bounding_box() {
        let mut ed = editor();
        // A right triangle: its centroid (a third of the way in) is not the
        // bounding-box center.
        let tri = Region::polygon([
            Point::new(0.1, 0.1),
            Point::new(0.7, 0.1),
            Point::new(0.1, 0.4),
        ])
        .unwrap();
        ed.add_edit(tri, Op::uniform_scale(0.5), None).unwrap();
        insta::assert_snapshot!(text(&ed), @r"
        paste2prompt edit list (user-authored)
        Coordinates are % of image width/height. Edits refer to the original image.
        #1  scale  lasso around (32%,20%) within (6%,5%)-(70%,41%)  →  scale ×0.5 about bounding-box center (38%,23%)
        ");
    }

    #[test]
    fn near_degenerate_lasso_falls_back_to_box_center() {
        let sliver = Region::polygon([
            Point::new(0.1, 0.5),
            Point::new(0.5, 0.5),
            Point::new(0.3, 0.5 + 1e-14),
        ])
        .unwrap();
        assert_eq!(
            describe_region(&sliver),
            "lasso around (30%,50%) within (10%,50%)-(50%,50%)"
        );
    }

    #[test]
    fn figure_eight_lasso_centroid_stays_in_its_box() {
        // Self-intersecting outlines (a bow tie / figure-eight) whose lobes
        // nearly cancel. On these straight-edged outlines the raw
        // signed-area centroid lands far left of the shape (at about -50% and
        // -13310% of image width respectively), so `centroid` falls back to
        // the bounding-box center.
        let lopsided = [
            Point::new(0.1, 0.1),
            Point::new(0.3, 0.3),
            Point::new(0.3, 0.1),
            Point::new(0.1, 0.32),
        ];
        let near_cancel = [
            Point::new(0.1, 0.1),
            Point::new(0.3, 0.3),
            Point::new(0.3, 0.1001),
            Point::new(0.1, 0.3),
        ];
        for pts in [lopsided, near_cancel] {
            let bounds =
                Rect::from_corners(Point::new(0.1, 0.1), Point::new(0.3, pts[3].y)).unwrap();
            let c = centroid(&pts, &bounds);
            assert_eq!(c, bounds.center());
            // As lassos, the same points are control points of a curve,
            // whose centroid also stays inside the curve's box.
            let region = Region::polygon(pts).unwrap();
            let bounds = region.bounds();
            let c = centroid(&region.outline(), &bounds);
            assert!(within(c, &bounds), "{c:?} outside {bounds:?}");
        }
        let region = Region::polygon(lopsided).unwrap();
        assert_eq!(
            describe_region(&region),
            "lasso around (20%,21%) within (8%,10%)-(32%,32%)"
        );
        let region = Region::polygon(near_cancel).unwrap();
        assert_eq!(
            describe_region(&region),
            "lasso around (20%,20%) within (8%,10%)-(32%,30%)"
        );
    }

    #[test]
    fn huge_values_neither_overflow_nor_panic() {
        // Op::validate now rejects these on load (translate within -1..=1),
        // so build the ops directly: this is defense in depth for the text.
        let r = rect_region(0.1, 0.1, 0.2, 0.2);
        let translate = |dx, dy| describe_op(&Op::Translate { dx, dy }, &r);
        assert_eq!(translate(-1e18, 0.0), "move left 1000000%");
        assert_eq!(translate(0.0, 1e18), "move down 1000000%");
        assert_eq!(
            translate(f64::MIN, f64::MAX),
            "move left 1000000%, down 1000000%"
        );
        assert_eq!(translate(f64::NAN, f64::NEG_INFINITY), "move up 1000000%");
        assert_eq!(pct(f64::NAN), 0);
        assert_eq!(pct(f64::INFINITY), 1_000_000);
        assert_eq!(pct(f64::NEG_INFINITY), -1_000_000);
        let anchor = Anchor::Point(Point::new(-1e300, 1e300));
        assert_eq!(
            describe_op(
                &Op::Rotate {
                    degrees: 1e300,
                    anchor
                },
                &r
            )
            .split(" about ")
            .nth(1),
            Some("point (-1000000%,1000000%)")
        );
        assert!(describe_op(&Op::uniform_scale(1e300), &r).starts_with("scale ×1"));
    }

    #[test]
    fn scale_near_one_is_not_a_no_op() {
        let r = rect_region(0.2, 0.2, 0.4, 0.4);
        let scale = |sx, sy| {
            describe_op(
                &Op::Scale {
                    sx,
                    sy,
                    anchor: Anchor::Center,
                },
                &r,
            )
        };
        assert_eq!(scale(1.0, 1.0), "scale ×1 about center (30%,30%)");
        assert_eq!(
            scale(1.004, 1.004),
            "scale less than 1% larger about center (30%,30%)"
        );
        assert_eq!(
            scale(0.996, 0.996),
            "scale less than 1% smaller about center (30%,30%)"
        );
        // Non-uniform stays visible even when both axes round to 1.
        assert_eq!(
            scale(1.001, 0.999),
            "scale width less than 1% larger, height less than 1% smaller about center (30%,30%)"
        );
        assert_eq!(
            scale(1.5, 1.002),
            "scale width ×1.5, height less than 1% larger about center (30%,30%)"
        );
        assert_eq!(
            scale(1.0, 0.999),
            "scale width ×1, height less than 1% smaller about center (30%,30%)"
        );
    }

    #[test]
    fn lasso_pivots_name_the_bounding_box_center() {
        let mut ed = editor();
        // Centroid (30%,20%); bounding-box center (40%,25%).
        let tri = || {
            Region::polygon([
                Point::new(0.1, 0.1),
                Point::new(0.7, 0.1),
                Point::new(0.1, 0.4),
            ])
            .unwrap()
        };
        ed.add_edit(
            tri(),
            Op::Rotate {
                degrees: 90.0,
                anchor: Anchor::Center,
            },
            None,
        )
        .unwrap();
        ed.add_edit(
            tri(),
            Op::Mirror {
                axis: MirrorAxis::Horizontal,
            },
            None,
        )
        .unwrap();
        ed.add_edit(
            tri(),
            Op::Mirror {
                axis: MirrorAxis::Vertical,
            },
            None,
        )
        .unwrap();
        ed.add_edit(
            tri(),
            Op::Scale {
                sx: 2.0,
                sy: 2.0,
                anchor: Anchor::Point(Point::new(0.1, 0.1)),
            },
            None,
        )
        .unwrap();
        insta::assert_snapshot!(text(&ed), @r"
        paste2prompt edit list (user-authored)
        Coordinates are % of image width/height. Edits refer to the original image.
        #1  rotate  lasso around (32%,20%) within (6%,5%)-(70%,41%)  →  rotate 90° clockwise about bounding-box center (38%,23%)
        #2  mirror  lasso around (32%,20%) within (6%,5%)-(70%,41%)  →  mirror horizontally across x=38% (flip left to right)
        #3  mirror  lasso around (32%,20%) within (6%,5%)-(70%,41%)  →  mirror vertically across y=23% (flip top to bottom)
        #4  scale   lasso around (32%,20%) within (6%,5%)-(70%,41%)  →  scale ×2 about point (10%,10%)
        ");
    }

    #[test]
    fn hint_quotes_cannot_break_out() {
        let r = rect_region(0.1, 0.1, 0.2, 0.2);
        let mut ed = editor();
        ed.add_edit(
            r.clone(),
            Op::Hint,
            Some(r#"say "done" then"  #2 translate"#.into()),
        )
        .unwrap();
        let Some(CaptionLine::Edit(e)) = caption_lines(ed.document()).pop() else {
            panic!("expected an edit row")
        };
        assert_eq!(e.description, r#""say 'done' then' #2 translate""#);
        assert_eq!(e.description.matches('"').count(), 2);
    }

    #[test]
    fn instructions_split_on_every_unicode_line_break() {
        let mut ed = editor();
        ed.set_instructions(
            "one\u{2028}two\u{2029}three\u{85}four\u{0B}five\u{0C}six\r\nseven\rlast",
        );
        let lines = caption_lines(ed.document());
        assert_eq!(lines[2], CaptionLine::Overall("one".into()));
        let rest: Vec<_> = lines[3..]
            .iter()
            .map(|l| match l {
                CaptionLine::OverallContinued(t) => t.as_str(),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            rest,
            ["two", "three", "four", "five", "six", "seven", "last"]
        );
        let text = text(&ed);
        assert!(!text.contains(&LINE_BREAKS[2..]));
        assert_eq!(text.lines().count(), 2 + 8);
    }

    #[test]
    fn public_layout_constants() {
        assert_eq!(ARROW, "→");
        assert_eq!(OVERALL_PREFIX, "Overall: ");
        assert_eq!(continuation_indent().len(), OVERALL_PREFIX.len());
    }

    #[test]
    fn hints_combined_with_ops() {
        let mut ed = editor();
        ed.add_edit(
            rect_region(0.05, 0.05, 0.25, 0.15),
            Op::Translate { dx: 0.5, dy: 0.1 },
            Some("and make it blue".into()),
        )
        .unwrap();
        ed.add_edit(
            rect_region(0.5, 0.5, 0.9, 0.9),
            Op::Rotate {
                degrees: -30.0,
                anchor: Anchor::Center,
            },
            Some("keep the  shadow\nunder it".into()),
        )
        .unwrap();
        ed.add_edit(
            rect_region(0.3, 0.3, 0.35, 0.35),
            Op::Hint,
            Some(r#"replace with the word "Save""#.into()),
        )
        .unwrap();
        ed.add_edit(rect_region(0.3, 0.6, 0.4, 0.7), Op::Hint, None)
            .unwrap();
        ed.add_edit(
            rect_region(0.0, 0.0, 1.0, 1.0),
            Op::Hint,
            Some("   \n ".into()),
        )
        .unwrap();
        insta::assert_snapshot!(text(&ed), @r#"
        paste2prompt edit list (user-authored)
        Coordinates are % of image width/height. Edits refer to the original image.
        #1  translate  region (5%,5%)-(25%,15%)    →  move right 50%, down 10%; also "and make it blue"
        #2  rotate     region (50%,50%)-(90%,90%)  →  rotate 30° counter-clockwise about center (70%,70%); also "keep the shadow under it"
        #3  hint       region (30%,30%)-(35%,35%)  →  "replace with the word 'Save'"
        #4  hint       region (30%,60%)-(40%,70%)  →  (no hint text)
        #5  hint       region (0%,0%)-(100%,100%)  →  (no hint text)
        "#);
    }

    #[test]
    fn multi_line_instructions() {
        let mut ed = editor();
        ed.add_edit(
            rect_region(0.1, 0.1, 0.2, 0.2),
            Op::Hint,
            Some("bold".into()),
        )
        .unwrap();
        ed.set_instructions(
            "  Match the brand palette.\r\n\n\n  - keep the logo\r- drop the footer  \n\n",
        );
        insta::assert_snapshot!(text(&ed), @r#"
        paste2prompt edit list (user-authored)
        Coordinates are % of image width/height. Edits refer to the original image.
        #1  hint  region (10%,10%)-(20%,20%)  →  "bold"
        Overall: Match the brand palette.
                 - keep the logo
                 - drop the footer
        "#);
    }

    #[test]
    fn blank_instructions_have_no_overall_line() {
        let mut ed = editor();
        ed.set_instructions(" \n\t\r\n ");
        let lines = caption_lines(ed.document());
        assert_eq!(lines.len(), 2);
        assert!(!text(&ed).contains("Overall"));
    }

    #[test]
    fn rounding_to_whole_numbers() {
        assert_eq!(pct(0.125), 13);
        assert_eq!(pct(0.124), 12);
        assert_eq!(pct(-0.004), 0);
        let times = |s: &str| Factor::Times(s.to_owned());
        assert_eq!(factor(1.0), times("1"));
        assert_eq!(factor(1.505), times("1.5"));
        assert_eq!(factor(0.333), times("0.33"));
        assert_eq!(factor(0.001), times("<0.01"));
        assert_eq!(
            describe_rotate(0.4, "about center"),
            "rotate less than 1° about center"
        );
        assert_eq!(
            describe_rotate(89.6, "about center"),
            "rotate 90° clockwise about center"
        );
        assert_eq!(
            describe_rotate(-450.0, "about center"),
            "rotate 90° counter-clockwise about center"
        );
        assert_eq!(
            describe_rotate(-180.0, "about center"),
            "rotate 180° (half turn) about center"
        );
    }

    #[test]
    fn display_matches_plain_text() {
        let mut ed = editor();
        ed.set_instructions("x");
        let c = Caption::from_document(ed.document());
        assert_eq!(c.to_string(), c.to_plain_text());
        assert!(!c.to_plain_text().ends_with('\n'));
    }

    #[test]
    fn named_regions_and_labels() {
        let mut ed = editor();
        ed.add_named_edit(
            rect_region(0.0, 0.0, 1.0, 0.08),
            Op::Label,
            None,
            Some("nav bar".into()),
        )
        .unwrap();
        ed.add_named_edit(
            rect_region(0.12, 0.30, 0.40, 0.52),
            Op::Translate {
                dx: 0.15,
                dy: -0.04,
            },
            None,
            Some("logo".into()),
        )
        .unwrap();
        ed.add_named_edit(
            Region::polygon([
                Point::new(0.1, 0.1),
                Point::new(0.7, 0.1),
                Point::new(0.1, 0.4),
            ])
            .unwrap(),
            Op::Rotate {
                degrees: 90.0,
                anchor: Anchor::Center,
            },
            Some("keep it crisp".into()),
            Some("hero image".into()),
        )
        .unwrap();
        ed.add_named_edit(
            rect_region(0.6, 0.6, 0.8, 0.7),
            Op::Label,
            Some("leave as is".into()),
            Some("search box".into()),
        )
        .unwrap();
        ed.add_edit(
            rect_region(0.3, 0.6, 0.4, 0.7),
            Op::Hint,
            Some("unnamed regions still say region".into()),
        )
        .unwrap();
        ed.set_instructions("put the logo next to the search box");
        insta::assert_snapshot!(text(&ed), @r#"
        paste2prompt edit list (user-authored)
        Coordinates are % of image width/height. Edits refer to the original image.
        Named regions can be referred to by name in the instructions and in your reply.
        #1  label      "nav bar" (0%,0%)-(100%,8%)                             →  named "nav bar"
        #2  translate  "logo" (12%,30%)-(40%,52%)                              →  move right 15%, up 4%
        #3  rotate     "hero image" around (32%,20%) within (6%,5%)-(70%,41%)  →  rotate 90° clockwise about bounding-box center (38%,23%); also "keep it crisp"
        #4  label      "search box" (60%,60%)-(80%,70%)                        →  named "search box"; also "leave as is"
        #5  hint       region (30%,60%)-(40%,70%)                              →  "unnamed regions still say region"
        Overall: put the logo next to the search box
        "#);
        let lines = caption_lines(ed.document());
        assert_eq!(lines[2], CaptionLine::Header(NAMES_HEADER));
        assert!(matches!(&lines[3], CaptionLine::Edit(e) if e.op == "label"));
    }

    #[test]
    fn names_header_only_when_something_is_named() {
        let mut ed = editor();
        let a = ed
            .add_edit(rect_region(0.1, 0.1, 0.2, 0.2), Op::Hint, Some("x".into()))
            .unwrap();
        assert!(!text(&ed).contains(NAMES_HEADER));
        ed.set_name(a, Some("box".into())).unwrap();
        assert_eq!(text(&ed).lines().nth(2), Some(NAMES_HEADER));
        ed.set_name(a, None).unwrap();
        assert!(!text(&ed).contains(NAMES_HEADER));
    }

    #[test]
    fn name_quotes_cannot_break_out() {
        let mut ed = editor();
        ed.add_named_edit(
            rect_region(0.1, 0.1, 0.2, 0.2),
            Op::Label,
            None,
            Some(r#"the "big"   button"#.into()),
        )
        .unwrap();
        let Some(CaptionLine::Edit(e)) = caption_lines(ed.document()).pop() else {
            panic!("expected an edit row")
        };
        assert_eq!(e.region, r#""the 'big' button" (10%,10%)-(20%,20%)"#);
        assert_eq!(e.description, r#"named "the 'big' button""#);
        assert_eq!(e.region.matches('"').count(), 2);
    }
}
