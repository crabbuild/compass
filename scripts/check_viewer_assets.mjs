import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

const root = path.resolve(new URL("..", import.meta.url).pathname);
const temporary = await mkdtemp(path.join(os.tmpdir(), "compass-viewer-"));
try {
  const npm = process.platform === "win32" ? "npm.cmd" : "npm";
  const result = spawnSync(npm, ["run", "build:viewer"], {
    cwd: root,
    env: { ...process.env, COMPASS_VIEWER_OUT_DIR: temporary },
    stdio: "inherit",
    shell: false
  });
  if (result.status !== 0) process.exit(result.status ?? 1);
  const manifest = JSON.parse(await readFile(
    path.join(root, "crates/compass-output/assets/viewer/manifest.json"),
    "utf8"
  ));
  for (const [file, expected] of Object.entries(manifest.files)) {
    const built = await readFile(path.join(temporary, file));
    const checked = await readFile(
      path.join(root, "crates/compass-output/assets/viewer", file)
    );
    const digest = createHash("sha256").update(built).digest("hex");
    if (digest !== expected.sha256 || built.byteLength !== expected.bytes) {
      throw new Error(`${file} differs from the checked viewer manifest`);
    }
    if (!built.equals(checked)) {
      throw new Error(`${file} differs from the checked viewer asset`);
    }
    const text = built.toString("utf8");
    for (const forbidden of [
      "https://unpkg.com",
      "https://cdn.jsdelivr.net",
      "src=\"http://",
      "src=\"https://"
    ]) {
      if (text.includes(forbidden)) throw new Error(`${file} contains remote runtime URL ${forbidden}`);
    }
  }
  process.stdout.write("Viewer assets match the deterministic manifest and use no remote runtime resources.\n");
} finally {
  await rm(temporary, { recursive: true, force: true });
}
