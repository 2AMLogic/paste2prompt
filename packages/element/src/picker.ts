/*!
 * Element picking for the paste2prompt overlay.
 *
 * Adapted from ProPanes' packages/widget/src/element-picker.ts
 * (https://github.com/tinkerer/propanes, commit b4139c1), which is under
 * this license:
 *
 * MIT License
 *
 * Copyright (c) 2026 Amir Hirsch
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */

// What changed from ProPanes: the picker's own boxes and bar live in the
// overlay's shadow root (so the capture and elementFromPoint already leave
// them out), a click toggles an element rather than ending the pick, the
// page under the picker doesn't see the clicks, and each pick records the
// label and role docs/EMBED.md ("Element identity") asks for.

/** An element picked from the page, and what to call it. */
export interface PickedElement {
  element: Element;
  /** A CSS selector that matched only this element when it was picked. */
  selector: string;
  /** The lowercase tag name. */
  tag: string;
  /** The `role` attribute, if any. */
  role?: string;
  /** `aria-label`, else `data-testid`, else the element's text, shortened. */
  label?: string;
}

/** A pick in progress. */
export interface PickSession {
  /** The picks, in the order they were made; `null` if cancelled. */
  done: Promise<PickedElement[] | null>;
  /** Ends the pick as cancelled. */
  cancel(): void;
}

/** Longest label kept, in characters (crates/core MAX_ELEMENT_LABEL_CHARS). */
const MAX_LABEL_CHARS = 40;
/** Longest selector kept, in characters (crates/core MAX_SELECTOR_CHARS). */
const MAX_SELECTOR_CHARS = 512;
/** How many ancestors a selector climbs before giving up on uniqueness. */
const MAX_SELECTOR_DEPTH = 5;
/** Classes per selector segment: enough to read, short of a utility-class wall. */
const MAX_CLASSES = 3;

/**
 * Starts picking elements of the page under `host` (the overlay element),
 * drawing in `root` (its shadow root). Clicks and taps on the page toggle
 * the element under them; Enter or **Capture** ends the pick, Esc or
 * **Cancel** cancels it.
 */
export function startPicker(root: ShadowRoot, host: Element): PickSession {
  const layer = document.createElement("div");
  layer.className = "pick-layer";
  layer.innerHTML = `
    <div class="pick-hover" hidden></div>
    <div class="pick-tag" hidden></div>
    <div class="pick-bar" role="toolbar" aria-label="Pick elements">
      <span class="pick-text" role="status"></span>
      <button type="button" class="pick-capture">Capture</button>
      <button type="button" class="pick-cancel">Cancel</button>
    </div>`;
  const $ = <T extends Element>(selector: string) => layer.querySelector<T>(selector)!;
  const hover = $<HTMLElement>(".pick-hover");
  const tagLabel = $<HTMLElement>(".pick-tag");
  const text = $<HTMLElement>(".pick-text");
  const captureButton = $<HTMLButtonElement>(".pick-capture");
  root.append(layer);

  const selected: { element: Element; box: HTMLElement }[] = [];
  let hovered: Element | null = null;
  let finish!: (picks: PickedElement[] | null) => void;
  const done = new Promise<PickedElement[] | null>((resolve) => (finish = resolve));

  const updateText = () => {
    const n = selected.length;
    text.textContent =
      n === 0
        ? "Click the elements to mark, then Capture. Esc cancels."
        : `${n} element${n === 1 ? "" : "s"} marked. Click again to unmark.`;
  };
  updateText();

  /** The page element at a point: never the overlay itself. */
  const targetAt = (x: number, y: number): Element | null => {
    const el = document.elementFromPoint(x, y);
    return !el || el === host || host.contains(el) ? null : el;
  };

  /** Whether an event started inside the overlay (its own buttons). */
  const fromOverlay = (e: Event) => e.composedPath().includes(host);

  const place = (box: HTMLElement, el: Element) => {
    const r = el.getBoundingClientRect();
    Object.assign(box.style, {
      top: `${r.top}px`,
      left: `${r.left}px`,
      width: `${r.width}px`,
      height: `${r.height}px`,
    });
  };

  const showHover = (el: Element | null) => {
    hovered = el;
    hover.hidden = tagLabel.hidden = !el;
    if (!el) return;
    place(hover, el);
    hover.classList.toggle("marked", selected.some((s) => s.element === el));
    tagLabel.textContent = shortName(el);
    const r = el.getBoundingClientRect();
    tagLabel.style.top = `${Math.max(0, r.top - 22)}px`;
    tagLabel.style.left = `${Math.max(0, r.left)}px`;
  };

  const toggle = (el: Element) => {
    const i = selected.findIndex((s) => s.element === el);
    if (i >= 0) {
      selected[i].box.remove();
      selected.splice(i, 1);
    } else {
      const box = document.createElement("div");
      box.className = "pick-box";
      place(box, el);
      layer.insertBefore(box, hover);
      selected.push({ element: el, box });
    }
    updateText();
    showHover(el);
  };

  const onMove = (e: PointerEvent) => {
    if (e.pointerType === "touch") return;
    showHover(fromOverlay(e) ? null : targetAt(e.clientX, e.clientY));
  };

  // The page shouldn't react to a press that only picks: follow a link,
  // open a menu, take focus.
  const swallow = (e: Event) => {
    if (fromOverlay(e)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
  };

  const onClick = (e: MouseEvent) => {
    if (fromOverlay(e)) return;
    swallow(e);
    const el = targetAt(e.clientX, e.clientY);
    if (el) toggle(el);
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      swallow(e);
      end(null);
    } else if (e.key === "Enter" && !fromOverlay(e)) {
      swallow(e);
      end(collect());
    }
  };

  const onScroll = () => {
    for (const s of selected) place(s.box, s.element);
    if (hovered) showHover(hovered);
  };

  const pressEvents = ["pointerdown", "mousedown", "pointerup", "mouseup", "dblclick", "auxclick"];
  document.addEventListener("pointermove", onMove, true);
  document.addEventListener("click", onClick, true);
  for (const type of pressEvents) document.addEventListener(type, swallow, true);
  document.addEventListener("keydown", onKey, true);
  window.addEventListener("scroll", onScroll, true);
  window.addEventListener("resize", onScroll);

  const collect = () => selected.map((s) => describe(s.element));

  const end = (picks: PickedElement[] | null) => {
    document.removeEventListener("pointermove", onMove, true);
    document.removeEventListener("click", onClick, true);
    for (const type of pressEvents) document.removeEventListener(type, swallow, true);
    document.removeEventListener("keydown", onKey, true);
    window.removeEventListener("scroll", onScroll, true);
    window.removeEventListener("resize", onScroll);
    layer.remove();
    finish(picks);
  };

  captureButton.addEventListener("click", () => end(collect()));
  $<HTMLButtonElement>(".pick-cancel").addEventListener("click", () => end(null));
  captureButton.focus();

  return { done, cancel: () => end(null) };
}

