import { createFileRoute } from "@tanstack/react-router";
import { getBlocks } from "../../data";
import { head } from "../../head";
import { BlocksPage } from "../../pages/blocks";
import { at } from "../../paths";

export const Route = createFileRoute("/blocks/")({
  loader: () => getBlocks(),
  head: () =>
    head({
      title: "Blocks",
      description: "Small circuits that work. Play each one live in your browser.",
      path: at.blocks,
    }),
  component: () => <BlocksPage blocks={Route.useLoaderData()} />,
});
