// The host-submit button's logic, apart from the DOM so it can be tested.
// Nothing here sends anything: the embed only hands the output to a callback
// the host supplied (CLAUDE.md rule 1, operator 2026-11-05).

/** What the host's `onSubmit` callback receives, after one explicit click. */
export interface SubmitPayload {
  /** The annotated export PNG. */
  png: Blob;
  /** The caption as plain text. */
  caption: string;
  /** The document (edits, picked elements included), parsed from the core's JSON. */
  document: unknown;
  /** The captured page's address (no query string). */
  page: string;
}

/** The host's callback. It may be async; a rejection is shown in the bar. */
export type SubmitHandler = (payload: SubmitPayload) => void | Promise<void>;

/** The editor methods the payload is built from. */
export interface SubmitSource {
  caption(): string;
  documentJson(): string;
  renderPng(): Promise<Blob>;
}

/**
 * Whether the submit button shows. It needs both a callback and a notice
 * naming where the output goes, so the user knows before clicking.
 */
export function submitEnabled(notice: string | null | undefined, handler: unknown): boolean {
  return typeof handler === "function" && !!notice?.trim();
}

/** Builds the payload from the editor. Throws if the document isn't JSON. */
export async function buildSubmitPayload(source: SubmitSource, page: string): Promise<SubmitPayload> {
  const png = await source.renderPng();
  const json = source.documentJson();
  return {
    png,
    caption: source.caption(),
    document: json ? JSON.parse(json) : null,
    page,
  };
}

/**
 * The click: builds the payload and calls the handler once. Returns an error
 * message to show, or `undefined` on success. A throwing handler is reported,
 * never left hanging.
 */
export async function runSubmit(
  handler: SubmitHandler,
  source: SubmitSource,
  page: string,
): Promise<string | undefined> {
  try {
    await handler(await buildSubmitPayload(source, page));
    return undefined;
  } catch (err) {
    console.error("paste2prompt: submit failed", err);
    return err instanceof Error && err.message ? `Submit failed: ${err.message}` : "Submit failed.";
  }
}
