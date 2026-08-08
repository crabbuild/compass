#!/usr/bin/env node

/**
 * Independent TypeScript/JavaScript module-resolution oracle.
 *
 * This is a qualification-only tool. It uses the pinned TypeScript compiler
 * API to resolve source-grounded module occurrences and never reads Compass
 * or Graphify output. Normal Compass execution must not invoke this script.
 */

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import ts from "typescript";

const SCHEMA = "compass.typescript-resolution-oracle/1";
const PROVIDER = "typescript_compiler_api_5_9_3";
const MAX_FILES = 20_000;
const MAX_SOURCE_BYTES = 128 * 1024 * 1024;
const MAX_CONFIGS = 256;
const MAX_CONFIG_BYTES = 2 * 1024 * 1024;
const MAX_MODULE_USES = 500_000;
const MAX_DIAGNOSTICS = 10_000;
const SOURCE_SUFFIXES = [
  ".ts",
  ".tsx",
  ".mts",
  ".cts",
  ".js",
  ".jsx",
  ".mjs",
  ".cjs",
];
const EXCLUDED_DIRECTORIES = new Set([
  ".git",
  ".compass",
  ".next",
  ".nuxt",
  ".turbo",
  "build",
  "coverage",
  "dist",
  "target",
  "node_modules",
]);

function fail(message) {
  console.error(`[typescript-resolution-oracle] ${message}`);
  process.exitCode = 2;
  throw new Error(message);
}

function sha256(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

function parseArguments(argv) {
  if (argv.length !== 2 || argv[0] !== "--root" || !argv[1]) {
    fail("usage: typescript-resolution-oracle.mjs --root PATH");
  }
  const absoluteRoot = path.resolve(argv[1]);
  let root;
  try {
    // TypeScript may realpath package targets (notably on macOS, where /tmp
    // is commonly exposed as /private/tmp). Canonicalize the boundary once so
    // containment checks do not misclassify an in-root target as external.
    root = fs.realpathSync.native(absoluteRoot);
  } catch (error) {
    fail(`source root is unavailable: ${error.message}`);
  }
  let stat;
  try {
    stat = fs.statSync(root);
  } catch (error) {
    fail(`source root is unavailable: ${error.message}`);
  }
  if (!stat.isDirectory()) fail(`source root is not a directory: ${root}`);
  return root;
}

function relativePath(root, fileName) {
  const relative = path.relative(root, fileName).split(path.sep).join("/");
  if (!relative || relative.startsWith("../") || relative === ".." || path.isAbsolute(relative)) {
    fail(`source path escapes root: ${fileName}`);
  }
  return relative;
}

function insideRoot(root, fileName) {
  const absolute = path.resolve(fileName);
  return absolute === root || absolute.startsWith(`${root}${path.sep}`);
}

function isSourceFile(fileName) {
  return SOURCE_SUFFIXES.some((suffix) => fileName.endsWith(suffix));
}

function isConfigFile(fileName) {
  const lower = fileName.toLowerCase();
  return (
    lower === "tsconfig.json" ||
    (lower.startsWith("tsconfig.") && lower.endsWith(".json")) ||
    lower === "jsconfig.json" ||
    (lower.startsWith("jsconfig.") && lower.endsWith(".json"))
  );
}

function collectFiles(root) {
  const sourceFiles = [];
  const configFiles = [];
  const rejectedFiles = [];
  const stack = [root];
  while (stack.length > 0) {
    const directory = stack.pop();
    let entries;
    try {
      entries = fs.readdirSync(directory, { withFileTypes: true });
    } catch (error) {
      fail(`cannot read directory ${relativePath(root, directory)}: ${error.message}`);
    }
    entries.sort((left, right) => left.name.localeCompare(right.name, "en"));
    for (const entry of entries) {
      const fileName = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        if (!EXCLUDED_DIRECTORIES.has(entry.name)) stack.push(fileName);
        continue;
      }
      if (entry.isSymbolicLink()) {
        if (isSourceFile(entry.name)) rejectedFiles.push(relativePath(root, fileName));
        continue;
      }
      if (isConfigFile(entry.name)) configFiles.push(fileName);
      if (isSourceFile(entry.name)) sourceFiles.push(fileName);
      if (sourceFiles.length + configFiles.length > MAX_FILES + MAX_CONFIGS) {
        fail(`source/config file count exceeds ${MAX_FILES + MAX_CONFIGS}`);
      }
    }
  }
  sourceFiles.sort((left, right) => relativePath(root, left).localeCompare(relativePath(root, right), "en"));
  configFiles.sort((left, right) => relativePath(root, left).localeCompare(relativePath(root, right), "en"));
  return { sourceFiles, configFiles, rejectedFiles };
}

