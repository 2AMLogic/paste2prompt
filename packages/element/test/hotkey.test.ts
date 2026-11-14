import assert from "node:assert/strict";
import { test } from "node:test";
import { matchesHotkey, parseHotkey } from "../src/hotkey.ts";

const ev = (over: Partial<KeyboardEvent> = {}) => ({
  key: "p",
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  code: "KeyP",
  ...over,
});

test("parses a chord with modifiers and a key", () => {
  const hk = parseHotkey("Alt+Shift+P");
  assert.deepEqual(hk, { alt: true, ctrl: false, meta: false, shift: true, key: "p" });
  assert.deepEqual(parseHotkey("ctrl+P"), { alt: false, ctrl: true, meta: false, shift: false, key: "p" });
  assert.deepEqual(parseHotkey("  meta   +   K  "), { alt: false, ctrl: false, meta: true, shift: false, key: "k" });
});

test("Control and Cmd are aliases of ctrl and meta", () => {
  assert.equal(parseHotkey("Control+K")?.ctrl, true);
  assert.equal(parseHotkey("Cmd+K")?.meta, true);
  assert.equal(parseHotkey("Command+K")?.meta, true);
});

test("Space is matched as the space key", () => {
  assert.equal(parseHotkey("Alt+Space")?.key, " ");
});

test("a bare key has no modifier and is rejected", () => {
  assert.equal(parseHotkey("P"), undefined);
  assert.equal(parseHotkey("Enter"), undefined);
});

test("a lone modifier is rejected (no key)", () => {
  assert.equal(parseHotkey("Ctrl"), undefined);
  assert.equal(parseHotkey("Ctrl+Alt"), undefined);
});

test("an unknown modifier is rejected", () => {
  assert.equal(parseHotkey("Hyper+P"), undefined);
  assert.equal(parseHotkey("Alt+Foo+P"), undefined);
});

test("the empty string and empty parts are rejected", () => {
  assert.equal(parseHotkey(""), undefined);
  assert.equal(parseHotkey("   "), undefined);
  assert.equal(parseHotkey("Alt++P"), undefined);
  assert.equal(parseHotkey("Alt+"), undefined);
});

test("matching requires the exact modifiers", () => {
  const hk = parseHotkey("Alt+Shift+P")!;
  assert.equal(matchesHotkey(hk, ev({ altKey: true, shiftKey: true, key: "P" })), true);
  // Modifier mismatch in either direction.
  assert.equal(matchesHotkey(hk, ev({ altKey: true, key: "p" })), false);
  assert.equal(matchesHotkey(hk, ev({ shiftKey: true, key: "P" })), false);
  // An extra modifier the chord does not name is a mismatch too.
  assert.equal(matchesHotkey(hk, ev({ altKey: true, ctrlKey: true, shiftKey: true, key: "P" })), false);
  assert.equal(matchesHotkey(hk, ev({ altKey: true, shiftKey: true, metaKey: true, key: "P" })), false);
});

test("matching is case-insensitive on the key", () => {
  const hk = parseHotkey("Alt+P")!;
  assert.equal(matchesHotkey(hk, ev({ altKey: true, key: "p" })), true);
  assert.equal(matchesHotkey(hk, ev({ altKey: true, key: "P" })), true);
  assert.equal(parseHotkey("Alt+P")!.key, "p");
  assert.equal(matchesHotkey(parseHotkey("Alt+p")!, ev({ altKey: true, key: "P" })), true);
});

test("a different key does not match", () => {
  const hk = parseHotkey("Alt+Shift+P")!;
  assert.equal(matchesHotkey(hk, ev({ altKey: true, shiftKey: true, key: "O", code: "KeyO" })), false);
});

test("the physical key matches when Alt produced another character (macOS)", () => {
  const hk = parseHotkey("Alt+Shift+P")!;
  // macOS with Alt+Shift held reports the produced character, not the letter.
  assert.equal(matchesHotkey(hk, ev({ altKey: true, shiftKey: true, key: "π", code: "KeyP" })), true);
  // But the code alone must not match a different key's chord.
  assert.equal(
    matchesHotkey(hk, ev({ altKey: true, shiftKey: true, key: "π", code: "KeyO" })),
    false,
  );
});

test("Space matches by the space key", () => {
  const hk = parseHotkey("Alt+Space")!;
  assert.equal(matchesHotkey(hk, ev({ altKey: true, key: " ", code: "Space" })), true);
  assert.equal(matchesHotkey(hk, ev({ altKey: true, key: "f", code: "KeyF" })), false);
});

test("named keys match case-insensitively", () => {
  const hk = parseHotkey("Ctrl+Enter")!;
  assert.equal(matchesHotkey(hk, ev({ ctrlKey: true, key: "Enter", code: "Enter" })), true);
  assert.equal(matchesHotkey(hk, ev({ ctrlKey: true, key: "Escape", code: "Escape" })), false);
});
