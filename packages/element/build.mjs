// Bundles src/ into dist/index.js. The editor's stylesheet
// (crates/web/style.css) is inlined as text, for the shadow root.
// html-to-image stays a dependency for the host's bundler to resolve, and
// the wasm-pack output in dist/wasm/ is loaded on first open (load-wasm.ts).
//
// `--demo` also writes dist/demo.js for demo/index.html: the same bundle
// with html-to-image included, since a browser can't resolve its imports
// without a bundler.
import { build } from "esbuild";

const common = {
  entryPoints: ["src/index.ts"],
  bundle: true,
  format: "esm",
  target: "es2022",
  loader: { ".css": "text" },
  logLevel: "warning",
};

await build({ ...common, outfile: "dist/index.js", external: ["html-to-image", "./wasm/*"] });
if (process.argv.includes("--demo")) {
  await build({ ...common, outfile: "dist/demo.js", external: ["./wasm/*"] });
}
