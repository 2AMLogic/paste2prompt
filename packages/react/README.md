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
| `onOpen`, `onClose` | | called once the editor shows / is gone |

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
