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
//! #2  rotate     region (60%,10%)-(80%,25%)                         →  rotate 90° clockwise about center
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
//!   decimals.
//! - A rectangle region is its top-left and bottom-right corners. A lasso
//!   region is its centroid plus its bounding box, since a list of lasso
//!   points means nothing to a reader.
//! - A rotation names its direction (clockwise or counter-clockwise) and is
//!   reduced to the equivalent turn of at most 180°.
//! - Hints are quoted, with runs of whitespace (including line breaks)
//!   collapsed to one space so each edit stays on one line.
//! - Image-level instructions follow as `Overall: …` only when they contain
//!   text. Each non-blank line of a multi-line instruction becomes its own
//!   caption line, indented under the first.

use std::fmt;

use crate::document::{Document, Edit};
use crate::geometry::{Point, Polygon, Rect, Region};
use crate::op::{Anchor, MirrorAxis, Op};

/// The fixed two-line header every caption starts with.
pub const CAPTION_HEADER: [&str; 2] = [
    "paste2prompt edit list (user-authored)",
    "Coordinates are % of image width/height. Edits refer to the original image.",
];

/// Prefix of the image-level instructions line.
const OVERALL_PREFIX: &str = "Overall: ";

/// Separator between the region column and the description column.
const ARROW: &str = "→";

/// One line of the caption block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptionLine {
    /// One of the two fixed [`CAPTION_HEADER`] lines.
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
    /// `lasso around (50%,70%) within (40%,60%)-(60%,80%)`.
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
            region: describe_region(edit.region()),
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

/// The non-blank lines of the instructions, trimmed. `\r\n` and lone `\r`
/// count as line breaks.
fn instruction_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split(['\n', '\r'])
        .map(str::trim)
        .filter(|l| !l.is_empty())
}

/// A normalized value as a whole percentage. `-0` prints as `0`.
fn pct(v: f64) -> i64 {
    // `as` saturates on out-of-range values; regions are within 0..=1 and
    // ops are validated finite, so this never loses meaning in practice.
    let r = (v * 100.0).round() as i64;
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
        Region::Polygon(p) => {
            let bounds = region.bounds();
            format!(
                "lasso around {} within {}",
                point(centroid(p, &bounds)),
                rect(&bounds)
            )
        }
    }
}

/// The area centroid of a lasso polygon (where its pixels are centered),
/// which for a lopsided lasso differs from the bounding-box center.
fn centroid(polygon: &Polygon, bounds: &Rect) -> Point {
    let pts = polygon.points();
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
    Point::new(cx / (3.0 * a2), cy / (3.0 * a2))
}

fn describe_edit(edit: &Edit) -> String {
    let hint = edit.hint().map(quote_hint).filter(|h| h.len() > 2);
    match (edit.op(), hint) {
        (Op::Hint, Some(h)) => h,
        (Op::Hint, None) => "(no hint text)".to_owned(),
        (op, Some(h)) => format!("{}; also {h}", describe_op(op)),
        (op, None) => describe_op(op),
    }
}

/// Quotes a hint, collapsing whitespace so it stays on one line.
fn quote_hint(hint: &str) -> String {
    let words: Vec<&str> = hint.split_whitespace().collect();
    format!("\"{}\"", words.join(" "))
}

fn describe_op(op: &Op) -> String {
    match *op {
        Op::Translate { dx, dy } => describe_translate(dx, dy),
        Op::Scale { sx, sy, anchor } => {
            let (x, y) = (factor(sx), factor(sy));
            let amount = if x == y {
                format!("×{x}")
            } else {
                format!("width ×{x}, height ×{y}")
            };
            format!("scale {amount} {}", about(anchor))
        }
        Op::Mirror { axis } => match axis {
            MirrorAxis::Horizontal => "mirror horizontally (flip left to right)".to_owned(),
            MirrorAxis::Vertical => "mirror vertically (flip top to bottom)".to_owned(),
        },
        Op::Rotate { degrees, anchor } => describe_rotate(degrees, anchor),
        Op::Hint => String::new(),
    }
}

