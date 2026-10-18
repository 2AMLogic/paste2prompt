import appCss from "../../../crates/web/style.css";
import chromeCss from "./chrome.css";
import { captureViewport } from "./capture.js";
import { type Editor, type IssueOptions, loadEditor } from "./load-wasm.js";

/** Where the launcher button floats. */
export type LauncherPosition = "bottom-right" | "bottom-left" | "top-right" | "top-left";

/** The custom element's tag name. */
export const TAG_NAME = "paste2prompt-overlay";

const DISCARD_PROMPT = "Close and discard your marks?";

// A pencil over a frame.
const LAUNCHER_ICON = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 8V5a1 1 0 0 1 1-1h3M16 4h3a1 1 0 0 1 1 1v3M20 16v3a1 1 0 0 1-1 1h-3M8 20H5a1 1 0 0 1-1-1v-3"/><path d="m9 15 1.5-4L16 5.5a1.4 1.4 0 0 1 2 2L12.5 13z"/></svg>`;

// Server-side rendering imports this module without a DOM; only the
// browser ever constructs the element.
const Base: typeof HTMLElement =
  typeof HTMLElement === "undefined" ? (class {} as typeof HTMLElement) : HTMLElement;

let sheet: CSSStyleSheet | undefined;

function styles(): CSSStyleSheet {
  if (!sheet) {
    sheet = new CSSStyleSheet();
    sheet.replaceSync(`${chromeCss}\n${appCss}`);
  }
  return sheet;
}

/**
 * `<paste2prompt-overlay>`: a floating launcher button that captures the
 * page and opens the paste2prompt editor over it.
 *
 * Attributes:
 * - `position`: where the launcher floats (`bottom-right` by default).
 * - `no-launcher`: hide the launcher; open with {@link open} instead.
 * - `issue-repo`: `owner/name` of a GitHub repository. The editor then has
 *   an **Open issue** button: it copies the annotated PNG and opens that
 *   repository's new-issue page, pre-filled with the page address and the
 *   caption, for the user to paste the image into and submit.
 * - `issue-labels`: comma-separated labels for that issue.
 *
 * The {@link issueContext} property adds lines to the issue body (a build
 * version, say).
 *
 * Events (on the element): `paste2prompt-open` once the editor shows,
 * `paste2prompt-close` once it is gone.
 *
 * Nothing leaves the browser: the capture goes straight into the editor,
 * and the editor's output goes to the clipboard or a download. Open issue
 * only opens a github.com page; the user submits it.
 */
export class Paste2PromptOverlay extends Base {
  #shadow: ShadowRoot;
  #launcher: HTMLButtonElement;
  #panel: HTMLElement;
  #title: HTMLElement;
  #page: HTMLElement;
  #editor: Editor | undefined;
  #busy = false;
  /** Bumped by every open and close, so a superseded open stops. */
  #attempt = 0;

  /** Extra lines for the body of an issue opened from the editor. */
  issueContext = "";

