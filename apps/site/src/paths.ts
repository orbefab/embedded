/** Every URL the site links to, under the one path the site's route serves. */
export const BASE = "/embedded/";

export const at = {
  home: BASE,
  parts: `${BASE}parts/`,
  part: (id: string) => `${BASE}parts/${id}/`,
  partView: (id: string) => `${BASE}parts/${id}/view.json`,
  blocks: `${BASE}blocks/`,
  block: (id: string) => `${BASE}blocks/${id}/`,
  registry: `${BASE}r/`,
  index: `${BASE}r/index.json`,
  file: (sha256: string) => `${BASE}r/files/${sha256}`,
  wasm: `${BASE}orbe-wasm/`,
  asset: (path: string) => `${BASE}${path}`,
  website: "https://orbefab.com/",
  repo: "https://github.com/orbefab/embedded",
} as const;
