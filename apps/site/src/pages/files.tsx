import type { ReactNode } from "react";
import { size } from "../format";
import { at } from "../paths";
import type { FileEntry } from "../records";
import { Table } from "./layout";

/** Every file of a record, each as a download. The license column stays when a file's license
 * differs from the box's expression. */
export function Files({ files, expression }: { files: readonly FileEntry[]; expression: string }) {
  const showLicense = files.some((file) => file.license !== expression);
  return (
    <Table
      head={showLicense ? ["File", "Size", "License"] : ["File", "Size"]}
      rows={files.map((file) => {
        const cells = [
          <a className="font-mono underline hover:text-brand" download={file.path.split("/").pop()} href={at.file(file.sha256)} key="path">
            {file.path}
          </a>,
          <span className="whitespace-nowrap font-mono text-muted-foreground" key="size">
            {size(file.bytes)}
          </span>,
        ];
        if (showLicense) {
          cells.push(
            <span className="font-mono" key="license">
              {file.license}
            </span>,
          );
        }
        return cells;
      })}
    />
  );
}

/** The closed license box. Parts show the SPDX expression in the summary. Blocks pass a shorter
 * summary and show the full SPDX expression once opened. */
export function LicenseBox({ expression, summary, children }: { expression: string; summary?: string; children: ReactNode }) {
  const heading = summary ?? expression;
  const showFull = heading !== expression;
  return (
    <details className="mt-14 border border-border" data-testid="license-files" id="files">
      <summary className="flex cursor-pointer flex-wrap items-baseline justify-between gap-x-4 gap-y-1 px-4 py-3">
        <span className="font-mono text-muted-foreground text-xs uppercase tracking-[0.16em]">License and files</span>
        <span className="min-w-0 break-words font-mono text-xs">{heading}</span>
      </summary>
      <div className="grid min-w-0 gap-4 px-4 pb-4">
        {showFull ? (
          <p className="min-w-0 break-words font-mono text-muted-foreground text-xs" data-testid="license-expression">
            {expression}
          </p>
        ) : null}
        {children}
      </div>
    </details>
  );
}
