#!/usr/bin/env python3
"""Privacy guard: fail if the web app can make a network request after load.

CLAUDE.md rule 1: no network calls with user content. The only request the
page may make is loading its own static assets (the .js glue and the .wasm,
from the same origin). This script checks that, in six places:

Every file in dist/ is walked once and dispatched on its lowercased suffix,
so `x.JS` is scanned exactly like `x.js`. A file type the guard does not know
fails closed instead of going unscanned.

1. All JS in dist/ (every .js and .mjs, any case, at any depth, including the
   dist/snippets/ that `#[wasm_bindgen(module = ...)]` and `inline_js`
   produce). Every call that reaches the network from wasm has to go through
   JS, so a surviving web-sys `fetch`/`XmlHttpRequest`/... binding, or a
   snippet that calls one directly, shows up here. The only allowed hits are
   wasm-bindgen's own .wasm loader, matched as exact source lines and only in
   the one glue file that index.html's inline loader imports (see
   LOADER_ALLOWLIST). Any other JS file gets no allowance.
2. HTML: only top-level .html is allowed. Inline scripts (trunk's loader) use
   no network API, no URL attribute (src, href, srcset, poster, action, ...)
   names another origin, and there is no `<meta http-equiv="refresh">`. CSS
   (any depth) fails closed: no construct that can load a URL at all, not
   even a same-origin one, and no backslash escape (CSS_RISKY). SVG (any depth,
   the favicon): only plain drawing elements, no event handlers, no href
   but a `#fragment`, no <style> or style attribute, no CSS-ish attribute
   value but a local url(#id), and text only in title/desc/text/tspan
   (check_svg).
   Images and fonts are not scanned, but must start with their format's
   magic bytes.
3. The import and data sections of every .wasm in dist/ (any depth). Import
   names must not name a network or navigation API, and every import must
   come from the wasm-bindgen glue (whose shims check 1 has already
   scanned). The data segments (where Rust string literals end up) must not
   contain `http://` or `https://` (except a W3C namespace URI, see below)
   or `http-equiv`.
4. The Rust sources and manifest of crates/web: no `#[server]`, `server_fn`,
   `gloo-net`, `reqwest`, or web-sys network types, and no network API in the
   contents of an `inline_js = "..."` string. Comments are not stripped
   (only the URL-literal check below skips whole `//` comment lines): a
   comment that names a network API is a (rare, cheap to reword) false
   positive, which is safer than a stripper that a `//` inside a string can
   fool. These would mostly fail (1) at build time anyway once linked;
   catching them in source gives a clearer message and covers code the
   linker happened to drop. At least one .rs file must exist, so a wrong
   --web-crate path fails instead of passing on nothing.
5. Every .js / .mjs under crates/web (sources for `#[wasm_bindgen(module)]`),
   plus any file a `module = "..."` attribute points to: no network API.
6. dist/_headers must be exactly scripts/write_headers.py's HEADERS_TEMPLATE
   with the sha256 hash of each inline script in dist/index.html filled in,
   compared byte for byte after normalizing line endings. write_headers.py is
   the only producer of the file, so there is nothing to interpret: another
   path block, a `!` header-removal line, a Report-Only policy, a dropped or
   repeated directive, or any other edit is a finding. The template itself is
   also checked here (csp_problems), so weakening it in write_headers.py
   fails too: default-src, base-uri, form-action and frame-ancestors are
   'none', connect-src is 'self', every directive has its own source
   allowlist (CSP_ALLOWED), and there is no reporting directive. The
   template's structure is checked too (template_problems): exactly one path
   block and it is `/*`, no `!` header-removal line, no Report-Only policy,
   no repeated header, and exactly one Content-Security-Policy line, equal to
   CSP_DIRECTIVES joined. The same structural rules run on
   render_headers()'s output and on the bytes of dist/_headers, with the
   hashes in place of the placeholder (rendered_problems), because both come
   from write_headers.py: a render path that appended a block or a `!` line
   would move the expected value with the file. `wrangler deploy` runs only
   this guard, not the tests, so these properties have to hold here.

Navigation (`open(`, `location = ...`, `location.href|host|hostname|... =`,
`location[...]`, `location.assign`, `location.replace`) can also carry data
off the page, and CSP has no working navigation directive, so checks 1-5
treat it like a network API. Two more navigation forms are caught by what
they are built from (#33), in every line of JS (1, 5), inline script (2),
inline_js and Rust source (4), and in wasm data (3):

- A runtime `<meta http-equiv="refresh">`: any runtime `http-equiv`
  (`httpEquiv`, `http-equiv`, `set_http_equiv`, `http_equiv`), whatever its
  value. The web-sys `HtmlMetaElement` feature and a `...httpequiv...`
  wasm import are findings too.
- An external URL in a string literal, so `a.href = "https://..."; a.click()`
  fails: a quote, then `http:`, `https:` or `//host`. In Rust only `"`
  opens a string and whole `//` comment lines are skipped, so doc-comment
  URLs pass. The one exemption is the fixed XML namespace URIs on
  www.w3.org (W3C_NAMESPACES: SVG, MathML, XHTML, XLink, XML, XMLNS).
  They are identifiers that are never fetched or navigated to, and Leptos
  (tachys) embeds the SVG one in every build with a reactive SVG attribute.
  The match is case-sensitive and covers the exact `http://www.w3.org/...`
  scheme, host and path. In JS and Rust the closing quote must follow the
  URI, so `"http://www.w3.org/2000/svgevil.example"` still fails there. In
  wasm data it is a prefix match, because rustc packs strings back to back
  with no terminator, so that same string would pass in wasm data alone.
  The host stays www.w3.org either way, so the exemption cannot reach
  another origin. Any other www.w3.org URL, https, or a lookalike host such
  as `http://www.w3.org.evil.example/2000/svg` is still a finding.

Threat model: this is a static scan for accidental or ordinary network code,
such as a web-sys binding, a helper crate's snippet, or a copied analytics
call. Known gaps, accepted on purpose:

- Deliberate obfuscation, for example `Reflect.get(globalThis, "fe" + "tch")`
  behind a generic shim, a computed property name, `"http-" + "equiv"`, or
  `"https:" + "//evil.example"`. No static scan can catch it.
- An anchor's `href` plus `click()` where the URL is not a literal: built at
  runtime from parts that are not each an external URL, or taken from the
  page, the clipboard or a dropped file. This stays allowed because
  downloading an exported image needs `<a download href=blob:...>` (#12);
  web-sys `set_href`, `click`, `set_download` and `create_object_url` are
  not banned, and `blob:` / `data:` literals pass.
- Literals with other schemes (`mailto:`, `ftp:`, a custom protocol): only
  http(s) and protocol-relative literals are flagged.
- A protocol-relative `//host` literal is caught only in the Rust source,
  not in wasm data, where random bytes contain `//` (a real build has
  `//g9`). URLs in a multi-line string literal whose line starts with the
  URL or with `//` are missed by the line-based scans.
- DNS prefetch, e.g. a JS-created `<link rel="dns-prefetch"
  href="//<data>.example">`. It is a DNS-only leak that CSP does not reliably
  block.

The backstops for the gaps that make requests are the CSP limits checked in (6):
`connect-src 'self'` blocks fetch/XHR/WebSocket/beacons to any other origin,
`form-action 'none'` blocks form posts, `base-uri 'none'` and
`default-src 'none'` stop any other load, and eval / `new Function` are
blocked because 'unsafe-eval' is never allowed (only 'wasm-unsafe-eval').
Top-level navigation is not covered by CSP (there is no shipping
`navigate-to`), which is why navigation is scanned for in (1)-(5). Scripts
that Cloudflare injects at the edge are zone settings (see wrangler.jsonc).

Usage: python3 scripts/privacy_check.py [--dist DIR] [--web-crate DIR]
Exit 0 on pass, 1 on any finding. Tests: scripts/test_privacy_check.py.
"""

