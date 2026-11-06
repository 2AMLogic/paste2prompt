// Guard for CLAUDE.md rule 1, Submit exception term (4): the embed adds no
// network code of its own. It scans the embed's own sources (never
// dependencies: `html-to-image` re-fetches page assets by design, see
// README § Capture, and `node_modules` is skipped) and fails on any network
// API, naming the file and line. `scripts/privacy_check.py` covers the
// standalone site and `crates/web`, not these directories.
//
// Allowlist entries match by file and exact stripped line content, each at
// most once, as LOADER_ALLOWLIST in scripts/privacy_check.py does, so
// moving, rewording or duplicating an allowed call fails.
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { extname, join, relative, resolve, sep } from "node:path";
import { test } from "node:test";

const ROOT = resolve(import.meta.dirname, "..", "..", "..");
const SCAN_DIRS = ["packages/element/src", "packages/react/src", "crates/embed/src"];
const CARGO_TOML = "crates/embed/Cargo.toml";

const JS_EXT = new Set([".ts", ".tsx", ".js", ".mjs", ".cjs"]);
const RUST_EXT = new Set([".rs"]);
// Files with no code in them. Any other extension fails rather than going
// unscanned.
const INERT_EXT = new Set([".css", ".md"]);

type Rule = { name: string; re: RegExp };

