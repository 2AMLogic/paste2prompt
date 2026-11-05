# paste2prompt embed — design draft

Status (2026-11-05): phases 1, 2 and 5 are built; phase 4 is built (Open
issue and `onSubmit`); phase 3 (the caption extension) is not started;
phase 6 (dogfooding) is not tracked here. Each phase below says what built it.
Open questions are listed at the end and block the phases that depend on them.
This draft began as a proposal on 2026-10-17; sections that describe design
choices (Architecture, Caption format extension) still read as design, and the
notes marked **Built** or **Not built** say where the code differs.

## Why

The standalone app takes a pasted image. For web app work that loses what the
model most needs: *which element* a mark refers to. A lasso at (60%,82%) makes
the model guess the component; a selector such as `button.save` names it.

The embed puts paste2prompt inside a host web app as an overlay. A floating
launcher button opens it; the user captures the page, picks elements or draws
regions, attaches edits, and gets the same self-describing PNG as today, with
element identity added to the caption.

The first host is our own `dashboard.2amlogic.com`: browse the dashboard, see
something wrong, mark it up, and turn it into a GitHub issue without leaving
the page.

## What it is not

- **Not an agent bridge.** [ProPanes](https://github.com/tinkerer/propanes)
  (MIT-licensed) pairs a similar overlay with a server that lets agents
  drive the live page. The embed has no server and opens no connections. It
  produces a PNG and hands it to the user or the host.
- **Not a fork of the editor.** The edit model, caption and PNG render stay in
  `crates/core`, so the app and the embed produce the same format.

Closest prior art beyond ProPanes: React Grab and Agentation (see
[`related-work.md`](related-work.md) §1). Both output text for an agent. The
embed outputs a PNG that any chat can read, with the text facts drawn into it.

## Flow

1. The host mounts the overlay. A launcher button floats in a corner (position
   configurable through the `position` attribute; hidden with `no-launcher`.
   **Not built:** a hotkey.)
2. **Capture.** Click the launcher: the overlay hides itself and captures the
   visible viewport. (**Not built:** capturing one picked element, cropped.
   The capture is always the viewport: `packages/element/src/capture.ts`.)
3. **Mark.** The editor opens over the page with the capture loaded. Tools:
   - **Element** (built, `packages/element/src/picker.ts`): hover outlines DOM elements in the live page beneath;
     a click adds a rectangle region from the element's bounding box and
     records its identity (below). Picking happens *before* the capture is
     frozen, so the boxes come from the real DOM.
   - **Rectangle** and **Lasso**: unchanged.
4. **Edit.** Attach ops exactly as in the app.
5. **Output.** Copy PNG, Download, Copy caption (as today), plus the host
   actions under [Outputs](#outputs).

## Element identity

For a picked element the embed records:

| Field | Source | Notes |
|---|---|---|
| `selector` | `[data-testid="…"]` if that is unique, else built upward from the element (`#id`, or `tag.class` with `:nth-of-type`) until `querySelectorAll` matches one node (ProPanes' approach); at most 512 characters, longer ones are shortened by dropping leading segments (`packages/element/src/selector.ts`) | the overlay is never picked: it lives in its own shadow root and `picker.ts` ignores events from it |
| `label` | `aria-label`, else `data-testid`, else trimmed text (≤ 40 chars) | what a human would call it; optional |
| `role` / `tag` | `role` attribute or tag name; each at most 64 characters | `role` is optional |
| `component` | React only, dev builds: nearest component display name from the fiber | **Not built.** Not in `crates/core/src/element.rs` or the picker |
| `rect` | `getBoundingClientRect()` × `devicePixelRatio`, offset by the capture origin | becomes the region |

The core checks every field when it is built and when a document is loaded:
one line each, no invisible formatting characters
(`crates/core/src/element.rs`, `MAX_SELECTOR_CHARS`,
`MAX_ELEMENT_LABEL_CHARS`).

## Caption format extension

**Not built.** `ElementRef` is stored in the document JSON only; the caption
does not print it (`crates/core/src/element.rs`, module comment). The lines
below are the proposal.

Rule 4 applies: this changes the output format, so it must show that it helps
a model read the image. Proposed lines:

```
paste2prompt edit list (user-authored)
Coordinates are % of image width/height. Edits refer to the original image.
Page: dashboard.2amlogic.com/runs at 1440×900
#1  hint  element button.save "Save" (60%,82%)-(70%,86%)  →  "make this primary"
#2  translate  region (12%,30%)-(40%,52%)  →  move right 15%, up 4%
Overall: the toolbar is too crowded
```

- `Page:` gives origin and path only. The query string and fragment are left
  out by default because they often carry tokens or IDs; the host can opt in.
- `element <selector> "<label>"` takes the place of `region`. The percentages
  stay, so the line still ties to the drawn mark.
- A named region keeps its quoted name first, then `element …`.

**Evidence to collect before merging this format:** the same 5 to 10 dashboard
edits, sent to two or three current models with and without the `element`
fields, asking each for the code change. We'd score whether it edits the
right component, and commit the PNGs and results under `docs/examples/embed/`.

## Architecture

```
host app
 └─ <paste2prompt-overlay>          web component, shadow DOM (styles isolated)
     ├─ launcher, element picker,   TypeScript: work on the live DOM
     │  capture
     └─ editor                      existing Leptos UI, mounted into the
         └─ paste2prompt-core       shadow root instead of <body>; same core
```

- **Packages**
  - `@2amlogic/paste2prompt-element` (built, `packages/element`): the web
    component plus the wasm. Works from a `<script>` tag or any framework.
  - `@2amlogic/paste2prompt-react` (built, `packages/react`): a thin
    wrapper, `<Paste2Prompt … />`, with `open()` and `close(force?)` on its
    ref. **Not built:** the `usePaste2Prompt()` hook and a public `capture()`.
- **Reuse the Leptos editor** (built: `crates/embed`, `crates/web/src/embed.rs`
  and `dom.rs`) by mounting it into the shadow root
  (`mount_to` in place of `mount_to_body`), with `style.css` adopted into the
  shadow root. That keeps one editor implementation. The alternative, a
  TypeScript editor over a headless wasm core, means two UIs to keep in step;
  we'd only choose it if Leptos doesn't behave inside a shadow root (global
  key handlers, focus and `document()` lookups need checking).
- **Lazy load** (built, `packages/element/src/load-wasm.ts`). The release
  wasm is roughly 580 KB gzipped (`packages/element/README.md`; approximate,
  re-measure with the release build before quoting it). The launcher is a
  few KB; the wasm loads on first open.
- **Capture**
  - Default (built, `packages/element/src/capture.ts`): `html-to-image`
    (MIT), which re-renders the DOM to a canvas. It
    needs no permission prompt but isn't pixel-exact: cross-origin images
    without CORS and some CSS effects drop out.
  - Opt-in: `getDisplayMedia`, for real pixels behind a browser prompt.
    **Not built.**
  - The overlay is always excluded from the capture and from picking.
- **Mobile.** The launcher works by tap; the element tool uses
  tap-to-preview, tap-again-to-pick (`picker.ts` handles touch pointers
  apart from mouse hover). Copying to the clipboard needs a user
  gesture, so the output step always ends on an explicit button.

## Outputs

| Action | Network? | Notes |
|---|---|---|
| Copy PNG / Download / Copy caption | no | as the app does today |
| **Open GitHub issue** (`issue-repo` attribute, `issueRepo` prop). **Built** (PR #89; `crates/embed/src/issue.rs`, `packages/element/src/overlay.ts`) | no, it only opens a URL | opens `github.com/<repo>/issues/new?title=…&body=<caption>` in a new tab and copies the PNG to the clipboard; the user pastes it into the issue. The body stays under GitHub's URL length limit (it's cut short with a note if needed). |
| **`onSubmit` host callback** (`onSubmit` property / prop with a `submit-notice`). **Built** (`packages/element/src/overlay.ts`, `submit.ts`) | the host's choice | receives `{ png: Blob, caption: string, document: <core JSON>, page }` once, on an explicit click of **Submit**, which shows only when the host sets both `submit-notice` (shown before the button) and `onSubmit`. The embed itself sends nothing. |

The GitHub REST API has no endpoint for uploading issue attachments, so a
one-click "file with image" flow needs somewhere to host the image. That
belongs to the host (for the dashboard, its own backend) through `onSubmit`,
not to the embed.

## Phases

0. **Decisions:** the open questions below. Questions 2 and 3 are resolved; 1 is
   still open.
1. **Shadow-root mount: built** (PRs #88, #94; `crates/embed`,
   `crates/web/src/embed.rs`). The Leptos editor mounts into a given shadow
   root and takes an image handed over from JS. `@2amlogic/paste2prompt-element`
   has the launcher and the capture (`packages/element/src/overlay.ts`,
   `capture.ts`).
2. **Element picker: built** (PR #94; `packages/element/src/picker.ts`,
   `crates/core/src/element.rs`). The region-with-identity data is stored in
   the document JSON; not printed in the caption.
3. **Caption extension: not started.** Run the evidence comparison above,
   then print the `Page:` and `element` lines.
4. **Outputs: built.** The GitHub issue button (PR #89,
   `crates/embed/src/issue.rs`) and the `onSubmit` Submit button
   (`packages/element/src/overlay.ts`, `submit.ts`).
5. **`@2amlogic/paste2prompt-react` wrapper: built** (PR #88,
   `packages/react/src/index.ts`; 0.2.0 in PR #100). **Release: partly
   built.** `.github/workflows/publish-packages.yml` publishes both packages
   to GitHub Packages (PR #90) and each `package.json` carries
   `PolyForm-Noncommercial-1.0.0`. A release to public npm under a decided
   licence is not built; it blocks on open question 1.
6. **Dogfood on dashboard.2amlogic.com:** not tracked in this repo.

## Open questions

1. **License of the embed packages.** The repo is PolyForm Noncommercial
   1.0.0, which forbids commercial host apps. Options: keep that, release the
   embed packages under MIT/Apache-2.0, or offer commercial licenses
   separately. Our own dashboard is fine either way; outside adoption isn't.
   Operator decision, to be recorded in `CLAUDE.md`.
2. **Rule 1 scope.** Two outputs carry user content off the page:
   - **Open GitHub issue** puts the caption text (plus title, labels and page
     address) in a github.com URL. Decided: the operator recorded an embed
     exception to rule 1 in `CLAUDE.md` on 2026-10-17, which lands with the
     Open issue code (PR #89). That exception covers only this button.
   - **`onSubmit`** hands the user's capture to host code that may upload it.
     Resolved (operator, 2026-11-05, recorded in `CLAUDE.md` rule 1): the
     library makes no network calls and the host decides, under five terms:
     opt-in with a destination notice shown before the button, callback on
     an explicit click only, no network code in the embed, `crates/web`
     untouched.
3. **Porting ProPanes code.** Resolved: ProPanes is MIT-licensed (its
   `LICENSE` landed in tinkerer/propanes `5082b26`, 2026-10-17). Ported code
   keeps an MIT header with the copyright line and permission notice, and
   the commit records the upstream path and commit.
