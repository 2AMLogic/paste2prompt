//! Edits and the document that holds them, plus the JSON format.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::geometry::{ImageSize, Region};
use crate::op::{Ghost, Op, OpError};

/// Version of the JSON document format written by [`Document::to_json`].
/// Bump it on any incompatible change; [`Document::from_json`] rejects other
/// versions.
pub const FORMAT_VERSION: u32 = 1;

/// A stable identifier for an edit within one [`Document`].
///
/// Ids are allocated by the document, never reused, and survive undo/redo and
/// JSON round trips. They are not the 1-based numbers shown on badges; those
/// come from an edit's position (see [`Document::number_of`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EditId(u64);

impl EditId {
    /// The raw id, e.g. for a DOM key.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for EditId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// One region-plus-operation pair, with optional hint text.
///
/// The region is always in original-image coordinates, and the op describes a
/// change to those original pixels. Edits don't compose: edit #2 never sees
/// edit #1's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "EditRepr")]
pub struct Edit {
    id: EditId,
    region: Region,
    op: Op,
    hint: Option<String>,
}

#[derive(Deserialize)]
struct EditRepr {
    id: EditId,
    region: Region,
    op: Op,
    hint: Option<String>,
}

impl TryFrom<EditRepr> for Edit {
    type Error = OpError;
    fn try_from(r: EditRepr) -> Result<Self, Self::Error> {
        r.op.validate()?;
        Ok(Self {
            id: r.id,
            region: r.region,
            op: r.op,
            hint: normalize_hint(r.hint),
        })
    }
}

/// An empty hint is the same as no hint.
pub(crate) fn normalize_hint(hint: Option<String>) -> Option<String> {
    hint.filter(|h| !h.is_empty())
}

impl Edit {
    pub(crate) fn new(id: EditId, region: Region, op: Op, hint: Option<String>) -> Self {
        Self {
            id,
            region,
            op,
            hint: normalize_hint(hint),
        }
    }

    /// This edit's id.
    pub fn id(&self) -> EditId {
        self.id
    }

    /// The source region on the original image.
    pub fn region(&self) -> &Region {
        &self.region
    }

    /// The operation.
    pub fn op(&self) -> &Op {
        &self.op
    }

    /// Optional free-text hint. Any op can carry one ("move this here *and*
    /// make it blue"); for [`Op::Hint`] it is the whole instruction. Never
    /// `Some("")`.
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    /// The display-only ghost of this edit's result, or `None` for a hint.
    /// See [`Ghost`]: it is never a region and can't be edited further.
    pub fn ghost(&self, size: ImageSize) -> Option<Ghost> {
        self.op.ghost(&self.region, size)
    }

    pub(crate) fn region_mut(&mut self) -> &mut Region {
        &mut self.region
    }

    pub(crate) fn op_mut(&mut self) -> &mut Op {
        &mut self.op
    }

    pub(crate) fn hint_mut(&mut self) -> &mut Option<String> {
        &mut self.hint
    }
}

/// The whole session: the original image's size, the ordered edits and the
/// image-level instructions.
///
/// Order expresses sequence and priority only. A `Document` is read-only from
/// outside the crate; change it through an [`Editor`](crate::Editor) so every
/// change lands in the undo history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "DocumentRepr", try_from = "DocumentRepr")]
pub struct Document {
    image_size: ImageSize,
    edits: Vec<Edit>,
    instructions: String,
    next_id: u64,
}

/// The on-disk shape. Field names and order are the stable v1 format.
#[derive(Serialize, Deserialize)]
struct DocumentRepr {
    version: u32,
    image_size: ImageSize,
    next_id: u64,
    edits: Vec<Edit>,
    instructions: String,
}

impl From<Document> for DocumentRepr {
    fn from(d: Document) -> Self {
        Self {
            version: FORMAT_VERSION,
            image_size: d.image_size,
            next_id: d.next_id,
            edits: d.edits,
            instructions: d.instructions,
        }
    }
}

impl TryFrom<DocumentRepr> for Document {
    type Error = DocumentError;
    fn try_from(r: DocumentRepr) -> Result<Self, Self::Error> {
        if r.version != FORMAT_VERSION {
            return Err(DocumentError::UnsupportedVersion(r.version));
        }
        let mut seen = std::collections::HashSet::new();
        for e in &r.edits {
            if e.id.0 >= r.next_id || !seen.insert(e.id) {
                return Err(DocumentError::BadEditId(e.id));
            }
        }
        Ok(Self {
            image_size: r.image_size,
            edits: r.edits,
            instructions: r.instructions,
            next_id: r.next_id,
        })
    }
}

impl Document {
    /// An empty document for an image of `image_size`.
    pub fn new(image_size: ImageSize) -> Self {
        Self {
            image_size,
            edits: Vec::new(),
            instructions: String::new(),
            next_id: 1,
        }
    }

    /// The original image's size.
    pub fn image_size(&self) -> ImageSize {
        self.image_size
    }

    /// The edits in order (first is #1).
    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    /// Looks up an edit by id.
    pub fn edit(&self, id: EditId) -> Option<&Edit> {
        self.edits.iter().find(|e| e.id == id)
    }

    /// The position of an edit in the list (0-based).
    pub fn index_of(&self, id: EditId) -> Option<usize> {
        self.edits.iter().position(|e| e.id == id)
    }

    /// The 1-based number shown on the edit's badge and in the caption.
    pub fn number_of(&self, id: EditId) -> Option<usize> {
        self.index_of(id).map(|i| i + 1)
    }

