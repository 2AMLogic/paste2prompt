// The editor (crates/embed, built by `npm run build:wasm` into dist/wasm/).
// Loaded on first open so the launcher alone costs the host page a few KB.

/** A mounted editor (crates/embed `Editor`). */
export interface Editor {
  hasEdits(): boolean;
  close(): void;
}

interface EmbedModule {
  default(): Promise<unknown>;
  open(shadow: ShadowRoot, container: HTMLElement, image: Blob, name?: string): Editor;
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
