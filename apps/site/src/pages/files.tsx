import type { ReactNode } from "react";
import { size } from "../format";
import { at } from "../paths";
import type { FileEntry } from "../records";
import { Table } from "./layout";

/** Every file of a record, each with its size and license, as a download. */
export function Files({ files }: { files: readonly FileEntry[] }) {
  return (
    <Table
      head={["File", "Size", "License"]}
      rows={files.map((file) => [
        <a className="font-mono underline hover:text-brand" download={file.path.split("/").pop()} href={at.file(file.sha256)} key="path">
          {file.path}
        </a>,
        <span className="whitespace-nowrap font-mono text-muted-foreground" key="size">
          {size(file.bytes)}
        </span>,
        <span className="font-mono" key="license">
          {file.license}
        </span>,
      ])}
    />
  );
}

/** The closed license box. The summary always shows the SPDX expression. The hash wraps, and
 * the full value stays in the text. */
export function LicenseBox({
  expression,
  children,
  identity,
  recordHref,
}: {
  expression: string;
  children: ReactNode;
  identity: string;
  recordHref: string;
}) {
  return (
    <details className="mt-14 border border-border" data-testid="license-files" id="files">
      <summary className="flex cursor-pointer flex-wrap items-baseline justify-between gap-x-4 gap-y-1 px-4 py-3">
        <span className="font-mono text-muted-foreground text-xs uppercase tracking-[0.16em]">License and files</span>
        <span className="min-w-0 break-words font-mono text-xs">{expression}</span>
      </summary>
      <div className="grid min-w-0 gap-4 px-4 pb-4">
        {children}
        <p className="font-mono text-muted-foreground text-xs">
          <span className="break-all">{identity}.</span>{" "}
          <a className="underline hover:text-foreground" href={recordHref}>
            Registry record
          </a>
        </p>
      </div>
    </details>
  );
}
