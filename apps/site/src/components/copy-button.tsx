import { useState } from "react";

/** Copies `text`. The label reads Copied for a moment after it works. `boxed` is the install
 * line's button, sitting on the command's right edge. */
export function CopyButton({ text, label = "Copy", boxed = false }: { text: string; label?: string; boxed?: boolean }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      className={
        boxed
          ? "shrink-0 border-border border-l px-3 font-mono text-xs hover:text-brand"
          : "font-mono text-muted-foreground text-xs hover:text-brand"
      }
      data-testid="copy"
      onClick={() => {
        void navigator.clipboard.writeText(text).then(() => {
          setCopied(true);
          window.setTimeout(() => setCopied(false), 1200);
        });
      }}
      type="button"
    >
      {copied ? "Copied" : label}
    </button>
  );
}
