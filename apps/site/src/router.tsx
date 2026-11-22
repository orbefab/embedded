import { createBrowserHistory, createRouter } from "@tanstack/react-router";
import { routeTree } from "./routeTree.gen";

// Pages link with plain anchors, so each page is its own prerendered HTML and nothing asks the
// server for data in the browser, where none runs.
const NOT_FOUND = "/orbe/404/";

// Workers static assets serve the /404 render, as 404.html, for every missing path. The client
// would match the address it was opened at, and that render would not match. This document is
// that render, so hydrate it as /404 and leave the address bar as it is.
function notFoundHistory() {
  if (typeof document === "undefined" || document.querySelector('meta[name="orbe-not-found"]') === null) return undefined;
  return createBrowserHistory({
    parseLocation: () => ({
      href: NOT_FOUND,
      pathname: NOT_FOUND,
      search: "",
      hash: "",
      state: window.history.state,
    }),
  });
}

export function getRouter() {
  const history = notFoundHistory();
  return createRouter({
    routeTree,
    trailingSlash: "always",
    // A hash click fires popstate. When the address did not change, the router treats that as a
    // stale reload and calls the route's server function. This site is prerendered, so that call
    // 404s. Loader data in the HTML stays fresh for the life of the page.
    defaultStaleTime: Number.POSITIVE_INFINITY,
    ...(history === undefined ? {} : { history }),
  });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof getRouter>;
  }
}
