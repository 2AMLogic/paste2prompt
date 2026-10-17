import {
  type LauncherPosition,
  type Paste2PromptOverlay,
  TAG_NAME,
  defineOverlay,
} from "@paste2prompt/element";
import { createElement, forwardRef, useEffect, useImperativeHandle, useRef } from "react";

export type { LauncherPosition } from "@paste2prompt/element";

export interface Paste2PromptProps {
  /** Where the launcher button floats. Default `bottom-right`. */
  position?: LauncherPosition;
  /**
   * Show the floating launcher button. Default `true`. Without it, open the
   * editor through the ref ({@link Paste2PromptHandle.open}).
   */
  launcher?: boolean;
  /** Called once the editor shows. */
  onOpen?: () => void;
  /** Called once the editor is gone. */
  onClose?: () => void;
}

/** What a ref to {@link Paste2Prompt} can do. */
export interface Paste2PromptHandle {
  /** Captures the viewport and opens the editor with it. */
  open(): Promise<void>;
  /** Closes the editor, asking first when there are marks to lose. */
  close(): void;
  /** Whether the editor is showing. */
  readonly isOpen: boolean;
}

/**
 * The paste2prompt overlay: a floating launcher button that captures the
 * page and opens the editor over it. Mount it once, near the root.
 *
 * ```tsx
 * <Paste2Prompt position="bottom-left" />
 * ```
 *
 * Everything stays in the browser: the output goes to the clipboard or a
 * download.
 */
export const Paste2Prompt = forwardRef<Paste2PromptHandle, Paste2PromptProps>(
  function Paste2Prompt({ position, launcher = true, onOpen, onClose }, ref) {
    const el = useRef<Paste2PromptOverlay>(null);

    useEffect(defineOverlay, []);

    useImperativeHandle(
      ref,
      () => ({
        open: () => el.current?.open() ?? Promise.resolve(),
        close: () => el.current?.close(),
        get isOpen() {
          return el.current?.isOpen ?? false;
        },
      }),
      [],
    );

    useEffect(() => {
      const node = el.current;
      if (!node) return;
      const opened = () => onOpen?.();
      const closed = () => onClose?.();
      node.addEventListener("paste2prompt-open", opened);
      node.addEventListener("paste2prompt-close", closed);
      return () => {
        node.removeEventListener("paste2prompt-open", opened);
        node.removeEventListener("paste2prompt-close", closed);
      };
    }, [onOpen, onClose]);

    // Plain string attributes, which React 18 and 19 both set as attributes
    // on a custom element.
    return createElement(TAG_NAME, {
      ref: el,
      position,
      "no-launcher": launcher ? undefined : "",
    });
  },
);