from __future__ import annotations

import argparse
import difflib
import os
import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from xml.etree import ElementTree

sys.path.insert(0, str(Path(__file__).resolve().parent))
import write_headers  # noqa: E402  (the canonical _headers template)

# Browser APIs that send a request, or navigate away with data in the URL.
# Network APIs are matched by their bare names so aliasing (`const f = fetch`)
# is still caught.
NETWORK_JS = re.compile(
    r"\b(fetch|XMLHttpRequest|WebSocket|sendBeacon|EventSource|WebTransport|"
    r"RTCPeerConnection|importScripts)\b"
    r"|\b(window|self|globalThis|top|parent|opener)\s*\.\s*open\s*\("
    r"|(?<![.\w$])open\s*\("
    r"|\blocation\s*(\.\s*(href|host|hostname|protocol|port|pathname|search)\s*)?=(?!=)"
    r"|\blocation\s*\["
    r"|\blocation\s*\.\s*(assign|replace)\b"
)

# wasm-bindgen's loader for the app's own .wasm. Matched as whole stripped
# lines, each at most once, and only in the one glue file that index.html's
# inline loader imports, so a copy anywhere else (a second top-level .js, a
# snippet) is still a finding. If a
# wasm-bindgen upgrade reformats these, the guard fails closed: review the
# new loader and update the lines here.
LOADER_ALLOWLIST = (
    "module_or_path = fetch(module_or_path);",
    "throw new Error(`failed to fetch Wasm: ${module.status} ${module.statusText} "
    "fetching '${module.url}'`);",
)

# Same idea for wasm import names (lowercased): web-sys names its shims
# `__wbg_<method>_<hash>`, e.g. `__wbg_fetch_1b2c...` or `__wbg_new_..._websocket`.
# `location` (the Window getter every Location navigation goes through),
# `open`, `assign` and `replace` cover navigation.
NETWORK_WASM_IMPORT = re.compile(
    r"fetch|xmlhttprequest|websocket|sendbeacon|eventsource|webtransport|rtcpeerconnection"
    r"|(^|_)(location|open\w*|assign|replace)(_|$)|httpequiv"
)

# Navigation that neither NETWORK_* pattern nor CSP stops (#33). Matched in
# every line of JS, inline_js, inline HTML script and crates/web Rust source.
#
# A `<meta http-equiv="refresh">` created at runtime navigates as the static
# one does. Any runtime http-equiv is flagged, whatever its value, because
# the value may come from a variable and the app has no use for one. This
# covers `m.httpEquiv = ...`, `setAttribute("http-equiv", ...)`, web-sys
# `set_http_equiv`, `http_equiv=` in a view! macro, and markup in a string.
# Not preceded or followed by a letter or digit, so `http_equivalent` passes.
RUNTIME_HTTP_EQUIV = re.compile(r"(?<![a-z0-9])http[-_]?equiv(?![a-z0-9])", re.IGNORECASE)
# Fixed XML namespace URIs (#33). They name a namespace, are never fetched or
# navigated to, and Leptos's tachys passes the SVG one to createElementNS for
# every SVG element it builds at runtime. Exact scheme, host and path,
# matched case-sensitively; see the module docstring for how each scan uses
# them.
W3C_NAMESPACES = (
    "http://www.w3.org/2000/svg",
    "http://www.w3.org/1998/Math/MathML",
    "http://www.w3.org/1999/xhtml",
    "http://www.w3.org/1999/xlink",
    "http://www.w3.org/XML/1998/namespace",
    "http://www.w3.org/2000/xmlns/",
)
_NS = "|".join(re.escape(u) for u in W3C_NAMESPACES)
# A string literal that names another origin: an opening quote (", ', `,
# also the `"` of a Rust raw or byte string), optional whitespace, then
# `http:` / `https:` or `//` followed by a host character. Assigned to an
# href and clicked, it navigates away. It is anchored on the quote, so a bare
# URL in a comment (`/// See https://...`) passes and only a quoted one
# trips. Blob downloads (#12) pass: `set_href(&blob_url)` has no literal and
# `blob:` / `data:` are not matched. The one exemption is a literal that is
# exactly one of W3C_NAMESPACES, closed by the same quote.
EXTERNAL_URL_LITERAL = re.compile(
    rf"""(["'`])(?!(?-i:{_NS})\1)\s*(https?:|//[^\s/"'`\\])""", re.IGNORECASE
)
# Rust: only `"` opens a string (`'` is a char or lifetime, a backtick is
# nothing; a real build has ``/// plain `http:` on another host``), and a
# line that is wholly a `//`, `///` or `//!` comment is not scanned for it,
# so a quoted URL in a doc comment passes. A trailing comment after code and
# a /* */ block are still scanned (reword them).
RUST_URL_LITERAL = re.compile(rf""""(?!(?-i:{_NS})")\s*(https?:|//[^\s/"\\])""", re.IGNORECASE)
# The same two in the wasm data section, where strings have no quotes. A
# bare `//` is not matched there: binary data has many (a real build has
# `//g9`), so a protocol-relative literal is caught only in the Rust source.
# No word boundaries either: rustc packs string literals back to back, so
# "http-equiv" is often followed directly by the next string's letters.
# For the same reason a W3C namespace URI is exempt as a prefix here: the
# next string's bytes follow it directly (a real build has
# `http://www.w3.org/2000/svg\x1f\x02`).
WASM_DATA_NAVIGATION = re.compile(
    rb"(?!(?-i:" + _NS.encode() + rb"))https?://|http[-_]?equiv", re.IGNORECASE
)

