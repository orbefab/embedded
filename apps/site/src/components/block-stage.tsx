import { useEffect, useState, type ReactNode } from "react";
import type { SourceFile } from "../circuit";
import { size } from "../format";
import { at } from "../paths";
import { CopyButton } from "./copy-button";

/** Preview shows the live player. Code shows the sketch, scene and test. Both are in the HTML,
 * so a reader that does not run the toggle still has the files. A link to `#files` opens the
 * license box. */
export function BlockStage({
  sources,
  install,
  children,
}: {
  sources: readonly SourceFile[];
  install: ReactNode;
  children: ReactNode;
}) {
  const [view, setView] = useState<"preview" | "code">("preview");
  const [path, setPath] = useState(sources[0]?.path ?? "");
  useEffect(() => {
    const open = () => {
      if (window.location.hash === "#files") document.getElementById("files")?.setAttribute("open", "");
    };
    const onClick = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (target.closest("a[href='#files']") === null) return;
      document.getElementById("files")?.setAttribute("open", "");
    };
    open();
    window.addEventListener("hashchange", open);
    document.addEventListener("click", onClick);
    return () => {
      window.removeEventListener("hashchange", open);
      document.removeEventListener("click", onClick);
    };
  }, []);
  const groups: SourceFile["group"][] = [];
  for (const source of sources) if (!groups.includes(source.group)) groups.push(source.group);
  return (
    <div className="mt-8">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="inline-flex self-start rounded-lg border border-border p-0.5" role="group" aria-label="View">
          {(
            [
              ["preview", "Preview"],
              ["code", "Code"],
            ] as const
          ).map(([name, label]) => (
            <button
              aria-pressed={view === name}
              className="rounded-md px-3 py-1.5 text-sm text-muted-foreground aria-pressed:bg-muted aria-pressed:text-foreground"
              data-testid={`view-${name}`}
              key={name}
              onClick={() => setView(name)}
              type="button"
            >
              {label}
            </button>
          ))}
        </div>
        <div className="min-w-0 sm:max-w-md sm:flex-1">{install}</div>
      </div>
      <div className="mt-3">
        <div data-testid="view-preview-panel" hidden={view !== "preview"}>
          {children}
          <p className="mt-3 text-muted-foreground text-sm">The real engine runs the firmware in your browser. Use the controls in the view.</p>
        </div>
        <div data-testid="view-code-panel" hidden={view !== "code"}>
          {sources.length === 0 ? (
            <p className="text-muted-foreground">No files yet.</p>
          ) : (
            <div className="grid min-w-0 overflow-hidden rounded-lg border border-border md:grid-cols-[13rem_minmax(0,1fr)]">
              <nav className="flex gap-1 overflow-x-auto border-border border-b p-2 md:block md:overflow-visible md:border-r md:border-b-0">
                {groups.map((group) => (
                  <div key={group}>
                    <p className="hidden px-2 pt-2 font-mono text-[0.65rem] text-muted-foreground uppercase tracking-[0.14em] md:block">{group}</p>
                    {sources
                      .filter((source) => source.group === group)
                      .map((source) => (
                        <button
                          aria-pressed={source.path === path}
                          className="shrink-0 rounded px-2 py-1 text-left font-mono text-muted-foreground text-xs aria-pressed:bg-muted aria-pressed:text-foreground md:block md:w-full"
                          data-testid="code-file-tab"
                          key={source.path}
                          onClick={() => setPath(source.path)}
                          type="button"
                        >
                          {source.path.split("/").pop()}
                        </button>
                      ))}
                  </div>
                ))}
              </nav>
              <div className="min-w-0">
                {sources.map((source) => (
                  <div data-path={source.path} data-testid="code-file" hidden={source.path !== path} key={source.path}>
                    <div className="flex flex-wrap items-center justify-between gap-2 border-border border-b px-3 py-2 font-mono text-xs">
                      <span className="min-w-0">
                        <span className="break-all">{source.path}</span>{" "}
                        <span className="text-muted-foreground">
                          {size(source.bytes)}, {source.license}
                        </span>
                      </span>
                      <span className="flex items-center gap-3">
                        <CopyButton text={source.text} />
                        <a className="text-muted-foreground hover:text-brand" href={at.file(source.sha256)}>
                          Raw
                        </a>
                      </span>
                    </div>
                    <pre className="max-h-[34rem] max-w-full overflow-auto p-4 font-mono text-xs leading-relaxed">{source.text}</pre>
                  </div>
                ))}
              </div>
            </div>
          )}
          <p className="mt-3 text-muted-foreground text-sm">
            {__ORBE_SITE_INSTALL__ ? (
              <>
                Sketch, scene and test are what <code className="font-mono">orbe add</code> writes. The firmware image and its licenses are under{" "}
                <a className="underline hover:text-foreground" href="#files">
                  License and files
                </a>
                .
              </>
            ) : (
              <>
                Sketch, scene and test. The firmware image and its licenses are under{" "}
                <a className="underline hover:text-foreground" href="#files">
                  License and files
                </a>
                .
              </>
            )}
          </p>
        </div>
      </div>
    </div>
  );
}
