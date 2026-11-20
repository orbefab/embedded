import { createFileRoute } from "@tanstack/react-router";
import { BlockPlayer } from "../../components/block-player";
import { getBlockPlayer } from "../../data";
import { head } from "../../head";
import { at } from "../../paths";

// A trailing underscore keeps this off the Block page's layout. The path is still
// /blocks/$id/embed. Nothing links here: the home frames it, and the build lists each one.
export const Route = createFileRoute("/blocks/$id_/embed")({
  loader: ({ params }) => getBlockPlayer({ data: params.id }),
  head: ({ loaderData, params }) => {
    const page = head({
      title: loaderData?.title ?? params.id,
      description: loaderData?.description ?? loaderData?.title ?? "",
      path: at.blockEmbed(params.id),
    });
    return { ...page, meta: [...page.meta, { name: "robots", content: "noindex" }] };
  },
  component: EmbedPage,
});

function EmbedPage() {
  const block = Route.useLoaderData();
  return (
    <div className="h-dvh min-w-0 bg-[var(--stage,#1b1d21)]" data-testid="block-embed">
      <BlockPlayer blocks={[{ name: block.name, title: block.title, button: block.button }]} fill registry={at.registry} wasm={at.wasm} />
    </div>
  );
}
