// orbe 0.2.0 fetches /embedded/r/, and the install line fetches /embedded/install.sh.
// Response.redirect drops a trailing slash, so /embedded/ would send people to /orbe.
// A hashed registry file never changes. Those 200s keep for a year. A path with @ is
// fetched already percent-encoded, so the asset server's 307 to %40 never happens.
const IMMUTABLE = /^\/orbe\/r\/(?:files\/[0-9a-f]{64}|[a-z][a-z0-9-]{0,63}@[0-9]+\.[0-9]+\.[0-9]+\.json)$/;

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/embedded" || url.pathname.startsWith("/embedded/")) {
      const rest = url.pathname.slice("/embedded".length);
      return new Response(null, {
        status: 301,
        headers: { Location: `${url.origin}/orbe${rest}${url.search}` },
      });
    }
    const assetUrl = request.url.replaceAll("@", "%40");
    const response = await env.ASSETS.fetch(new Request(assetUrl, request));
    if (response.status !== 200 || !IMMUTABLE.test(url.pathname)) return response;
    const headers = new Headers(response.headers);
    headers.set("Cache-Control", "public, max-age=31536000, immutable");
    return new Response(response.body, { status: 200, headers });
  },
};
