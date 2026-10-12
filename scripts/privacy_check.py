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
   (any depth): no url() or @import names another origin.
3. The import section of every .wasm in dist/ (any depth). Import names must
   not name a network or navigation API, and every import must come from the
   wasm-bindgen glue (whose shims check 1 has already scanned).
4. The Rust sources and manifest of crates/web: no `#[server]`, `server_fn`,
   `gloo-net`, `reqwest`, or web-sys network types, and no network API in the
   contents of an `inline_js = "..."` string. Comments are not stripped: a
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
   allowlist (CSP_ALLOWED), and there is no reporting directive.

Navigation (`open(`, `location = ...`, `location.href|host|hostname|... =`,
`location[...]`, `location.assign`, `location.replace`) can also carry data
off the page, and CSP has no working navigation directive, so checks 1-5
treat it like a network API.

Threat model: this is a static scan for accidental or ordinary network code,
such as a web-sys binding, a helper crate's snippet, or a copied analytics
call. Known gaps, accepted on purpose:

- Deliberate obfuscation, for example `Reflect.get(globalThis, "fe" + "tch")`
  behind a generic shim, or a computed property name. No static scan can
  catch it.
- An anchor's `href` plus `click()` (`a.href = "https://..."; a.click()`).
  This stays allowed because downloading an exported image needs
  `<a download href=blob:...>` (#12); web-sys `set_href`, `click`,
  `set_download` and `create_object_url` are not banned.
- DNS prefetch, e.g. a JS-created `<link rel="dns-prefetch"
  href="//<data>.example">`. It is a DNS-only leak that CSP does not reliably
  block.

The backstops for all three are the CSP limits checked in (6):
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
    r"|(^|_)(location|open\w*|assign|replace)(_|$)"
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
    r"RtcPeerConnection|WebTransport|Location)\"",
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
# and fonts; no .svg (it can carry script) and no .htm.
JS_SUFFIXES = {".js", ".mjs"}
INERT_SUFFIXES = {".png", ".ico", ".jpg", ".webp", ".woff2"}
DIST_NAMES = {"_headers"}

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
CSS_URL = re.compile(r"url\(\s*(['\"]?)(.*?)\1\s*\)|@import\s+(['\"])(.*?)\3", re.IGNORECASE)


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


def check_js(path: Path, f: Findings, allowlist: tuple[str, ...] = ()) -> None:
    allowed_left = list(allowlist)
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
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
    return raw.scripts


def check_css(path: Path, f: Findings) -> None:
    for m in CSS_URL.finditer(path.read_text(encoding="utf-8")):
        url = m.group(2) if m.group(2) is not None else m.group(4)
        if EXTERNAL_URL.match(url):
            f.add(str(path), f"CSS reference to another origin: {m.group(0)[:160]}")


def _uleb(data: bytes, i: int) -> tuple[int, int]:
    result = shift = 0
    while True:
        b = data[i]
        i += 1
        result |= (b & 0x7F) << shift
        if not b & 0x80:
            return result, i
        shift += 7


def wasm_imports(data: bytes) -> list[tuple[str, str]]:
    """Return (module, name) for every import in a wasm binary."""
    if data[:4] != b"\0asm":
        raise ValueError("not a wasm binary")
    i = 8
    while i < len(data):
        section, i = data[i], i + 1
        size, i = _uleb(data, i)
        if section != 2:
            i += size
            continue
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
    return []


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
    return len(imports)


def _scan_js_text(where: str, text: str, f: Findings, what: str) -> None:
    for offset, line in enumerate(text.splitlines()):
        for m in NETWORK_JS.finditer(line):
            f.add(f"{where}+{offset}", f"network or navigation API {m.group(0)!r} in {what}")


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


def check_headers(dist: Path, f: Findings) -> None:
    path = dist / "_headers"
    where = str(path)
    # The template itself must be a strict policy, so weakening it in
    # write_headers.py fails here even though dist/_headers would match it.
    policy = write_headers.template_policy().replace(write_headers.SCRIPT_HASHES, _PLACEHOLDER_HASH)
    for problem in csp_problems(policy):
        f.add("scripts/write_headers.py", f"template: {problem}")
    if not path.is_file():
        f.add(where, "missing; run scripts/write_headers.py after trunk build")
        return
    try:
        expected = write_headers.expected_headers(dist)
    except (ValueError, OSError, UnicodeDecodeError) as e:
        f.add(where, f"cannot compute the expected file: {e}")
        return
    try:
        actual = path.read_bytes().decode("utf-8")
    except UnicodeDecodeError as e:
        f.add(where, f"not UTF-8: {e}")
        return
    actual = actual.replace("\r\n", "\n").replace("\r", "\n")
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
    js, html, css, wasm = [], [], [], []
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
        elif suffix in INERT_SUFFIXES or (path.parent == dist and path.name in DIST_NAMES):
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
        f"privacy_check: OK ({len(js)} JS, {len(html)} HTML, {len(wasm)} wasm with "
        f"{n_imports} imports, crates/web sources and {n_crate_js} crate JS, _headers exact)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
