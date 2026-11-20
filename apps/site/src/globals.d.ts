declare module "virtual:orbe-site" {
  const data: import("./records").SiteData;
  export default data;
}

/** `ORBE_SITE_INSTALL=1` at build time: the binaries are public, so install lines show.
 * vite.config.ts defines it as a literal, so a build without it holds none of their text,
 * in its pages or in its scripts. The footer links the repo either way. */
declare const __ORBE_SITE_INSTALL__: boolean;
