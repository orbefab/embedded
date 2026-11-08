import { createFileRoute } from "@tanstack/react-router";
import { getPart } from "../../data";
import { head } from "../../head";
import { matches, PartPage } from "../../pages/part";
import { at } from "../../paths";

export const Route = createFileRoute("/parts/$id")({
  loader: ({ params }) => getPart({ data: params.id }),
  head: ({ loaderData, params }) => {
    const record = loaderData?.item.record;
    return head({
      title: record?.title ?? params.id,
      description: record === undefined ? "" : (record.description ?? `${matches(record)}.`),
      path: at.part(params.id),
    });
  },
  component: () => <PartPage part={Route.useLoaderData()} />,
});
