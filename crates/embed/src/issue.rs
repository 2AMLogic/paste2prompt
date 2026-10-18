//! **Open issue**: the new-issue URL for the host's GitHub repository,
//! pre-filled from the caption. Pure, so `cargo test` covers it.
//!
//! The PNG can't travel in a URL, so `packages/element` copies it to the
//! clipboard and the body asks the user to paste it. Opening this URL sends
//! the title, labels, page address and caption text to github.com; that is
//! the embed's one exception to CLAUDE.md rule 1, and it lives here, not in
//! `crates/web`, so the standalone site never contains it.

/// Where Open issue files, and what it adds to the body. Set by the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IssueTarget {
    /// `owner/name`.
    pub repo: String,
    /// Labels for the new issue. GitHub applies them only when the user may
    /// label issues in the repo, and ignores them otherwise.
    pub labels: Vec<String>,
    /// Extra lines for the body, such as the host app's build version.
    pub context: String,
    /// The page the capture shows (origin, path and hash; no query string).
    pub page: String,
}

/// Longest URL to hand GitHub. Its new-issue page refuses much more than
/// 8 KB of URL; the caption is cut to fit under this.
pub const MAX_URL_LEN: usize = 8_000;

/// Longest title, in characters, before it is cut with `…`.
const MAX_TITLE_CHARS: usize = 80;

/// The line that stands in for cut caption lines.
const CUT_NOTE: &str = "… (cut short: the full edit list is in the image)";

/// The line in the body where the image goes.
const PASTE_HERE: &str =
    "<!-- The annotated screenshot is on your clipboard: paste it here (⌘V / Ctrl+V). -->";

/// The new-issue URL: title from the first line of the image-level
/// instructions (or the page), body with the page, the host's context and
/// the caption, cut to fit [`MAX_URL_LEN`].
pub fn issue_url(target: &IssueTarget, instructions: &str, caption: &str) -> String {
    let title = title(instructions, &target.page);
    let lines: Vec<&str> = caption.lines().collect();
    let mut keep = lines.len();
    loop {
        let shown = if keep < lines.len() {
            let mut shown = lines[..keep].join("\n");
            shown.push('\n');
            shown.push_str(CUT_NOTE);
            shown
        } else {
            caption.to_owned()
        };
        let url = build(target, &title, &body(target, &shown));
        if url.len() <= MAX_URL_LEN || keep == 0 {
            return url;
        }
        keep -= 1;
    }
}

fn build(target: &IssueTarget, title: &str, body: &str) -> String {
    let mut url = format!(
        "https://github.com/{}/issues/new?title={}&body={}",
        target.repo,
        encode(title),
        encode(body)
    );
    if !target.labels.is_empty() {
        url.push_str("&labels=");
        url.push_str(&encode(&target.labels.join(",")));
    }
    url
}

fn body(target: &IssueTarget, caption: &str) -> String {
    let mut body = format!("{PASTE_HERE}\n\n");
    if !target.page.is_empty() {
        body.push_str(&format!("**Page:** {}\n", target.page));
    }
    let context = target.context.trim();
    if !context.is_empty() {
        body.push_str(context);
        body.push('\n');
    }
    body.push_str(&format!("\n```text\n{}\n```\n", caption.trim_end()));
    body
}

fn title(instructions: &str, page: &str) -> String {
    let first = instructions.lines().map(str::trim).find(|l| !l.is_empty());
    let Some(line) = first else {
        return if page.is_empty() {
            "Feedback".to_owned()
        } else {
            format!("Feedback on {page}")
        };
    };
    if line.chars().count() <= MAX_TITLE_CHARS {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(MAX_TITLE_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// Percent-encodes everything but RFC 3986's unreserved characters, as
/// `encodeURIComponent` does (and also `!'()*`, which it leaves alone).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> IssueTarget {
        IssueTarget {
            repo: "2AMLogic/loom-ui".to_owned(),
            labels: vec!["loom:triage".to_owned()],
            context: "Build: 1.2.3".to_owned(),
            page: "https://dashboard.2amlogic.com/#/runs".to_owned(),
        }
    }

    /// The query parameter `name`, decoded.
    fn param(url: &str, name: &str) -> Option<String> {
        let query = url.split_once('?')?.1;
        let value = query
            .split('&')
            .find_map(|kv| kv.strip_prefix(&format!("{name}=")))?;
        let bytes = value.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                out.push(u8::from_str_radix(&value[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        Some(String::from_utf8(out).unwrap())
    }

    const CAPTION: &str = "paste2prompt edit list (user-authored)\n\
        Coordinates are % of image width/height. Edits refer to the original image.\n\
        #1  hint  region (3%,5%)-(21%,24%)  →  \"make the count red\"\n";

    #[test]
    fn url_carries_title_body_and_labels() {
        let url = issue_url(&target(), "ETA column is wrong\nmore detail", CAPTION);
        assert!(url.starts_with("https://github.com/2AMLogic/loom-ui/issues/new?title="));
        assert_eq!(param(&url, "title").unwrap(), "ETA column is wrong");
        assert_eq!(param(&url, "labels").unwrap(), "loom:triage");
        let body = param(&url, "body").unwrap();
        assert!(body.starts_with(PASTE_HERE));
        assert!(body.contains("**Page:** https://dashboard.2amlogic.com/#/runs\nBuild: 1.2.3\n"));
        assert!(body.contains(&format!("```text\n{}\n```", CAPTION.trim_end())));
    }

    #[test]
    fn title_falls_back_to_the_page_and_is_cut() {
        assert_eq!(
            title("  \n ", "https://x.test/#/a"),
            "Feedback on https://x.test/#/a"
        );
        assert_eq!(title("", ""), "Feedback");
        let long = "é".repeat(200);
        let t = title(&long, "");
        assert_eq!(t.chars().count(), MAX_TITLE_CHARS);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn no_labels_no_labels_param() {
        let mut t = target();
        t.labels.clear();
        assert_eq!(param(&issue_url(&t, "", CAPTION), "labels"), None);
    }

    #[test]
    fn long_caption_is_cut_to_fit() {
        let mut caption = String::from(CAPTION);
        for i in 0..500 {
            caption.push_str(&format!(
                "#{i}  hint  region (1%,1%)-(2%,2%)  →  \"a fairly long hint\"\n"
            ));
        }
        let url = issue_url(&target(), "", &caption);
        assert!(url.len() <= MAX_URL_LEN);
        let body = param(&url, "body").unwrap();
        assert!(body.contains(CUT_NOTE));
        assert!(body.contains("paste2prompt edit list"));
    }

    #[test]
    fn encoding_matches_encode_uri_component_for_reserved_characters() {
        assert_eq!(encode("a b&c=d/é#"), "a%20b%26c%3Dd%2F%C3%A9%23");
        assert_eq!(encode("A-z_0.9~"), "A-z_0.9~");
    }
}
