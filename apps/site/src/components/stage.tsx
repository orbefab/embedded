import { type ReactNode, useLayoutEffect, useRef, useState } from "react";

/** True when this browser can create a WebGL context. A failed probe is a blank view, not a
 * scene that is still loading. */
function hasWebgl(): boolean {
  try {
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2") ?? canvas.getContext("webgl");
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
}: {
  label: string;
  children: () => ReactNode;
  onWebgl?: (webgl: boolean) => void;
}) {
  const [webgl, setWebgl] = useState<boolean | null>(null);
  const report = useRef(onWebgl);
  report.current = onWebgl;
  useLayoutEffect(() => {
    const ok = hasWebgl();
    setWebgl(ok);
    report.current?.(ok);
  }, []);
  return (
    <div className="relative h-[min(62vh,560px)] overflow-hidden rounded-lg border border-border bg-[#1b1d21]">
      {webgl === false ? (
        <p className="absolute inset-0 grid place-items-center px-6 text-center text-sm" data-testid="no-webgl">
          This browser has no WebGL, so this view stays blank.
        </p>
      ) : (
        <>
          <p className="absolute inset-0 grid place-items-center font-mono text-muted-foreground text-xs uppercase tracking-[0.16em]">
            {label}
          </p>
          {webgl === true ? <div className="absolute inset-0">{children()}</div> : null}
        </>
      )}
    </div>
  );
}
