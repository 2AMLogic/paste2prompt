# paste2prompt — v1 spec

**Status: v1 draft.** Ordering semantics and UI framework decided 2026-10-12.
The evidence behind design choices is in [`related-work.md`](related-work.md).

## The flow

1. **Load an image** (see Input).
2. **Pick a region tool** (rectangle or lasso), draw a region, then pick an
   operation for it. Each region-plus-operation pair is one **edit**.
3. Edits form an ordered **history**. Undo and redo walk that history.
4. Type **image-level instructions** in the text box under the canvas.
5. **Export** one PNG (see Output): the original image with numbered marks,
   plus a caption block below it.

## Input

Every input path produces the same in-memory image. Loading a new image
while edits exist asks for confirmation first, because it replaces the
session.

| Source | Mechanism | Notes |
|---|---|---|
| Paste | `paste` event on the document, read `clipboardData.items` / `getAsFile()` | No permission prompt in any browser. This is the primary path. |
| File browser | `<input type="file" accept="image/*">` behind an **Open** button and `⌘O` / `Ctrl+O` | |
| Drag and drop | `dragover` / `drop` on the whole window, with a visible drop target while dragging | Accept the first image file; ignore non-images with a message. |

Supported formats are whatever the browser decodes (PNG, JPEG, WebP, GIF
first frame, and so on). Images are decoded once and kept at full
resolution. The canvas view is scaled; exports are not.

## Operations

| Op | Parameters | How it is set |
|---|---|---|
| Translate | dx, dy | Drag the region to where it should go |
| Scale | factor (uniform, or x/y), anchor | Drag a corner handle; the anchor defaults to the region's center |
| Mirror | axis (horizontal or vertical) | Toggle |
| Rotate | degrees, anchor | Drag the rotation handle; Shift snaps to 15° |
| Hint | free text | Text field |

A **hint** can sit on its own or ride along with any other op ("move this
here *and* make it blue").

All coordinates are stored and printed as **% of image width and height**,
so they survive chat apps downscaling the image. Pixel values can be shown
alongside them.

**Ordering semantics (decided 2026-10-12).** Every edit refers to the
**original** image. Order expresses sequence and priority only; edits don't
compose. Every mark is therefore drawn on the pixels the model actually
sees, and the caption states the rule.

As a consequence, **a moved region can't be selected.** The destination
ghost of a translate, rotate or scale is display-only: a new region is
always drawn on original pixels, and clicking a ghost selects nothing. To
change where a region goes, select the *edit* (`V`, or click its row in the
history) and adjust its parameters. Don't stack a second edit on the moved
copy.

## Output

### Image layout

- **Top:** the original image at full resolution, with marks drawn over it.
  - Each region is drawn with a high-contrast outline and a large numbered
    badge.
  - Translate draws a ghost outline at the destination and an arrow to it.
  - Rotate and scale draw a ghost of the transformed outline.
- **Bottom:** a caption band in a clearly separate color, with a fixed header:
  ```
  paste2prompt edit list (user-authored)
  Coordinates are % of image width/height. Edits refer to the original image.
  #1  translate  region (12%,30%)-(40%,52%)  →  move right 15%, up 4%
  #2  rotate     region (60%,10%)-(80%,25%)  →  rotate 90° clockwise about center
  #3  hint       lasso around (50%,70%)      →  "make this button red"
  Overall: tighten the spacing between the cards
  ```
- The caption wraps to the image width, with a minimum width so it stays
  legible on narrow images.

### Export actions

| Action | Shortcut | Mechanism |
|---|---|---|
| **Copy image** | `⌘C` / `Ctrl+C` (when focus isn't in a text field) | `navigator.clipboard.write` with a `ClipboardItem` built **synchronously in the gesture handler**, its value a `Promise<Blob>` (required by Safari) |
| **Download PNG** | `⌘S` / `Ctrl+S` | `<a download="paste2prompt-<timestamp>.png">` holding a blob URL |
| Copy caption as text | `⌘⌥C` / `Ctrl+Alt+C` | `navigator.clipboard.writeText`, for chats that accept text alongside the image |

If Copy image fails (browser without support, insecure context, permission
denied), show a message and offer Download in its place.

Also embed the edit list as JSON in a PNG `iTXt` chunk, so paste2prompt can
reopen its own exports. Chat apps re-encode images, so the visible caption
is what actually carries the instructions.

## Keyboard shortcuts

Single-key tool shortcuts (as in Figma and Excalidraw) are disabled while a
text field has focus. Avoid browser-reserved chords: `Ctrl/⌘+Shift+C`
opens the inspector, `Ctrl/⌘+Shift+T` reopens a tab, and `Ctrl/⌘+W` / `T` / `N`
can't be overridden at all.

| Key | Action |
|---|---|
| `V` | Select / move an existing edit's region |
| `R` | Rectangle region tool |
| `L` | Lasso region tool |
| `T` | Operation: translate |
| `S` | Operation: scale |
| `M` | Operation: mirror (press again to switch axis) |
| `O` | Operation: rotate |
| `H` | Operation: hint (focus the hint field) |
| `Esc` | Cancel the current region or operation |
| `Delete` / `Backspace` | Delete the selected edit |
| `⌘Z` / `Ctrl+Z` | Undo |
| `⌘⇧Z` / `Ctrl+Shift+Z`, `Ctrl+Y` | Redo |
| `⌘V` / `Ctrl+V` | Paste image |
| `⌘O` / `Ctrl+O` | Open file |
| `⌘S` / `Ctrl+S` | Download PNG |
| `⌘C` / `Ctrl+C` | Copy image (outside text fields) |
| `⌘⌥C` / `Ctrl+Alt+C` | Copy caption as text |
| `?` | Show the shortcut sheet |

## Architecture

- **Rust core (`crates/core`)**, pure and with no browser dependency:
  - the edit model, history, and undo/redo;
  - coordinate normalization;
  - caption text generation;
  - export composition (rendering to RGBA, plus PNG encode with the
    `iTXt` chunk).

  Tested natively with `cargo test`.
- **Web shell (`crates/web`)**: the canvas view, input handling, clipboard
  and file I/O, and shortcuts, through `wasm-bindgen` / `web-sys`.
- **UI framework (decided 2026-10-12): Leptos with a Canvas2D view and
  DOM controls.** It gives native text inputs for hints and instructions,
  and accessible buttons. The canvas draws the image, marks and ghosts. The
  export doesn't come from the canvas: the core renders it.
- **Rendering:** draw the export with `tiny-skia` in the core, so the output
  is identical in every browser. Text uses `cosmic-text` with one bundled
  open font (Inter or Noto Sans).
- **Build:** `trunk` produces a static `dist/`. Hosting is a static site
  (Cloudflare Pages / Workers assets) at paste2prompt.com.
- **Privacy:** no network requests after the page loads. CI checks this
  (no `fetch` / `XMLHttpRequest` in the bundle outside the asset loader).

## Out of scope for v1

- Multiple images per session.
- Persisting sessions across reloads (beyond a per-viewer draft of the
  instruction text).
- Sending anything straight to an LLM API.
- Browser extensions or a desktop build.
