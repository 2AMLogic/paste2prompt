//! What a region was picked from: a DOM element in the page the embed
//! captured (docs/EMBED.md, "Element identity").
//!
//! Stored with the edit in the document JSON only. The caption doesn't print
//! it yet: that is a change to the output format (CLAUDE.md rule 4), which
//! waits for the model comparison EMBED.md describes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::document::is_format_char;

/// The longest selector, in characters.
pub const MAX_SELECTOR_CHARS: usize = 512;

/// The longest element label, in characters: about what fits on a caption
/// row next to the selector.
pub const MAX_ELEMENT_LABEL_CHARS: usize = 40;

/// The longest tag or role, in characters.
const MAX_TAG_CHARS: usize = 64;

/// The DOM element a region was picked from.
///
/// Every field is checked when it is built and when a document is loaded:
/// one line each, no invisible formatting characters, bounded lengths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ElementRepr")]
pub struct ElementRef {
    selector: String,
    tag: String,
    role: Option<String>,
    label: Option<String>,
}

#[derive(Deserialize)]
struct ElementRepr {
    selector: String,
    tag: String,
    role: Option<String>,
    label: Option<String>,
}

impl TryFrom<ElementRepr> for ElementRef {
    type Error = ElementError;
    fn try_from(r: ElementRepr) -> Result<Self, Self::Error> {
        Self::new(r.selector, r.tag, r.role, r.label)
    }
}

impl ElementRef {
    /// Checks and builds an element reference.
    ///
    /// - `selector`: a CSS selector that matched only this element when it
    ///   was picked. Required, at most [`MAX_SELECTOR_CHARS`] characters.
    /// - `tag`: the lowercase tag name (`button`, `my-widget`).
    /// - `role`: the `role` attribute, if any.
    /// - `label`: what a person would call it (`aria-label`, `data-testid`
    ///   or trimmed text), at most [`MAX_ELEMENT_LABEL_CHARS`] characters.
    ///
    /// Surrounding whitespace is trimmed; an empty role or label is none.
    pub fn new(
        selector: String,
        tag: String,
        role: Option<String>,
        label: Option<String>,
    ) -> Result<Self, ElementError> {
        let selector = one_line(selector, MAX_SELECTOR_CHARS, ElementError::Selector)?
            .ok_or(ElementError::Selector)?;
        let tag = tag.trim();
        if tag.is_empty()
            || tag.chars().count() > MAX_TAG_CHARS
            || !tag
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(ElementError::Tag);
        }
        Ok(Self {
            selector,
            tag: tag.to_owned(),
            role: role
                .map(|r| one_line(r, MAX_TAG_CHARS, ElementError::Role))
                .transpose()?
                .flatten(),
            label: label
                .map(|l| one_line(l, MAX_ELEMENT_LABEL_CHARS, ElementError::Label))
                .transpose()?
                .flatten(),
        })
    }

    /// The CSS selector, unique in the page when it was picked.
    pub fn selector(&self) -> &str {
        &self.selector
    }

    /// The lowercase tag name.
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// The `role` attribute, if it had one.
    pub fn role(&self) -> Option<&str> {
        self.role.as_deref()
    }

    /// The human-facing name, if it had one.
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

/// Trims `s` and checks it is one line of visible text of at most `max`
/// characters; `None` if it is empty after trimming.
fn one_line(s: String, max: usize, err: ElementError) -> Result<Option<String>, ElementError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > max
        || trimmed
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') || is_format_char(c))
    {
        return Err(err);
    }
    Ok(Some(trimmed.to_owned()))
}

/// Which field of an [`ElementRef`] was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementError {
    /// Empty, too long, or not one line of visible text.
    Selector,
    /// Empty, too long, or not a lowercase tag name.
    Tag,
    /// Too long, or not one line of visible text.
    Role,
    /// Too long, or not one line of visible text.
    Label,
}

impl fmt::Display for ElementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Selector => "invalid element selector",
            Self::Tag => "invalid element tag name",
            Self::Role => "invalid element role",
            Self::Label => "invalid element label",
        })
    }
}

impl std::error::Error for ElementError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(selector: &str, tag: &str, role: Option<&str>, label: Option<&str>) -> ElementRef {
        ElementRef::new(
            selector.into(),
            tag.into(),
            role.map(Into::into),
            label.map(Into::into),
        )
        .unwrap()
    }

    #[test]
    fn fields_are_trimmed_and_empty_ones_dropped() {
        let e = el(" button.save ", "button", Some("  "), Some(" Save "));
        assert_eq!(e.selector(), "button.save");
        assert_eq!(e.tag(), "button");
        assert_eq!(e.role(), None);
        assert_eq!(e.label(), Some("Save"));
        assert_eq!(
            el("x-y > a", "my-widget2", Some("tab"), None).role(),
            Some("tab")
        );
    }

    #[test]
    fn bad_fields_are_rejected() {
        let new = |s: &str, t: &str, r: Option<&str>, l: Option<&str>| {
            ElementRef::new(s.into(), t.into(), r.map(Into::into), l.map(Into::into))
        };
        assert_eq!(new("  ", "a", None, None), Err(ElementError::Selector));
        assert_eq!(new("a\nb", "a", None, None), Err(ElementError::Selector));
        let long = "a".repeat(MAX_SELECTOR_CHARS + 1);
        assert_eq!(new(&long, "a", None, None), Err(ElementError::Selector));
        assert!(new(&long[1..], "a", None, None).is_ok());
        for tag in ["", "Button", "a b", "a_b", &"a".repeat(65)] {
            assert_eq!(new("a", tag, None, None), Err(ElementError::Tag), "{tag:?}");
        }
        assert_eq!(new("a", "a", Some("x\ty"), None), Err(ElementError::Role));
        let label = "é".repeat(MAX_ELEMENT_LABEL_CHARS);
        assert!(new("a", "a", None, Some(&label)).is_ok());
        assert_eq!(
            new("a", "a", None, Some(&format!("{label}é"))),
            Err(ElementError::Label)
        );
        assert_eq!(
            new("a", "a", None, Some("Sa\u{202E}ve")),
            Err(ElementError::Label)
        );
    }

    #[test]
    fn json_is_validated_on_load() {
        let e = el("#save", "button", None, Some("Save"));
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(
            json,
            r##"{"selector":"#save","tag":"button","role":null,"label":"Save"}"##
        );
        assert_eq!(serde_json::from_str::<ElementRef>(&json).unwrap(), e);
        let bad = json.replace(r#""tag":"button""#, r#""tag":"BUTTON""#);
        assert!(serde_json::from_str::<ElementRef>(&bad).is_err());
    }
}
