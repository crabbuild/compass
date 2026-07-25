import path from "node:path";

const root = import.meta.dir;
const source = (name) => path.join(root, name);

const result = await Bun.build({
  entrypoints: [source("entry.js")],
  outdir: path.dirname(root),
  naming: "pierre-diffs-v1.2.12.js",
  target: "browser",
  format: "esm",
  minify: true,
  sourcemap: "none",
  plugins: [
    {
      name: "compass-plain-text-diffs",
      setup(build) {
        build.onResolve(
          { filter: /highlighter\/shared_highlighter\.js$/ },
          () => ({ path: source("highlighter.js") })
        );
        build.onResolve(
          { filter: /highlighter\/languages\/areLanguagesAttached\.js$/ },
          () => ({ path: source("languages.js") })
        );
        build.onResolve(
          { filter: /highlighter\/themes\/areThemesAttached\.js$/ },
          () => ({ path: source("themes.js") })
        );
      }
    }
  ]
});

if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}
