import { createRootRoute, HeadContent, Outlet, Scripts } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { getSite } from "../data";
import { NotFoundPage } from "../pages/home";
import { Shell } from "../pages/layout";
import { at } from "../paths";
import css from "../site.css?url";

export const Route = createRootRoute({
  loader: () => getSite(),
  head: () => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1, viewport-fit=cover" },
      { name: "theme-color", content: "#111111" },
      { property: "og:type", content: "website" },
      { property: "og:site_name", content: "Orbefab" },
      { property: "og:image", content: `https://orbefab.com${at.asset("logo.png")}` },
    ],
    links: [
      { rel: "stylesheet", href: css },
      { rel: "icon", href: at.asset("favicon.ico"), sizes: "any" },
      { rel: "icon", href: at.asset("favicon.svg"), type: "image/svg+xml" },
      { rel: "apple-touch-icon", href: at.asset("apple-touch-icon.png"), sizes: "180x180" },
    ],
  }),
  shellComponent: Document,
  component: () => (
    <Shell site={Route.useLoaderData()}>
      <Outlet />
    </Shell>
  ),
  notFoundComponent: NotFoundPage,
});

function Document({ children }: { children: ReactNode }) {
  return (
    <html className="dark" lang="en">
      <head>
        <HeadContent />
      </head>
      <body>
        {children}
        <Scripts />
      </body>
    </html>
  );
}
