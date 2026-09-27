// Assembles the website into .site/ (site/ plus the app's UI in src/ and the demo pages from
// design/demo/, which the live demo loads) and, unless --build is passed, serves it.
//   pnpm site          build and serve at http://localhost:8080 (the scripted demos for
//                      recording GIFs are at /design/demo/ and /design/demo/accounts.html)
//   pnpm site --build  build only (used by the Pages workflow)
import { cpSync, rmSync, readFileSync, writeFileSync, existsSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize } from "node:path";

const out = ".site";
rmSync(out, { recursive: true, force: true });
cpSync("site", out, { recursive: true });
cpSync("src", join(out, "src"), { recursive: true });
cpSync("design/demo", join(out, "design/demo"), { recursive: true });
// The app's pages, to run in the browser: resolved against src/, with the Tauri API stubbed out
// (design/demo/host.js). The frames pick their window with ?label=.
const shim = `<base href="../../src/" />
  <script src="../design/demo/backend.js"></script>
  <script src="../design/demo/host.js"></script>`;
for (const [page, from] of [["app.html", "index.html"], ["settings.html", "settings.html"]]) {
  const html = readFileSync(join("src", from), "utf8").replace(/<meta charset[^>]*>/, (meta) => `${meta}\n  ${shim}`);
  writeFileSync(join(out, "design/demo", page), html);
}
console.log(`Built ${out}/`);

if (!process.argv.includes("--build")) {
  const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".gif": "image/gif", ".svg": "image/svg+xml", ".json": "application/json" };
  const server = createServer((req, res) => {
    const path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname)).replace(/^([/\\])+/, "");
    let file = join(out, path);
    if (existsSync(file) && statSync(file).isDirectory()) file = join(file, "index.html");
    if (!file.startsWith(out) || !existsSync(file)) {
      res.writeHead(404).end("Not found");
      return;
    }
    res.writeHead(200, { "Content-Type": types[extname(file)] ?? "application/octet-stream" }).end(readFileSync(file));
  });
  // Start at 8080 (or $PORT) and move up if the port is taken.
  let port = Number(process.env.PORT) || 8080;
  server.on("error", (err) => {
    if (err.code !== "EADDRINUSE") throw err;
    port += 1;
    server.listen(port);
  });
  server.on("listening", () => console.log(`Serving at http://localhost:${port}`));
  server.listen(port);
}
