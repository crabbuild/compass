#!/usr/bin/env node

/**
 * Independent TypeScript/JavaScript source oracle for Compass qualification.
 *
 * This is deliberately a developer-side oracle, not a Compass runtime
 * dependency. It uses the pinned TypeScript compiler API to parse a bounded
 * source tree and emits source-grounded constructs. It does not read a graph
 * produced by Compass or Graphify and never resolves a target by name.
 */

import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import ts from "typescript";

const SCHEMA = "compass.typescript-source-oracle/1";
const JSONL_SCHEMA = "compass.typescript-source-oracle-jsonl/1";
const PROVIDER = "typescript_compiler_api_5_9_3";
const MAX_FILES = 20_000;
const MAX_SOURCE_BYTES = 128 * 1024 * 1024;
const MAX_CONSTRUCTS = 500_000;
const MAX_CONFIGS = 256;
const MAX_CONFIG_BYTES = 2 * 1024 * 1024;
const MAX_DIAGNOSTICS = 10_000;
const MAX_PROJECT_FILE_REFERENCES = 100_000;
const MAX_PROJECT_DEPTH = 32;
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
  "node_modules",
  "target",
]);

function fail(message) {
  console.error(`[typescript-source-oracle] ${message}`);
  process.exitCode = 2;
  throw new Error(message);
}

function sha256(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

function parseArguments(argv) {
  if (
    (argv.length !== 2 && argv.length !== 3) ||
    argv[0] !== "--root" ||
    !argv[1] ||
    (argv.length === 3 && argv[2] !== "--jsonl")
  ) {
    fail("usage: typescript-source-oracle.mjs --root PATH [--jsonl]");
  }
  let root;
  let stat;
  try {
    root = fs.realpathSync.native(path.resolve(argv[1]));
    stat = fs.statSync(root);
  } catch (error) {
    fail(`source root is unavailable: ${error.message}`);
  }
  if (!stat.isDirectory()) {
    fail(`source root is not a directory: ${root}`);
  }
  return { root, jsonl: argv.length === 3 };
}

function relativePath(root, fileName) {
  const relative = path.relative(root, fileName).split(path.sep).join("/");
  if (!relative || relative.startsWith("../") || relative === ".." || path.isAbsolute(relative)) {
    fail(`source path escapes root: ${fileName}`);
  }
  return relative;
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

function isExcludedPath(root, fileName) {
  const relative = path.relative(root, path.resolve(fileName));
  if (!relative || relative === ".") return false;
  return relative.split(path.sep).some((part) => EXCLUDED_DIRECTORIES.has(part));
}

function collectFiles(root) {
  const files = [];
  const configs = [];
  const rejected = [];
  const stack = [root];
  while (stack.length > 0) {
    const directory = stack.pop();
    let entries;
    try {
      entries = fs.readdirSync(directory, { withFileTypes: true });
    } catch (error) {
      fail(`cannot read source directory ${relativePath(root, directory)}: ${error.message}`);
    }
    entries.sort((left, right) => left.name.localeCompare(right.name, "en"));
    for (const entry of entries) {
      if (entry.isDirectory()) {
        if (!EXCLUDED_DIRECTORIES.has(entry.name)) {
          stack.push(path.join(directory, entry.name));
        }
        continue;
      }
      const fileName = path.join(directory, entry.name);
      if (entry.isSymbolicLink()) {
        if (isSourceFile(entry.name)) rejected.push(relativePath(root, fileName));
        continue;
      }
      if (isConfigFile(entry.name)) configs.push(fileName);
      if (!isSourceFile(entry.name)) continue;
      const relative = relativePath(root, fileName);
      files.push(fileName);
      if (files.length + configs.length > MAX_FILES + MAX_CONFIGS) {
        fail(`source file count exceeds ${MAX_FILES}`);
      }
    }
  }
  files.sort((left, right) => relativePath(root, left).localeCompare(relativePath(root, right), "en"));
  configs.sort((left, right) => relativePath(root, left).localeCompare(relativePath(root, right), "en"));
  rejected.sort();
  return { files, configs, rejected };
}

function diagnosticText(diagnostic) {
  return ts.flattenDiagnosticMessageText(diagnostic.messageText, " ");
}

function insideRoot(root, fileName) {
  const absolute = path.resolve(fileName);
  return absolute === root || absolute.startsWith(`${root}${path.sep}`);
}

function safeRead(root, fileName, limit) {
  const absolute = path.resolve(fileName);
  if (!insideRoot(root, absolute) || isExcludedPath(root, absolute)) return undefined;
  let stat;
  try {
    stat = fs.lstatSync(absolute);
  } catch {
    return undefined;
  }
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > limit) return undefined;
  try {
    return fs.readFileSync(absolute);
  } catch {
    return undefined;
  }
}

function safeFileExists(root, fileName) {
  const absolute = path.resolve(fileName);
  if (!insideRoot(root, absolute) || isExcludedPath(root, absolute)) return false;
  try {
    const stat = fs.lstatSync(absolute);
    return stat.isFile() && !stat.isSymbolicLink();
  } catch {
    return false;
  }
}

function isBaseConfiguration(configFile, config) {
  const basename = path.basename(configFile).toLowerCase();
  if (basename === "tsconfig.json" || basename === "jsconfig.json") return false;
  if (!config || typeof config !== "object" || Array.isArray(config)) return false;
  return (
    !("files" in config) &&
    !("include" in config) &&
    !("references" in config)
  );
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
      if (!insideRoot(root, directory) || isExcludedPath(root, directory)) return [];
      return ts.sys
        .readDirectory(directory, extensions, excludes, includes, depth)
        .filter((fileName) => insideRoot(root, fileName) && !isExcludedPath(root, fileName));
    },
    fileExists: (fileName) => safeFileExists(root, fileName),
    readFile,
  };
}

