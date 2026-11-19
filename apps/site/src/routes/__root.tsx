import { createRootRoute, HeadContent, Outlet, Scripts, useRouterState } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { getSite } from "../data";
import { NotFoundPage } from "../pages/home";
import { Shell } from "../pages/layout";
import { at, sitePath } from "../paths";
import css from "../site.css?url";
import { THEME_INIT } from "../theme";

/** The embed is the player alone, so the home page can frame it. */
const isEmbed = (pathname: string) => /^\/blocks\/[^/]+\/embed\/?$/.test(sitePath(pathname));

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
  component: RootView,
  notFoundComponent: NotFoundPage,
});

function RootView() {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  if (isEmbed(pathname)) return <Outlet />;
  return (
    <Shell site={Route.useLoaderData()}>
      <Outlet />
    </Shell>
  );
}

function Document({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: THEME_INIT }} suppressHydrationWarning />
        <HeadContent />
      </head>
      <body>
        {children}
        <Scripts />
      </body>
    </html>
  );
}
