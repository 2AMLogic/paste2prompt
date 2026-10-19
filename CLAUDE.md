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
   - **Embed exception (operator, 2026-10-17):** the embed (`crates/embed`,
     `packages/`) may, on an explicit click of **Open issue**, open
     `github.com/<host-configured owner/name>/issues/new` with the title,
     labels, page address (no query string) and caption text in the URL,
     plus any context lines the host app sets (`issueContext`, e.g. a build
     version; operator, 2026-10-19). The image only goes to the clipboard;
     the user attaches it. Nothing else is sent. This code stays out of `crates/web`, so the standalone site and
     its privacy guard are unchanged.
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

<!-- BEGIN LOOM ORCHESTRATION -->
This repository uses [Loom](https://github.com/rjwalters/loom) for AI-powered development orchestration — see the Loom repository for the full guide (roles, labels, worktrees, configuration). When installed, Loom also writes a locally-substituted copy of that guide to `.loom/CLAUDE.md`.

Work is coordinated through `loom:` labels on issues and pull requests, and the same roles run either under `loom-daemon` or by hand in an attended session — daemon mode is optional. Create the labels once with `.loom/scripts/sync-labels.sh` (an install ships `.github/labels.yml` but does not create the labels on the forge). A pull request ready for review carries `loom:review-requested`; Judge reviews it and applies `loom:pr` (approved) or `loom:changes-requested`; Doctor fixes a `loom:changes-requested` pull request and returns it to `loom:review-requested`. Only a `loom:pr` pull request gets merged, and always via this repo's merge script (`.loom/scripts/merge-pr.sh`) — never a raw forge merge command such as `gh pr merge`. Full state machine: `.loom/docs/label-state-machine.md`.
<!-- END LOOM ORCHESTRATION -->

<!-- BEGIN REPO-SKILLS -->
This repository has [Repo Skills](https://github.com/rjwalters/repo) v0.21.19 installed —
general repository hygiene and environment commands invoked as `/repo:<command>`. Run
`/repo:help` for the command list, or see `.claude/skills/repo/SKILL.md` for the full
guide. Hygiene commands apply safe, reversible fixes by default and report each
change; run with `--ask` to review first, and `--prune` to allow irreversible
removals. Managed by `install.sh` — edit outside the markers only.
<!-- END REPO-SKILLS -->
<!-- BEGIN SQUAD -->
## Squad — cross-agent collaboration

This repo has [Squad](https://github.com/rjwalters/squad) installed. Claude and
Codex share the same room and MCP tools. Before touching shared state, read
and follow the installed Squad skill, including its room/identity conventions:

- Claude: `.claude/skills/squad/SKILL.md`
- Codex: `.agents/skills/squad/SKILL.md` (invoke `$squad` or ask naturally)

Both expose join, goals, card, fanout, steward, and clear workflows. Claude aliases are
`/squad:<workflow>`; legacy Codex prompts are `/squad-<workflow>`.

**Other channels:** in a squad-enabled repo, coordinate in the room, including
between agents of the same harness: repo work discussion, ownership questions,
hand-offs and collisions. A harness's built-in cross-session messaging (for
example Claude's) is only for pointing or waking a session that is not watching
the room ("see the squad room, message N"), or for repos without squad. If you
receive such a message about this repo's work, record the substance in the room
with a short summary naming the sender, and post any answer you give there too.
Rooms persist for later joiners; direct messages do not.

For research work, discover and reuse the durable Science Card node IDs surfaced
by join/node list; the shared card workflow connects dependencies, committed
artifacts and revision-bound bank provenance.
<!-- END SQUAD -->
