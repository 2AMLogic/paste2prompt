//! Pure decision rules for the export actions: which key press triggers
//! which action, the download file name, when to show the busy label, and
//! the user-facing copy-failure messages. No browser types here, so
//! `cargo test` covers them natively.

pub use crate::input::rules::Mods;

/// One of the three export actions (spec §Output → Export actions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Copy the export PNG to the clipboard.
    CopyImage,
    /// Download the export PNG.
    Download,
    /// Copy the caption as plain text.
    CopyCaption,
}

/// Where a key press happened, reduced to what the rules need.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyContext {
    /// Focus is in a text field or `contenteditable` element.
    pub editable: bool,
    /// The page has a non-empty text selection.
    pub text_selected: bool,
}

/// The export action bound to this key press, if any (spec §Keyboard
/// shortcuts):
///
/// - `⌘C` / `Ctrl+C`: Copy image, only outside text fields and with no page
///   text selected; otherwise the browser's own copy applies.
/// - `⌘S` / `Ctrl+S`: Download, everywhere (the browser's "Save page" is
///   never what the user wants here).
/// - `C` with no modifier: Copy caption, only outside text fields. Not
///   `⌘⌥C`, which opens DevTools on macOS.
pub fn shortcut(key: &str, mods: Mods, ctx: KeyContext) -> Option<Action> {
    let is = |c: &str| key.eq_ignore_ascii_case(c);
    let chord = (mods.ctrl || mods.meta) && !mods.shift && !mods.alt;
    let bare = !(mods.ctrl || mods.meta || mods.shift || mods.alt);
    if chord && is("s") {
        Some(Action::Download)
    } else if chord && is("c") && !ctx.editable && !ctx.text_selected {
        Some(Action::CopyImage)
    } else if bare && is("c") && !ctx.editable {
        Some(Action::CopyCaption)
    } else {
        None
    }
}

