# paste2prompt

Show an LLM the visual change you want, instead of describing it.

paste2prompt is a small web app that runs entirely in your browser. You paste
an image in, mark regions and say what should happen to them, and copy out a
single PNG you can paste into any multimodal LLM chat. Your image never leaves
your machine.

> **Status:** early. The v1 flow below works end to end; the full design is in
> [`docs/SPEC.md`](docs/SPEC.md).

## How it works

1. **Load** an image: paste it from the clipboard (`⌘V` / `Ctrl+V`), open a
   file (**Open** or `⌘O` / `Ctrl+O`), or drag and drop it onto the window.
2. **Mark a region** with the rectangle or lasso tool.
3. **Attach a transformation** to that region:
   - translate
   - scale
   - mirror
   - rotate
   - text hint (free-form, e.g. "make this button red")
4. **Repeat.** Each edit goes into an ordered history and can be undone.
5. **Add image-level instructions** in the text box at the bottom.
6. **Copy out** one PNG: the original image with numbered annotations drawn on
   it, plus a caption block rendered below that lists every transformation in
   order and the image-level text. Where the browser can't put an image on the
   clipboard, download the PNG instead.

Paste that PNG into Claude, ChatGPT, Gemini or a coding agent. The image
describes itself, so the model doesn't need a plugin, a schema or an API.

## Why an image and not text?

Coordinates in text ("move the logo 40px left") are error-prone for both the
writer and the model. A drawn marker next to a numbered caption ties each
instruction to the exact pixels it refers to. Every chat UI already accepts a
pasted image, so the format works with any model.

## Stack

- Rust compiled to WebAssembly; a static site with no server component.
- Clipboard in and out through the browser Clipboard API (needs a secure
  context).

## Run it locally

The Rust toolchain is pinned in `rust-toolchain.toml`. With
[rustup](https://rustup.rs) installed, the first `cargo` command fetches the
pinned compiler and the `wasm32-unknown-unknown` target. You also need
[trunk](https://trunkrs.dev) (`cargo install trunk --locked`, or a prebuilt
binary from its releases page).

```sh
cd crates/web
trunk serve --open        # dev server with live reload at http://127.0.0.1:8080
trunk build --release     # static site in ./dist at the repo root
```

The repo is a Cargo workspace:

- `crates/core`: the edit model, caption text and PNG export, with no browser
  dependencies. Tested natively.
- `crates/web`: the Leptos (CSR) browser shell, built with trunk. It also
  mounts inside a host page's shadow root, for the embed.
- `crates/embed`: the embed's wasm entry points, built with wasm-pack for
  `packages/element`.

The embed's npm packages are an npm workspace:

- `packages/element`: the `<paste2prompt-overlay>` web component (launcher,
  page capture, the editor in a shadow root). See its README.
- `packages/react`: a React wrapper around it.

Checks, as CI runs them:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check licenses   # needs cargo-deny; permissive licenses only
python3 -B scripts/test_privacy_check.py
```

CI also builds the site with trunk and runs the privacy guard on `dist/`
(see Deploy).

## Deploy

paste2prompt.com is a Cloudflare Workers static-assets site in the 2AM
Logic account (`wrangler.jsonc`). Deploy from a clean checkout of `main`,
with a Cloudflare API token scoped to this site exported as
`CLOUDFLARE_API_TOKEN` (and `CLOUDFLARE_ACCOUNT_ID`). Load them from your own
credential store; never put them in the repo:

```sh
npm ci
npx --no-install wrangler deploy
```

`wrangler deploy` runs the build step: trunk, then `scripts/write_headers.py`,
then the privacy guard. If the guard fails, nothing is uploaded.

The zone keeps every script-injecting feature off: Rocket Loader, Web
Analytics automatic setup, Zaraz. `www` redirects to the apex with a 301.

## License

[PolyForm Noncommercial 1.0.0](LICENSE). Free for noncommercial use. For
commercial licensing, contact [2AM Logic](https://2amlogic.com).

Copyright © 2026 2AM Logic Engineering, Inc.
