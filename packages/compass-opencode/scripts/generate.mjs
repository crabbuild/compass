import { readFile, writeFile } from "node:fs/promises"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

import ts from "typescript"

const root = join(dirname(fileURLToPath(import.meta.url)), "..")
const sourcePath = join(root, "src", "index.ts")
const outputPath = join(root, "src", "index.js")
const source = await readFile(sourcePath, "utf8")
const result = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ESNext,
    newLine: ts.NewLineKind.LineFeed,
    target: ts.ScriptTarget.ES2022,
  },
  fileName: sourcePath,
  reportDiagnostics: true,
})
const errors = (result.diagnostics ?? []).filter(
  (diagnostic) => diagnostic.category === ts.DiagnosticCategory.Error,
)
if (errors.length > 0) {
  const host = {
    getCanonicalFileName: (name) => name,
    getCurrentDirectory: () => root,
    getNewLine: () => "\n",
  }
  throw new Error(ts.formatDiagnostics(errors, host))
}
const generated = result.outputText.replaceAll("\r\n", "\n")
if (process.argv.includes("--check")) {
  const current = await readFile(outputPath, "utf8").catch(() => "")
  if (current !== generated) {
    throw new Error("src/index.js is stale; run npm run build -w @compass/opencode-plugin")
  }
} else {
  await writeFile(outputPath, generated, "utf8")
}
