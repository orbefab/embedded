import { createFileRoute } from "@tanstack/react-router";
import { getBlock } from "../../data";
import { head } from "../../head";
import { BlockPage } from "../../pages/blocks";
import { at } from "../../paths";
import { useSite } from "../../use-site";

export const Route = createFileRoute("/blocks/$id")({
  loader: ({ params }) => getBlock({ data: params.id }),
  head: ({ loaderData, params }) => {
    const record = loaderData?.item.record;
    return head({ title: record?.title ?? params.id, description: record?.description ?? record?.title ?? "", path: at.block(params.id) });
  },
  component: () => <BlockPage block={Route.useLoaderData()} site={useSite()} />,
});
