import { createFileRoute } from "@tanstack/react-router";
import { getHome } from "../data";
import { head } from "../head";
import { HomePage } from "../pages/home";
import { at } from "../paths";

export const Route = createFileRoute("/")({
  loader: () => getHome(),
  head: () =>
    head({
      title: "Get orbe",
      description: "orbe runs real firmware on simulated Arduino boards, so your agent can test a circuit before you build it.",
      path: at.home,
    }),
  component: () => <HomePage home={Route.useLoaderData()} />,
});
