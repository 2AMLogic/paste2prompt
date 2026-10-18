// Builds crates/embed into dist/wasm/: wasm-pack without its own optimiser,
// then wasm-opt -Os, as the site's trunk build does (crates/web/index.html).
//
// wasm-opt comes from $WASM_OPT, else trunk's cached copy, else PATH, and
// must be binaryen 123 or later (the version crates/web/Trunk.toml pins):
// older ones can't parse what current rustc emits. CI downloads 123.
import { execFileSync } from "node:child_process";
import { existsSync, renameSync, rmSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const MIN_BINARYEN = 123;
const out = "dist/wasm";
const wasm = join(out, "paste2prompt_embed_bg.wasm");

execFileSync(
  "wasm-pack",
  ["build", "../../crates/embed", "--target", "web", "--release", "--no-pack", "--no-opt", "--out-dir", `../../packages/element/${out}`],
  { stdio: "inherit" },
);
// wasm-pack writes a .gitignore of "*" here, which npm pack would honour,
// leaving the editor out of the package.
rmSync(join(out, ".gitignore"), { force: true });

const wasmOpt = findWasmOpt();
execFileSync(wasmOpt, [wasm, "-Os", "-o", `${wasm}.opt`], { stdio: "inherit" });
renameSync(`${wasm}.opt`, wasm);

function findWasmOpt() {
  const trunkCache = [
    join(homedir(), "Library/Caches/dev.trunkrs.trunk"),
    join(homedir(), ".cache/trunk"),
  ].map((dir) => join(dir, `wasm-opt-version_${MIN_BINARYEN}`, "bin", "wasm-opt"));
  const candidates = [process.env.WASM_OPT, ...trunkCache.filter(existsSync), "wasm-opt"].filter(Boolean);
  for (const bin of candidates) {
    let version;
    try {
      version = execFileSync(bin, ["--version"], { encoding: "utf8" });
    } catch {
      continue;
    }
    const n = Number(/version[ _](\d+)/.exec(version)?.[1]);
    if (n >= MIN_BINARYEN) return bin;
    console.warn(`build-wasm: skipping ${bin} (${version.trim()}): need binaryen ${MIN_BINARYEN} or later`);
  }
  throw new Error(
    `build-wasm: no wasm-opt from binaryen ${MIN_BINARYEN}+; set WASM_OPT, or run trunk build once in crates/web`,
  );
}