function byteOffsets(text) {
  const offsets = new Uint32Array(text.length + 1);
  let byteOffset = 0;
  let index = 0;
  while (index < text.length) {
    const codePoint = text.codePointAt(index);
    const width = codePoint > 0xffff ? 2 : 1;
    offsets[index] = byteOffset;
    if (width === 2) offsets[index + 1] = byteOffset;
    byteOffset += Buffer.byteLength(text.slice(index, index + width), "utf8");
    index += width;
    offsets[index] = byteOffset;
  }
  return offsets;
}

function scriptKind(fileName) {
  return ts.getScriptKindFromFileName(fileName);
}

function diagnosticText(diagnostic) {
  return ts.flattenDiagnosticMessageText(diagnostic.messageText, " ");
}

function moduleResolutionName(value) {
  switch (value) {
    case ts.ModuleResolutionKind.Classic:
      return "classic";
    case ts.ModuleResolutionKind.Node10:
      return "node10";
    case ts.ModuleResolutionKind.Node16:
      return "node16";
    case ts.ModuleResolutionKind.NodeNext:
      return "nodenext";
    case ts.ModuleResolutionKind.Bundler:
      return "bundler";
    default:
      return "unspecified";
  }
}

function moduleKindName(value) {
  switch (value) {
    case ts.ModuleKind.CommonJS:
      return "commonjs";
    case ts.ModuleKind.Node16:
      return "node16";
    case ts.ModuleKind.NodeNext:
      return "nodenext";
    case ts.ModuleKind.Preserve:
      return "preserve";
    case ts.ModuleKind.ES2015:
    case ts.ModuleKind.ES2020:
    case ts.ModuleKind.ES2022:
    case ts.ModuleKind.ESNext:
      return "es";
    default:
      return "unspecified";
  }
}

function extensionName(value) {
  for (const [key, name] of Object.entries(ts.Extension)) {
    if (value === name) return key.toLowerCase();
  }
  return "unknown";
}

function safeRead(root, fileName, limit) {
  const absolute = path.resolve(fileName);
  if (!insideRoot(root, absolute)) return undefined;
  let stat;
  try {
    stat = fs.statSync(absolute);
  } catch {
    return undefined;
  }
  if (!stat.isFile() || stat.size > limit) return undefined;
  try {
    return fs.readFileSync(absolute);
  } catch {
    return undefined;
  }
}

function makeConfigHost(root) {
  const readFile = (fileName) => {
    const raw = safeRead(root, fileName, MAX_CONFIG_BYTES);
    return raw === undefined ? undefined : raw.toString("utf8");
  };
  return {
    useCaseSensitiveFileNames: true,
    onUnRecoverableConfigFileDiagnostic: () => {},
    readDirectory(directory, extensions, excludes, includes, depth) {
      if (!insideRoot(root, directory)) return [];
      return ts.sys.readDirectory(directory, extensions, excludes, includes, depth)
        .filter((fileName) => insideRoot(root, fileName));
    },
    fileExists: (fileName) => safeRead(root, fileName, MAX_CONFIG_BYTES) !== undefined,
    readFile,
  };
}

