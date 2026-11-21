/** Every URL the site links to, under the one path the site's route serves. */
export const BASE = "/embedded/";

/** The router pathname, whether or not it still has the `/embedded` base on it. */
export function sitePath(pathname: string): string {
  if (pathname === "/embedded") return "/";
  if (pathname.startsWith("/embedded/")) return pathname.slice("/embedded".length);
  return pathname;
}

export const at = {
  home: BASE,
  parts: `${BASE}parts/`,
  part: (id: string) => `${BASE}parts/${id}/`,
  partView: (id: string) => `${BASE}parts/${id}/view.json`,
  blocks: `${BASE}blocks/`,
  block: (id: string) => `${BASE}blocks/${id}/`,
  blockEmbed: (id: string) => `${BASE}blocks/${id}/embed/`,
  registry: `${BASE}r/`,
  index: `${BASE}r/index.json`,
  file: (sha256: string) => `${BASE}r/files/${sha256}`,
  wasm: `${BASE}orbe-wasm/`,
  asset: (path: string) => `${BASE}${path}`,
  website: "https://orbefab.com/",
} as const;
