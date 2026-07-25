import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import http from "node:http";
import path from "node:path";

const root = path.resolve("fixtures/out");
http.createServer(async (request, response) => {
  const target = path.resolve(root, `.${new URL(request.url, "http://localhost").pathname}`);
  if (!target.startsWith(root)) {
    response.writeHead(403).end();
    return;
  }
  try {
    const info = await stat(target);
    if (!info.isFile()) throw new Error("not file");
    response.setHeader("content-type", target.endsWith(".html") ? "text/html" : target.endsWith(".css") ? "text/css" : "text/javascript");
    createReadStream(target).pipe(response);
  } catch {
    response.writeHead(404).end();
  }
}).listen(4178, "127.0.0.1");
