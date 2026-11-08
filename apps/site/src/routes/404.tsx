import { createFileRoute } from "@tanstack/react-router";
import { head } from "../head";
import { NotFoundPage } from "../pages/home";
import { BASE } from "../paths";

// The page Workers static assets serve, as 404.html, for a path that holds no file.
export const Route = createFileRoute("/404")({
  head: () => head({ title: "Not here", description: "Nothing is at this address.", path: `${BASE}404.html` }),
  component: NotFoundPage,
});
