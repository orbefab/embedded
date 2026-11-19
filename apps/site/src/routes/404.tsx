import { createFileRoute } from "@tanstack/react-router";
import { head } from "../head";
import { NotFoundPage } from "../pages/home";
import { BASE } from "../paths";

// The page Workers static assets serve, as 404.html, for a path that holds no file. The
// orbe-not-found marker tells the client this document is that render. See router.tsx.
export const Route = createFileRoute("/404")({
  head: () => {
    const page = head({ title: "Not found", description: "Nothing moves here.", path: `${BASE}404.html` });
    return { ...page, meta: [...page.meta, { name: "orbe-not-found", content: "1" }] };
  },
  component: NotFoundPage,
});
