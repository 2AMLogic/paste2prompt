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
    /// [`issue_url`] keeps the hash only when it is a route (`#/…`), cut at
    /// its own `?` or `&`; any other hash can carry a token, so it is dropped.
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

/// Whether `repo` is `owner/name`: two parts of letters, digits, `-`, `_`
/// and `.`, neither empty nor `.` or `..` (which would climb out of the repo
/// path in the URL).
pub fn valid_repo(repo: &str) -> bool {
    let part = |p: &str| {
        !p.is_empty()
            && p != "."
            && p != ".."
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    let mut parts = repo.split('/');
    matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None) if part(a) && part(b))
}

/// The page address with its hash made safe to send. An OAuth-style hash
/// (`#access_token=…`, `#code=…&state=…`) is a credential, so the hash is
/// kept only when it starts `#/` (a router path), cut at the first `?` or
/// `&` because `#/cb?access_token=…` is still a leak. One rule, not a list
/// of token names that could never be complete.
fn clean_page(page: &str) -> String {
    let Some((base, hash)) = page.split_once('#') else {
        return page.to_owned();
    };
    if !hash.starts_with('/') {
        return base.to_owned();
    }
    let route = hash.split(['?', '&']).next().unwrap_or("");
    format!("{base}#{route}")
}

/// The new-issue URL: title from the first line of the image-level
/// instructions (or the page), body with the page, the host's context and
/// the caption, cut to fit [`MAX_URL_LEN`].
pub fn issue_url(target: &IssueTarget, instructions: &str, caption: &str) -> String {
    let page = clean_page(&target.page);
    let title = title(instructions, &page);
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
        let url = build(target, &title, &body(target, &page, &shown));
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

fn body(target: &IssueTarget, page: &str, caption: &str) -> String {
    let mut body = format!("{PASTE_HERE}\n\n");
    if !page.is_empty() {
        body.push_str(&format!("**Page:** {page}\n"));
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
            repo: "your-org/your-app".to_owned(),
            labels: vec!["bug".to_owned()],
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
        assert!(url.starts_with("https://github.com/your-org/your-app/issues/new?title="));
        assert_eq!(param(&url, "title").unwrap(), "ETA column is wrong");
        assert_eq!(param(&url, "labels").unwrap(), "bug");
        let body = param(&url, "body").unwrap();
        assert!(body.starts_with(PASTE_HERE));
        assert!(body.contains("**Page:** https://dashboard.2amlogic.com/#/runs\nBuild: 1.2.3\n"));
        assert!(body.contains(&format!("```text\n{}\n```", CAPTION.trim_end())));
    }

    #[test]
    fn credential_hashes_never_reach_the_url() {
        for hash in [
            "#access_token=SECRET",
            "#id_token=SECRET",
            "#code=SECRET&state=s",
            "#",
            "#state",
        ] {
            let mut t = target();
            t.page = format!("https://x.test/cb{hash}");
            for instructions in ["", "needs a title"] {
                let url = issue_url(&t, instructions, CAPTION);
                assert!(!url.contains("SECRET"), "{hash}: {url}");
                let body = param(&url, "body").unwrap();
                assert!(body.contains("**Page:** https://x.test/cb\n"), "{hash}");
                assert!(!param(&url, "title").unwrap().contains('#'), "{hash}");
            }
        }
    }

    #[test]
    fn route_hashes_are_kept_up_to_their_own_query() {
        assert_eq!(clean_page("https://x.test/#/runs"), "https://x.test/#/runs");
        assert_eq!(clean_page("https://x.test/#/"), "https://x.test/#/");
        assert_eq!(
            clean_page("https://x.test/#/cb?access_token=SECRET"),
            "https://x.test/#/cb"
        );
        assert_eq!(
            clean_page("https://x.test/#/cb&code=SECRET"),
            "https://x.test/#/cb"
        );
        assert_eq!(clean_page("https://x.test/a"), "https://x.test/a");
        assert_eq!(clean_page(""), "");
        let mut t = target();
        t.page = "https://x.test/#/cb?access_token=SECRET".to_owned();
        let url = issue_url(&t, "", CAPTION);
        assert!(!url.contains("SECRET"));
        assert_eq!(
            param(&url, "title").unwrap(),
            "Feedback on https://x.test/#/cb"
        );
    }

    #[test]
    fn repo_must_be_owner_slash_name_without_dot_parts() {
        for ok in ["your-org/your-app", "a.b/c_d-e", "a/b"] {
            assert!(valid_repo(ok), "{ok}");
        }
        for bad in [
            "../x", "x/..", "./x", "x/.", "..", ".", "", "a", "a/", "/b", "a/b/c", "a b/c", "a/b?x",
        ] {
            assert!(!valid_repo(bad), "{bad}");
        }
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
