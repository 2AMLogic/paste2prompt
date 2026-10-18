# @paste2prompt/element

paste2prompt as an overlay for any web page, in a web component:

```html
<paste2prompt-overlay position="bottom-right"></paste2prompt-overlay>
<script type="module">
  import { defineOverlay } from "@paste2prompt/element";
  defineOverlay();
</script>
```

Clicking the launcher captures the viewport and opens the editor (the
standalone app's editor, `crates/web`, mounted in the element's shadow root)
with the capture loaded. The output is the usual paste2prompt PNG, copied or
downloaded. Nothing is sent anywhere.

- **Attributes:** `position` (`bottom-right`, `bottom-left`, `top-right`,
  `top-left`), `no-launcher` (hide the button; call `open()` yourself),
  `issue-repo` and `issue-labels` (below).
- **Property:** `issueContext`, extra lines for the issue body.
- **Methods:** `open()`, `close(force?)` (asks before discarding marks
  unless `force`), `isOpen`.
- **Events:** `paste2prompt-open`, `paste2prompt-close`.

The editor is about 580 KB of wasm (gzip), loaded on first open; until then
the page only carries the launcher. Load it through a bundler that handles
`new URL("…", import.meta.url)` (Vite, webpack 5, esbuild with a copy step).

## Open issue

With `issue-repo="owner/name"`, the editor gets an **Open issue** button. It
copies the annotated PNG to the clipboard, then opens the repository's
new-issue page with these filled in:
- a title from the first line of the image-level instructions;
- `issue-labels`, which GitHub applies only if the user may label issues;
- a body with the page address, `issueContext` and the caption text.

The user pastes the image into the issue and submits it on github.com, so
nothing is sent by the page. The address leaves out the query string, which
often carries tokens, but keeps the hash, where hash-routed apps keep their
route. If the browser blocks the new tab, the editor shows a link to it
instead.

```html
<paste2prompt-overlay issue-repo="2AMLogic/loom-ui" issue-labels="loom:triage"></paste2prompt-overlay>
<script type="module">
  document.querySelector("paste2prompt-overlay").issueContext = `**Build:** ${BUILD}`;
</script>
```

## Capture

The viewport is captured with [html-to-image](https://github.com/bubkoo/html-to-image)
(MIT), which re-renders the DOM into a canvas: no permission prompt, but not
pixel-exact. Cross-origin images without CORS headers come out blank, and
`position: fixed` elements are misplaced on a scrolled page. To inline the
page's own images and fonts it fetches them again, from wherever the page
loaded them.

## Build

Needs the Rust toolchain (`rust-toolchain.toml`) and
[wasm-pack](https://rustwasm.github.io/wasm-pack/).

```sh
npm run build -w @paste2prompt/element   # wasm + bundle + types into dist/
npm run demo -w @paste2prompt/element    # dist/demo.js for demo/index.html
python3 -m http.server                   # from the repo root; open /packages/element/demo/
```
