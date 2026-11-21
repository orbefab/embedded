import { useRef, useState } from "react";

/** Copies `text`. The label reads Copied after it works. When the clipboard is missing or refuses
 * the write, the visible text is selected and the label says so. `boxed` is the install line's
 * button, sitting on the command's right edge. */
export function CopyButton({ text, label = "Copy", boxed = false }: { text: string; label?: string; boxed?: boolean }) {
  const [status, setStatus] = useState<"idle" | "copied" | "failed">("idle");
  const timer = useRef<number | undefined>(undefined);
  const failed = status === "failed";
  const copied = status === "copied";
  const mark = (next: "idle" | "copied" | "failed") => {
    window.clearTimeout(timer.current);
    setStatus(next);
    if (next === "copied") timer.current = window.setTimeout(() => setStatus("idle"), 1200);
  };
  return (
    <button
      aria-label={failed ? "Couldn't copy. The text is selected." : undefined}
      className={
        boxed
          ? "shrink-0 border-border border-l px-3 font-mono text-xs hover:text-brand"
          : "font-mono text-muted-foreground text-xs hover:text-brand"
      }
      data-copy-state={copied ? "copied" : failed ? "failed" : "idle"}
      data-testid="copy"
      onClick={(event) => {
        const button = event.currentTarget;
        const fail = () => {
          const root = button.closest("[data-copy-root]");
          const node = root?.querySelector("[data-copy-text]");
          const selection = window.getSelection();
          if (node instanceof HTMLElement && selection !== null) {
            const range = document.createRange();
            range.selectNodeContents(node);
            selection.removeAllRanges();
            selection.addRange(range);
          }
          mark("failed");
        };
        try {
          const clipboard = navigator.clipboard;
          if (clipboard?.writeText === undefined) throw new Error("clipboard unavailable");
          void Promise.resolve(clipboard.writeText(text)).then(
            () => mark("copied"),
            () => fail(),
          );
        } catch {
          fail();
        }
      }}
      type="button"
    >
      {copied ? "Copied" : failed ? "Couldn't copy" : label}
    </button>
  );
}
