import { BASE } from "./paths";

/** A page's title, description and address, as its head shows them. */
export function head({ title, description, path }: { title: string; description: string; path: string }) {
  const full = `${title} | Orbefab`;
  const url = `https://orbefab.com${path}`;
  return {
    meta: [
      { title: full },
      { name: "description", content: description },
      { property: "og:title", content: full },
      { property: "og:description", content: description },
      { property: "og:url", content: url },
    ],
    links: path.startsWith(BASE) && path.endsWith("/") ? [{ rel: "canonical", href: url }] : [],
  };
}
