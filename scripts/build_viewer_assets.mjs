import { createHash } from "node:crypto";
import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";

const npm = process.platform === "win32" ? "npm.cmd" : "npm";
const result = spawnSync(npm, ["run", "build:viewer"], {
  cwd: new URL("..", import.meta.url),
  stdio: "inherit",
  shell: false
});
if (result.status !== 0) process.exit(result.status ?? 1);

const source = new URL("../packages/compass-viewer/dist/", import.meta.url);
const destination = new URL("../crates/compass-output/assets/viewer/", import.meta.url);
await mkdir(destination, { recursive: true });
const files = [
  ["graph.js", "graph.js"],
  ["viewer.css", "viewer.css"]
];
const manifest = {
  schema: "compass.viewer.assets/1",
  viewerSchema: "compass.viewer.graph/1",
  files: {}
};
for (const [input, output] of files) {
  await copyFile(new URL(input, source), new URL(output, destination));
  const bytes = await readFile(new URL(output, destination));
  manifest.files[output] = {
    bytes: bytes.byteLength,
    sha256: createHash("sha256").update(bytes).digest("hex")
  };
}
await writeFile(
  new URL("manifest.json", destination),
  `${JSON.stringify(manifest, null, 2)}\n`
);
