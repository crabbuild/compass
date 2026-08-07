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
const PROVIDER = "typescript_compiler_api_5_9_3";
const MAX_FILES = 20_000;
const MAX_SOURCE_BYTES = 128 * 1024 * 1024;
const MAX_CONSTRUCTS = 500_000;
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
  if (argv.length !== 2 || argv[0] !== "--root" || !argv[1]) {
    fail("usage: typescript-source-oracle.mjs --root PATH");
  }
  const root = path.resolve(argv[1]);
  let stat;
  try {
    stat = fs.statSync(root);
  } catch (error) {
    fail(`source root is unavailable: ${error.message}`);
  }
  if (!stat.isDirectory()) {
    fail(`source root is not a directory: ${root}`);
  }
  return root;
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

function collectSourceFiles(root) {
  const files = [];
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
      if (!isSourceFile(entry.name)) {
        continue;
      }
      const relative = relativePath(root, fileName);
      files.push(fileName);
      if (files.length > MAX_FILES) {
        fail(`source file count exceeds ${MAX_FILES}`);
      }
      if (entry.isSymbolicLink()) {
        rejected.push(relative);
      }
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

function parseFile(root, fileName, raw, constructs) {
  const text = raw.toString("utf8");
  const sourceFile = ts.createSourceFile(
    fileName,
    text,
    ts.ScriptTarget.Latest,
    true,
    scriptKind(fileName),
  );
  if (sourceFile.parseDiagnostics && sourceFile.parseDiagnostics.length > 0) {
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
  const root = parseArguments(process.argv.slice(2));
  const { files, rejected: initialRejected } = collectSourceFiles(root);
  const rejected = new Set(initialRejected);
  const constructs = [];
  let totalBytes = 0;
  let parsedFiles = 0;
  for (const fileName of files) {
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
    if (parseFile(root, fileName, raw, constructs)) parsedFiles += 1;
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
  const script = fs.readFileSync(fileURLToPath(import.meta.url));
  const payload = {
    schema: SCHEMA,
    provider: PROVIDER,
    metadata: {
      compilerVersion: ts.version,
      nodeVersion: process.version,
      scriptSha256: sha256(script),
      platform: `${process.platform}/${process.arch}`,
    },
    scannedFiles: files.length,
    parsedFiles,
    rejectedFiles: [...rejected].sort(),
    constructs,
  };
  process.stdout.write(`${JSON.stringify(payload)}\n`);
}

try {
  main();
} catch (error) {
  if (process.exitCode !== 2) {
    console.error(`[typescript-source-oracle] ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 2;
  }
}
