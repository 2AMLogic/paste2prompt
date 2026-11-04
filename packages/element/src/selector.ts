// Selector and role helpers for the element picker. Kept free of DOM
// access (the page is reached through callbacks) so they can be tested in
// plain Node.

/** Longest selector kept, in characters (crates/core MAX_SELECTOR_CHARS). */
export const MAX_SELECTOR_CHARS = 512;
/** Longest role kept, in characters (crates/core MAX_TAG_CHARS). */
export const MAX_ROLE_CHARS = 64;

const SEPARATOR = " > ";

/**
 * `CSS.escape`, plus a hex escape for the control and invisible formatting
 * characters it leaves as they are (U+200B, U+202E…). The core rejects a
 * selector holding one; as `\200b ` it stays valid and on one line.
 */
export function escapeIdent(s: string): string {
  return CSS.escape(s).replace(
    /[\p{Cc}\p{Cf}\u2028\u2029]/gu,
    (c) => `\\${c.codePointAt(0)!.toString(16)} `,
  );
}

/** A role the core accepts, or none: a cut-off role would mean nothing. */
export function fitRole(role: string): string | undefined {
  return role && Array.from(role).length <= MAX_ROLE_CHARS ? role : undefined;
}

function length(parts: string[]): number {
  return Array.from(parts.join(SEPARATOR)).length;
}

/**
 * Drops leading segments, never cutting inside one, until the selector fits
 * the core's limit. Returns the shortest suffix of `parts` that fits and
 * still `unique`ly matches, or undefined if there is none.
 */
function dropToFit(parts: string[], unique: (s: string) => boolean): string | undefined {
  for (let i = 0; i < parts.length; i++) {
    const rest = parts.slice(i);
    if (length(rest) > MAX_SELECTOR_CHARS) continue;
    const selector = rest.join(SEPARATOR);
    if (unique(selector)) return selector;
  }
  return undefined;
}

/**
 * Makes a selector of `parts` (outermost first) fit the core's limit
 * without losing what it matches. A selector that is short enough is kept
 * as it is. Otherwise leading segments are dropped while it stays unique;
 * if that fails, the `:nth-of-type` path from the document root
 * (`nthPath`, outermost first) is shortened the same way. As a last resort
 * the longest-fitting tail of that path, which is valid if not unique.
 */
export function fitSelector(
  parts: string[],
  unique: (s: string) => boolean,
  nthPath: () => string[],
): string {
  if (length(parts) <= MAX_SELECTOR_CHARS) return parts.join(SEPARATOR);
  const fitted = dropToFit(parts, unique);
  if (fitted) return fitted;
  const path = nthPath();
  const fromPath = dropToFit(path, unique);
  if (fromPath) return fromPath;
  for (let i = 0; i < path.length; i++) {
    if (length(path.slice(i)) <= MAX_SELECTOR_CHARS) return path.slice(i).join(SEPARATOR);
  }
  return path[path.length - 1] ?? parts[parts.length - 1];
}