fn describe_translate(dx: f64, dy: f64) -> String {
    let (x, y) = (pct(dx), pct(dy));
    let mut parts = Vec::new();
    if x != 0 {
        let dir = if x > 0 { "right" } else { "left" };
        parts.push(format!("{dir} {}%", x.abs()));
    }
    if y != 0 {
        let dir = if y > 0 { "down" } else { "up" };
        parts.push(format!("{dir} {}%", y.abs()));
    }
    if parts.is_empty() {
        "move less than 1% (about in place)".to_owned()
    } else {
        format!("move {}", parts.join(", "))
    }
}

fn describe_rotate(degrees: f64, anchor: Anchor) -> String {
    // Reduce to the equivalent turn in (-180, 180], so a drag that wound past
    // a full turn reads as the turn the model should actually make.
    let mut d = degrees.rem_euclid(360.0);
    if d > 180.0 {
        d -= 360.0;
    }
    let whole = d.round() as i64;
    let about = about(anchor);
    match whole {
        0 => format!("rotate less than 1° {about}"),
        180 | -180 => format!("rotate 180° (half turn) {about}"),
        w if w > 0 => format!("rotate {w}° clockwise {about}"),
        w => format!("rotate {}° counter-clockwise {about}", -w),
    }
}

/// The pivot phrase for scale and rotate.
fn about(anchor: Anchor) -> String {
    match anchor {
        Anchor::Center => "about center".to_owned(),
        Anchor::Point(p) => format!("about point {}", point(p)),
    }
}

/// A scale factor with up to two decimals and no trailing zeros (`1.5`, `2`,
/// `0.75`).
fn factor(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "0" {
        "<0.01".to_owned()
    } else {
        s.to_owned()
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
        #2  rotate     region (60%,10%)-(80%,25%)                         →  rotate 90° clockwise about center
        #3  hint       lasso around (50%,70%) within (40%,60%)-(60%,80%)  →  "make this button red"
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
                description: "mirror vertically (flip top to bottom)".into(),
            })
        );
        assert_eq!(lines[3], CaptionLine::Overall("one".into()));
        assert_eq!(lines[4], CaptionLine::OverallContinued("two".into()));
        assert_eq!(
            lines[2].text(),
            "#1 mirror region (10%,20%)-(30%,40%) → mirror vertically (flip top to bottom)"
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
        #4   scale      region (20%,20%)-(40%,30%)  →  scale ×1.5 about center
        #5   scale      region (20%,20%)-(40%,30%)  →  scale ×2 about center
        #6   scale      region (20%,20%)-(40%,30%)  →  scale width ×1.25, height ×0.75 about point (10%,90%)
        #7   mirror     region (20%,20%)-(40%,30%)  →  mirror horizontally (flip left to right)
        #8   mirror     region (20%,20%)-(40%,30%)  →  mirror vertically (flip top to bottom)
        #9   rotate     region (20%,20%)-(40%,30%)  →  rotate 15° clockwise about center
        #10  rotate     region (20%,20%)-(40%,30%)  →  rotate 45° counter-clockwise about point (50%,50%)
        #11  rotate     region (20%,20%)-(40%,30%)  →  rotate 90° counter-clockwise about center
        #12  rotate     region (20%,20%)-(40%,30%)  →  rotate 180° (half turn) about center
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
        #1  scale  lasso around (30%,20%) within (10%,10%)-(70%,40%)  →  scale ×0.5 about center
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
        #2  rotate     region (50%,50%)-(90%,90%)  →  rotate 30° counter-clockwise about center; also "keep the shadow under it"
        #3  hint       region (30%,30%)-(35%,35%)  →  "replace with the word "Save""
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
        assert_eq!(factor(1.0), "1");
        assert_eq!(factor(1.505), "1.5");
        assert_eq!(factor(0.333), "0.33");
        assert_eq!(factor(0.001), "<0.01");
        assert_eq!(
            describe_rotate(0.4, Anchor::Center),
            "rotate less than 1° about center"
        );
        assert_eq!(
            describe_rotate(89.6, Anchor::Center),
            "rotate 90° clockwise about center"
        );
        assert_eq!(
            describe_rotate(-450.0, Anchor::Center),
            "rotate 90° counter-clockwise about center"
        );
        assert_eq!(
            describe_rotate(-180.0, Anchor::Center),
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
}