function projectReferenceConfig(root, reference, baseDirectory) {
  if (typeof reference !== "string" || reference.length === 0) return null;
  let candidate = path.resolve(baseDirectory, reference);
  if (!insideRoot(root, candidate) || isExcludedPath(root, candidate)) return null;
  try {
    if (fs.statSync(candidate).isDirectory()) candidate = path.join(candidate, "tsconfig.json");
  } catch {
    return null;
  }
  return isConfigFile(path.basename(candidate)) ? candidate : null;
}

function parseProjects(root, discoveredConfigs, diagnostics) {
  if (discoveredConfigs.length > MAX_CONFIGS) {
    fail(`project configuration count exceeds ${MAX_CONFIGS}`);
  }
  const host = makeConfigHost(root);
  const queue = discoveredConfigs.map((configFile) => ({ configFile, depth: 0 }));
  const queued = new Set(discoveredConfigs);
  const projects = [];
  let projectFileReferences = 0;
  while (queue.length > 0) {
    const item = queue.shift();
    const configFile = item.configFile;
    const raw = safeRead(root, configFile, MAX_CONFIG_BYTES);
    if (raw === undefined) {
      diagnostics.push({
        file: relativePath(root, configFile),
        message: "config is unreadable or exceeds the configured limit",
      });
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
    const baseConfiguration = isBaseConfiguration(configFile, read.config);
    for (const error of parsed.errors ?? []) {
      diagnostics.push({ file: relativePath(root, configFile), message: diagnosticText(error) });
      if (diagnostics.length >= MAX_DIAGNOSTICS) {
        fail(`diagnostic count exceeds ${MAX_DIAGNOSTICS}`);
      }
    }
    if (baseConfiguration) continue;
    const fileNames = [...new Set(parsed.fileNames.map((fileName) => path.resolve(fileName)))]
      .filter((fileName) => insideRoot(root, fileName) && !isExcludedPath(root, fileName) && isSourceFile(fileName))
      .sort((left, right) => relativePath(root, left).localeCompare(relativePath(root, right), "en"));
    projectFileReferences += fileNames.length;
    if (projectFileReferences > MAX_PROJECT_FILE_REFERENCES) {
      fail(`project file references exceed ${MAX_PROJECT_FILE_REFERENCES}`);
    }
    projects.push({
      configFile,
      directory: path.dirname(configFile),
      fileNames,
      configDigest: sha256(raw),
      references: (parsed.projectReferences ?? [])
        .map((reference) =>
          projectReferenceConfig(
            root,
            reference.path ?? reference.sourceFile,
            path.dirname(configFile),
          ),
        )
        .filter((reference) => reference !== null),
    });
    for (const reference of projects.at(-1).references) {
      if (queued.has(reference)) continue;
      queued.add(reference);
      if (item.depth + 1 > MAX_PROJECT_DEPTH) {
        fail(`project reference depth exceeds ${MAX_PROJECT_DEPTH}`);
      }
      queue.push({ configFile: reference, depth: item.depth + 1 });
      if (queued.size > MAX_CONFIGS) fail(`project configuration count exceeds ${MAX_CONFIGS}`);
    }
  }
  projects.sort((left, right) => relativePath(root, left.configFile).localeCompare(relativePath(root, right.configFile), "en"));
  return projects;
}

function byteOffsets(text) {
  const offsets = new Uint32Array(text.length + 1);
  let byteOffset = 0;
  let index = 0;
  while (index < text.length) {
    const codePoint = text.codePointAt(index);
    const width = codePoint > 0xffff ? 2 : 1;
    offsets[index] = byteOffset;
    if (width === 2) {
      offsets[index + 1] = byteOffset;
    }
    byteOffset += Buffer.byteLength(text.slice(index, index + width), "utf8");
    index += width;
    offsets[index] = byteOffset;
  }
  return offsets;
}

function moduleName(root, fileName) {
  let relative = relativePath(root, fileName);
  for (const suffix of SOURCE_SUFFIXES.sort((left, right) => right.length - left.length)) {
    if (relative.endsWith(suffix)) {
      relative = relative.slice(0, -suffix.length);
      break;
    }
  }
  return relative.split("/").filter(Boolean).join(".") || "<root>";
}

function nodeText(sourceFile, node) {
  return node ? node.getText(sourceFile).trim() : "";
}

function identifierText(sourceFile, node) {
  if (!node) return "";
  if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) return node.text;
  if (ts.isStringLiteral(node) || ts.isNumericLiteral(node)) return node.text;
  return nodeText(sourceFile, node);
}

