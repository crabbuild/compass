import * as esbuild from "esbuild";
import { spawnSync } from "node:child_process";
import { copyFile, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import process from "node:process";

const watch = process.argv.includes("--watch");
const npm = process.platform === "win32" ? "npm.cmd" : "npm";
const assetBuild = spawnSync(
  npm,
  ["run", "build:viewer"],
  {
    cwd: fileURLToPath(new URL("../..", import.meta.url)),
    stdio: "inherit",
    shell: false
  }
);
if (assetBuild.status !== 0) process.exit(assetBuild.status ?? 1);

const shared = {
  bundle: true,
  sourcemap: true,
  logLevel: "info",
  target: "es2022"
};
const builds = [
  {
    ...shared,
    entryPoints: ["src/extension.ts"],
    outfile: "dist/extension.cjs",
    format: "cjs",
    platform: "node",
    external: ["vscode"]
  },
  {
    ...shared,
    entryPoints: ["src/test/runTest.ts"],
    outfile: "dist/test/runTest.cjs",
    format: "cjs",
    platform: "node"
  },
  {
    ...shared,
    entryPoints: ["src/test/suite/index.ts"],
    outfile: "dist/test/suite/index.cjs",
    format: "cjs",
    platform: "node",
    external: ["vscode"]
  },
  {
    ...shared,
    entryPoints: ["src/test/suite/extension.integration.ts"],
    outfile: "dist/test/suite/extension.integration.cjs",
    format: "cjs",
    platform: "node",
    external: ["vscode"]
  },
  {
    ...shared,
    entryPoints: {
      graph: "src/webviews/graph.tsx",
      callGraph: "src/webviews/callGraph.tsx",
      architecture: "src/webviews/architecture.tsx",
      query: "src/webviews/query.tsx",
      history: "src/webviews/history.tsx"
    },
    outdir: "dist/webviews",
    format: "iife",
    platform: "browser"
  }
];

if (watch) {
  const contexts = await Promise.all(builds.map((options) => esbuild.context(options)));
  await Promise.all(contexts.map((context) => context.watch()));
} else {
  await Promise.all(builds.map((options) => esbuild.build(options)));
  await mkdir("dist/webviews", { recursive: true });
  await copyFile(
    "../../packages/compass-viewer/dist/viewer.css",
    "dist/webviews/viewer.css"
  );
}
