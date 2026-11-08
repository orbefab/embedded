import { createFileRoute } from "@tanstack/react-router";
import { getParts } from "../../data";
import { head } from "../../head";
import { PartsPage } from "../../pages/parts";
import { at } from "../../paths";

export const Route = createFileRoute("/parts/")({
  loader: () => getParts(),
  head: ({ loaderData }) =>
    head({
      title: "Parts",
      description: `The ${loaderData?.length ?? 0} parts of the registry, each with its pins, ratings, 3D model and files.`,
      path: at.parts,
    }),
  component: () => <PartsPage parts={Route.useLoaderData()} />,
});