/// Local date and time, as the download file name uses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    pub year: u32,
    /// 1–12.
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// `paste2prompt-YYYYMMDD-HHMMSS.png`, in local time.
pub fn file_name(t: Timestamp) -> String {
    format!(
        "paste2prompt-{:04}{:02}{:02}-{:02}{:02}{:02}.png",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

/// Images at or above this many pixels show a "Rendering…" label while the
/// export renders. Smaller ones render in well under a frame or two, so a
/// label would only flicker.
pub const BUSY_LABEL_PIXELS: u64 = 4_000_000;

/// `true` if rendering an image this size should show the busy label.
pub fn shows_busy_label(width: u32, height: u32) -> bool {
    u64::from(width) * u64::from(height) >= BUSY_LABEL_PIXELS
}

/// Why Copy image (or Copy caption) could not use the clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyFailure {
    /// No async clipboard API, or no `ClipboardItem` (older browsers).
    Unsupported,
    /// The page isn't a secure context (plain `http:` on another host).
    InsecureContext,
    /// The browser or the user refused clipboard access.
    Denied,
    /// Anything else the clipboard reported.
    Other,
}

impl CopyFailure {
    /// Classify a rejected clipboard promise by its `DOMException` name.
    pub fn from_exception_name(name: &str) -> Self {
        match name {
            "NotAllowedError" | "SecurityError" => Self::Denied,
            "NotSupportedError" => Self::Unsupported,
            _ => Self::Other,
        }
    }

    /// A short message for the toast. Copy image adds a Download offer.
    pub fn message(self) -> &'static str {
        match self {
            Self::Unsupported => "This browser can't copy to the clipboard.",
            Self::InsecureContext => "Copying needs a secure (https) page.",
            Self::Denied => "The browser blocked clipboard access.",
            Self::Other => "Couldn't copy to the clipboard.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods {
        ctrl: false,
        meta: false,
        shift: false,
        alt: false,
    };
    const CMD: Mods = Mods { meta: true, ..NONE };
    const CTRL: Mods = Mods { ctrl: true, ..NONE };
    const PLAIN: KeyContext = KeyContext {
        editable: false,
        text_selected: false,
    };
    const IN_FIELD: KeyContext = KeyContext {
        editable: true,
        text_selected: false,
    };
    const SELECTED: KeyContext = KeyContext {
        editable: false,
        text_selected: true,
    };

    #[test]
    fn copy_image_chord() {
        assert_eq!(shortcut("c", CMD, PLAIN), Some(Action::CopyImage));
        assert_eq!(shortcut("c", CTRL, PLAIN), Some(Action::CopyImage));
        assert_eq!(shortcut("C", CTRL, PLAIN), Some(Action::CopyImage)); // Caps Lock
    }

    #[test]
    fn copy_image_yields_to_text_copy() {
        // In a text field or with page text selected, ⌘C copies the text.
        assert_eq!(shortcut("c", CMD, IN_FIELD), None);
        assert_eq!(shortcut("c", CMD, SELECTED), None);
        assert_eq!(shortcut("c", CTRL, IN_FIELD), None);
    }

    #[test]
    fn reserved_c_chords_are_left_alone() {
        // ⌘⇧C / Ctrl+Shift+C: inspector. ⌥⌘C / Ctrl+Alt+C: DevTools / AltGr.
        assert_eq!(shortcut("c", Mods { shift: true, ..CMD }, PLAIN), None);
        assert_eq!(
            shortcut(
                "C",
                Mods {
                    shift: true,
                    ..CTRL
                },
                PLAIN
            ),
            None
        );
        assert_eq!(shortcut("c", Mods { alt: true, ..CMD }, PLAIN), None);
        assert_eq!(shortcut("ç", Mods { alt: true, ..CMD }, PLAIN), None);
        assert_eq!(shortcut("c", Mods { alt: true, ..CTRL }, PLAIN), None);
    }

    #[test]
    fn download_chord_works_everywhere() {
        assert_eq!(shortcut("s", CMD, PLAIN), Some(Action::Download));
        assert_eq!(shortcut("s", CTRL, PLAIN), Some(Action::Download));
        assert_eq!(shortcut("S", CMD, PLAIN), Some(Action::Download));
        // Saving is not a text action, so it also works from a text field.
        assert_eq!(shortcut("s", CMD, IN_FIELD), Some(Action::Download));
        assert_eq!(shortcut("s", CTRL, SELECTED), Some(Action::Download));
        // ⌘⇧S (Save As in some apps) and ⌘⌥S are not ours.
        assert_eq!(shortcut("s", Mods { shift: true, ..CMD }, PLAIN), None);
        assert_eq!(shortcut("s", Mods { alt: true, ..CTRL }, PLAIN), None);
        // `S` alone is the scale operation (#13), not Download.
        assert_eq!(shortcut("s", NONE, PLAIN), None);
    }

    #[test]
    fn copy_caption_single_key() {
        assert_eq!(shortcut("c", NONE, PLAIN), Some(Action::CopyCaption));
        assert_eq!(shortcut("C", NONE, PLAIN), Some(Action::CopyCaption)); // Caps Lock
        // A page text selection doesn't matter for a single key.
        assert_eq!(shortcut("c", NONE, SELECTED), Some(Action::CopyCaption));
        // Typing "c" in a text field is typing.
        assert_eq!(shortcut("c", NONE, IN_FIELD), None);
        // Shift+C (key "C" with shift) and Alt+C are not the bare key.
        assert_eq!(
            shortcut(
                "C",
                Mods {
                    shift: true,
                    ..NONE
                },
                PLAIN
            ),
            None
        );
        assert_eq!(shortcut("c", Mods { alt: true, ..NONE }, PLAIN), None);
    }

    #[test]
    fn other_keys_are_ignored() {
        for key in ["v", "o", "x", "Enter", "Escape", "", "cc"] {
            assert_eq!(shortcut(key, CMD, PLAIN), None, "{key}");
            assert_eq!(shortcut(key, NONE, PLAIN), None, "{key}");
        }
    }

    #[test]
    fn file_name_is_zero_padded_local_timestamp() {
        let t = Timestamp {
            year: 2026,
            month: 3,
            day: 7,
            hour: 9,
            minute: 5,
            second: 2,
        };
        assert_eq!(file_name(t), "paste2prompt-20260307-090502.png");
        let t = Timestamp {
            year: 2026,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
        };
        assert_eq!(file_name(t), "paste2prompt-20261231-235959.png");
    }

    #[test]
    fn busy_label_threshold() {
        assert!(!shows_busy_label(1920, 1080)); // 2.1 MP
        assert!(!shows_busy_label(1999, 2000));
        assert!(shows_busy_label(2000, 2000));
        assert!(shows_busy_label(5120, 2880)); // 5K screenshot
        assert!(shows_busy_label(u32::MAX, u32::MAX)); // no overflow
    }

    #[test]
    fn copy_failure_classification() {
        assert_eq!(
            CopyFailure::from_exception_name("NotAllowedError"),
            CopyFailure::Denied
        );
        assert_eq!(
            CopyFailure::from_exception_name("SecurityError"),
            CopyFailure::Denied
        );
        assert_eq!(
            CopyFailure::from_exception_name("NotSupportedError"),
            CopyFailure::Unsupported
        );
        assert_eq!(
            CopyFailure::from_exception_name("DataError"),
            CopyFailure::Other
        );
        assert_eq!(CopyFailure::from_exception_name(""), CopyFailure::Other);
        for f in [
            CopyFailure::Unsupported,
            CopyFailure::InsecureContext,
            CopyFailure::Denied,
            CopyFailure::Other,
        ] {
            assert!(!f.message().is_empty());
        }
    }
}
