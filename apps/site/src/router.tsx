import { createRouter } from "@tanstack/react-router";
import { routeTree } from "./routeTree.gen";

// Pages link with plain anchors, so each page is its own prerendered HTML and nothing asks the
// server for data in the browser, where none runs.
export function getRouter() {
  return createRouter({ routeTree, trailingSlash: "always" });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof getRouter>;
  }
}
