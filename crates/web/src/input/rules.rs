//! Pure decision rules for the input paths: which clipboard item or dropped
//! file to load. No browser types here, so `cargo test` covers them
//! natively. The `⌘O` / `Ctrl+O` chord is in [`crate::keymap::rules`].

/// One entry of a `DataTransfer` (clipboard item or dropped file), reduced to
/// what the rules need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    /// `true` for a file (`DataTransferItem.kind == "file"`, or any `File`).
    pub is_file: bool,
    /// The MIME type the browser reports, possibly empty.
    pub mime: &'a str,
}

/// What to do with a `paste` event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteAction {
    /// Load the entry at this index and suppress the default paste.
    Load(usize),
    /// Nothing loadable; tell the user why.
    NotImage,
    /// Leave the event alone (e.g. ordinary text paste into a text field).
    Ignore,
}

/// What to do with a `drop` event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAction {
    /// Load the file at this index.
    Load(usize),
    /// Files were dropped, but none is an image.
    NotImage,
    /// No files at all (dragged text or a link).
    NoFiles,
}

/// `true` if the browser-reported MIME type is an image type.
pub fn is_image_mime(mime: &str) -> bool {
    let mime = mime.trim();
    const PREFIX: &str = "image/";
    mime.len() > PREFIX.len()
        && mime
            .get(..PREFIX.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(PREFIX))
}

/// Index of the first file entry with an image MIME type.
pub fn first_image(entries: &[Entry<'_>]) -> Option<usize> {
    entries
        .iter()
        .position(|e| e.is_file && is_image_mime(e.mime))
}

/// Decide how to handle a paste. `target_editable` is whether the paste
/// landed in a text field or other editable element.
pub fn paste_action(entries: &[Entry<'_>], target_editable: bool) -> PasteAction {
    // In a text field, a plain-text rendition means the user is pasting text
    // (Office apps add an image rendition of the same selection).
    let has_text = entries
        .iter()
        .any(|e| !e.is_file && e.mime.eq_ignore_ascii_case("text/plain"));
    if target_editable && has_text {
        return PasteAction::Ignore;
    }
    match first_image(entries) {
        Some(i) => PasteAction::Load(i),
        None if target_editable => PasteAction::Ignore,
        None => PasteAction::NotImage,
    }
}

/// Decide how to handle a drop of `files` (all entries are files).
pub fn drop_action(files: &[Entry<'_>]) -> DropAction {
    if files.is_empty() {
        return DropAction::NoFiles;
    }
    first_image(files).map_or(DropAction::NotImage, DropAction::Load)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(mime: &str) -> Entry<'_> {
        Entry {
            is_file: true,
            mime,
        }
    }

    fn text(mime: &str) -> Entry<'_> {
        Entry {
            is_file: false,
            mime,
        }
    }

    #[test]
    fn image_mime_detection() {
        assert!(is_image_mime("image/png"));
        assert!(is_image_mime("IMAGE/JPEG"));
        assert!(is_image_mime(" image/webp"));
        assert!(!is_image_mime(""));
        assert!(!is_image_mime("text/plain"));
        assert!(!is_image_mime("application/pdf"));
        assert!(!is_image_mime("image"));
        assert!(!is_image_mime("imagé/x")); // non-ASCII must not panic
    }

    #[test]
    fn first_image_skips_non_images_and_string_items() {
        // A string item that claims an image type is not a file; skip it.
        let entries = [
            text("image/png"),
            file("application/pdf"),
            file("image/gif"),
            file("image/png"),
        ];
        assert_eq!(first_image(&entries), Some(2));
        assert_eq!(first_image(&[file("text/plain")]), None);
        assert_eq!(first_image(&[]), None);
    }

    #[test]
    fn paste_image_outside_text_field_loads_it() {
        // Copying an image in a browser gives an image file plus text/html.
        let entries = [text("text/html"), file("image/png")];
        assert_eq!(paste_action(&entries, false), PasteAction::Load(1));
    }

    #[test]
    fn paste_into_text_field_prefers_text_when_present() {
        // Office apps put text/plain and an image rendition on the clipboard.
        let entries = [text("text/plain"), text("text/html"), file("image/png")];
        assert_eq!(paste_action(&entries, true), PasteAction::Ignore);
    }

    #[test]
    fn paste_image_only_into_text_field_still_loads() {
        // A screenshot has no text rendition, so a text field can't use it.
        assert_eq!(
            paste_action(&[file("image/png")], true),
            PasteAction::Load(0)
        );
    }

    #[test]
    fn paste_without_image() {
        assert_eq!(
            paste_action(&[text("text/plain")], false),
            PasteAction::NotImage
        );
        assert_eq!(
            paste_action(&[file("application/pdf")], false),
            PasteAction::NotImage
        );
        assert_eq!(paste_action(&[], false), PasteAction::NotImage);
        assert_eq!(
            paste_action(&[text("text/plain")], true),
            PasteAction::Ignore
        );
        assert_eq!(
            paste_action(&[file("application/pdf")], true),
            PasteAction::Ignore
        );
    }

    #[test]
    fn drop_takes_first_image() {
        let files = [file("text/plain"), file("image/jpeg"), file("image/png")];
        assert_eq!(drop_action(&files), DropAction::Load(1));
        assert_eq!(
            drop_action(&[file("application/zip")]),
            DropAction::NotImage
        );
        assert_eq!(drop_action(&[]), DropAction::NoFiles);
    }
}
