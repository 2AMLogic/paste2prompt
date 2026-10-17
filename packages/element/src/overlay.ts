import appCss from "../../../crates/web/style.css";
import chromeCss from "./chrome.css";
import { captureViewport } from "./capture.js";
import { type Editor, loadEditor } from "./load-wasm.js";

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
 *
 * Events (on the element): `paste2prompt-open` once the editor shows,
 * `paste2prompt-close` once it is gone.
 *
 * Nothing leaves the browser: the capture goes straight into the editor,
 * and the editor's output goes to the clipboard or a download.
 */
export class Paste2PromptOverlay extends Base {
  #shadow: ShadowRoot;
  #launcher: HTMLButtonElement;
  #panel: HTMLElement;
  #title: HTMLElement;
  #page: HTMLElement;
  #editor: Editor | undefined;
  #busy = false;

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
          <span class="title" role="status"></span>
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
    this.#busy = true;
    this.#launcher.setAttribute("aria-busy", "true");
    try {
      // Start loading the editor while the page is captured.
      const editor = loadEditor();
      let image: Blob;
      try {
        image = await captureViewport(this);
      } catch (err) {
        console.error("paste2prompt: page capture failed", err);
        return;
      }
      this.#title.textContent = `Annotating ${location.host}${location.pathname}`;
      this.#title.classList.remove("error");
      this.#panel.hidden = false;
      let mod;
      try {
        mod = await editor;
      } catch (err) {
        console.error("paste2prompt: the editor failed to load", err);
        this.#title.textContent = "The editor failed to load. See the console for details.";
        this.#title.classList.add("error");
        return;
      }
      this.#editor = mod.open(this.#shadow, this.#page, image, "Page capture");
      this.dispatchEvent(new CustomEvent("paste2prompt-open"));
    } finally {
      this.#busy = false;
      this.#launcher.removeAttribute("aria-busy");
    }
  }

  /**
   * Closes the editor. Asks first when there are marks to lose, unless
   * `force` is set.
   */
  close(force = false): void {
    const editor = this.#editor;
    if (editor && !force && editor.hasEdits() && !window.confirm(DISCARD_PROMPT)) return;
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