# wasm-bindgen's glue module, the only place imports may come from: "wbg" or,
# in current versions, "./<crate>_bg.js".
WBG_MODULE = re.compile(r"^(wbg|\./[A-Za-z0-9_-]+_bg\.js)$")

# `import ... from '/<crate>-<hash>.js'` in trunk's inline loader: names the glue.
LOADER_IMPORT = re.compile(r"""\bfrom\s*['"]([^'"]+)['"]|\bimport\s*\(\s*['"]([^'"]+)['"]""")

# Rust-side ways to add a network call or a navigation.
NETWORK_RUST = re.compile(
    r"#\s*\[\s*(leptos::)?server\b|\bserver_fn\b|\bgloo_net\b|\breqwest\b|"
    r"\bfetch_with_(str|request)\b|\bXmlHttpRequest\b|\bWebSocket\b|\bEventSource\b|"
    r"\bsend_beacon\b|\bRtcPeerConnection\b|\bWebTransport\b|\bfetch\s*\(|"
    r"\bopen_with_url\w*\b|\.location\s*\(\s*\)"
)
NETWORK_MANIFEST = re.compile(
    r"^\s*(gloo-net|gloo_net|reqwest|server_fn|ehttp|ureq)\s*[=.]"
    r"|\"(Request|RequestInit|XmlHttpRequest|WebSocket|EventSource|"
    r"RtcPeerConnection|WebTransport|Location|HtmlMetaElement)\"",
    re.MULTILINE,
)

# `inline_js = "..."` or `inline_js = r#"..."#` in a #[wasm_bindgen] attribute.
INLINE_JS = re.compile(
    r"\binline_js\s*=\s*(?:r(?P<hashes>#*)\"(?P<raw>.*?)\"(?P=hashes)|\"(?P<str>(?:[^\"\\]|\\.)*)\")",
    re.DOTALL,
)
# `module = "/path.js"` in a #[wasm_bindgen] attribute.
MODULE_ATTR = re.compile(r"\bmodule\s*=\s*\"([^\"]+)\"")

# File types trunk produces today (plus static asset types the app may add).
# Anything else in dist/ fails closed until this guard is taught to read it.
# Suffixes are compared lowercased. Only the scanned types plus inert images
# and fonts; no .htm. SVG is markup that can carry script, so it is scanned
# (check_svg), not inert. An inert file must start with its format's magic
# bytes, so markup renamed to .ico or .png is a finding, not a pass. Each
# pattern is matched against the first 16 bytes (.webp is RIFF, a 4-byte
# size, then WEBP, so a renamed WAV or AVI is a finding too).
JS_SUFFIXES = {".js", ".mjs"}
INERT_MAGIC = {
    ".png": re.compile(rb"\x89PNG\r\n\x1a\n"),
    ".ico": re.compile(rb"\x00\x00\x01\x00"),
    ".jpg": re.compile(rb"\xff\xd8\xff"),
    ".webp": re.compile(rb"RIFF.{4}WEBP", re.DOTALL),
    ".woff2": re.compile(rb"wOF2"),
}
DIST_NAMES = {"_headers"}

# SVG (#58, the favicon). Parsed as XML and checked against an allowlist of
# plain drawing elements in the SVG namespace, so <script>, <foreignObject>
# (HTML inside SVG), <a>, <image>, <feImage>, and <animate>/<set> (which can
# rewrite an href at runtime) are all findings. No DOCTYPE or entity
# declarations, and no processing instruction but the XML declaration (an
# <?xml-stylesheet?> loads a URL). No on* event-handler attribute. Every href
# (plain or xlink:) must be a same-document `#fragment`, and there is no
# xml:base. CSS fails closed rather than being parsed (#62): no <style>
# element and no style attribute at all (use presentation attributes such as
# fill and stroke). No other attribute value may hold a backslash (a CSS
# escape, so `\75rl(` is `url(`), `url(`, `image-set`, `@import` or
# `expression(`, except a value that is exactly a local `url(#id)` (a
# gradient, clip or mask; CSS_RISKY, shared with check_css). Text is allowed only inside <title>, <desc>,
# <text> and <tspan> (any other non-whitespace text is a finding, wherever
# ElementTree keeps it: an element's .text or a child's .tail).
SVG_NS = "{http://www.w3.org/2000/svg}"
SVG_ELEMENTS = {
    "svg", "g", "defs", "title", "desc", "symbol", "use",
    "path", "circle", "ellipse", "rect", "line", "polyline", "polygon",
    "text", "tspan", "linearGradient", "radialGradient", "stop", "clipPath", "mask",
}
SVG_DECL = re.compile(r"<!(DOCTYPE|ENTITY|ELEMENT|ATTLIST)", re.IGNORECASE)
SVG_PI = re.compile(r"<\?(?!xml\s)", re.IGNORECASE)
SVG_TEXT_ELEMENTS = {"title", "desc", "text", "tspan"}
SVG_LOCAL_URL = re.compile(r"\s*url\(#[A-Za-z_][A-Za-z0-9_.-]*\)\s*")

