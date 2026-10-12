# paste2prompt — repo rules

These rules sit on top of the 2AM workspace rules in
[`2AMLogic/2am` `CLAUDE.md`](https://github.com/2AMLogic/2am/blob/main/CLAUDE.md).
Where they conflict, the workspace rules win.

## What this repo is

A client-side web app, Rust compiled to WebAssembly. The user pastes an image,
marks regions (rectangle or lasso), and attaches ordered transformations to
them: translate, scale, mirror, rotate, or a text hint. The app then renders a
single PNG with the annotations drawn on it and a caption block below, ready
to paste into any LLM chat as a visual prompt.

## Rules

1. **Client-side only.** No server, no telemetry, no network calls with user
   content. A user's image never leaves the browser. Anything that would
   change this needs an operator decision recorded in this file.
2. **Public repo.** Everything pushed here is world-readable at once. The
   invention firewall applies (workspace `CLAUDE.md` rule 3): never work here
   in a session that holds `notebook` content.
3. **License is PolyForm Noncommercial 1.0.0.** Every new source file is
   covered by `LICENSE`. Don't add dependencies whose licenses forbid this
   (e.g. anything that would force a copyleft relicense). Prefer MIT, Apache-2.0
   or similar permissive licenses.
4. **The output PNG is the product.** Its layout (annotation markers, numbering,
   caption block) is a format that models have to read reliably. Changes to
   it should say why they help a model read it, ideally with an example.
5. **Commits** land on `main` through merge commits only, with a body that
   explains the reasoning and carries the provenance trailers
   (workspace `CLAUDE.md` rules 7 and 11).
