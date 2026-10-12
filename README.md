# paste2prompt

Show an LLM the visual change you want, instead of describing it.

paste2prompt is a small web app that runs entirely in your browser. You paste
an image in, mark regions and say what should happen to them, and copy out a
single PNG you can paste into any multimodal LLM chat. Your image never leaves
your machine.

> **Status:** pre-alpha. The design is settled and the code is not written yet.

## How it works

1. **Paste** an image from the clipboard (`⌘V` / `Ctrl+V`).
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
   order and the image-level text.

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
- `crates/web`: the Leptos (CSR) browser shell, built with trunk.

Checks, as CI runs them:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check licenses   # needs cargo-deny; permissive licenses only
```

## Deploy

paste2prompt.com is a Cloudflare Workers static-assets site in the 2AM
Logic account (`wrangler.jsonc`). Deploy from a clean checkout of `main`:

```sh
set -a; . ~/.cloudflare/2amlogic/paste2prompt-deploy.env; set +a
npm ci
npx --no-install wrangler deploy
```

`wrangler deploy` runs the build step: trunk, then `scripts/write_headers.py`,
then the privacy guard. If the guard fails, nothing is uploaded. The token is
`paste2prompt-deploy` in 2am's `infra/cloudflare/token-manifest.yml`.

The zone keeps every script-injecting feature off: Rocket Loader, Web
Analytics automatic setup, Zaraz. `www` redirects to the apex with a 301.

## License

[PolyForm Noncommercial 1.0.0](LICENSE). Free for noncommercial use. For
commercial licensing, contact [2AM Logic](https://2amlogic.com).

Copyright © 2026 2AM Logic Engineering, Inc.