  constructor() {
    super();
    this.#shadow = this.attachShadow({ mode: "open" });
    this.#shadow.adoptedStyleSheets = [styles()];
    this.#shadow.innerHTML = `
      <button type="button" class="launcher" part="launcher"
        aria-label="Annotate this page with paste2prompt"
        title="Annotate this page (paste2prompt)">${LAUNCHER_ICON}</button>
      <div class="panel" part="panel" role="dialog" aria-modal="true"
        aria-label="paste2prompt" hidden>
        <div class="bar">
          <span class="title"></span>
          <button type="button" class="close">Close</button>
        </div>
        <div class="p2p-page"></div>
      </div>`;
    const $ = <T extends Element>(selector: string) => this.#shadow.querySelector<T>(selector)!;
    this.#launcher = $("button.launcher");
    this.#panel = $(".panel");
    this.#title = $(".title");
    this.#page = $(".p2p-page");
    this.#launcher.addEventListener("click", () => void this.open());
    $("button.close").addEventListener("click", () => this.close());
  }

  /** Whether the editor is showing. */
  get isOpen(): boolean {
    return this.#editor !== undefined;
  }

  /**
   * Captures the viewport and opens the editor with it. Does nothing if the
   * editor is already open or opening.
   */
  async open(): Promise<void> {
    if (this.#editor || this.#busy) return;
    const attempt = ++this.#attempt;
    const stale = () => attempt !== this.#attempt;
    this.#busy = true;
    this.#launcher.setAttribute("aria-busy", "true");
    try {
      // Start loading the editor while the page is captured.
      const editor = loadEditor();
      // Handled below; this only stops an unhandled rejection when the
      // capture fails first.
      editor.catch(() => {});
      // The page address without the query string, which often carries
      // tokens; the hash stays, since hash-routed apps keep the route there.
      const page = `${location.origin}${location.pathname}${location.hash}`;
      this.#title.textContent = `Annotating ${location.host}${location.pathname}${location.hash}`;
      // The panel is part of this element, which the capture leaves out.
      this.#status("Capturing the page…");
      this.#panel.hidden = false;
      let image: Blob;
      try {
        image = await captureViewport(this);
        if (stale()) return;
      } catch (err) {
        if (stale()) return;
        console.error("paste2prompt: page capture failed", err);
        this.#failed("Couldn't capture the page. See the console for details.");
        return;
      }
      this.#status("Loading the editor…");
      let mod;
      try {
        mod = await editor;
        if (stale()) return;
      } catch (err) {
        if (stale()) return;
        console.error("paste2prompt: the editor failed to load", err);
        this.#failed("The editor failed to load. See the console for details.");
        return;
      }
      this.#page.replaceChildren();
      this.#editor = mod.open(this.#shadow, this.#page, image, "Page capture", this.#issue(page));
      this.dispatchEvent(new CustomEvent("paste2prompt-open"));
    } finally {
      if (!stale()) this.#idle();
    }
  }

  #idle(): void {
    this.#busy = false;
    this.#launcher.removeAttribute("aria-busy");
  }

  /** The Open issue settings from the attributes, if `issue-repo` is set. */
  #issue(page: string): IssueOptions | undefined {
    const repo = this.getAttribute("issue-repo")?.trim();
    if (!repo) return undefined;
    const labels = (this.getAttribute("issue-labels") ?? "")
      .split(",")
      .map((l) => l.trim())
      .filter(Boolean);
    return { repo, labels, context: this.issueContext, page };
  }

  /** Shows progress where the editor will be, in visible text. */
  #status(message: string): void {
    const p = document.createElement("p");
    p.className = "p2p-status";
    p.setAttribute("role", "status");
    p.textContent = message;
    this.#page.replaceChildren(p);
  }

  /** Shows why opening failed, with a retry. */
  #failed(message: string): void {
    const p = document.createElement("p");
    p.className = "p2p-status error";
    p.setAttribute("role", "alert");
    p.textContent = message;
    const retry = document.createElement("button");
    retry.type = "button";
    retry.textContent = "Try again";
    retry.addEventListener("click", () => {
      this.#panel.hidden = true;
      void this.open();
    });
    this.#page.replaceChildren(p, retry);
  }

  /**
   * Closes the editor. Asks first when there are marks to lose, unless
   * `force` is set.
   */
  close(force = false): void {
    const editor = this.#editor;
    if (editor && !force && editor.hasEdits() && !window.confirm(DISCARD_PROMPT)) return;
    // Also stops an open still capturing or loading.
    this.#attempt++;
    this.#idle();
    this.#editor = undefined;
    editor?.close();
    this.#page.replaceChildren();
    this.#panel.hidden = true;
    if (editor) {
      this.dispatchEvent(new CustomEvent("paste2prompt-close"));
      this.#launcher.focus();
    }
  }

  disconnectedCallback(): void {
    this.close(true);
  }
}

/** Registers `<paste2prompt-overlay>` once. Safe to call during SSR. */
export function defineOverlay(): void {
  if (typeof customElements !== "undefined" && !customElements.get(TAG_NAME)) {
    customElements.define(TAG_NAME, Paste2PromptOverlay);
  }
}

declare global {
  interface HTMLElementTagNameMap {
    [TAG_NAME]: Paste2PromptOverlay;
  }
}
