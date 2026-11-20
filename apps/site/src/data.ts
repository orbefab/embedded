// The data each route loads. Each is a server function, so it runs where the page renders, at
// build time, and its result is in the page's HTML.
import { notFound } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";
import * as server from "./site.server";

export type { BlockCard, Linked, Listed } from "./site.server";

function found<T>(value: T | null): T {
  if (value === null) throw notFound();
  return value;
}

export const getSite = createServerFn({ method: "GET" }).handler(() => server.site());
export const getHome = createServerFn({ method: "GET" }).handler(() => server.home());
export const getParts = createServerFn({ method: "GET" }).handler(() => server.parts());
export const getBlocks = createServerFn({ method: "GET" }).handler(() => server.blockCards());

export const getPart = createServerFn({ method: "GET" })
  .validator((id: string) => id)
  .handler(({ data }) => found(server.part(data)));

export const getBlock = createServerFn({ method: "GET" })
  .validator((id: string) => id)
  .handler(({ data }) => found(server.block(data)));

export const getBlockPlayer = createServerFn({ method: "GET" })
  .validator((id: string) => id)
  .handler(({ data }) => found(server.blockPlayer(data)));