# Properties of write_headers.py's CSP template (check 6 runs csp_problems on
# it). Per-directive source allowlists: lowercased keywords; hash bodies are
# base64 and kept as written. A directive not listed may only be 'self' or
# 'none'. Reporting directives send requests, so they are banned. The
# REQUIRED directives have no fallback to default-src (or are the fallback).
_HASH = r"'sha(256|384|512)-[A-Za-z0-9+/]+={0,2}'"
CSP_ALLOWED = {
    "default-src": re.compile(r"^'none'$", re.IGNORECASE),
    "script-src": re.compile(rf"^('self'|'wasm-unsafe-eval'|{_HASH})$", re.IGNORECASE),
    "img-src": re.compile(r"^('self'|'none'|blob:|data:)$", re.IGNORECASE),
    "connect-src": re.compile(r"^('self'|'none')$", re.IGNORECASE),
}
CSP_DEFAULT_ALLOWED = re.compile(r"^('self'|'none')$", re.IGNORECASE)
CSP_BANNED_DIRECTIVES = {"report-uri", "report-to"}
CSP_REQUIRED = {
    "default-src": ["'none'"],
    "connect-src": ["'self'"],
    "base-uri": ["'none'"],
    "form-action": ["'none'"],
    "frame-ancestors": ["'none'"],
}

# A URL that leaves the page: protocol-relative, or any scheme except the
# page-local blob: and data:. Leading whitespace is ignored, as browsers do.
EXTERNAL_URL = re.compile(r"^\s*(//|(?!(blob|data):)[a-z][a-z0-9+.-]*:)", re.IGNORECASE)
URL_ATTRS = {"src", "href", "srcset", "poster", "action", "formaction", "data", "ping",
             "background", "manifest", "imagesrcset"}

# CSS, wherever it appears in dist/ (a .css file, an SVG attribute value),
# fails closed instead of being parsed (#62, #63): it may not contain any
# construct that can make the browser load a URL, same-origin or not, nor a
# backslash. The app's stylesheet needs none of these, and parsing CSS well
# enough to allow some is where the old url()/@import regex was bypassed:
#   - a backslash is a CSS escape, so `\75rl(` and `\0075rl(` are `url(`,
#     and `@\69mport` is `@import`;
#   - url( (any case, any whitespace), and src( (CSS Values 4 string URL);
#   - image-set( / -webkit-image-set( and image(, whose plain strings are URLs;
#   - @import with or without a space or quotes, @font-face (its src loads a
#     font) and @namespace;
#   - expression( (old IE script in CSS).
# Comments are not stripped first: a `/*` inside a string would make a naive
# stripper hide what follows, and none of these tokens may appear even in a
# comment. A comment cannot split a token into a live one either (`ur/**/l(`
# is the ident `ur` then the function `l(`).
CSS_RISKY = re.compile(
    r"\\|url\s*\(|src\s*\(|image-set|image\s*\(|@import|@font-face|@namespace|expression\s*\(",
    re.IGNORECASE,
)


class Findings:
    def __init__(self) -> None:
        self.items: list[str] = []

    def add(self, where: str, what: str) -> None:
        self.items.append(f"{where}: {what}")


def _attr_urls(key: str, value: str) -> list[str]:
    if key in ("srcset", "imagesrcset"):
        return [c.strip().split()[0] for c in value.split(",") if c.strip()]
    if key == "ping":
        return value.split()
    return [value]


class InlineScripts(HTMLParser):
    def __init__(self, convert_charrefs: bool) -> None:
        super().__init__(convert_charrefs=convert_charrefs)
        self.scripts: list[str] = []
        self.external: list[str] = []
        self._buf: list[str] | None = None

    def handle_starttag(self, tag, attrs):
        for key, value in attrs:
            if key in URL_ATTRS and value:
                for url in _attr_urls(key, value):
                    if EXTERNAL_URL.match(url):
                        self.external.append(f"<{tag} {key}={value!r}>")
                        break
        if tag == "meta" and any(
            k == "http-equiv" and (v or "").strip().lower() == "refresh" for k, v in attrs
        ):
            self.external.append(f"<meta http-equiv='refresh'> navigates away: {attrs!r}")
        if tag == "script" and not any(k == "src" for k, _ in attrs):
            self._buf = []

    def handle_endtag(self, tag):
        if tag == "script" and self._buf is not None:
            self.scripts.append("".join(self._buf))
            self._buf = None

    def handle_data(self, data):
        if self._buf is not None:
            self._buf.append(data)


def navigation_literals(text: str, rust: bool = False) -> list[str]:
    """Runtime http-equiv and external URL literals in JS or Rust text (#33)."""
    out = [f"runtime <meta http-equiv> (a refresh navigates away): {m.group(0)!r}"
           for m in RUNTIME_HTTP_EQUIV.finditer(text)]
    if rust and text.lstrip().startswith("//"):
        return out
    literal = RUST_URL_LITERAL if rust else EXTERNAL_URL_LITERAL
    out += [f"string literal naming another origin: {m.group(0).strip()!r}"
            for m in literal.finditer(text)]
    return out


def check_js(path: Path, f: Findings, allowlist: tuple[str, ...] = ()) -> None:
    allowed_left = list(allowlist)
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        for what in navigation_literals(line):
            f.add(f"{path}:{lineno}", f"{what} in JS: {line.strip()[:160]}")
        if not NETWORK_JS.search(line):
            continue
        stripped = line.strip()
        if stripped in allowed_left:
            allowed_left.remove(stripped)
            continue
        f.add(f"{path}:{lineno}", f"network or navigation API in JS: {stripped[:160]}")