function declarationName(sourceFile, node) {
  if (!node || !node.name) return null;
  if (ts.isIdentifier(node.name) || ts.isPrivateIdentifier(node.name)) return node.name.text;
  if (ts.isStringLiteral(node.name) || ts.isNumericLiteral(node.name)) return node.name.text;
  return null;
}

function isOwnerNode(node) {
  return (
    ts.isClassLike(node) ||
    ts.isFunctionLike(node) ||
    ts.isMethodDeclaration(node) ||
    ts.isMethodSignature(node) ||
    ts.isConstructorDeclaration(node) ||
    ts.isGetAccessorDeclaration(node) ||
    ts.isSetAccessorDeclaration(node) ||
    ts.isInterfaceDeclaration(node) ||
    ts.isTypeAliasDeclaration(node) ||
    ts.isEnumDeclaration(node) ||
    ts.isModuleDeclaration(node)
  );
}

function ownerName(root, sourceFile, node) {
  const enclosing = [];
  let current = node.parent;
  while (current && !ts.isSourceFile(current)) {
    if (isOwnerNode(current)) {
      const name = declarationName(sourceFile, current);
      if (name) enclosing.push(name);
    }
    current = current.parent;
  }
  return [moduleName(root, sourceFile.fileName), ...enclosing.reverse()].join(".");
}

function targetNode(node) {
  if (!node) return null;
  if (ts.isPropertyAccessExpression(node)) return node.name;
  if (ts.isElementAccessExpression(node) && node.argumentExpression) {
    if (ts.isStringLiteral(node.argumentExpression) || ts.isNumericLiteral(node.argumentExpression)) {
      return node.argumentExpression;
    }
  }
  if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) return node;
  return node;
}

function targetQualifier(sourceFile, node) {
  if (ts.isPropertyAccessExpression(node)) return node.expression;
  if (ts.isElementAccessExpression(node) && node.expression) return node.expression;
  return null;
}

