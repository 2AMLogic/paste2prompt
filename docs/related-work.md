# Related work

Survey taken 2026-10-12. Star counts are from the GitHub API on that date.
Anything marked **[unverified]** could not be confirmed directly and needs
checking before anyone relies on it.

## Summary

There is one close relative (Pinpoint) and a crowded field of tools that
point at a live UI so an agent can edit the code. No tool combines all four
properties paste2prompt aims for:

- **Works with any image and any LLM.** Other tools are bound to a DOM, an
  IDE, a particular agent, or macOS. paste2prompt takes any pasted image
  (photos, charts, mockups, games) and its output pastes into any chat.
- **One self-describing PNG.** Only Pinpoint bakes its legend into the
  image. The rest rely on a text or markdown sidecar that chat apps may drop.
- **Ordered transformations with parameters.** Existing tools offer
  "comment here". None has translate, scale, mirror or rotate operations.
  Spatial edits are exactly where models are weakest (VIBE, below), so
  spelling them out in the caption adds real value.
- **Fully client-side, with no install or account.**

## 1. Direct competitors and near-identical ideas

### Closest: PNG plus text, for any chat

- **[Pinpoint](https://github.com/croustibat/Pinpoint)** (MIT, ~15★, active
  2026-09). A native macOS menu-bar app. You capture a region, drop numbered
  markers with notes, and copy an annotated image plus structured text: the
  image size, each marker's position and note, then your instructions.
  This is the nearest relative.
  [Write-up](https://dev.to/croustibat44/stop-describing-pixels-to-your-ai-agent-annotate-them-instead-1ofp).
  Two choices worth copying:
  - **Percentage coordinates.** Positions stay correct after chat apps
    downscale the image.
  - **The legend is drawn into the image**, because chat apps can drop text
    from the clipboard. paste2prompt's caption block rests on the same
    reasoning.

  What it lacks: it only runs on macOS, it has point markers but no regions
  or lasso, and it has no transformations.
- **[MarkUp](https://markupextension.com/)**, a Chrome extension. Numbered
  callouts, with one click copying a PNG plus a markdown changelog that
  includes CSS selectors. Live webpages only.
- **[screenshot-annotator](https://github.com/KI4JLU/screenshot-annotator)**,
  a Chrome extension. You mark an element or area, add a comment, and the
  annotated screenshot is copied for Claude Code or Codex.
- **[ImageToolForLLM](https://github.com/chldbwnstm/ImageToolForLLM)**, a
  Claude Code plugin. You draw numbered boxes, and it writes
  `*.annotated.png` plus `*.regions.json` (each box as `[x,y,w,h]`). It is
  tied to Claude Code.

### Point at the live UI, the agent edits the code (DOM-aware, framework-bound)

- **[React Grab](https://github.com/aidenybai/react-grab)** (~7.7k★). Hover
  an element and press Cmd+C to copy its file, component and HTML for an
  agent.
- **[Agentation](https://github.com/benjitaylor/agentation)** (~4.9k★).
  Annotate a live page by clicking elements, selecting text or drawing areas.
  Outputs markdown with selectors, styles and the component tree. Has an MCP
  server, and agents can acknowledge or resolve annotations.
- **[Stagewise](https://github.com/stagewise-io/stagewise)** (~6.8k★). Began
  as a browser toolbar that sends DOM, screenshot and metadata to IDE agents.
  It now describes itself as an "agentic IDE".
- **Drawbridge**, Browser Annotations, Qursor: Chrome extensions that queue
  comment and rectangle tasks for Cursor or Claude Code.
- **[Onlook](https://github.com/onlook-dev/onlook)** (~26.9k★). A Figma-like
  visual editor over live Next.js and Tailwind code.
- **Hosted builders:**
  - [v0 Design Mode](https://v0.app/docs/design-mode).
  - [Bolt Visual Edits](https://support.bolt.new/building/visual-edits).
  - Lovable Visual Edits **[unverified]**.
  - Cursor Design Mode (Cursor 3, 2026).
- **Figma Dev Mode MCP server** and Figma Make.
- **[tldraw Make Real](https://github.com/tldraw/make-real)** (~5.4k★). The
  canvas selection goes to a vision model as a PNG and comes back as HTML.
  This is the original "the drawing is the prompt" demo.

How these relate to paste2prompt: every tool in this group needs a running
web app or a particular editor or agent. None accepts an arbitrary pasted
image and returns a portable PNG.

## 2. Screenshot annotation tools (closest UX analogues)

| Tool | Platform | Paste in | Copy PNG out | Notes |
|---|---|---|---|---|
| Excalidraw (133.7k★) | Web, client-side, open source | Yes | Yes | Good reference for the paste flow and undo |
| tldraw (50.9k★) | Web SDK | Yes | Yes | Production use needs a license key **[unverified current terms]** |
| Flameshot (31.2k★) | Desktop (Qt) | No | Yes | Its numbered "circle counter" tool is a direct analogue |
| ShareX (40k★) | Windows | Partial | Yes | Has a step-counter tool |
| Shottr, CleanShot X, Annotely, Skitch | macOS, native | Partial **[unverified]** | Yes | Skitch is effectively abandoned |
| Markup Hero | Web and desktop, cloud | Yes | Yes | Needs an account; not private |
| CopyPasteDesign, Cobalt Capture, MarkShot | Web or extension | Yes | Yes | Closest paste, mark up, copy flow, but nothing aimed at LLMs |

Two conventions are common across these tools and worth adopting: numbered
step markers, and a single action that copies the result.

## 3. Research on visual prompting of multimodal LLMs

- **What does CLIP know about a red circle?** (Shtedritski et al., 2023,
  [2304.06712](https://arxiv.org/abs/2304.06712)). Models trained on web
  data already read a red circle as "look here". High-contrast red outlines
  make a good default.
- **Set-of-Mark prompting** (Yang et al., 2023,
  [2310.11441](https://arxiv.org/abs/2310.11441)). Number labels on regions
  let text refer to "region 3" reliably. Number the marks, and repeat the
  numbers in the caption.
- **List Items One by One** (2024, [2404.16375](https://arxiv.org/abs/2404.16375)).
  Training models to enumerate tagged items improves how well they follow
  numbered marks.
- **ViP-LLaVA** (CVPR 2024, [2312.00784](https://arxiv.org/abs/2312.00784)).
  Models can read free-form marks drawn into the image (boxes, arrows,
  scribbles), especially when the text names the mark.
- **Draw-and-Understand / SPHINX-V** (ICLR 2025,
  [2403.20271](https://arxiv.org/abs/2403.20271)). Points, boxes and
  free-form shapes, trained on data that includes screenshots. Lasso-style
  regions are a supported input.
- **Ferret** ([2310.07704](https://arxiv.org/abs/2310.07704)) and **Osprey**
  ([2312.10032](https://arxiv.org/abs/2312.10032)) take region inputs
  natively. Lesson: put explicit coordinates next to the visual marks, so the
  caption still works when a mark is misread.
- **Visual Prompting in MLLMs: A Survey** (2024,
  [2409.15310](https://arxiv.org/abs/2409.15310)). A taxonomy of marker types.
- **VP-Bench** (AAAI 2026, [2511.11438](https://arxiv.org/abs/2511.11438)).
  Tests how well models perceive 8 marker shapes. The per-shape ranking is
  **[unverified]**.
- **Visually Prompted Benchmarks Are Surprisingly Fragile** (2025,
  [2512.17875](https://arxiv.org/abs/2512.17875)). Marker color, size, shape,
  label position, font size and even JPEG quality change accuracy by up to
  ~21%, and no single style is best for every model. **Design implication:**
  - Encode every mark redundantly: color, number, and coordinates in the
    text.
  - Use large labels in a consistent position.
  - Export lossless PNG.
- **VIBE: How Well Do Models Follow Visual Instructions?** (2026,
  [2602.01851](https://arxiv.org/abs/2602.01851)). Of the edits it tested,
  translation and reorientation are the hardest, and combining several drawn
  instructions degrades results sharply. **Design implication:** state every
  move, rotate and scale as explicit text in the caption ("move #2 right by
  15% of the width"). Don't rely on arrows alone.
- **SketchVLM** (2026, [2604.22875](https://arxiv.org/abs/2604.22875)). The
  model produces editable SVG overlays. It works in the reverse direction,
  but suggests vector marks as a shared language between user and model.
- **Typographic prompts and injection risk:**
  - FigStep (AAAI 2025, [2311.05608](https://arxiv.org/abs/2311.05608)):
    instructions typed into an image bypass safety alignment.
  - AgentTypo ([2510.04257](https://arxiv.org/abs/2510.04257)): optimizes
    text in images to hijack agents.

  These confirm that models read rendered text, which is the premise of this
  tool. They also mean:
  - Providers may increasingly treat instructions found inside images as
    untrusted, so a short accompanying text prompt may be needed ("follow
    the caption in this image").
  - The pasted source image may itself contain injected text. Render the
    caption in a clearly separate band with a fixed header, so the model can
    tell user intent from image content.

## 4. Prior art for edit-instruction formats

- **InstructPix2Pix** ([2211.09800](https://arxiv.org/abs/2211.09800)) takes
  a single free-text instruction for the whole image. **Emu Edit**
  ([2311.10089](https://arxiv.org/abs/2311.10089)) defines a task taxonomy
  (add, remove, background, global, local), which could inform the categories
  of text hint.
- **Drag-based editing:**
  - DragGAN ([2305.10973](https://arxiv.org/abs/2305.10973)).
  - DragDiffusion ([2306.14435](https://arxiv.org/abs/2306.14435)).
  - DragonDiffusion ([2307.02421](https://arxiv.org/abs/2307.02421)).

  They share one format: a handle point, a target point, and an optional mask
  of the area allowed to change. That maps directly onto translate (a region
  plus a delta vector), and it is the most borrowable structure here.
- **Mask- and brush-based editing:** inpainting with masks (SD, FLUX Fill),
  MagicQuill **[unverified arXiv ID]**, and Nano Banana's sketch mode.
- **[Bria FIBO-Edit](https://docs.bria.ai/image-editing-overview)**: JSON
  edit instructions plus an optional mask. It is the closest existing
  structured edit schema. Its fields are **[unverified]**.
- **Agent-style editors** (e.g. GenArtist **[unverified arXiv ID]**) break
  an edit into ordered tool calls over bounding boxes, which resembles
  paste2prompt's ordered history.

There is no standard ordered edit-list format, so we define our own. A
starting point:

```
#n · op (translate | scale | mirror | rotate | hint)
   · region (bbox or polygon, in % of image size)
   · params (dx,dy in % | factor | axis | degrees, plus anchor point)
   · note
```

**Open design decision:** do coordinates in later items refer to the
original image, or to the image after the earlier items are applied?
Pick one, and state the rule in the caption itself.

## 5. Rust/WASM building blocks and gotchas

**UI shell:**
- **egui/eframe** (31.3k★): the fastest route to a canvas editor. Weak on
  accessibility. Its web clipboard support is text only, so image paste
  needs our own `paste` event listener.
- **Leptos, Yew or Dioxus** with a `web-sys` Canvas2D (or an SVG overlay):
  a DOM-based UI with native text inputs, which suits the instruction box
  better.

**Rendering the export:**
- **tiny-skia**: deterministic CPU rasterizing of strokes, lassos and fills.
- **resvg**: use it if the annotations live as SVG.
- **`image`**: PNG encode and decode.
- Or use the browser's `canvas.toBlob("image/png")`, which is simpler and
  smaller to ship.

**Caption text:**
- **cosmic-text** handles shaping, wrapping and bidirectional text.
- **ab_glyph** or **fontdue** are lighter, but we would write the wrapping
  ourselves.
- Or draw the caption with Canvas2D `fillText` and wrap manually.
- Bundling a font (Inter or Noto Sans) adds to the size of the download.

**Clipboard:**
- **Paste in:** listen for the `paste` event and read
  `clipboardData.items` / `getAsFile()`. This works everywhere with no
  permission prompt. Offer `navigator.clipboard.read()` only as an optional
  button, because Firefox shows a confirmation menu for it.
- **Copy out:** `navigator.clipboard.write([new ClipboardItem({"image/png": promise})])`.
  - It needs a secure context (HTTPS, or localhost) and a user gesture.
  - Chrome accepts only `image/png` for images.
  - **Safari:** create the `ClipboardItem` synchronously inside the click
    handler, with a `Promise<Blob>` as its value
    ([WebKit 222262](https://bugs.webkit.org/show_bug.cgi?id=222262)). In
    Rust, pass `wasm_bindgen_futures::future_to_promise(render_png())`
    straight into the item. Awaiting the render first throws
    `NotAllowedError`.
  - Firefox supports it by default since about version 127. Keep a
    **Download PNG** fallback for older versions.
- `web-sys` features needed: `Clipboard`, `ClipboardItem`, `ClipboardEvent`,
  `DataTransfer`. Check whether they still need `web_sys_unstable_apis`
  **[unverified]**.
- `arboard` is native-only and doesn't help here.

**Metadata:** we could also embed the edit list as JSON in a PNG `iTXt`
chunk, so the app can re-open its own exports. Chat apps re-encode images,
so treat that copy as a bonus: the visible caption is what actually carries
the instructions.

## Ideas to borrow

- Percentage coordinates and a legend drawn into the image (Pinpoint).
- Large numeric labels, repeated in the caption (Set-of-Mark).
- Redundant encoding of every mark: color, number, coordinates (the
  fragile-benchmarks paper).
- A handle-to-target vector for translate, and an anchor point for rotate
  and scale (the drag-editing papers).
- A fixed caption header, e.g. `paste2prompt edit list (user-authored)`,
  marking the caption as user intent rather than image content.
- An optional **Copy text too** button that puts the same edit list on the
  clipboard as text.
- Later, optionally: a loop where the agent acknowledges or resolves each
  annotation (Agentation).

Before fixing our output format, read Pinpoint's (MIT). That way we can
deliberately differ from it, or stay compatible with it.
