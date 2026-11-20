import { createFileRoute } from "@tanstack/react-router";
import { getPart } from "../../data";
import { head } from "../../head";
import { PartPage } from "../../pages/part";
import { at } from "../../paths";

export const Route = createFileRoute("/parts/$id")({
  loader: ({ params }) => getPart({ data: params.id }),
  head: ({ loaderData, params }) => {
    const record = loaderData?.item.record;
    return head({
      title: record?.title ?? params.id,
      description: record?.description ?? "",
      path: at.part(params.id),
    });
  },
  component: () => <PartPage part={Route.useLoaderData()} />,
});