function isDeclarationName(node) {
  const parent = node.parent;
  if (!parent) return false;
  if (parent.name === node) return true;
  if (ts.isImportSpecifier(parent) && (parent.name === node || parent.propertyName === node)) return true;
  if (ts.isExportSpecifier(parent) && (parent.name === node || parent.propertyName === node)) return true;
  if (ts.isNamespaceImport(parent) || ts.isImportClause(parent)) return true;
  if (ts.isQualifiedName(parent) || ts.isPropertyAccessExpression(parent)) return false;
  return false;
}

function scriptKind(fileName) {
  return ts.getScriptKindFromFileName(fileName);
}

function parseFile(root, fileName, raw, constructs, diagnostics) {
  const text = raw.toString("utf8");
  const sourceFile = ts.createSourceFile(
    fileName,
    text,
    ts.ScriptTarget.Latest,
    true,
    scriptKind(fileName),
  );
  if (sourceFile.parseDiagnostics && sourceFile.parseDiagnostics.length > 0) {
    for (const diagnostic of sourceFile.parseDiagnostics) {
      diagnostics.push({
        file: relativePath(root, fileName),
        message: diagnosticText(diagnostic),
      });
      if (diagnostics.length >= MAX_DIAGNOSTICS) {
        fail(`diagnostic count exceeds ${MAX_DIAGNOSTICS}`);
      }
    }
    return false;
  }
  const offsets = byteOffsets(text);
  const add = (
    relation,
    capability,
    node,
    qualifierNode = null,
    spelling = null,
    ownerNode = node,
  ) => {
    const target = targetNode(node);
    if (!target) return;
    const start = target.getStart(sourceFile, false);
    const end = target.getEnd();
    if (!(end > start) || end > offsets.length - 1) return;
    const targetSpelling = spelling ?? identifierText(sourceFile, target);
    if (!targetSpelling) return;
    if (constructs.length >= MAX_CONSTRUCTS) {
      fail(`construct count exceeds ${MAX_CONSTRUCTS}`);
    }
    constructs.push({
      sourceFile: relativePath(root, fileName),
      relation,
      capability,
      ownerQualifiedName: ownerName(root, sourceFile, ownerNode),
      targetSpelling,
      qualifier: qualifierNode ? nodeText(sourceFile, qualifierNode) || null : null,
      startByte: offsets[start],
      endByte: offsets[end],
      startLine: sourceFile.getLineAndCharacterOfPosition(start).line + 1,
    });
  };

  const visit = (node) => {
    if (ts.isImportDeclaration(node) && node.moduleSpecifier) {
      add("imports", "imports", node.moduleSpecifier, null, node.moduleSpecifier.text);
      if (node.importClause?.namedBindings && ts.isNamedImports(node.importClause.namedBindings)) {
        for (const element of node.importClause.namedBindings.elements) {
          add("imports", "imports", element.name, node.moduleSpecifier, identifierText(sourceFile, element.propertyName ?? element.name));
        }
      }
      if (node.importClause?.name) {
        add("imports", "imports", node.importClause.name, node.moduleSpecifier, "default");
      }
    } else if (ts.isExportDeclaration(node) && node.moduleSpecifier) {
      add("reexports", "reexports", node.moduleSpecifier, null, node.moduleSpecifier.text);
      if (node.exportClause && ts.isNamedExports(node.exportClause)) {
        for (const element of node.exportClause.elements) {
          add("reexports", "reexports", element.name, node.moduleSpecifier, identifierText(sourceFile, element.propertyName ?? element.name));
        }
      }
    } else if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference)) {
      add("imports", "imports", node.moduleReference.expression, null, node.moduleReference.expression.text);
    } else if (ts.isCallExpression(node)) {
      add("calls", "calls", node.expression, targetQualifier(sourceFile, node.expression));
    } else if (ts.isNewExpression(node)) {
      add("instantiates", "construction", node.expression, targetQualifier(sourceFile, node.expression));
    } else if (ts.isPropertyAccessExpression(node)) {
      add("accesses", "members", node.name, node.expression);
    } else if (ts.isElementAccessExpression(node) && node.argumentExpression && (ts.isStringLiteral(node.argumentExpression) || ts.isNumericLiteral(node.argumentExpression))) {
      add("accesses", "members", node.argumentExpression, node.expression, node.argumentExpression.text);
    } else if (ts.isHeritageClause(node)) {
      const relation = node.token === ts.SyntaxKind.ExtendsKeyword ? "extends" : "implements";
      for (const type of node.types) add(relation, "base_types", type.expression, null);
    } else if (ts.isTypeReferenceNode(node)) {
      // The parser represents `as const` as a TypeReferenceNode, but `const`
      // is an assertion keyword there rather than a named type reference.
      // Keep the source oracle aligned with the semantic construct being
      // measured so the candidate is not penalized for correctly omitting it.
      if (
        !(ts.isAsExpression(node.parent) &&
          node.parent.type === node &&
          node.typeName.getText(sourceFile) === "const")
      ) {
        add("references", "type_references", node.typeName, null);
      }
    } else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument) && ts.isStringLiteral(node.argument.literal)) {
      add("imports", "imports", node.argument.literal, null, node.argument.literal.text);
    } else if (ts.isJsxOpeningLikeElement(node)) {
      add("references", "jsx", node.tagName, null);
    } else if (ts.isIdentifier(node) && !isDeclarationName(node)) {
      const parent = node.parent;
      if (
        !ts.isCallExpression(parent) &&
        !ts.isNewExpression(parent) &&
        !ts.isTypeReferenceNode(parent) &&
        !ts.isExpressionWithTypeArguments(parent) &&
        !ts.isJsxOpeningLikeElement(parent) &&
        !ts.isJsxClosingElement(parent) &&
        !(ts.isPropertyAccessExpression(parent) && parent.name === node) &&
        !(ts.isImportSpecifier(parent) || ts.isExportSpecifier(parent))
      ) {
        add("references", "references", node);
      }
    }

    if (
      (ts.isFunctionDeclaration(node) ||
        ts.isClassDeclaration(node) ||
        ts.isInterfaceDeclaration(node) ||
        ts.isTypeAliasDeclaration(node) ||
        ts.isEnumDeclaration(node) ||
        ts.isModuleDeclaration(node) ||
        ts.isMethodDeclaration(node) ||
        ts.isMethodSignature(node) ||
        ts.isConstructorDeclaration(node) ||
        ts.isGetAccessorDeclaration(node) ||
        ts.isSetAccessorDeclaration(node) ||
        ts.isPropertyDeclaration(node) ||
        ts.isPropertySignature(node) ||
        ts.isParameter(node) ||
        ts.isTypeParameterDeclaration(node) ||
        ts.isVariableDeclaration(node)) &&
      node.name &&
      (ts.isIdentifier(node.name) || ts.isPrivateIdentifier(node.name))
    ) {
      add("declares", "declarations", node.name, null, null, node);
    }
    ts.forEachChild(node, visit);
  };
  visit(sourceFile);
  return true;
}

