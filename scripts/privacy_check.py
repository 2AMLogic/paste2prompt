#!/usr/bin/env python3
"""Privacy guard: fail if the web app can make a network request after load.

CLAUDE.md rule 1: no network calls with user content. The only request the
page may make is loading its own static assets (the .js glue and the .wasm,
from the same origin). This script checks that, in five places:

1. JS glue in dist/*.js. Every call that reaches the network from wasm has to
   go through a JS import shim that wasm-bindgen writes into this file, so a
   surviving web-sys `fetch`/`XmlHttpRequest`/... binding shows up here. The
   only allowed hits are wasm-bindgen's own .wasm loader, matched as exact
   source lines (see LOADER_ALLOWLIST).
2. Inline scripts in dist/*.html (trunk's loader). No network API at all.
3. The wasm import section of dist/*.wasm. Import names must not name a
   network API, and every import must come from the wasm-bindgen glue
   (whose shims check 1 has already scanned).
4. The Rust sources and manifest of crates/web: no `#[server]`, `server_fn`,
   `gloo-net`, `reqwest`, or web-sys network types. These would fail (1) at
   build time anyway once linked; catching them in source gives a clearer
   message and covers code the linker happened to drop.
5. dist/_headers (written by scripts/write_headers.py): a CSP is present, and
   every source in it is 'self', 'none', a hash, 'wasm-unsafe-eval', blob: or
   data:, so no directive names another origin.

Usage: python3 scripts/privacy_check.py [--dist DIR] [--web-crate DIR]
Exit 0 on pass, 1 on any finding.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import os
import re
import sys
from html.parser import HTMLParser
from pathlib import Path

# Browser APIs that send a request. `navigator.sendBeacon` and friends are
# matched by their bare names so aliasing (`const f = fetch`) is still caught.
NETWORK_JS = re.compile(
    r"\b(fetch|XMLHttpRequest|WebSocket|sendBeacon|EventSource|WebTransport|"
    r"RTCPeerConnection|importScripts)\b"
)

# wasm-bindgen's loader for the app's own .wasm. Matched as whole stripped
# lines, each at most once per file, so a copy elsewhere is still a finding.
# If a wasm-bindgen upgrade reformats these, the guard fails closed: review
# the new loader and update the lines here.
LOADER_ALLOWLIST = (
    "module_or_path = fetch(module_or_path);",
    "throw new Error(`failed to fetch Wasm: ${module.status} ${module.statusText} "
    "fetching '${module.url}'`);",
)

# Same idea for wasm import names (lowercased): web-sys names its shims
# `__wbg_<method>_<hash>`, e.g. `__wbg_fetch_1b2c...` or `__wbg_new_..._websocket`.
NETWORK_WASM_IMPORT = re.compile(
    r"fetch|xmlhttprequest|websocket|sendbeacon|eventsource|webtransport|rtcpeerconnection"
)

# wasm-bindgen's glue module, the only place imports may come from: "wbg" or,
# in current versions, "./<crate>_bg.js".
WBG_MODULE = re.compile(r"^(wbg|\./[A-Za-z0-9_-]+_bg\.js)$")

# Rust-side ways to add a network call.
NETWORK_RUST = re.compile(
    r"#\s*\[\s*(leptos::)?server\b|\bserver_fn\b|\bgloo_net\b|\breqwest\b|"
    r"\bfetch_with_(str|request)\b|\bXmlHttpRequest\b|\bWebSocket\b|\bEventSource\b|"
    r"\bsend_beacon\b|\bRtcPeerConnection\b|\bWebTransport\b|\bfetch\s*\("
)
NETWORK_MANIFEST = re.compile(
    r"^\s*(gloo-net|gloo_net|reqwest|server_fn|ehttp|ureq)\s*[=.]"
    r"|\"(Request|RequestInit|XmlHttpRequest|WebSocket|EventSource|"
    r"RtcPeerConnection|WebTransport)\"",
    re.MULTILINE,
)

CSP_ALLOWED_SOURCE = re.compile(
    r"^('self'|'none'|'wasm-unsafe-eval'|'sha(256|384|512)-[A-Za-z0-9+/=]+'|blob:|data:)$"
)


class Findings:
    def __init__(self) -> None:
        self.items: list[str] = []

    def add(self, where: str, what: str) -> None:
        self.items.append(f"{where}: {what}")


class InlineScripts(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=False)
        self.scripts: list[str] = []
        self.external: list[str] = []
        self._buf: list[str] | None = None

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        for key in ("src", "href"):
            url = attrs.get(key) or ""
            if re.match(r"^([a-z][a-z0-9+.-]*:)?//", url, re.IGNORECASE):
                self.external.append(f"<{tag} {key}={url!r}>")
        if tag == "script" and "src" not in attrs:
            self._buf = []

    def handle_endtag(self, tag):
        if tag == "script" and self._buf is not None:
            self.scripts.append("".join(self._buf))
            self._buf = None

    def handle_data(self, data):
        if self._buf is not None:
            self._buf.append(data)


def check_js(path: Path, f: Findings) -> None:
    allowed_left = list(LOADER_ALLOWLIST)
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not NETWORK_JS.search(line):
            continue
        stripped = line.strip()
        if stripped in allowed_left:
            allowed_left.remove(stripped)
            continue
        f.add(f"{path}:{lineno}", f"network API in JS glue: {stripped[:160]}")


def check_html(path: Path, f: Findings) -> list[str]:
    parser = InlineScripts()
    parser.feed(path.read_text(encoding="utf-8"))
    parser.close()
    for ref in parser.external:
        f.add(str(path), f"reference to another origin: {ref}")
    for text in parser.scripts:
        for m in NETWORK_JS.finditer(text):
            f.add(str(path), f"network API {m.group(0)!r} in inline script")
    return parser.scripts


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
    imports = wasm_imports(path.read_bytes())
    for module, name in imports:
        if not WBG_MODULE.match(module):
            f.add(str(path), f"import from unexpected module {module!r}: {name}")
        if NETWORK_WASM_IMPORT.search(name.lower()):
            f.add(str(path), f"network API import: {module}.{name}")
    return len(imports)


def check_rust(crate: Path, f: Findings) -> None:
    for path in sorted((crate / "src").rglob("*.rs")):
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            code = line.split("//", 1)[0]
            m = NETWORK_RUST.search(code)
            if m:
                f.add(f"{path}:{lineno}", f"network code in crates/web: {m.group(0)!r}")
    manifest = crate / "Cargo.toml"
    for m in NETWORK_MANIFEST.finditer(manifest.read_text(encoding="utf-8")):
        f.add(str(manifest), f"network dependency or web-sys feature: {m.group(0).strip()!r}")


def check_headers(dist: Path, inline_scripts: list[str], f: Findings) -> None:
    path = dist / "_headers"
    if not path.is_file():
        f.add(str(path), "missing; run scripts/write_headers.py after trunk build")
        return
    policies = [
        line.split(":", 1)[1].strip()
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip().lower().startswith("content-security-policy:")
    ]
    if len(policies) != 1:
        f.add(str(path), f"expected one Content-Security-Policy, found {len(policies)}")
        return
    directives = {}
    for part in policies[0].split(";"):
        words = part.split()
        if words:
            directives[words[0]] = words[1:]
    for name, sources in directives.items():
        for src in sources:
            if not CSP_ALLOWED_SOURCE.match(src):
                f.add(str(path), f"CSP {name} allows {src!r}; only same-origin sources are allowed")
    if directives.get("default-src") != ["'none'"]:
        f.add(str(path), "CSP default-src must be 'none'")
    connect = directives.get("connect-src", [])
    if any(s not in ("'self'", "'none'") for s in connect):
        f.add(str(path), f"CSP connect-src must be 'self' or 'none', got {connect}")
    script = directives.get("script-src", [])
    for text in inline_scripts:
        digest = base64.b64encode(hashlib.sha256(text.encode("utf-8")).digest()).decode()
        if f"'sha256-{digest}'" not in script:
            f.add(str(path), "CSP script-src lacks the hash of an inline script in index.html")


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--dist", default="dist", type=Path)
    ap.add_argument("--web-crate", default="crates/web", type=Path)
    args = ap.parse_args(argv)

    f = Findings()
    js = sorted(args.dist.glob("*.js"))
    html = sorted(args.dist.glob("*.html"))
    wasm = sorted(args.dist.glob("*.wasm"))
    if not (js and html and wasm):
        print(f"privacy_check: {args.dist} has no .js, .html or .wasm; build first", file=sys.stderr)
        return 1

    for path in js:
        check_js(path, f)
    inline = []
    for path in html:
        inline += check_html(path, f)
    n_imports = sum(check_wasm(path, f) for path in wasm)
    check_rust(args.web_crate, f)
    check_headers(args.dist, inline, f)

    if f.items:
        print("privacy_check: FAILED. The app must not make network requests (CLAUDE.md rule 1).")
        for item in f.items:
            print(f"  {item}")
            if "GITHUB_ACTIONS" in os.environ:
                print(f"::error::privacy_check: {item}")
        return 1
    print(
        f"privacy_check: OK ({len(js)} JS, {len(html)} HTML, {len(wasm)} wasm with "
        f"{n_imports} imports, crates/web sources, CSP)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
