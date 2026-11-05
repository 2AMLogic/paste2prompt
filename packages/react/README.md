# @2amlogic/paste2prompt-react

The paste2prompt overlay as a React component. A floating launcher button
captures what the page shows and opens the paste2prompt editor over it. The
user marks regions, attaches edits, and copies out the annotated PNG. The
capture and the PNG never leave the browser.

```tsx
import { Paste2Prompt } from "@2amlogic/paste2prompt-react";

export function Root() {
  return (
    <>
      <App />
      <Paste2Prompt position="bottom-right" />
    </>
  );
}
```

For bug reports straight into your own repository (there is no default; without
`issueRepo` there is no Open issue button):

```tsx
<Paste2Prompt issueRepo="your-org/your-app" issueLabels={["bug"]} issueContext={`**Build:** ${BUILD}`} />
```

To take the output yourself, pass both `submitNotice` (shown before the
button, so say where it goes) and `onSubmit`. The button shows only with both,
and the callback runs once per click with `{ png, caption, document, page }`:

```tsx
<Paste2Prompt submitNotice="Sends to dashboard.example.com" onSubmit={async (p) => upload(p)} />
```

Without the floating button, open the editor from your own control:

```tsx
const p2p = useRef<Paste2PromptHandle>(null);

<button onClick={() => p2p.current?.open()}>Report a problem</button>
<Paste2Prompt ref={p2p} launcher={false} onClose={() => console.log("closed")} />
```

| Prop | Default | |
|---|---|---|
| `position` | `"bottom-right"` | `bottom-left`, `top-right`, `top-left` |
| `launcher` | `true` | show the floating button |
| `issueRepo` | | `owner/name`: adds **Open issue**, which copies the PNG and opens a pre-filled new-issue page there |
| `issueLabels` | | labels for that issue |
| `issueContext` | | extra lines for its body, e.g. a build version |
| `submitNotice` | | text naming where `onSubmit` sends the output; shown before **Submit** |
| `onSubmit` | | called with `{ png, caption, document, page }` on a click of **Submit**; the button needs this and `submitNotice` |
| `onOpen`, `onClose` | | called once the editor shows / is gone |

The ref's `close(force?)` asks before discarding marks; `close(true)` closes
without asking. Only one overlay per page can be open: opening a second while
the first is picking, capturing or showing its editor does nothing.

It wraps the `<paste2prompt-overlay>` web component from
[`@2amlogic/paste2prompt-element`](../element/README.md), which also works without
React. Safe to import during server-side rendering; the element registers on
mount.

## Install

Published to GitHub Packages for 2AMLogic repos, not to npmjs.com. Point
the `@2amlogic` scope at it in the project's `.npmrc`:

```ini
@2amlogic:registry=https://npm.pkg.github.com
```

Then authenticate:
- **Locally:** a token with `read:packages` in your user `~/.npmrc`
  (`//npm.pkg.github.com/:_authToken=…`).
- **GitHub Actions:** `actions/setup-node` with `registry-url:
  https://npm.pkg.github.com` and `NODE_AUTH_TOKEN: ${{ github.token }}`.
  This works only after an org admin grants the repo read access under the
  package's **Manage Actions access**.

New versions publish from `main` when the `version` in `package.json` is
bumped (`.github/workflows/publish-packages.yml`).

```sh
npm install @2amlogic/paste2prompt-react
```

Licence: the repo's PolyForm Noncommercial 1.0.0, while the licence for the
embed packages is an open question (#86).