def check_html(path: Path, f: Findings) -> list[str]:
    # The CSP hash must match the raw script text, so collect scripts with a
    # parser that leaves character references alone; attributes are decoded.
    raw = InlineScripts(convert_charrefs=False)
    text = path.read_text(encoding="utf-8")
    raw.feed(text)
    raw.close()
    decoded = InlineScripts(convert_charrefs=True)
    decoded.feed(text)
    decoded.close()
    for ref in decoded.external:
        if ref.startswith("<meta"):
            f.add(str(path), ref)
        else:
            f.add(str(path), f"reference to another origin: {ref}")
    for script in raw.scripts:
        for m in NETWORK_JS.finditer(script):
            f.add(str(path), f"network or navigation API {m.group(0)!r} in inline script")
        for what in navigation_literals(script):
            f.add(str(path), f"{what} in inline script")
    return raw.scripts


def css_risks(text: str) -> list[str]:
    """Every CSS_RISKY token in CSS text, as written."""
    return [m.group(0) for m in CSS_RISKY.finditer(text)]


def check_css(path: Path, f: Findings) -> None:
    # Whole-text scan: a token may span lines (`url\n(`), so never line by line.
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()
    for m in CSS_RISKY.finditer(text):
        lineno = text.count("\n", 0, m.start()) + 1
        f.add(f"{path}:{lineno}", f"CSS construct that can load a URL ({m.group(0)!r}): {lines[lineno - 1].strip()[:160]}")


def check_svg(path: Path, f: Findings) -> None:
    where = str(path)
    try:
        text = path.read_bytes().decode("utf-8")
    except UnicodeDecodeError as e:
        f.add(where, f"SVG is not UTF-8 ({e}); not scanned")
        return
    if SVG_DECL.search(text):
        f.add(where, "SVG has a DOCTYPE or entity declaration")
        return
    if SVG_PI.search(text):
        f.add(where, "SVG has a processing instruction (e.g. xml-stylesheet)")
    try:
        root = ElementTree.fromstring(text)
    except ElementTree.ParseError as e:
        f.add(where, f"SVG does not parse as XML ({e}); not scanned")
        return
    if root.tag != SVG_NS + "svg":
        f.add(where, f"SVG root element is {root.tag!r}, not <svg> in the SVG namespace")
    for el in root.iter():
        tag = el.tag if isinstance(el.tag, str) else repr(el.tag)
        name = tag[len(SVG_NS):] if tag.startswith(SVG_NS) else None
        if name not in SVG_ELEMENTS:
            f.add(where, f"SVG element {tag!r} is not an allowed drawing element")
        for key, value in el.attrib.items():
            local = key.rsplit("}", 1)[-1].lower()
            if local.startswith("on"):
                f.add(where, f"SVG event-handler attribute {key}={value[:80]!r}")
            if local == "style":
                f.add(where, f"SVG style attribute {key}={value[:80]!r}; use presentation attributes")
            elif local == "base":
                f.add(where, f"SVG {key}={value[:80]!r} attribute is not allowed")
            if local == "href" and not value.strip().startswith("#"):
                f.add(where, f"SVG {key}={value[:80]!r} is not a same-document #fragment")
            if css_risks(value) and not SVG_LOCAL_URL.fullmatch(value):
                f.add(where, f"SVG attribute {key}={value[:80]!r} holds CSS that is not a local url(#id)")
        # Text directly in el, and text after each child (ElementTree keeps
        # that in the child's .tail, but it is el's content to a browser).
        if name not in SVG_TEXT_ELEMENTS:
            for text in [el.text, *(child.tail for child in el)]:
                if text and text.strip():
                    f.add(where, f"SVG {tag!r} has text content {text.strip()[:80]!r}")


def _uleb(data: bytes, i: int) -> tuple[int, int]:
    result = shift = 0
    while True:
        b = data[i]
        i += 1
        result |= (b & 0x7F) << shift
        if not b & 0x80:
            return result, i
        shift += 7


def _wasm_section(data: bytes, wanted: int) -> int | None:
    """Offset of the body of the first section with id `wanted`, or None."""
    if data[:4] != b"\0asm":
        raise ValueError("not a wasm binary")
    i = 8
    while i < len(data):
        section, i = data[i], i + 1
        size, i = _uleb(data, i)
        if i + size > len(data):
            # Fail closed: stopping here would leave the rest of the file
            # (and the section we want) unscanned.
            raise ValueError(f"section {section} claims {size} bytes, past the end of the file")
        if section == wanted:
            return i
        i += size
    return None


def wasm_data(data: bytes) -> list[bytes]:
    """Return the bytes of every segment in a wasm binary's data section."""
    j = _wasm_section(data, 11)
    if j is None:
        return []
    count, j = _uleb(data, j)
    out = []
    for _ in range(count):
        flags, j = _uleb(data, j)
        if flags == 2:  # active, explicit memory index
            _, j = _uleb(data, j)
        if flags in (0, 2):  # offset expression: one const or global.get, then end
            op, j = data[j], j + 1
            if op not in (0x41, 0x42, 0x23):  # i32.const, i64.const, global.get
                raise ValueError(f"unknown data offset opcode {op:#x}")
            _, j = _uleb(data, j)  # an sleb128 has the same length as a uleb128
            if data[j] != 0x0B:
                raise ValueError("data offset expression is not a single instruction")
            j += 1
        elif flags != 1:  # 1: passive
            raise ValueError(f"unknown data segment flags {flags}")
        n, j = _uleb(data, j)
        if j + n > len(data):
            raise ValueError("data segment runs past the end of the file")
        out.append(data[j : j + n])
        j += n
    return out


def wasm_imports(data: bytes) -> list[tuple[str, str]]:
    """Return (module, name) for every import in a wasm binary."""
    i = _wasm_section(data, 2)
    if i is None:
        return []
    count, j = _uleb(data, i)
    out = []
    for _ in range(count):
        n, j = _uleb(data, j)
        module = data[j : j + n].decode("utf-8")
        j += n
        n, j = _uleb(data, j)
        name = data[j : j + n].decode("utf-8")
        j += n
        kind, j = data[j], j + 1
        if kind == 0:  # func: type index
            _, j = _uleb(data, j)
        elif kind == 1:  # table: reftype + limits
            j += 1
            flags, j = _uleb(data, j)
            _, j = _uleb(data, j)
            if flags & 1:
                _, j = _uleb(data, j)
        elif kind == 2:  # memory: limits
            flags, j = _uleb(data, j)
            _, j = _uleb(data, j)
            if flags & 1:
                _, j = _uleb(data, j)
        elif kind == 3:  # global: valtype + mut
            j += 2
        elif kind == 4:  # tag: attribute + type index
            j += 1
            _, j = _uleb(data, j)
        else:
            raise ValueError(f"unknown import kind {kind}")
        out.append((module, name))
    return out


