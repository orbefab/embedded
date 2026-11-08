import { getRouteApi } from "@tanstack/react-router";

const root = getRouteApi("__root__");

/** What the build says of itself, which the root route loads for every page. */
export const useSite = () => root.useLoaderData();
