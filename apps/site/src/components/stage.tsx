import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";

/** True when this browser can create a WebGL 2 context. The current three.js needs WebGL 2, so a
 * WebGL 1 context is a blank view, not a scene that is still loading. */
function hasWebgl(): boolean {
  try {
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2");
    if (gl === null) return false;
    gl.getExtension("WEBGL_lose_context")?.loseContext();
    return true;
  } catch {
    return false;
  }
}

/** The box a 3D view draws in. The server renders it empty, and the browser fills it once the
 * page hydrates, so the page's text never waits for three.js. With no WebGL the box says so
 * and draws nothing, so the controls never appear over a blank stage. */
export function Stage({
  label,
  children,
  onWebgl,
  fill = false,
}: {
  label: string;
  children: () => ReactNode;
  onWebgl?: (webgl: boolean) => void;
  /** Fill the parent instead of the fixed stage height. The embed route uses this. */
  fill?: boolean;
}) {
  const [webgl, setWebgl] = useState<boolean | null>(null);
  const [drawn, setDrawn] = useState(false);
  const frame = useRef<HTMLDivElement>(null);
  const report = useRef(onWebgl);
  report.current = onWebgl;
  useLayoutEffect(() => {
    const ok = hasWebgl();
    setWebgl(ok);
    report.current?.(ok);
  }, []);
  useEffect(() => {
    if (webgl !== true) return;
    const node = frame.current;
    if (node === null) return;
    const see = () => setDrawn(node.querySelector("canvas") !== null);
    see();
    const observer = new MutationObserver(see);
    observer.observe(node, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, [webgl]);
  return (
    <div
      className={`relative overflow-hidden border border-border bg-[var(--stage,#1b1d21)] ${fill ? "h-full min-h-0" : "h-[min(62vh,560px)]"}`}
    >
      {webgl === false ? (
        <p className="absolute inset-0 grid place-items-center px-6 text-center text-sm" data-testid="no-webgl">
          Your browser can't show 3D here.
        </p>
      ) : (
        <>
          {drawn ? null : (
            <p className="absolute inset-0 grid place-items-center font-mono text-muted-foreground text-xs uppercase tracking-[0.16em]">
              {label}
            </p>
          )}
          {webgl === true ? (
            <div className="absolute inset-0" ref={frame}>
              {children()}
            </div>
          ) : null}
        </>
      )}
    </div>
  );
}
