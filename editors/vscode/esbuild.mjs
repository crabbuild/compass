import * as esbuild from "esbuild";
import { copyFile, mkdir } from "node:fs/promises";
import process from "node:process";

const watch = process.argv.includes("--watch");
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
    outfile: "dist/extension.js",
    format: "cjs",
    platform: "node",
    external: ["vscode"]
  },
  {
    ...shared,
    entryPoints: {
      graph: "src/webviews/graph.tsx"
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
  await copyFile("../../packages/compass-viewer/dist/viewer.css", "dist/webviews/viewer.css");
}
