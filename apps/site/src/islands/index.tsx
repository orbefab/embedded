import { type ComponentProps, type ComponentType, createElement } from "react";
import { BlockPlayer } from "./block-player";
import { PartModel } from "./part-model";
import { PartSearch } from "./part-search";

/** The parts of a page that run in the browser, by the name the page's HTML gives each. */
export const ISLANDS = {
  "block-player": BlockPlayer,
  "part-model": PartModel,
  "part-search": PartSearch,
} as const;

export type IslandName = keyof typeof ISLANDS;
export type IslandProps<N extends IslandName> = ComponentProps<(typeof ISLANDS)[N]>;

/** An island as the server renders it: its HTML, inside an element that names it and holds
 * its props, which the client hydrates. */
export function Island<N extends IslandName>({ name, props }: { name: N; props: IslandProps<N> }) {
  return (
    <div data-island={name} data-props={JSON.stringify(props)}>
      {createElement(ISLANDS[name] as ComponentType<object>, props)}
    </div>
  );
}