function parseProjects(root, configFiles, diagnostics) {
  if (configFiles.length > MAX_CONFIGS) {
    fail(`project configuration count exceeds ${MAX_CONFIGS}`);
  }
  const host = makeConfigHost(root);
  const projects = [];
  for (const configFile of configFiles) {
    const raw = safeRead(root, configFile, MAX_CONFIG_BYTES);
    if (raw === undefined) {
      diagnostics.push({ file: relativePath(root, configFile), message: "config is unreadable or exceeds the configured limit" });
      continue;
    }
    const read = ts.readConfigFile(configFile, host.readFile);
    if (read.error) {
      diagnostics.push({ file: relativePath(root, configFile), message: diagnosticText(read.error) });
      continue;
    }
    let parsed;
    try {
      parsed = ts.parseJsonConfigFileContent(
        read.config,
        host,
        path.dirname(configFile),
        {},
        configFile,
      );
    } catch (error) {
      diagnostics.push({ file: relativePath(root, configFile), message: String(error) });
      continue;
    }
    for (const error of parsed.errors ?? []) {
      diagnostics.push({ file: relativePath(root, configFile), message: diagnosticText(error) });
      if (diagnostics.length >= MAX_DIAGNOSTICS) fail(`diagnostic count exceeds ${MAX_DIAGNOSTICS}`);
    }
    const fileNames = [...new Set(parsed.fileNames.map((fileName) => path.resolve(fileName)))]
      .filter((fileName) => insideRoot(root, fileName));
    projects.push({
      configFile,
      directory: path.dirname(configFile),
      options: parsed.options,
      fileNames,
      configDigest: sha256(raw),
      references: (parsed.projectReferences ?? []).map((reference) => path.resolve(reference.sourceFile)),
    });
  }
  projects.sort((left, right) => relativePath(root, left.configFile).localeCompare(relativePath(root, right.configFile), "en"));
  return projects;
}

function syntheticProject(root) {
  return {
    configFile: null,
    directory: root,
    options: {
      allowJs: true,
      module: ts.ModuleKind.CommonJS,
      moduleResolution: ts.ModuleResolutionKind.Node10,
      target: ts.ScriptTarget.Latest,
    },
    fileNames: [],
    configDigest: "",
    references: [],
  };
}

function chooseProject(root, fileName, projects, synthetic) {
  const candidates = projects.filter((project) => project.fileNames.includes(fileName));
  if (candidates.length === 0) return synthetic;
  let deepest = Math.max(...candidates.map((project) => project.directory.split(path.sep).length));
  const nearest = candidates.filter((project) => project.directory.split(path.sep).length === deepest);
  return nearest.length === 1 ? nearest[0] : null;
}

function makeResolutionHost(root) {
  const read = (fileName) => {
    const raw = safeRead(root, fileName, MAX_SOURCE_BYTES);
    return raw === undefined ? undefined : raw.toString("utf8");
  };
  return {
    fileExists: (fileName) => safeRead(root, fileName, MAX_SOURCE_BYTES) !== undefined,
    readFile: read,
    directoryExists: (directory) => {
      const absolute = path.resolve(root, directory);
      return insideRoot(root, absolute) && ts.sys.directoryExists(absolute);
    },
    realpath: (fileName) => {
      const absolute = path.resolve(root, fileName);
      if (!insideRoot(root, absolute)) return fileName;
      try {
        return fs.realpathSync.native(absolute);
      } catch {
        return absolute;
      }
    },
    getCurrentDirectory: () => root,
    getDirectories: (directory) => {
      const absolute = path.resolve(root, directory);
      return insideRoot(root, absolute) ? ts.sys.getDirectories(absolute) : [];
    },
  };
}

function addUse(root, sourceFile, literal, context, kind, typeOnly, uses) {
  if (!literal || typeof literal.text !== "string" || literal.text.length === 0) return;
  const start = literal.getStart(sourceFile, false);
  const end = literal.getEnd();
  const offsets = sourceFile.__byteOffsets;
  if (!offsets || end <= start || end >= offsets.length) return;
  if (uses.length >= MAX_MODULE_USES) fail(`module occurrence count exceeds ${MAX_MODULE_USES}`);
  uses.push({
    sourceFile: relativePath(root, sourceFile.fileName),
    absoluteSourceFile: sourceFile.fileName,
    specifier: literal.text,
    context,
    kind,
    typeOnly: Boolean(typeOnly),
    startByte: offsets[start],
    endByte: offsets[end],
    startLine: sourceFile.getLineAndCharacterOfPosition(start).line + 1,
    literal,
  });
}

