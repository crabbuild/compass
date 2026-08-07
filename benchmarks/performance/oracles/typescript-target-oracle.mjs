#!/usr/bin/env node

/**
 * Independent TypeScript/JavaScript target-identity oracle.
 *
 * This is a qualification-only tool. It uses the pinned TypeScript compiler
 * checker to adjudicate source-backed target declarations. It never reads a
 * Compass or Graphify graph and is not part of normal Compass execution.
 */

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import ts from "typescript";

const SCHEMA = "compass.typescript-target-oracle/1";
const PROVIDER = "typescript_checker_api_5_9_3";
const MAX_FILES = 20_000;
const MAX_SOURCE_BYTES = 128 * 1024 * 1024;
const MAX_CONSTRUCTS = 500_000;
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
  "node_modules",
  "target",
]);

function fail(message) {
  console.error(`[typescript-target-oracle] ${message}`);
  process.exitCode = 2;
  throw new Error(message);
}

function sha256(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

function parseArguments(argv) {
  if (argv.length !== 2 || argv[0] !== "--root" || !argv[1]) {
    fail("usage: typescript-target-oracle.mjs --root PATH");
  }
  let root;
  try {
    root = fs.realpathSync.native(path.resolve(argv[1]));
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
  if (
    !relative ||
    relative === ".." ||
    relative.startsWith("../") ||
    path.isAbsolute(relative)
  ) {
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

function collectFiles(root) {
  const files = [];
  const rejected = [];
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
        if (isSourceFile(entry.name)) rejected.push(relativePath(root, fileName));
        continue;
      }
      if (isSourceFile(entry.name)) files.push(fileName);
      if (files.length > MAX_FILES) fail(`source file count exceeds ${MAX_FILES}`);
    }
  }
  files.sort((left, right) => relativePath(root, left).localeCompare(relativePath(root, right), "en"));
  rejected.sort();
  return { files, rejected };
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

function identifierText(sourceFile, node) {
  if (!node) return "";
  if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) return node.text;
  if (ts.isStringLiteral(node) || ts.isNumericLiteral(node)) return node.text;
  return node.getText(sourceFile).trim();
}

function targetNode(node) {
  if (!node) return null;
  if (ts.isPropertyAccessExpression(node)) return node.name;
  if (ts.isElementAccessExpression(node) && node.argumentExpression) {
    if (
      ts.isStringLiteral(node.argumentExpression) ||
      ts.isNumericLiteral(node.argumentExpression)
    ) {
      return node.argumentExpression;
    }
  }
  if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) return node;
  return node;
}

function isDeclarationName(node) {
  const parent = node.parent;
  if (!parent) return false;
  if (parent.name === node) return true;
  if (
    ts.isImportSpecifier(parent) ||
    ts.isExportSpecifier(parent)
  ) {
    return parent.name === node || parent.propertyName === node;
  }
  if (ts.isNamespaceImport(parent) || ts.isImportClause(parent)) return true;
  if (ts.isQualifiedName(parent) || ts.isPropertyAccessExpression(parent)) return false;
  return false;
}

function declarationNodes(node) {
  return (
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
  );
}

function constructMap(sourceFile) {
  const offsets = sourceFile.__byteOffsets;
  const constructs = new Map();
  const add = (relation, capability, node) => {
    const target = targetNode(node);
    if (!target) return;
    const start = target.getStart(sourceFile, false);
    const end = target.getEnd();
    if (!(end > start) || end >= offsets.length) return;
    const key = `${relation}\0${capability}\0${offsets[start]}\0${offsets[end]}`;
    if (!constructs.has(key)) {
      constructs.set(key, {
        relation,
        capability,
        node,
        target,
        startByte: offsets[start],
        endByte: offsets[end],
        targetSpelling: identifierText(sourceFile, target),
      });
    }
  };
  const visit = (node) => {
    if (ts.isCallExpression(node)) add("calls", "calls", node.expression);
    else if (ts.isNewExpression(node)) add("instantiates", "construction", node.expression);
    else if (ts.isPropertyAccessExpression(node)) add("accesses", "members", node.name);
    else if (
      ts.isElementAccessExpression(node) &&
      node.argumentExpression &&
      (ts.isStringLiteral(node.argumentExpression) ||
        ts.isNumericLiteral(node.argumentExpression))
    ) {
      add("accesses", "members", node.argumentExpression);
    } else if (ts.isHeritageClause(node)) {
      const relation =
        node.token === ts.SyntaxKind.ExtendsKeyword ? "extends" : "implements";
      for (const type of node.types) add(relation, "base_types", type.expression);
    } else if (ts.isTypeReferenceNode(node)) {
      // `as const` is a literal assertion, not a named type reference.
      if (
        !(ts.isAsExpression(node.parent) && node.parent.type === node &&
          node.typeName.getText(sourceFile) === "const")
      ) {
        add("references", "type_references", node.typeName);
      }
    } else if (ts.isJsxOpeningLikeElement(node)) {
      add("references", "jsx", node.tagName);
    }

    if (declarationNodes(node)) add("declares", "declarations", node.name);
    ts.forEachChild(node, visit);
  };
  visit(sourceFile);
  return constructs;
}