function main() {
  const { root, jsonl } = parseArguments(process.argv.slice(2));
  const { files: discoveredFiles, configs, rejected: initialRejected } = collectFiles(root);
  const diagnostics = [];
  const projects = parseProjects(root, configs, diagnostics);
  const discovered = new Set(discoveredFiles);
  const selected = projects.length > 0
    ? [...new Set(projects.flatMap((project) => project.fileNames))]
        .filter((fileName) => discovered.has(fileName))
    : discoveredFiles;
  selected.sort((left, right) => relativePath(root, left).localeCompare(relativePath(root, right), "en"));
  if (selected.length > MAX_FILES) fail(`source file count exceeds ${MAX_FILES}`);
  const selectedSet = new Set(selected);
  const rejected = new Set(
    initialRejected.filter((fileName) => selectedSet.has(path.resolve(root, fileName))),
  );
  const constructs = [];
  let totalBytes = 0;
  let parsedFiles = 0;
  const sourceDigest = crypto.createHash("sha256");
  for (const fileName of selected) {
    const relative = relativePath(root, fileName);
    if (rejected.has(relative)) continue;
    let raw;
    try {
      const stat = fs.statSync(fileName);
      if (stat.size > MAX_SOURCE_BYTES) {
        rejected.add(relative);
        continue;
      }
      raw = fs.readFileSync(fileName);
    } catch (error) {
      rejected.add(relative);
      continue;
    }
    totalBytes += raw.byteLength;
    if (totalBytes > MAX_SOURCE_BYTES) {
      fail(`source byte count exceeds ${MAX_SOURCE_BYTES}`);
    }
    sourceDigest.update(relative).update("\0").update(raw);
    if (parseFile(root, fileName, raw, constructs, diagnostics)) parsedFiles += 1;
    else rejected.add(relative);
  }
  constructs.sort((left, right) => {
    const fields = ["sourceFile", "relation", "capability", "ownerQualifiedName", "targetSpelling", "qualifier", "startByte", "endByte", "startLine"];
    for (const field of fields) {
      const leftValue = left[field] ?? "";
      const rightValue = right[field] ?? "";
      if (leftValue < rightValue) return -1;
      if (leftValue > rightValue) return 1;
    }
    return 0;
  });
  diagnostics.sort((left, right) => {
    if (left.file < right.file) return -1;
    if (left.file > right.file) return 1;
    return left.message.localeCompare(right.message, "en");
  });
  const script = fs.readFileSync(fileURLToPath(import.meta.url));
  const configDigest = sha256(
    projects
      .map((project) => `${relativePath(root, project.configFile)}\0${project.configDigest}`)
      .join("\n"),
  );
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
      projectMode: projects.length > 0 ? "project" : configs.length > 0 ? "fallback" : "tree",
      diagnosticCount: String(diagnostics.length),
    },
    scannedFiles: selected.length,
    parsedFiles,
    rejectedFiles: [...rejected].sort(),
    projects: projects.map((project) => ({
      configFile: relativePath(root, project.configFile),
      fileCount: project.fileNames.filter((fileName) => selectedSet.has(fileName)).length,
      files: project.fileNames
        .filter((fileName) => selectedSet.has(fileName))
        .map((fileName) => relativePath(root, fileName)),
      references: project.references
        .map((reference) => relativePath(root, reference))
        .sort(),
      configDigest: project.configDigest,
    })),
    diagnostics: diagnostics.slice(0, MAX_DIAGNOSTICS),
    constructs,
  };
  if (!jsonl) {
    process.stdout.write(`${JSON.stringify(payload)}\n`);
    return;
  }
  const coverage = selected.map((fileName) => {
    const file = relativePath(root, fileName);
    return {
      recordType: "file",
      file,
      status: rejected.has(file) ? "rejected" : "parsed",
    };
  });
  const records = [
    {
      schema: JSONL_SCHEMA,
      provider: PROVIDER,
      recordType: "header",
      metadata: payload.metadata,
      scannedFiles: payload.scannedFiles,
      parsedFiles: payload.parsedFiles,
      projectCount: payload.projects.length,
      diagnosticCount: payload.diagnostics.length,
      constructCount: payload.constructs.length,
    },
    ...payload.projects.map((project) => ({ recordType: "project", ...project })),
    ...coverage,
    ...payload.diagnostics.map((diagnostic) => ({ recordType: "diagnostic", ...diagnostic })),
    ...payload.constructs.map((construct) => ({ recordType: "construct", ...construct })),
    {
      schema: JSONL_SCHEMA,
      provider: PROVIDER,
      recordType: "footer",
      scannedFiles: payload.scannedFiles,
      parsedFiles: payload.parsedFiles,
      rejectedFiles: payload.rejectedFiles,
      projectCount: payload.projects.length,
      diagnosticCount: payload.diagnostics.length,
      constructCount: payload.constructs.length,
      sourceDigest: payload.metadata.sourceDigest,
      configDigest: payload.metadata.configDigest,
    },
  ];
  process.stdout.write(records.map((record) => JSON.stringify(record)).join("\n"));
  process.stdout.write("\n");
}

try {
  main();
} catch (error) {
  if (process.exitCode !== 2) {
    console.error(`[typescript-source-oracle] ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 2;
  }
}
