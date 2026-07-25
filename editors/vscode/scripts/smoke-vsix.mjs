import { readdir } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";

const directory = path.resolve(new URL("..", import.meta.url).pathname);
const vsix = (await readdir(directory)).find((file) => file.endsWith(".vsix"));
if (!vsix) throw new Error("No packaged VSIX found");
const unzip = spawnSync("unzip", ["-l", path.join(directory, vsix)], {
  encoding: "utf8",
  shell: false
});
if (unzip.status !== 0) throw new Error(unzip.stderr);
const listing = unzip.stdout;
const entries = listing.split(/\r?\n/).map((line) => line.trim().split(/\s+/).at(-1));
for (const required of [
  "extension/dist/extension.cjs",
  "extension/dist/webviews/graph.js",
  "extension/dist/webviews/callGraph.js",
  "extension/dist/webviews/history.js",
  "extension/media/icon.png"
]) {
  if (!listing.includes(required)) throw new Error(`VSIX is missing ${required}`);
}
if (entries.some((entry) => entry === "extension/compass" || entry === "extension/compass.exe")) {
  throw new Error("VSIX must not bundle the native Compass CLI");
}
process.stdout.write(`VSIX smoke check passed: ${vsix}\n`);