function safeRead(root, fileName, limit = MAX_SOURCE_BYTES) {
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

function makeHost(root, options) {
  const base = ts.createCompilerHost(options, true);
  const read = (fileName) => {
    const raw = safeRead(root, fileName);
    return raw === undefined ? undefined : raw.toString("utf8");
  };
  return {
    ...base,
    fileExists: (fileName) => safeRead(root, fileName) !== undefined,
    readFile: read,
    getSourceFile(fileName, languageVersion) {
      const text = read(fileName);
      return text === undefined
        ? undefined
        : ts.createSourceFile(fileName, text, languageVersion, true, scriptKind(fileName));
    },
    realpath(fileName) {
      const absolute = path.resolve(fileName);
      if (!insideRoot(root, absolute)) return fileName;
      try {
        return fs.realpathSync.native(absolute);
      } catch {
        return absolute;
      }
    },
    getCurrentDirectory: () => root,
    writeFile: () => {},
  };
}

function enclosingClass(node) {
  let current = node.parent;
  while (current) {
    if (ts.isClassLike(current)) return current;
    current = current.parent;
  }
  return null;
}

function normalizeDeclaration(declaration) {
  if (ts.isConstructorDeclaration(declaration)) {
    const body = declaration.parent;
    const owner = body?.parent;
    if (owner && ts.isClassLike(owner)) return owner;
  }
  return declaration;
}

function symbolFromLocation(checker, node) {
  let symbol;
  try {
    symbol = checker.getSymbolAtLocation(node);
    if (symbol && (symbol.flags & ts.SymbolFlags.Alias) !== 0) {
      const aliased = checker.getAliasedSymbol(symbol);
      if (aliased && aliased !== symbol) symbol = aliased;
    }
  } catch {
    return undefined;
  }
  return symbol;
}

function superSymbol(checker, node) {
  if (node.kind !== ts.SyntaxKind.SuperKeyword) return undefined;
  const owner = enclosingClass(node);
  if (!owner) return undefined;
  let type;
  try {
    type = checker.getTypeAtLocation(owner);
    const bases = checker.getBaseTypes(type) ?? [];
    if (bases.length !== 1) return undefined;
    return bases[0].symbol;
  } catch {
    return undefined;
  }
}

function targetDeclaration(checker, construct) {
  let symbol = symbolFromLocation(checker, construct.target) ?? superSymbol(checker, construct.target);
  if (!symbol && (construct.relation === "calls" || construct.relation === "instantiates")) {
    try {
      const signature = checker.getResolvedSignature(construct.node);
      if (signature?.declaration) return [normalizeDeclaration(signature.declaration)];
    } catch {
      return [];
    }
  }
  if (!symbol) return [];
  let declarations = (symbol.declarations ?? []).map(normalizeDeclaration);
  if (declarations.length === 0 && symbol.valueDeclaration) {
    declarations = [normalizeDeclaration(symbol.valueDeclaration)];
  }
  const unique = new Map();
  for (const declaration of declarations) {
    const sourceFile = declaration.getSourceFile?.();
    const name = declaration.name;
    const start = name?.getStart(sourceFile, false) ?? declaration.getStart(sourceFile, false);
    const end = name?.getEnd() ?? declaration.getEnd();
    unique.set(`${sourceFile?.fileName}\0${start}\0${end}`, declaration);
  }
  return [...unique.values()];
}

function targetRecord(root, admittedFiles, sourceFile, construct, declarations) {
  const target = declarations.length === 1 ? declarations[0] : undefined;
  const sourceFileName = target?.getSourceFile?.().fileName;
  if (!target || !sourceFileName) {
    return {
      resolutionKind: declarations.length > 1 ? "ambiguous" : "unresolved",
      targetFile: null,
      targetStartByte: null,
      targetEndByte: null,
      targetDeclarationKind: null,
      targetName: null,
    };
  }
  const absolute = path.resolve(sourceFileName);
  if (!insideRoot(root, absolute)) {
    return {
      resolutionKind: "external",
      targetFile: null,
      targetStartByte: null,
      targetEndByte: null,
      targetDeclarationKind: ts.SyntaxKind[target.kind] ?? null,
      targetName: target.name ? identifierText(target.getSourceFile(), target.name) : null,
    };
  }
  const relative = relativePath(root, absolute);
  if (!admittedFiles.has(relative)) {
    return {
      resolutionKind: "external",
      targetFile: relative,
      targetStartByte: null,
      targetEndByte: null,
      targetDeclarationKind: ts.SyntaxKind[target.kind] ?? null,
      targetName: target.name ? identifierText(target.getSourceFile(), target.name) : null,
    };
  }
  const targetSource = target.getSourceFile();
  const raw = safeRead(root, absolute);
  const offsets = raw === undefined ? undefined : byteOffsets(raw.toString("utf8"));
  const name = target.name ?? target;
  const start = name.getStart(targetSource, false);
  const end = name.getEnd();
  if (!offsets || end >= offsets.length) {
    return {
      resolutionKind: "unresolved",
      targetFile: relative,
      targetStartByte: null,
      targetEndByte: null,
      targetDeclarationKind: ts.SyntaxKind[target.kind] ?? null,
      targetName: target.name ? identifierText(targetSource, target.name) : null,
    };
  }
  return {
    resolutionKind: "source",
    targetFile: relative,
    targetStartByte: offsets[start],
    targetEndByte: offsets[end],
    targetDeclarationKind: ts.SyntaxKind[target.kind] ?? null,
    targetName: target.name ? identifierText(targetSource, target.name) : identifierText(targetSource, target),
  };
}

function main() {
  const root = parseArguments(process.argv.slice(2));
  const { files, rejected: initialRejected } = collectFiles(root);
  const options = {
    allowJs: true,
    checkJs: true,
    noLib: true,
    skipLibCheck: true,
    target: ts.ScriptTarget.Latest,
    module: ts.ModuleKind.CommonJS,
    moduleResolution: ts.ModuleResolutionKind.Node10,
    jsx: ts.JsxEmit.Preserve,
  };
  const host = makeHost(root, options);
  const absoluteFiles = files.map((fileName) => path.resolve(fileName));
  const admittedFiles = new Set(files.map((fileName) => relativePath(root, fileName)));
  const program = ts.createProgram(absoluteFiles, options, host);
  const checker = program.getTypeChecker();
  const rejected = new Set(initialRejected);
  const diagnostics = [];
  const constructs = [];
  let totalBytes = 0;
  let parsedFiles = 0;
  const sourceDigest = crypto.createHash("sha256");
  for (const fileName of files) {
    const relative = relativePath(root, fileName);
    const raw = safeRead(root, fileName);
    if (raw === undefined) {
      rejected.add(relative);
      continue;
    }
    totalBytes += raw.byteLength;
    if (totalBytes > MAX_SOURCE_BYTES) fail(`source byte count exceeds ${MAX_SOURCE_BYTES}`);
    sourceDigest.update(relative).update("\0").update(raw);
    const sourceFile = program.getSourceFile(path.resolve(fileName));
    if (!sourceFile || sourceFile.parseDiagnostics?.length > 0) {
      rejected.add(relative);
      for (const diagnostic of sourceFile?.parseDiagnostics ?? []) {
        diagnostics.push({ file: relative, message: ts.flattenDiagnosticMessageText(diagnostic.messageText, " ") });
        if (diagnostics.length >= MAX_DIAGNOSTICS) fail(`diagnostic count exceeds ${MAX_DIAGNOSTICS}`);
      }
      continue;
    }
    sourceFile.__byteOffsets = byteOffsets(raw.toString("utf8"));
    parsedFiles += 1;
    const map = constructMap(sourceFile);
    for (const construct of map.values()) {
      if (constructs.length >= MAX_CONSTRUCTS) fail(`construct count exceeds ${MAX_CONSTRUCTS}`);
      const declarations = targetDeclaration(checker, construct);
      constructs.push({
        sourceFile: relative,
        relation: construct.relation,
        capability: construct.capability,
        targetSpelling: construct.targetSpelling,
        startByte: construct.startByte,
        endByte: construct.endByte,
        ...targetRecord(root, admittedFiles, sourceFile, construct, declarations),
      });
    }
  }
  constructs.sort((left, right) => {
    for (const field of ["sourceFile", "relation", "capability", "startByte", "endByte", "targetFile", "targetStartByte", "targetEndByte"]) {
      const a = left[field] ?? "";
      const b = right[field] ?? "";
      if (a < b) return -1;
      if (a > b) return 1;
    }
    return 0;
  });
  const script = fs.readFileSync(fileURLToPath(import.meta.url));
  const payload = {
    schema: SCHEMA,
    provider: PROVIDER,
    metadata: {
      compilerVersion: ts.version,
      nodeVersion: process.version,
      scriptSha256: sha256(script),
      platform: `${process.platform}/${process.arch}`,
      sourceDigest: sourceDigest.digest("hex"),
    },
    scannedFiles: files.length,
    parsedFiles,
    rejectedFiles: [...rejected].sort(),
    diagnostics: diagnostics.slice(0, MAX_DIAGNOSTICS),
    constructs,
  };
  process.stdout.write(`${JSON.stringify(payload)}\n`);
}

try {
  main();
} catch (error) {
  if (process.exitCode !== 2) {
    console.error(`[typescript-target-oracle] ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 2;
  }
}
