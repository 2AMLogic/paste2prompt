// The editor (crates/embed, built by `npm run build:wasm` into dist/wasm/).
// Loaded on first open so the launcher alone costs the host page a few KB.

/** A mounted editor (crates/embed `Editor`). */
export interface Editor {
  hasEdits(): boolean;
  /** The caption as plain text, or "" without an image. */
  caption(): string;
  /** The edits as document JSON (picked elements included), or "" without an image. */
  documentJson(): string;
  /** The export PNG. Call in the click handler, for `ClipboardItem`. */
  renderPng(): Promise<Blob>;
  /** The pre-filled new-issue URL. Throws without an image. */
  issueUrl(issue: IssueOptions): string;
  close(): void;
}

/** Where **Open issue** files, and what goes in the body. */
export interface IssueOptions {
  /** `owner/name`. */
  repo: string;
  labels?: string[];
  /** Extra lines for the issue body. */
  context?: string;
  /** The captured page, for the body. */
  page?: string;
}

/**
 * An element picked from the page, for the editor to start with a hint
 * edit on: its box as fractions of the image's width and height, and what
 * it is.
 */
export interface Pick {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
  selector: string;
  tag: string;
  role?: string;
  label?: string;
}

interface EmbedModule {
  default(): Promise<unknown>;
  open(
    shadow: ShadowRoot,
    container: HTMLElement,
    image: Blob,
    name?: string,
    picks?: Pick[],
  ): Editor;
}

let loading: Promise<EmbedModule> | undefined;

/** The wasm module, initialised. Retries on the next call if it failed. */
export function loadEditor(): Promise<EmbedModule> {
  loading ??= (async () => {
    // A static specifier, so the host's bundler copies the glue and the
    // .wasm it points at (new URL(..., import.meta.url)).
    // @ts-expect-error -- generated at build time, next to dist/index.js
    const mod = (await import("./wasm/paste2prompt_embed.js")) as EmbedModule;
    await mod.default();
    return mod;
  })().catch((err: unknown) => {
    loading = undefined;
    throw err;
  });
  return loading;
}
