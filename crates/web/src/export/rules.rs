//! Pure decision rules for the export actions: the download file name, when
//! to show the busy label, and the user-facing copy-failure messages. No
//! browser types here, so `cargo test` covers them natively. The keys that
//! trigger the actions are in [`crate::keymap::rules`].

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
