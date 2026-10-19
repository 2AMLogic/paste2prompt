import { toBlob } from "html-to-image";

/**
 * Captures what the viewport shows, at device resolution, leaving out
 * `exclude` (the overlay itself).
 *
 * html-to-image re-renders the DOM into a canvas rather than reading the
 * screen, so it needs no permission prompt, but it isn't pixel-exact:
 * cross-origin images without CORS headers come out blank, and
 * `position: fixed` elements are placed relative to the page, not the
 * viewport, when the page is scrolled.
 */
export async function captureViewport(exclude: Element): Promise<Blob> {
  const body = document.body;
  const html = document.documentElement;
  // The body's offset from the viewport's top-left, which includes the
  // scroll position: the clone is moved by that much so the viewport's
  // part of the page lands on the canvas.
  const rect = body.getBoundingClientRect();
  const blob = await toBlob(body, {
    width: html.clientWidth,
    height: html.clientHeight,
    pixelRatio: window.devicePixelRatio || 1,
    backgroundColor: pageBackground(),
    style: {
      margin: "0",
      transform: `translate(${rect.left}px, ${rect.top}px)`,
      transformOrigin: "0 0",
    },
    filter: (node) => node !== exclude,
  });
  if (!blob) throw new Error("The page capture came out empty.");
  return blob;
}

/** The colour behind the page: the body's, else the root's, else white. */
function pageBackground(): string {
  for (const el of [document.body, document.documentElement]) {
    const color = getComputedStyle(el).backgroundColor;
    if (color && color !== "transparent" && color !== "rgba(0, 0, 0, 0)") return color;
  }
  return "#ffffff";
}
