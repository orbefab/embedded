// orbe 0.2.0 fetches /embedded/r/, and the install line fetches /embedded/install.sh.
// Response.redirect drops a trailing slash, so /embedded/ would send people to /orbe.
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
    return env.ASSETS.fetch(request);
  },
};
