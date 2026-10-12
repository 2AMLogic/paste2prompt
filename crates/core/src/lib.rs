//! Pure-Rust core of paste2prompt.
//!
//! This crate will hold the edit model, history, coordinate normalization,
//! caption generation and PNG export (see `docs/SPEC.md` §Architecture). It
//! must not depend on `web-sys` or anything else browser-specific, so it can
//! be tested natively and compiled for `wasm32-unknown-unknown` alike.

/// The product name, as shown in the UI and in the export caption header.
pub const APP_NAME: &str = "paste2prompt";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_lowercase_product_name() {
        assert_eq!(APP_NAME, "paste2prompt");
    }
}
