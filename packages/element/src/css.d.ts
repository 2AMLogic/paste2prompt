// esbuild inlines .css imports as text (build.mjs).
declare module "*.css" {
  const text: string;
  export default text;
}
