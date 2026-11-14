# paste2prompt roadmap

paste2prompt turns a pasted image plus marked regions and ordered edits into
one self-describing PNG that any multimodal LLM chat can read, entirely in the
browser.

Status as of 2026-11-13, `main` @ `d0bbb70`.

Design: [`SPEC.md`](SPEC.md) (standalone app) and [`EMBED.md`](EMBED.md) (overlay
for host web apps).

## Where it stands

**Standalone app (v1 flow): built** (README: "early", v1 flow works end to end).

- `crates/core` holds the edit model, history, caption text and PNG export
  with no browser dependency. CI checks that it builds for
  `wasm32-unknown-unknown` without `web-sys`, `js-sys` or `wasm-bindgen`
  (`.github/workflows/ci.yml`).
- All six operations in SPEC §Operations exist (`crates/core/src/op.rs`):
  translate, scale, mirror, rotate, hint and label, with named regions.
  Lassos are closed centripetal Catmull-Rom splines
  (`crates/core/src/geometry.rs`).
- The export is pinned by golden PNGs (`crates/core/tests/golden/`) and by
  text-layout snapshots (`crates/core/src/render/text/snapshots/`, #51). The
  text path is harfrust + read-fonts + unicode-linebreak over a bundled Inter
  subset; cosmic-text was removed (#43).
- `crates/web` is the Leptos shell: paste, file open and drag and drop
  (`src/input/`), the keymap with its non-Latin layout fallback
  (`src/keymap/`, #53, #57, #64), responsive layout (#69, #71, #73), Start
  over and Clear edits (#68).
- The privacy guard (`scripts/privacy_check.py`, tests in
  `scripts/test_privacy_check.py`) runs on the built `dist/` in CI and in the
  deploy build; a failing guard uploads nothing (README §Deploy).

**Embed (epic #86): phases 1, 2, 4 and 5 built; phase 3 not started.**

| Phase | State | Evidence |
|---|---|---|
| 0. Decisions | resolved | `EMBED.md` §Open questions 1 to 3; `CLAUDE.md` rules 1 and 3 |
| 1. Shadow-root mount | built | PRs #88, #94; `crates/embed`, `crates/web/src/embed.rs` |
| 2. Element picker | built | PR #94; `packages/element/src/picker.ts`, `crates/core/src/element.rs` |
| 3. Caption extension | not started | #119; `EMBED.md` §Caption format extension |
| 4. Outputs | built | Open issue (PR #89), Submit / `onSubmit` (PR #123) |
| 5. React wrapper and release | wrapper built; release to GitHub Packages only | PRs #88, #90, #100; both packages at 0.2.0 |
| 6. Dogfooding | not tracked in this repo | `EMBED.md` §Phases item 6 |

Hardening since: #95, #96, #97 (fixes), #98 and #115 (packages built and tested
on pull requests), #124 (a test fails the build on network code in the embed's
own sources, `packages/element/test/no-network.test.ts`).

## Now

Each item can start today and fits one pull request. All three are filed as
`loom:triage` issues, in the order below: #130, #131, #132.

1. **Bring `SPEC.md` in line with the code.** Three statements are stale:
   §Architecture says text uses `cosmic-text` (removed in #43); §Operations
   says the JSON document format is version 2 (`FORMAT_VERSION` is 3 in
   `crates/core/src/document.rs`, which added element identity); §Architecture
   names "Cloudflare Pages / Workers assets" where `wrangler.jsonc` and the
   README settle on Workers static assets.
   Done when: `SPEC.md` names the current text stack, describes format
   version 3 and which older versions still load, and names the hosting the
   repo uses; no code changes.
2. **Launcher hotkey for the embed.** `EMBED.md` §Flow lists a hotkey as
   "Not built"; today the overlay opens from the launcher button or `open()`.
   Done when: `<paste2prompt-overlay>` takes an opt-in `hotkey` attribute (no
   default binding), the React wrapper passes it through, the key parsing is a
   pure function with `node:test` cases, both package READMEs document it, and
   `EMBED.md` §Flow says it is built.
3. **Measure the embed's wasm size instead of quoting it.** `EMBED.md` and
   `packages/element/README.md` give "about 580 KB gzip" and `EMBED.md` says to
   re-measure before quoting.
   Done when: `packages/element/build-wasm.mjs` prints the raw and gzip byte
   counts of the wasm it produces, and both documents carry the measured
   figure with the command that produced it.

## Next

- **Phase 3, format stage (#119, stage 2).** Print `Page:` and
  `element <selector> "<label>"` in the caption (`crates/core/src/caption.rs`),
  with the page passed as an optional argument so the document format does not
  change and standalone captions stay byte-identical.
  Done when: the acceptance list in #119 is met.
  Unblocked by: the stage 1 evidence under `docs/examples/embed/` (see
  Blocked). If the evidence says the lines do not help, the outcome is a note
  in `EMBED.md` and no format change.
- **Update the epic.** #86's checklist still shows every phase unticked; once
  #119 resolves either way, bring it in line with `EMBED.md`.

## Later

Direction from the design documents, each with the question to answer first.

- **Capture one picked element, cropped** (`EMBED.md` §Flow, "Not built").
  Question: does a cropped capture still give a model enough surrounding
  context, and how do percentage coordinates read on a crop? Rule 4 asks for
  an example before the format changes.
- **Real-pixel capture through `getDisplayMedia`** (`EMBED.md` §Architecture,
  "Not built"). Question: which host pages lose enough under `html-to-image`
  (cross-origin images, CSS effects) to justify a browser permission prompt?
- **React component name in element identity** (`EMBED.md` §Element identity,
  "Not built"). Question: is the fiber lookup stable enough across React
  versions and available outside dev builds, and does it need a new document
  format version?
- **Reopening exports** (`SPEC.md` §Out of scope: the edit list as JSON in a
  PNG `iTXt` chunk). Question: is Download-only round-tripping worth it when
  clipboard copies can strip the chunk?

## Blocked and waiting

- **#119, stage 1 (evidence) needs a person.** The Builder stopped on
  2026-11-05 without changing code: the comparison needs 5 to 10 real
  dashboard captures with picks and access to two or three current models,
  and the fleet host has neither. The issue carries `loom:operator`. Until the
  PNGs, prompts, answers and scored table are committed under
  `docs/examples/embed/`, phase 3 cannot move.
- **#1 (bootstrap).** One box is open, "Fleet admission", marked as pending an
  operator decision; the Curator routed it to the operator on 2026-11-04.
  Nothing in it is buildable.
- **Release to public npm.** Not built, and the license ruling on #86 says it
  does not decide whether to publish there (`EMBED.md` §Phases item 5). Needs
  an operator decision before any work.

No open issue is dispatchable today: #119 and #1 are with the operator and #86
is the epic. The three Now items above are the dispatchable work once filed.

## Not planned

From `SPEC.md` §Out of scope and `EMBED.md` §What it is not:

- Sending anything to an LLM API, any server component, or network code in
  the embed itself (rule 1); an upload belongs to the host's `onSubmit` code.
- An agent bridge that lets agents drive the live page.
- Browser extensions, a desktop build, or multiple images per session.