    /// The image-level instructions ("Overall: ...").
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// Serializes to the v1 JSON format. Not embedded in exports in v1 (spec
    /// §Out of scope), but kept stable for a later `iTXt` follow-up.
    pub fn to_json(&self) -> String {
        // Infallible: every field is a plain struct, string or finite number.
        serde_json::to_string(self).expect("Document serializes to JSON")
    }

    /// Parses the JSON format, validating every region, op and id.
    pub fn from_json(json: &str) -> Result<Self, DocumentError> {
        // Parse the repr first so version and id errors keep their own
        // variants instead of being folded into a serde message.
        let repr: DocumentRepr =
            serde_json::from_str(json).map_err(|e| DocumentError::Json(e.to_string()))?;
        Self::try_from(repr)
    }

    pub(crate) fn alloc_id(&mut self) -> EditId {
        let id = EditId(self.next_id);
        self.next_id += 1;
        id
    }

    pub(crate) fn edits_mut(&mut self) -> &mut Vec<Edit> {
        &mut self.edits
    }

    pub(crate) fn instructions_mut(&mut self) -> &mut String {
        &mut self.instructions
    }
}

/// Why a JSON document was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentError {
    /// Malformed JSON, or a region, op or size that failed validation.
    Json(String),
    /// The `version` field is not [`FORMAT_VERSION`].
    UnsupportedVersion(u32),
    /// An edit id is duplicated or not below `next_id`.
    BadEditId(EditId),
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "invalid document JSON: {e}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported document version {v}"),
            Self::BadEditId(id) => write!(f, "duplicate or out-of-range edit id {id}"),
        }
    }
}

impl std::error::Error for DocumentError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point;
    use crate::op::{Anchor, MirrorAxis};

    fn sample() -> Document {
        let mut d = Document::new(ImageSize::new(1280, 720).unwrap());
        let ops = [
            Op::Translate {
                dx: 0.15,
                dy: -0.04,
            },
            Op::Scale {
                sx: 1.5,
                sy: 0.75,
                anchor: Anchor::Point(Point::new(0.1, 0.9)),
            },
            Op::Mirror {
                axis: MirrorAxis::Vertical,
            },
            Op::Rotate {
                degrees: 1.0 / 3.0,
                anchor: Anchor::Center,
            },
            Op::Hint,
        ];
        for (i, op) in ops.into_iter().enumerate() {
            let id = d.alloc_id();
            let region = if i % 2 == 0 {
                Region::rect(Point::new(0.12, 0.3), Point::new(0.4, 0.52)).unwrap()
            } else {
                Region::polygon([
                    Point::new(0.1, 0.1),
                    Point::new(0.7, 0.2),
                    Point::new(0.3, 0.6),
                ])
                .unwrap()
            };
            let hint = (i != 2).then(|| format!("hint {i} with \"quotes\" and ünïcode"));
            d.edits.push(Edit::new(id, region, op, hint));
        }
        d.instructions = "tighten the spacing\nbetween the cards".into();
        d
    }

    #[test]
    fn json_round_trips_losslessly() {
        let d = sample();
        let json = d.to_json();
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back, d);
        // Byte-stable too: re-serializing gives identical JSON.
        assert_eq!(back.to_json(), json);
    }

    #[test]
    fn json_shape_is_stable() {
        let mut d = Document::new(ImageSize::new(10, 20).unwrap());
        let id = d.alloc_id();
        d.edits.push(Edit::new(
            id,
            Region::rect(Point::new(0.0, 0.0), Point::new(0.5, 0.25)).unwrap(),
            Op::Translate { dx: 0.5, dy: 0.0 },
            None,
        ));
        assert_eq!(
            d.to_json(),
            r#"{"version":1,"image_size":{"width":10,"height":20},"next_id":2,"edits":[{"id":1,"region":{"kind":"rect","min":{"x":0.0,"y":0.0},"max":{"x":0.5,"y":0.25}},"op":{"kind":"translate","dx":0.5,"dy":0.0},"hint":null}],"instructions":""}"#
        );
    }

    #[test]
    fn from_json_rejects_bad_documents() {
        let good = sample().to_json();
        let v2 = good.replacen(r#""version":1"#, r#""version":2"#, 1);
        assert_eq!(
            Document::from_json(&v2),
            Err(DocumentError::UnsupportedVersion(2))
        );
        let low_next = good.replacen(r#""next_id":6"#, r#""next_id":3"#, 1);
        assert!(matches!(
            Document::from_json(&low_next),
            Err(DocumentError::BadEditId(_))
        ));
        let bad_scale = good.replacen(r#""sx":1.5"#, r#""sx":-1.5"#, 1);
        assert!(matches!(
            Document::from_json(&bad_scale),
            Err(DocumentError::Json(_))
        ));
        assert!(Document::from_json("{").is_err());
    }

    #[test]
    fn empty_hint_reads_as_none() {
        let d = sample();
        let e = &d.edits()[0];
        let blank = Edit::new(e.id(), e.region().clone(), Op::Hint, Some(String::new()));
        assert_eq!(blank.hint(), None);
        let json = d.to_json().replacen(
            r#""hint":"hint 0 with \"quotes\" and ünïcode""#,
            r#""hint":"""#,
            1,
        );
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back.edits()[0].hint(), None);
        assert_eq!(back.number_of(e.id()), Some(1));
    }
}