function collectModuleUses(root, sourceFile, uses) {
  const visit = (node) => {
    if (ts.isImportDeclaration(node) && node.moduleSpecifier) {
      addUse(root, sourceFile, node.moduleSpecifier, "import", "import", node.importClause?.isTypeOnly, uses);
    } else if (ts.isExportDeclaration(node) && node.moduleSpecifier) {
      addUse(root, sourceFile, node.moduleSpecifier, "import", "export", node.isTypeOnly, uses);
    } else if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference)) {
      addUse(root, sourceFile, node.moduleReference.expression, "import", "import-equals", false, uses);
    } else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)) {
      addUse(root, sourceFile, node.argument.literal, "import", "import-type", true, uses);
    } else if (ts.isCallExpression(node) && node.arguments.length > 0) {
      const argument = node.arguments[0];
      const isLiteral = ts.isStringLiteral(argument) || ts.isNoSubstitutionTemplateLiteral(argument);
      if (isLiteral && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
        addUse(root, sourceFile, argument, "dynamic-import", "dynamic-import", false, uses);
      } else if (
        isLiteral &&
        ts.isIdentifier(node.expression) &&
        node.expression.text === "require"
      ) {
        addUse(root, sourceFile, argument, "require", "require", false, uses);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(sourceFile);
}

function normalizeTarget(root, resolvedFileName) {
  if (!resolvedFileName) return { targetFile: null, resolutionKind: "unresolved" };
  const target = path.resolve(resolvedFileName);
  if (!insideRoot(root, target)) return { targetFile: null, resolutionKind: "external" };
  return {
    targetFile: relativePath(root, target),
    resolutionKind: "source",
  };
}

function resolveUses(root, uses, projectByFile, synthetic, host) {
  const cacheByProject = new Map();
  const resolutions = [];
  for (const use of uses) {
    const project = projectByFile.get(use.absoluteSourceFile) ?? synthetic;
    if (project === null) {
      resolutions.push({ ...use, project: null, moduleResolution: "ambiguous", mode: "unknown", targetFile: null, resolutionKind: "ambiguous" });
      continue;
    }
    const sourceFile = use.sourceFile;
    const parsedSource = use.parsedSourceFile;
    const inferredMode = ts.getModeForUsageLocation(parsedSource, use.literal, project.options);
    const suffix = path.extname(use.absoluteSourceFile).toLowerCase();
    const mode = inferredMode ?? (
      suffix === ".mts" || suffix === ".mjs"
        ? ts.ModuleKind.ESNext
        : suffix === ".cts" || suffix === ".cjs"
          ? ts.ModuleKind.CommonJS
          : undefined
    );
    let cache = cacheByProject.get(project);
    if (!cache) {
      cache = ts.createModuleResolutionCache(root, (value) => value, project.options);
      cacheByProject.set(project, cache);
    }
    const result = ts.resolveModuleName(
      use.specifier,
      use.absoluteSourceFile,
      project.options,
      host,
      cache,
      undefined,
      mode,
    );
    const normalized = normalizeTarget(root, result.resolvedModule?.resolvedFileName);
    resolutions.push({
      sourceFile,
      specifier: use.specifier,
      context: use.context,
      kind: use.kind,
      typeOnly: use.typeOnly,
      startByte: use.startByte,
      endByte: use.endByte,
      startLine: use.startLine,
      project: project.configFile ? relativePath(root, project.configFile) : null,
      moduleResolution: moduleResolutionName(project.options.moduleResolution),
      module: moduleKindName(project.options.module),
      mode: mode === ts.ModuleKind.CommonJS ? "commonjs" : mode === ts.ModuleKind.ESNext ? "es" : "unspecified",
      extension: result.resolvedModule ? extensionName(result.resolvedModule.extension) : null,
      isExternalLibraryImport: Boolean(result.resolvedModule?.isExternalLibraryImport),
      ...normalized,
    });
  }
  resolutions.sort((left, right) => {
    for (const field of ["sourceFile", "startByte", "endByte", "specifier", "context", "kind"]) {
      const a = left[field] ?? "";
      const b = right[field] ?? "";
      if (a < b) return -1;
      if (a > b) return 1;
    }
    return 0;
  });
  return resolutions.map(({ literal, absoluteSourceFile, parsedSourceFile, ...resolution }) => resolution);
}

function main() {
  const root = parseArguments(process.argv.slice(2));
  const { sourceFiles, configFiles, rejectedFiles: initialRejected } = collectFiles(root);
  if (sourceFiles.length > MAX_FILES) fail(`source file count exceeds ${MAX_FILES}`);
  const diagnostics = [];
  const projects = parseProjects(root, configFiles, diagnostics);
  const synthetic = syntheticProject(root);
  const projectByFile = new Map();
  for (const fileName of sourceFiles) {
    projectByFile.set(fileName, chooseProject(root, fileName, projects, synthetic));
  }
  const uses = [];
  const rejectedFiles = new Set(initialRejected);
  const sourceDigest = crypto.createHash("sha256");
  let totalBytes = 0;
  let parsedFiles = 0;
  for (const fileName of sourceFiles) {
    const relative = relativePath(root, fileName);
    const raw = safeRead(root, fileName, MAX_SOURCE_BYTES);
    if (raw === undefined) {
      rejectedFiles.add(relative);
      continue;
    }
    totalBytes += raw.byteLength;
    if (totalBytes > MAX_SOURCE_BYTES) fail(`source byte count exceeds ${MAX_SOURCE_BYTES}`);
    sourceDigest.update(relative).update("\0").update(raw);
    const text = raw.toString("utf8");
    const sourceFile = ts.createSourceFile(fileName, text, ts.ScriptTarget.Latest, true, scriptKind(fileName));
    if (sourceFile.parseDiagnostics?.length > 0) {
      rejectedFiles.add(relative);
      for (const diagnostic of sourceFile.parseDiagnostics) {
        diagnostics.push({ file: relative, message: diagnosticText(diagnostic) });
        if (diagnostics.length >= MAX_DIAGNOSTICS) fail(`diagnostic count exceeds ${MAX_DIAGNOSTICS}`);
      }
      continue;
    }
    sourceFile.__byteOffsets = byteOffsets(text);
    parsedFiles += 1;
    const before = uses.length;
    collectModuleUses(root, sourceFile, uses);
    for (let index = before; index < uses.length; index += 1) {
      uses[index].parsedSourceFile = sourceFile;
    }
  }
  const host = makeResolutionHost(root);
  const resolutions = resolveUses(root, uses, projectByFile, synthetic, host);
  const configDigest = sha256(
    projects
      .filter((project) => project.configFile)
      .map((project) => `${relativePath(root, project.configFile)}\0${project.configDigest}`)
      .join("\n"),
  );
  const script = fs.readFileSync(fileURLToPath(import.meta.url));
  const payload = {
    schema: SCHEMA,
    provider: PROVIDER,
    metadata: {
      compilerVersion: ts.version,
      nodeVersion: process.version,
      scriptSha256: sha256(script),
      platform: `${process.platform}/${process.arch}`,
      configDigest,
      sourceDigest: sourceDigest.digest("hex"),
    },
    scannedFiles: sourceFiles.length,
    parsedFiles,
    rejectedFiles: [...rejectedFiles].sort(),
    projects: projects.map((project) => ({
      configFile: project.configFile ? relativePath(root, project.configFile) : null,
      fileCount: project.fileNames.length,
      moduleResolution: moduleResolutionName(project.options.moduleResolution),
      module: moduleKindName(project.options.module),
      references: project.references
        .filter((reference) => insideRoot(root, reference))
        .map((reference) => relativePath(root, reference))
        .sort(),
    })),
    diagnostics: diagnostics.slice(0, MAX_DIAGNOSTICS),
    resolutions,
  };
  process.stdout.write(`${JSON.stringify(payload)}\n`);
}

try {
  main();
} catch (error) {
  if (process.exitCode !== 2) {
    console.error(`[typescript-resolution-oracle] ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 2;
  }
}
