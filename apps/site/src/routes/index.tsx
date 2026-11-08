import { createFileRoute } from "@tanstack/react-router";
import { getHome } from "../data";
import { head } from "../head";
import { HomePage } from "../pages/home";
import { at } from "../paths";
import { useSite } from "../use-site";

export const Route = createFileRoute("/")({
  loader: () => getHome(),
  head: () =>
    head({
      title: "orbe, a hardware simulator for AI agents",
      description: "Describe a circuit as a scene file, flash it with real firmware, and drive it from the command line. Blocks play live in 3D here.",
      path: at.home,
    }),
  component: () => <HomePage home={Route.useLoaderData()} site={useSite()} />,
});
