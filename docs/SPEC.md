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

**Regions.** A region is a rectangle (`R`) or a lasso (`L`). Regions only
need to be approximate, so a lasso is a smooth closed curve through a few
**control points**, not the raw pointer trace:

- **Curve.** A closed **centripetal** Catmull-Rom spline (alpha = 0.5)
  through every control point. It passes through each stored point and
  doesn't overshoot, cusp or loop when points sit close together. Where it
  would bulge past the image edge it is clamped onto the edge.
- **Point reduction.** When a lasso is finished, Ramer–Douglas–Peucker
  simplifies the pointer trace with a tolerance in **screen pixels**, so it
  behaves the same at any zoom. If more than **10** points survive, the
  tolerance is raised (binary search) until 10 or fewer remain. A trace that
  leaves fewer than 3 points, or points enclosing no area, is not a region
  (as with a rectangle of zero width), and the press counts as a click.
- **Storage.** The control points are what is stored (the JSON keeps its
  `polygon` kind and `points` list). Everything that draws, hit-tests or
  measures a lasso, on the canvas and in the export alike, uses the same
  flattened curve: its outline, ghosts, bounding box, center, badge
  placement, and the caption's centroid and box.
- **Editing.** With `V`, a selected lasso shows a handle on each control
  point. Dragging one reshapes the curve; the drag is one undo step, and
  the point stays inside the image.

| Op | Parameters | How it is set |
|---|---|---|
| Translate | dx, dy | With translate active, drag from the region to where it should go. This sets the destination |
| Scale | factor (uniform, or x/y), anchor | Drag a corner handle. The drag scales x and y independently; hold Shift to keep the aspect ratio (uniform). The anchor defaults to the region's center |
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

**Dragging with `V` versus dragging with translate.** The two drags do
different things:

- With `V`, you select an edit. Dragging its region moves the **source**
  region on the original image, so the edit re-picks which original pixels
  it refers to. The operation's parameters (such as a translate's dx/dy)
  stay the same.
- With the translate operation active, dragging sets the **destination**
  (dx, dy). The source region stays where it is.

## Output

### Image layout

- **Top:** the original image at full resolution, with marks drawn over it.
  - Each region is drawn with a high-contrast outline and a large numbered
    badge. The badge sits just outside the region's top-left corner,
    touching the outline, so it never covers the pixels it labels. Where
    that doesn't fit on the image it goes beside or below the region, and
    only when nothing outside fits (e.g. a region filling the image) does it
    sit on the corner, inside. The canvas uses the same placement.
  - Translate draws a ghost outline at the destination and an arrow to it.
  - Rotate and scale draw a ghost of the transformed outline.
  - Mirror draws a ghost of the mirrored outline, plus a glyph on the
    mirror axis.
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
| **Copy image** | `⌘C` / `Ctrl+C`, only when focus isn't in a text field **and** there is no non-empty page text selection; otherwise the browser's normal copy applies | `navigator.clipboard.write` with a `ClipboardItem` built **synchronously in the gesture handler**, its value a `Promise<Blob>` (required by Safari) |
| **Download PNG** | `⌘S` / `Ctrl+S` | `<a download="paste2prompt-<timestamp>.png">` holding a blob URL |
| Copy caption as text | `C` (only when focus isn't in a text field), plus a **Copy caption** button | `navigator.clipboard.writeText`, for chats that accept text alongside the image |

If Copy image fails (browser without support, insecure context, permission
denied), show a message and offer Download in its place.

The visible caption is what carries the instructions. Chat apps re-encode
images, so no metadata survives them. Embedding the edit list in the PNG is
not part of v1 (see Out of scope).

## Keyboard shortcuts

Single-key shortcuts (as in Figma and Excalidraw), including `C` for Copy
caption, are disabled while a text field has focus. Avoid browser-reserved
chords:

- `⌘⇧C` / `Ctrl+Shift+C` opens the inspector.
- `⌥⌘C` / `Ctrl+Alt+C`: on macOS, `⌥⌘C` opens DevTools (the element
  inspector in Chrome and Firefox, and the JavaScript console in Safari
  when the Develop menu is on). Avoid `Ctrl+Alt+C` too, since Ctrl+Alt is
  AltGr on many European layouts.
- `⌘⇧T` / `Ctrl+Shift+T` reopens a closed tab.
- `⌘W` / `Ctrl+W` (close tab), `⌘T` / `Ctrl+T` (new tab) and `⌘N` / `Ctrl+N`
  (new window) can't be overridden at all.

| Key | Action |
|---|---|
| `V` | Select an existing edit; dragging moves its source region on the original image |
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
| `⌘C` / `Ctrl+C` | Copy image (outside text fields, and only with no page text selected) |
| `C` | Copy caption as text (also a button) |
| `?` | Show the shortcut sheet |

Letters match the character printed on the key, so on AZERTY or Dvorak
`M` is the key labelled M. When a layout types a letter from a non-Latin
script (Cyrillic, Greek, Hebrew, Arabic, Indic, Thai, Khmer, Tibetan,
Ethiopic, Korean, Japanese kana…), the physical key decides instead
(`KeyboardEvent.code`), so `Ctrl+O` is the key with O printed on it. Latin
letters outside ASCII (Turkish `ı`, BÉPO `è`, Ewe `ɔ`) and symbols (Linux
AltGr's `µ`) never fall back to the physical key, and nothing fires with
AltGr held. The non-Latin scripts are an allowlist of Unicode blocks, one
for the common non-Latin scripts with OS keyboard layouts (an unlisted
script simply gets no shortcut fallback), and only the letters in those blocks count. Many symbols are letters to
Unicode (letterlike `ℓ`, circled `ⓜ`, Roman numeral `Ⅿ`, fullwidth `Ｍ`).
A denylist of Latin blocks would let each one through. With the allowlist,
a script nobody listed only leaves the shortcuts on that layout doing
nothing. `?` matches the typed character, and holding
it doesn't toggle the sheet. The sheet shows ⌘ on macOS and Ctrl elsewhere.

## Architecture

- **Rust core (`crates/core`)**, pure and with no browser dependency:
  - the edit model, history, and undo/redo;
  - coordinate normalization;
  - caption text generation;
  - export composition (rendering to RGBA, plus PNG encode).

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
  That bundle scan can't see scripts injected at the edge, so edge-injected
  scripts such as Cloudflare Web Analytics or Zaraz must stay disabled for
  the site.

## Out of scope for v1

- Multiple images per session.
- Persisting sessions across reloads (beyond a per-viewer draft of the
  instruction text).
- Sending anything straight to an LLM API.
- Browser extensions or a desktop build.
- **Reopening exports (optional follow-up, later than v1).** A later
  version may embed the edit list as JSON in a PNG `iTXt` chunk so that
  paste2prompt can read it back. Only Download keeps that chunk reliably:
  clipboard copies may re-encode the PNG and strip `iTXt`. The exported
  pixels already have the marks and caption drawn in, so "reopen" would
  restore the edit list as data, not a clean original.
