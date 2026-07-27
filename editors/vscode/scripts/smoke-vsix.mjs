import { readFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";

const directory = path.resolve(new URL("..", import.meta.url).pathname);
const manifest = JSON.parse(await readFile(path.join(directory, "package.json"), "utf8"));
const vsix = `${manifest.name}-${manifest.version}.vsix`;
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
  "extension/dist/webviews/viewer.css",
  "extension/dist/webviews/callGraph.js",
  "extension/dist/webviews/history.js",
  "extension/media/icon.png"
]) {
  if (!listing.includes(required)) throw new Error(`VSIX is missing ${required}`);
}
const viewerStyles = spawnSync(
  "unzip",
  ["-p", path.join(directory, vsix), "extension/dist/webviews/viewer.css"],
  { encoding: "utf8", shell: false }
);
if (viewerStyles.status !== 0) throw new Error(viewerStyles.stderr);
if (!viewerStyles.stdout.includes(".compass-source-card")) {
  throw new Error("VSIX viewer stylesheet is stale: missing .compass-source-card");
}
if (entries.some((entry) => entry === "extension/compass" || entry === "extension/compass.exe")) {
  throw new Error("VSIX must not bundle the native Compass CLI");
}
process.stdout.write(`VSIX smoke check passed: ${vsix}\n`);
