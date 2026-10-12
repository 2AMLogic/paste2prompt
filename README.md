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

## License

[PolyForm Noncommercial 1.0.0](LICENSE). Free for noncommercial use. For
commercial licensing, contact [2AM Logic](https://2amlogic.com).

Copyright © 2026 2AM Logic Engineering, Inc.