def check_wasm(path: Path, f: Findings) -> int:
    try:
        imports = wasm_imports(path.read_bytes())
    except (ValueError, IndexError, UnicodeDecodeError) as e:
        f.add(str(path), f"cannot parse wasm import section ({e}); not scanned")
        return 0
    for module, name in imports:
        if not WBG_MODULE.match(module):
            f.add(str(path), f"import from unexpected module {module!r}: {name}")
        if NETWORK_WASM_IMPORT.search(name.lower()):
            f.add(str(path), f"network or navigation API import: {module}.{name}")
    try:
        segments = wasm_data(path.read_bytes())
    except (ValueError, IndexError) as e:
        f.add(str(path), f"cannot parse wasm data section ({e}); not scanned")
        return len(imports)
    for k, segment in enumerate(segments):
        for m in WASM_DATA_NAVIGATION.finditer(segment):
            context = segment[m.start() : m.start() + 80].split(b"\0")[0]
            f.add(f"{path} data segment {k} at {m.start()}",
                  f"URL or http-equiv in wasm data: {context.decode('utf-8', 'replace')!r}")
    return len(imports)


def _scan_js_text(where: str, text: str, f: Findings, what: str) -> None:
    for offset, line in enumerate(text.splitlines()):
        for m in NETWORK_JS.finditer(line):
            f.add(f"{where}+{offset}", f"network or navigation API {m.group(0)!r} in {what}")
        for problem in navigation_literals(line):
            f.add(f"{where}+{offset}", f"{problem} in {what}")


def _files_with_suffix(root: Path, suffixes: set[str]) -> list[Path]:
    """Files under root whose lowercased suffix is in suffixes (globs are case-sensitive)."""
    return sorted(p for p in root.rglob("*") if p.is_file() and p.suffix.lower() in suffixes)


def check_rust(crate: Path, f: Findings) -> list[Path]:
    """Scan crates/web Rust sources; return JS files named by `module = ...`."""
    sources = _files_with_suffix(crate / "src", {".rs"})
    if not sources:
        f.add(str(crate / "src"), "no .rs files; is --web-crate right?")
    modules: list[Path] = []
    for path in sources:
        text = path.read_text(encoding="utf-8")
        for lineno, line in enumerate(text.splitlines(), 1):
            m = NETWORK_RUST.search(line)
            if m:
                f.add(f"{path}:{lineno}", f"network code in crates/web: {m.group(0)!r}")
            for what in navigation_literals(line, rust=True):
                f.add(f"{path}:{lineno}", f"{what} in crates/web: {line.strip()[:160]}")
        for m in INLINE_JS.finditer(text):
            body = m.group("raw") if m.group("raw") is not None else m.group("str")
            if m.group("str") is not None:
                try:
                    body = body.encode("utf-8").decode("unicode_escape")
                except UnicodeDecodeError:
                    pass  # e.g. Rust's \u{..}; scan the text as written
            lineno = text.count("\n", 0, m.start()) + 1
            _scan_js_text(f"{path}:{lineno}", body, f, "inline_js")
        for m in MODULE_ATTR.finditer(text):
            spec = m.group(1)
            if spec.startswith("/"):
                modules.append(crate / spec.lstrip("/"))
            elif spec.startswith("."):
                modules.append(path.parent / spec)
            else:
                f.add(str(path), f"wasm_bindgen module {spec!r} is not a local file")
    manifest = crate / "Cargo.toml"
    for m in NETWORK_MANIFEST.finditer(manifest.read_text(encoding="utf-8")):
        f.add(str(manifest), f"network dependency or web-sys feature: {m.group(0).strip()!r}")
    return modules


def check_crate_js(crate: Path, modules: list[Path], f: Findings) -> int:
    files = {p.resolve() for p in _files_with_suffix(crate, JS_SUFFIXES)}
    for path in modules:
        if not path.is_file():
            f.add(str(path), "wasm_bindgen module file not found")
        else:
            files.add(path.resolve())
    for path in sorted(files):
        check_js(path, f)
    return len(files)


def parse_csp(policy: str) -> tuple[dict[str, list[str]], list[str]]:
    """Directives by lowercased name, plus problems found while parsing."""
    directives: dict[str, list[str]] = {}
    problems = []
    for part in policy.split(";"):
        words = part.split()
        if not words:
            continue
        name = words[0].lower()
        if name in directives:
            problems.append(f"CSP directive {name} appears more than once; browsers enforce only the first")
            continue
        directives[name] = words[1:]
    return directives, problems


def csp_problems(policy: str) -> list[str]:
    """Why a CSP is not acceptable for this app ([] if it is).

    Check 6 runs this on write_headers.py's template (the hash placeholder
    replaced by a real-looking hash); the tests run it on weakened policies.
    """
    directives, problems = parse_csp(policy)
    for name, sources in directives.items():
        if name in CSP_BANNED_DIRECTIVES:
            problems.append(f"CSP {name} sends violation reports over the network")
            continue
        allowed = CSP_ALLOWED.get(name, CSP_DEFAULT_ALLOWED)
        for src in sources:
            if not allowed.match(src):
                problems.append(f"CSP {name} allows {src}, which is not allowed in {name}")
        if any(s.lower() == "'none'" for s in sources) and len(sources) > 1:
            problems.append(f"CSP {name} mixes 'none' with other sources")
    for name, want in CSP_REQUIRED.items():
        if [s.lower() for s in directives.get(name, [])] != want:
            problems.append(f"CSP {name} must be exactly {' '.join(want)}")
    if not any(re.fullmatch(_HASH, s) for s in directives.get("script-src", [])):
        problems.append("CSP script-src has no inline-script hash")
    return problems