const JS_RULES: Rule[] = [
  { name: "fetch", re: /\bfetch\s*\(/ },
  {
    name: "network API",
    re: /\b(XMLHttpRequest|sendBeacon|WebSocket|EventSource|importScripts|RTCPeerConnection)\b/,
  },
  { name: "new Request", re: /\bnew\s+Request\s*\(/ },
  // The package's own wasm glue is the only dynamic import.
  { name: "dynamic import", re: /\bimport\s*\(\s*(?!["']\.\/wasm\/)/ },
  { name: "window.open", re: /\b(window|self|globalThis|top|parent)\s*\.\s*open\s*\(/ },
  { name: "location navigation", re: /\blocation\s*\.\s*(href\s*=(?!=)|assign\s*\(|replace\s*\()/ },
];

const RUST_RULES: Rule[] = [
  { name: "reqwest", re: /\breqwest\b/ },
  { name: "gloo-net", re: /\bgloo[-_]net\b/ },
  { name: "web_sys::Request", re: /\bweb_sys::(Request|RequestInit|Response)\b/ },
  { name: "network API", re: /\b(XmlHttpRequest|WebSocket|EventSource|send_beacon)\b/ },
];

// `web-sys` features that reach the network (Navigator is the beacon API).
const NETWORK_FEATURE =
  /^(Request\w*|Response\w*|Headers|XmlHttpRequest\w*|WebSocket|EventSource|Navigator|Rtc\w*|WebTransport\w*)$/;

// Navigation the Open-issue exception allows, in `overlay.ts` only.
const ALLOWLIST: Record<string, string[]> = {
  "packages/element/src/overlay.ts": [
    'const tab = window.open("", "_blank");',
    "tab.location.href = url;",
  ],
};

export type Finding = { file: string; line: number; rule: string; text: string };

const isComment = (s: string) =>
  s.startsWith("//") || s.startsWith("/*") || s.startsWith("*");

/** Scans one file's text. `file` is a repo-relative path with `/` separators. */
export function findNetworkUse(
  file: string,
  text: string,
  allowlist: Record<string, string[]> = ALLOWLIST,
): Finding[] {
  const ext = extname(file);
  const findings: Finding[] = [];
  const add = (line: number, rule: string, t: string) =>
    findings.push({ file, line, rule, text: t });
  if (INERT_EXT.has(ext)) return findings;
  if (!JS_EXT.has(ext) && !RUST_EXT.has(ext)) {
    add(1, "unknown file type (not scanned)", ext || file);
    return findings;
  }
  const allowed = [...(allowlist[file] ?? [])];
  let inlineJs = false;
  text.split("\n").forEach((raw, i) => {
    const s = raw.trim();
    if (!s || isComment(s)) return;
    const rules: Rule[] = [];
    if (JS_EXT.has(ext)) rules.push(...JS_RULES);
    else {
      rules.push(...RUST_RULES);
      // From an `inline_js` on, the file holds JS: apply the JS rules too.
      if (/\binline_js\b/.test(s)) inlineJs = true;
      if (inlineJs) rules.push(...JS_RULES);
    }
    for (const { name, re } of rules) {
      if (!re.test(s)) continue;
      const at = allowed.indexOf(s);
      if (at >= 0) allowed.splice(at, 1);
      else add(i + 1, name, s);
      break;
    }
  });
  return findings;
}

/** The `features = [...]` of `web-sys` in the embed's Cargo.toml. */
export function webSysFeatures(toml: string): string[] | null {
  const section = /^\[dependencies\.web-sys\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(toml);
  const inline = /^web-sys\s*=\s*\{[^\n]*$/m.exec(toml);
  const body = section ? section[1] : inline ? inline[0] : null;
  if (body === null) return null;
  const list = /features\s*=\s*\[([^\]]*)\]/.exec(body);
  if (!list) return [];
  return [...list[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
}

export function networkFeatures(toml: string): string[] {
  const features = webSysFeatures(toml);
  assert.ok(features, "crates/embed/Cargo.toml has no web-sys dependency to check");
  return features.filter((f) => NETWORK_FEATURE.test(f));
}

function walk(dir: string): string[] {
  const out: string[] = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (e.name === "node_modules") continue;
    const p = join(dir, e.name);
    if (e.isDirectory()) out.push(...walk(p));
    else out.push(p);
  }
  return out;
}

const fmt = (f: Finding) => `${f.file}:${f.line}: ${f.rule}: ${f.text}`;

test("the embed's own sources contain no network code", () => {
  const findings: Finding[] = [];
  let scanned = 0;
  for (const dir of SCAN_DIRS) {
    for (const path of walk(join(ROOT, dir))) {
      const file = relative(ROOT, path).split(sep).join("/");
      findings.push(...findNetworkUse(file, readFileSync(path, "utf8")));
      scanned++;
    }
  }
  assert.ok(scanned > 0, "no files scanned: SCAN_DIRS is wrong");
  assert.deepEqual(findings.map(fmt), []);
});

test("crates/embed web-sys features include no network feature", () => {
  const toml = readFileSync(join(ROOT, CARGO_TOML), "utf8");
  assert.deepEqual(networkFeatures(toml), []);
  assert.ok((webSysFeatures(toml) ?? []).length > 0);
});

test("the matcher reports JS network use with the line", () => {
  const f = findNetworkUse("a.ts", 'const x = 1;\nawait fetch("x");');
  assert.equal(f.length, 1);
  assert.equal(f[0].line, 2);
  for (const bad of [
    "new XMLHttpRequest()",
    "navigator.sendBeacon(u)",
    "new WebSocket(u)",
    "new EventSource(u)",
    "new Request(u)",
    "importScripts(u)",
    "new RTCPeerConnection()",
    'import("https://x.test/a.js")',
    "import(name)",
    "window.open(u)",
    "location.href = u;",
  ]) {
    assert.equal(findNetworkUse("a.ts", bad).length, 1, bad);
  }
  assert.equal(findNetworkUse("a.ts", 'await import("./wasm/paste2prompt_embed.js");').length, 0);
  assert.equal(findNetworkUse("a.ts", "// fetch(x) in a comment").length, 0);
});

test("the matcher reports Rust network use, including inside inline_js", () => {
  assert.equal(findNetworkUse("a.rs", "use reqwest::Client;").length, 1);
  assert.equal(findNetworkUse("a.rs", "use gloo_net::http::Request;").length, 1);
  assert.equal(findNetworkUse("a.rs", "let r = web_sys::Request::new();").length, 1);
  assert.equal(findNetworkUse("a.rs", "let s = WebSocket::new(u);").length, 1);
  assert.equal(findNetworkUse("a.rs", "nav.send_beacon(u);").length, 1);
  const inline = '#[wasm_bindgen(inline_js = "export function f(){")]\nfetch(u);';
  assert.equal(findNetworkUse("a.rs", inline).length, 1);
  assert.equal(findNetworkUse("a.rs", "/// mentions reqwest").length, 0);
});

test("allowlisted lines pass once per file; a copy or another file fails", () => {
  const file = "packages/element/src/overlay.ts";
  const [open, nav] = ALLOWLIST[file];
  assert.equal(findNetworkUse(file, `${open}\n  ${nav}`).length, 0);
  assert.equal(findNetworkUse(file, `${open}\n${nav}\n${open}`).length, 1);
  assert.equal(findNetworkUse("packages/element/src/other.ts", open).length, 1);
});

test("an unknown file type fails rather than going unscanned", () => {
  assert.equal(findNetworkUse("packages/element/src/x.wasm", "").length, 1);
  assert.equal(findNetworkUse("packages/element/src/x.css", "a{}").length, 0);
});

test("the Cargo feature check flags a network feature", () => {
  const toml = '[dependencies.web-sys]\nworkspace = true\nfeatures = ["Blob", "Request"]\n';
  assert.deepEqual(networkFeatures(toml), ["Request"]);
  assert.deepEqual(webSysFeatures("[dependencies]\nfoo = 1\n"), null);
});