/**
 * Where each pick is in the viewport now, as fractions of its width and
 * height (the capture's coordinates). Picks no longer on screen are left
 * out.
 */
export function pickBoxes(picks: PickedElement[]) {
  const w = document.documentElement.clientWidth;
  const h = document.documentElement.clientHeight;
  return picks.flatMap(({ element, ...identity }) => {
    if (!element.isConnected) return [];
    const r = element.getBoundingClientRect();
    if (r.right <= 0 || r.bottom <= 0 || r.left >= w || r.top >= h) return [];
    return [{ x0: r.left / w, y0: r.top / h, x1: r.right / w, y1: r.bottom / h, ...identity }];
  });
}

function describe(el: Element): PickedElement {
  const tag = el.tagName.toLowerCase();
  const role = oneLine(el.getAttribute("role") ?? "");
  const label = shorten(
    oneLine(el.getAttribute("aria-label") ?? "") ||
      oneLine(el.getAttribute("data-testid") ?? "") ||
      oneLine(el.textContent ?? ""),
  );
  return {
    element: el,
    selector: selectorFor(el),
    // The core accepts lowercase letters, digits and `-`; an SVG or
    // namespaced tag can have others.
    tag: /^[a-z0-9-]+$/.test(tag) ? tag : "element",
    role: role || undefined,
    label: label || undefined,
  };
}

/**
 * The shortest selector, built upward from the element, that matches only
 * it: an id or `data-testid` where one is unique, else tag, a few classes
 * and `:nth-of-type` (ProPanes' approach).
 */
function selectorFor(el: Element): string {
  const unique = (s: string) => {
    try {
      return document.querySelectorAll(s).length === 1;
    } catch {
      return false;
    }
  };
  const testId = el.getAttribute("data-testid");
  if (testId) {
    const s = `[data-testid="${CSS.escape(testId)}"]`;
    if (unique(s)) return s;
  }
  const segments: string[] = [];
  let current: Element | null = el;
  for (let depth = 0; current && depth < MAX_SELECTOR_DEPTH; depth++) {
    if (current.id) {
      const candidate = [`#${CSS.escape(current.id)}`, ...segments].join(" > ");
      if (unique(candidate)) return fit(candidate);
    }
    let seg = current.tagName.toLowerCase();
    const classes = Array.from(current.classList).slice(0, MAX_CLASSES);
    if (classes.length > 0) seg += classes.map((c) => `.${CSS.escape(c)}`).join("");
    const parent: Element | null = current.parentElement;
    if (parent) {
      const tag = current.tagName;
      const siblings = Array.from(parent.children).filter((c) => c.tagName === tag);
      if (siblings.length > 1) seg += `:nth-of-type(${siblings.indexOf(current) + 1})`;
    }
    segments.unshift(seg);
    const candidate = segments.join(" > ");
    if (unique(candidate)) return fit(candidate);
    current = parent;
  }
  return fit(segments.join(" > "));
}

/** Drops leading segments until the selector fits the core's limit. */
function fit(selector: string): string {
  const parts = selector.split(" > ");
  while (parts.length > 1 && Array.from(parts.join(" > ")).length > MAX_SELECTOR_CHARS) {
    parts.shift();
  }
  return Array.from(parts.join(" > ")).slice(0, MAX_SELECTOR_CHARS).join("");
}

/** `tag#id.class`, for the hover label. */
function shortName(el: Element): string {
  let name = el.tagName.toLowerCase();
  if (el.id) name += `#${el.id}`;
  const classes = Array.from(el.classList).slice(0, MAX_CLASSES);
  if (classes.length) name += `.${classes.join(".")}`;
  return name;
}

/**
 * Text as one line of visible characters: control and invisible formatting
 * characters become spaces, and runs of whitespace one space.
 */
function oneLine(s: string): string {
  return s
    .replace(/[\p{Cc}\p{Cf}\u2028\u2029]/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/** At most MAX_LABEL_CHARS characters, ending in `…` when cut. */
function shorten(s: string): string {
  const chars = Array.from(s);
  return chars.length <= MAX_LABEL_CHARS
    ? s
    : `${chars.slice(0, MAX_LABEL_CHARS - 1).join("").trimEnd()}…`;
}