_PLACEHOLDER_HASH = "'sha256-" + "A" * 43 + "='"


# The whitespace JavaScript's String.prototype.trim() removes (WhiteSpace and
# LineTerminator in ECMA-262). Python's str.strip() differs: it keeps U+FEFF
# and strips U+001C..U+001F and U+0085, so it would read some lines unlike
# Cloudflare does.
_JS_WHITESPACE = (
    "\t\n\v\f\r \u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006"
    "\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff"
)
# Cloudflare's LINE_IS_PROBABLY_A_PATH, /^([^\s]+:\/\/|^\/)/, tested on the
# trimmed line (JS \s is the same set as trim()).
_CF_PATH_LINE = re.compile("(?:[^" + re.escape(_JS_WHITESPACE) + "]+://|/)")
# Cloudflare's MAX_LINE_LENGTH: a longer line is skipped with only a warning,
# so a CSP line that grew past it would deploy no CSP at all.
CF_MAX_LINE_LENGTH = 2000


def cf_line_length(line: str) -> int:
    """A line's length as JavaScript counts it (UTF-16 code units)."""
    return len(line.encode("utf-16-le")) // 2


def _structure_problems(text: str) -> tuple[list[str], list[str]]:
    """Why a _headers text is not one strict `/*` block, plus its CSP values.

    Lines are read the way Cloudflare's parser
    reads them (workers-sdk parseHeaders.ts): split on "\n", trim, skip blank
    and `#` lines, ignore lines over 2000 characters, and call a line a path
    if it starts with `/` or `scheme://`, whatever its indentation. Any other
    line is a header: `Name: value`, or `!Name` (no colon) to remove one.
    The text may have comments, exactly one path line (`/*`, unindented,
    first), and indented `Name: value` headers with non-empty values under
    it. Anything else is a finding, including lines Cloudflare would only
    warn about and drop. template_problems and rendered_problems share this
    and differ only in what the one CSP value must be.
    """
    problems = []
    paths: list[str] = []
    seen: set[str] = set()
    csp: list[str] = []
    for lineno, line in enumerate(text.split("\n"), 1):
        at = f"line {lineno}"
        length = cf_line_length(line)
        if length > CF_MAX_LINE_LENGTH:
            # Checked before comments, so no line of the file can get there.
            problems.append(f"{at}: {length} characters; Cloudflare ignores lines over "
                            f"{CF_MAX_LINE_LENGTH}")
            continue
        stripped = line.strip(_JS_WHITESPACE)
        if not stripped or stripped.startswith("#"):
            continue
        indented = line[0] in _JS_WHITESPACE
        if _CF_PATH_LINE.match(stripped):
            paths.append(stripped)
            if stripped != "/*":
                problems.append(f"{at}: path block {stripped[:120]!r}; the only block must be /*")
            if indented:
                problems.append(f"{at}: indented path line {stripped[:120]!r}; "
                                "Cloudflare reads it as a new path block")
            continue
        if not indented:
            problems.append(f"{at}: unindented header line {stripped[:120]!r}")
        if not paths:
            problems.append(f"{at}: header line before the /* block: {stripped[:120]!r}")
        if stripped.startswith("!"):
            problems.append(f"{at}: header-removal line {stripped[:120]!r}")
            continue
        name, colon, value = stripped.partition(":")
        name = name.strip(_JS_WHITESPACE).lower()
        value = value.strip(_JS_WHITESPACE)
        if not colon or not name or any(c.isspace() or c in _JS_WHITESPACE for c in name):
            problems.append(f"{at}: not a 'Name: value' header: {stripped[:120]!r}")
            continue
        if not value:
            problems.append(f"{at}: header {name} has an empty value, which Cloudflare drops")
            continue
        if name in seen:
            problems.append(f"{at}: header {name} set more than once")
        seen.add(name)
        if name == "content-security-policy":
            csp.append(value)
    if "report-only" in text.lower():
        problems.append("has a Report-Only policy, which enforces nothing")
    if len(paths) != 1:
        problems.append(f"must have exactly one path block (/*), found {len(paths)}")
    if len(csp) != 1:
        problems.append(f"must set Content-Security-Policy exactly once, found {len(csp)}")
    return problems, csp


def template_problems(template: str) -> list[str]:
    """Why write_headers.py's HEADERS_TEMPLATE is not one strict `/*` block.

    dist/_headers must equal the rendered template, so the exact-match check
    cannot see a wrong template. See _structure_problems for the rules; the
    CSP line must be CSP_DIRECTIVES joined, placeholder included.
    """
    problems, csp = _structure_problems(template)
    if len(csp) == 1 and csp[0] != write_headers.template_policy():
        problems.append("Content-Security-Policy line is not '; '.join(CSP_DIRECTIVES)")
    n = template.count(write_headers.SCRIPT_HASHES)
    if n != 1:
        problems.append(f"must contain the {write_headers.SCRIPT_HASHES} placeholder once, found {n}")
    return problems


_SHA256_SOURCE = re.compile(r"'sha256-[A-Za-z0-9+/]{43}='")


def rendered_problems(text: str) -> list[str]:
    """Why a rendered _headers (hashes filled in) is not one strict `/*` block.

    The same rules as template_problems, applied to the rendered text: the
    output of write_headers.render_headers() and the bytes of dist/_headers.
    Both come from write_headers.py, the module this guard checks, so a
    render path that adds lines outside the template would change the file
    and the guard's expected value alike and pass the exact-match check.
    Here the CSP line must be CSP_DIRECTIVES joined with the placeholder
    replaced by one or more 'sha256-...' sources, and must pass csp_problems.
    """
    problems, csp = _structure_problems(text)
    if len(csp) == 1:
        before, _, after = write_headers.template_policy().partition(write_headers.SCRIPT_HASHES)
        value = csp[0]
        fits = (len(value) >= len(before) + len(after)
                and value.startswith(before) and value.endswith(after))
        hashes = value[len(before):len(value) - len(after)].split(" ") if fits else []
        if not fits or not all(_SHA256_SOURCE.fullmatch(h) for h in hashes):
            problems.append("Content-Security-Policy line is not CSP_DIRECTIVES joined with "
                            "the inline-script hashes filled in")
        problems.extend(csp_problems(value))
    n = text.count(write_headers.SCRIPT_HASHES)
    if n:
        problems.append(f"contains the {write_headers.SCRIPT_HASHES} placeholder {n} time(s)")
    return problems


