import assert from "node:assert/strict";
import { test } from "node:test";
import {
  escapeIdent,
  fitRole,
  fitSelector,
  MAX_SELECTOR_CHARS,
} from "../src/selector.ts";

// Node has no CSS.escape; this stand-in escapes what the tests use.
(globalThis as { CSS?: unknown }).CSS = {
  escape: (s: string) => s.replace(/[^\w\u0080-￿-]/g, (c) => `\\${c}`),
};

const chars = (s: string) => Array.from(s).length;
// What the core's one_line rejects: control and format characters.
const invalid = /[\p{Cc}\p{Cf}\u2028\u2029]/u;

test("a role of 64 characters is kept, a longer one is omitted", () => {
  assert.equal(fitRole("a".repeat(64)), "a".repeat(64));
  assert.equal(fitRole("a".repeat(65)), undefined);
  assert.equal(fitRole(""), undefined);
});

test("an id with an invisible character becomes a valid single-line selector", () => {
  for (const id of ["save​btn", "​", "a‮b"]) {
    const escaped = escapeIdent(id);
    assert.ok(!invalid.test(escaped), JSON.stringify(escaped));
  }
  assert.equal(escapeIdent("save​btn"), "save\\200b btn");
});

test("a selector that fits is kept whole", () => {
  assert.equal(fitSelector(["main", "div.a", "button"], () => false, () => []), "main > div.a > button");
});

const longParts = Array.from({ length: 6 }, (_, i) => `div.${"x".repeat(100)}${i}`);

test("a long selector drops whole leading segments, staying unique", () => {
  const unique = (s: string) => s.endsWith(longParts[5]);
  const out = fitSelector(longParts, unique, () => []);
  assert.ok(chars(out) <= MAX_SELECTOR_CHARS);
  assert.ok(out.endsWith(longParts[5]));
  for (const seg of out.split(" > ")) assert.ok(longParts.includes(seg), seg);
});

test("when dropping segments makes it ambiguous, the nth-of-type path is used", () => {
  const path = ["html", "body", "div:nth-of-type(2)", "span:nth-of-type(3)"];
  const full = longParts.join(" > ");
  // Only the whole chain (too long) or the nth path is unique.
  const unique = (s: string) => s === full || s === path.join(" > ") || s === path.slice(1).join(" > ");
  const out = fitSelector(longParts, unique, () => path);
  assert.equal(out, path.join(" > "));
});

test("a path too long to be unique still ends on a segment boundary within the limit", () => {
  const out = fitSelector(longParts, () => false, () => longParts);
  assert.ok(chars(out) <= MAX_SELECTOR_CHARS);
  for (const seg of out.split(" > ")) assert.ok(longParts.includes(seg), seg);
});
