# @paste2prompt/react

The paste2prompt overlay as a React component. A floating launcher button
captures what the page shows and opens the paste2prompt editor over it. The
user marks regions, attaches edits, and copies out the annotated PNG. The
capture and the PNG never leave the browser.

```tsx
import { Paste2Prompt } from "@paste2prompt/react";

export function Root() {
  return (
    <>
      <App />
      <Paste2Prompt position="bottom-right" />
    </>
  );
}
```

For bug reports straight into a repository:

```tsx
<Paste2Prompt issueRepo="2AMLogic/loom-ui" issueLabels={["loom:triage"]} issueContext={`**Build:** ${BUILD}`} />
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
[`@paste2prompt/element`](../element/README.md), which also works without
React. Safe to import during server-side rendering; the element registers on
mount.

Status: not published yet. The licence for the embed packages is an open
question (#86); until it is settled they are `private` and carry the repo's
PolyForm Noncommercial 1.0.0 licence.