def check_headers(dist: Path, f: Findings) -> None:
    path = dist / "_headers"
    where = str(path)
    # The template itself must be a strict policy, so weakening it in
    # write_headers.py fails here even though dist/_headers would match it.
    policy = write_headers.template_policy().replace(write_headers.SCRIPT_HASHES, _PLACEHOLDER_HASH)
    for problem in csp_problems(policy):
        f.add("scripts/write_headers.py", f"template: {problem}")
    for problem in template_problems(write_headers.HEADERS_TEMPLATE):
        f.add("scripts/write_headers.py", f"HEADERS_TEMPLATE: {problem}")
    if not path.is_file():
        f.add(where, "missing; run scripts/write_headers.py after trunk build")
        return
    try:
        expected = write_headers.expected_headers(dist)
    except (ValueError, OSError, UnicodeDecodeError) as e:
        f.add(where, f"cannot compute the expected file: {e}")
        return
    # The exact-match check below trusts write_headers.py for the expected
    # value, so both the render output and the deployed bytes get the
    # structural check too: a render path that appends a block, a `!` line
    # or a Report-Only header fails here even when the file matches it. The
    # hashes also make the rendered CSP line longer than the template's, and
    # Cloudflare silently ignores a line over its limit.
    for problem in rendered_problems(expected):
        f.add("scripts/write_headers.py", f"render_headers() output: {problem}")
    try:
        actual = path.read_bytes().decode("utf-8")
    except UnicodeDecodeError as e:
        f.add(where, f"not UTF-8: {e}")
        return
    actual = actual.replace("\r\n", "\n").replace("\r", "\n")
    for problem in rendered_problems(actual):
        f.add(where, problem)
    if actual == expected:
        return
    diff = list(difflib.unified_diff(
        expected.splitlines(), actual.splitlines(),
        "expected (scripts/write_headers.py)", where, lineterm="", n=0,
    ))
    f.add(where, "differs from scripts/write_headers.py's output; regenerate it, never edit it:")
    for line in diff[2:22]:
        f.add(where, f"  {line[:200]}")
    if len(diff) > 22:
        f.add(where, f"  ... {len(diff) - 22} more diff lines")


def find_glue(dist: Path, inline_scripts: list[str], f: Findings) -> Path | None:
    """The one top-level .js that index.html's inline loader imports."""
    specs = set()
    for script in inline_scripts:
        for m in LOADER_IMPORT.finditer(script):
            specs.add(m.group(1) or m.group(2))
    paths = set()
    for spec in specs:
        rel = spec[1:] if spec.startswith("/") else spec[2:] if spec.startswith("./") else None
        if rel is None or "/" in rel or not rel.lower().endswith(".js"):
            f.add(f"{dist / 'index.html'}", f"inline loader imports {spec!r}, not a top-level .js")
            continue
        paths.add(dist / rel)
    if len(paths) != 1:
        f.add(f"{dist / 'index.html'}",
              f"expected the inline loader to import exactly one glue .js, found {len(paths)}")
        return None
    glue = paths.pop()
    if not glue.is_file():
        f.add(str(glue), "glue .js imported by index.html not found")
        return None
    return glue


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--dist", default="dist", type=Path)
    ap.add_argument("--web-crate", default="crates/web", type=Path)
    args = ap.parse_args(argv)
    dist: Path = args.dist

    f = Findings()
    index = dist / "index.html"
    if not index.is_file():
        print(f"privacy_check: {index} not found; build first", file=sys.stderr)
        return 1

    # One walk; every decision is on the lowercased suffix.
    js, html, css, wasm, svg = [], [], [], [], []
    for path in sorted(p for p in dist.rglob("*") if p.is_file()):
        suffix = path.suffix.lower()
        if suffix in JS_SUFFIXES:
            js.append(path)
        elif suffix == ".html":
            if path.parent == dist:
                html.append(path)
            else:
                f.add(str(path), "HTML outside the dist root is not allowed")
        elif suffix == ".css":
            css.append(path)
        elif suffix == ".wasm":
            wasm.append(path)
        elif suffix == ".svg":
            svg.append(path)
        elif suffix in INERT_MAGIC:
            with path.open("rb") as fh:
                head = fh.read(16)
            if not INERT_MAGIC[suffix].match(head):
                f.add(str(path), f"{suffix} file does not start with the {suffix} magic bytes")
        elif path.parent == dist and path.name in DIST_NAMES:
            pass
        else:
            f.add(str(path), "file type the privacy guard does not scan; teach it or remove the file")
    if not (js and wasm):
        print(f"privacy_check: {dist} has no .js or .wasm; build first", file=sys.stderr)
        return 1

    inline: list[str] = []
    for path in html:
        scripts = check_html(path, f)
        if path == index:
            inline = scripts
    glue = find_glue(dist, inline, f)
    for path in js:
        check_js(path, f, LOADER_ALLOWLIST if path == glue else ())
    for path in css:
        check_css(path, f)
    for path in svg:
        check_svg(path, f)
    n_imports = sum(check_wasm(path, f) for path in wasm)
    modules = check_rust(args.web_crate, f)
    n_crate_js = check_crate_js(args.web_crate, modules, f)
    check_headers(dist, f)

    if f.items:
        print("privacy_check: FAILED. The app must not make network requests (CLAUDE.md rule 1).")
        for item in f.items:
            print(f"  {item}")
            if "GITHUB_ACTIONS" in os.environ:
                print(f"::error::privacy_check: {item}")
        return 1
    print(
        f"privacy_check: OK ({len(js)} JS, {len(html)} HTML, {len(svg)} SVG, {len(wasm)} wasm with "
        f"{n_imports} imports, crates/web sources and {n_crate_js} crate JS, _headers exact)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
