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
