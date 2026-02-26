const htmlPath = new URL("./index.html", import.meta.url);
const html = await Bun.file(htmlPath).text();

const port = Number(process.env.PORT ?? 5173);
const controlApiUrl = process.env.CONTROL_API_URL ?? "http://localhost:3000";

Bun.serve({
  port,
  async fetch(request) {
    const url = new URL(request.url);

    if (url.pathname === "/") {
      return new Response(html, {
        headers: {
          "content-type": "text/html; charset=utf-8",
          "cache-control": "no-store",
        },
      });
    }

    if (url.pathname.startsWith("/api/")) {
      const upstreamPath = url.pathname.slice("/api".length) || "/";
      const upstreamUrl = new URL(
        `${upstreamPath}${url.search}`,
        controlApiUrl,
      );
      return fetch(new Request(upstreamUrl, request));
    }

    return new Response("Not Found", { status: 404 });
  },
});

console.log(`dashboard-ui listening on :${port}`);
