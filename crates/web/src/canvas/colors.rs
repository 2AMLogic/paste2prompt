//! Per-edit colors for the app, taken from the export's palette
//! (core [`mark_color`]) so a badge number has the same color on the
//! canvas, in the history panel and in the exported image.
//!
//! - The canvas sets colors from Rust ([`mark_css`]), which the CSP allows.
//! - The history panel can't: the CSP is `style-src 'self'`, so there is no
//!   inline `style=`. It uses one class per palette entry instead
//!   ([`mark_class`]: `mark-1` … `mark-N`), defined in `style.css`. The
//!   tests below parse `style.css` and fail if those rules drift from the
//!   palette, so a palette change in core must be copied there by hand.
//!
//! Selection never uses a palette color. It is a neutral ring (canvas:
//! [`SELECTION_RING`] outside a white halo; panel: a `CanvasText` ring), so a
//! selected edit stands out whatever its color.

use paste2prompt_core::mark_color;

/// The neutral ring around the selected edit's outline and badge on the
/// canvas. Not a palette color, and paired with a white halo so it shows on
/// light and dark images.
pub const SELECTION_RING: &str = "#111418";

/// Upper bound on the palette size, so [`palette_len`] always terminates.
const MAX_PALETTE: usize = 64;

/// How many colors the palette has: the period of [`mark_color`]. Core keeps
/// the palette private, so this is measured from `mark_color` itself.
pub fn palette_len() -> usize {
    (1..MAX_PALETTE)
        .find(|&n| mark_color(n + 1) == mark_color(1))
        .unwrap_or(MAX_PALETTE)
}

/// Edit `number`'s color (1-based, as on its badge) as a CSS hex color, for
/// the canvas.
pub fn mark_css(number: usize) -> String {
    let [r, g, b] = mark_color(number);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The `style.css` class that gives edit `number`'s badge its color in the
/// history panel: `mark-1` for the first palette entry, cycling like
/// [`mark_color`].
pub fn mark_class(number: usize) -> String {
    format!("mark-{}", number.saturating_sub(1) % palette_len() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE_CSS: &str = include_str!("../../style.css");

    /// `style.css` as `(selector, declarations)` pairs, comments removed.
    /// Enough for this file, which has no at-rules or nesting.
    fn css_rules() -> Vec<(String, Vec<(String, String)>)> {
        let mut css = String::new();
        let mut rest = STYLE_CSS;
        while let Some(open) = rest.find("/*") {
            css.push_str(&rest[..open]);
            let close = rest[open..].find("*/").expect("closed comment");
            rest = &rest[open + close + 2..];
        }
        css.push_str(rest);
        css.split('}')
            .filter_map(|rule| rule.split_once('{'))
            .map(|(selector, body)| {
                let decls = body
                    .split(';')
                    .filter_map(|d| d.split_once(':'))
                    .map(|(p, v)| (p.trim().to_owned(), v.trim().to_ascii_lowercase()))
                    .collect();
                (selector.trim().to_owned(), decls)
            })
            .collect()
    }

    /// The `background` of each `.badge.mark-N` rule, as `(N, "#rrggbb")`,
    /// in file order.
    fn css_mark_rules() -> Vec<(usize, String)> {
        css_rules()
            .into_iter()
            .filter_map(|(selector, decls)| {
                let n = selector.strip_prefix(".badge.mark-")?;
                let n: usize = n.parse().expect("a lone .badge.mark-N selector");
                let bg = decls
                    .into_iter()
                    .find(|(p, _)| p == "background")
                    .map(|(_, v)| v)
                    .unwrap_or_else(|| panic!(".badge.mark-{n} has no background"));
                Some((n, bg))
            })
            .collect()
    }

    fn css_color_for_class(class: &str) -> String {
        let n: usize = class["mark-".len()..].parse().unwrap();
        css_mark_rules()
            .into_iter()
            .find(|(k, _)| *k == n)
            .map(|(_, bg)| bg)
            .unwrap_or_else(|| panic!("style.css has no .badge.{class} rule"))
    }

    #[test]
    fn palette_len_is_the_period_of_mark_color() {
        let len = palette_len();
        assert!((2..MAX_PALETTE).contains(&len), "palette of {len}");
        for n in 1..=len {
            assert_eq!(mark_color(n), mark_color(n + len));
        }
        for n in 2..=len {
            assert_ne!(mark_color(n), mark_color(1), "palette repeats early");
        }
    }

    #[test]
    fn style_css_has_exactly_one_rule_per_palette_entry_in_sync() {
        let rules = css_mark_rules();
        let expected: Vec<(usize, String)> =
            (1..=palette_len()).map(|n| (n, mark_css(n))).collect();
        assert_eq!(
            rules, expected,
            "style.css .badge.mark-N rules must list core's PALETTE in order \
             (mark_color(N) as #rrggbb); update style.css by hand"
        );
    }

    #[test]
    fn a_badge_number_has_one_color_on_canvas_panel_and_export() {
        for n in 1..=3 * palette_len() {
            let [r, g, b] = mark_color(n); // the export's color
            let export = format!("#{r:02x}{g:02x}{b:02x}");
            assert_eq!(mark_css(n), export, "canvas color of edit {n}");
            assert_eq!(
                css_color_for_class(&mark_class(n)),
                export,
                "panel color of edit {n}"
            );
        }
    }

    #[test]
    fn mark_class_cycles_from_one() {
        let len = palette_len();
        assert_eq!(mark_class(1), "mark-1");
        assert_eq!(mark_class(len), format!("mark-{len}"));
        assert_eq!(mark_class(len + 1), "mark-1");
        // Like mark_color, 0 is treated as 1.
        assert_eq!(mark_class(0), "mark-1");
    }

    #[test]
    fn selection_is_not_a_palette_color() {
        let palette: Vec<String> = (1..=palette_len()).map(mark_css).collect();
        assert!(!palette.contains(&SELECTION_RING.to_owned()));
        // The panel's selected treatment is a neutral ring: no selected rule
        // may change a badge's fill or the row's color to a palette color.
        for (selector, decls) in css_rules() {
            if !selector.contains(".selected") {
                continue;
            }
            for (prop, value) in decls {
                assert!(
                    !(selector.contains(".badge") && prop.starts_with("background")),
                    "{selector} swaps the badge fill"
                );
                assert!(
                    !palette.iter().any(|c| value.contains(c.as_str())),
                    "{selector} uses palette color {value}"
                );
            }
        }
    }
}
