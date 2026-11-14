// Parsing and matching of the overlay's `hotkey` chord. Kept free of DOM
// access so it can be tested in plain Node, like `selector.ts` and
// `submit.ts`. The keydown listener that uses it lives in `overlay.ts`.

/** A parsed chord: which modifiers must be held, and the key. */
export interface Hotkey {
  alt: boolean;
  ctrl: boolean;
  meta: boolean;
  shift: boolean;
  /** The key, lowercased; the space key is a single space, `" "`. */
  key: string;
}

/** The part of a keyboard event the chord match needs. */
export interface KeyEventLike {
  key: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  /** Physical key, e.g. `KeyP`; matched when Alt turned the letter into another character. */
  code?: string;
}

/** Modifier names accepted in a chord, mapped to their flag. */
const MODIFIERS: Record<string, "alt" | "ctrl" | "meta" | "shift"> = {
  alt: "alt",
  ctrl: "ctrl",
  control: "ctrl",
  shift: "shift",
  meta: "meta",
  cmd: "meta",
  command: "meta",
};

/**
 * Parses one chord, `Mod+Mod+Key`, such as `Alt+Shift+P`. Modifier names are
 * case-insensitive (`Ctrl`, `Control`, `Cmd`, `Command`, `Meta`), as is the
 * key; `Space` names the space key. Returns undefined for anything else: an
 * empty string or part, an unknown modifier, no modifier at all, or no key.
 */
export function parseHotkey(value: string): Hotkey | undefined {
  const parts = value.split("+").map((part) => part.trim());
  if (parts.some((part) => part === "")) return undefined;
  const mods = { alt: false, ctrl: false, meta: false, shift: false };
  for (const part of parts.slice(0, -1)) {
    const mod = MODIFIERS[part.toLowerCase()];
    if (!mod) return undefined;
    mods[mod] = true;
  }
  const keyPart = parts[parts.length - 1]!;
  // A chord is modifier plus key: a lone modifier, or a modifier where the
  // key should be, names no key.
  if (MODIFIERS[keyPart.toLowerCase()]) return undefined;
  if (!mods.alt && !mods.ctrl && !mods.meta && !mods.shift) return undefined;
  const key = /^(space|spacebar)$/i.test(keyPart) ? " " : keyPart.toLowerCase();
  return { ...mods, key };
}

/** Whether the event's key is the chord's key, by key or physical code. */
function keyMatches(hotkey: Hotkey, event: KeyEventLike): boolean {
  if (event.key.toLowerCase() === hotkey.key) return true;
  // With Alt held, macOS reports the produced character (Alt+Shift+P is
  // "π"); the physical code still says KeyP. Letters only, so a chord key
  // like `f1` cannot sneak in through a code.
  return /^[a-z]$/.test(hotkey.key) && event.code === `Key${hotkey.key.toUpperCase()}`;
}

/**
 * Whether the event is exactly the chord: every modifier it names held, none
 * other, and the key pressed.
 */
export function matchesHotkey(hotkey: Hotkey, event: KeyEventLike): boolean {
  return (
    event.altKey === hotkey.alt &&
    event.ctrlKey === hotkey.ctrl &&
    event.metaKey === hotkey.meta &&
    event.shiftKey === hotkey.shift &&
    keyMatches(hotkey, event)
  );
}
