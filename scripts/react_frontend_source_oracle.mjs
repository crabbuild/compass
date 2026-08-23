#!/usr/bin/env node

/**
 * Independent frontend source oracle used only by the release qualification
 * harness.  It composes the pinned TypeScript compiler source oracle and adds
 * conservative, source-grounded frontend facts.  It never reads a Compass
 * graph, executes project code, resolves a target by name, or imports a
 * framework package.
 */

import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const SCHEMA = "compass.react-frontend-source-oracle/1";
const PROVIDER = "typescript_compiler_api_5_9_3_frontend_projection";
const MAX_FACTS = 500_000;
const SOURCE_SUFFIXES = new Set([".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"]);
const ROOT = path.dirname(fileURLToPath(import.meta.url));
const TYPESCRIPT_ORACLE = path.join(
  ROOT,
  "..",
  "benchmarks",
  "performance",
  "oracles",
  "typescript-source-oracle.mjs",
);

function fail(message) {
  console.error(`[react-frontend-source-oracle] ${message}`);
  process.exit(2);
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function parseArguments(argv) {
  if (argv.length < 2 || argv[0] !== "--root" || !argv[1]) {
    fail("usage: react_frontend_source_oracle.mjs --root PATH [--framework ID] [--output PATH]");
  }
  let root;
  try {
    root = fs.realpathSync.native(path.resolve(argv[1]));
    if (!fs.statSync(root).isDirectory()) fail(`root is not a directory: ${root}`);
  } catch (error) {
    fail(`root is unavailable: ${error.message}`);
  }
  let framework = "react";
  let output = null;
  for (let index = 2; index < argv.length; index += 1) {
    if (argv[index] === "--framework" && argv[index + 1]) {
      framework = argv[++index];
    } else if (argv[index] === "--output" && argv[index + 1]) {
      output = path.resolve(argv[++index]);
    } else {
      fail(`unknown argument ${argv[index]}`);
    }
  }
  return { root, framework, output };
}

function relativePath(root, file) {
  const relative = path.relative(root, file).split(path.sep).join("/");
  if (!relative || relative === "." || relative.startsWith("../") || path.isAbsolute(relative)) {
    fail(`source path escapes root: ${file}`);
  }
  return relative;
}

function runTypescriptOracle(root) {
  const result = spawnSync(
    process.execPath,
    [TYPESCRIPT_ORACLE, "--root", root, "--jsonl"],
    {
      cwd: ROOT,
      encoding: "utf8",
      maxBuffer: 512 * 1024 * 1024,
      env: { ...process.env, NODE_OPTIONS: "--max-old-space-size=2048" },
    },
  );
  if (result.status !== 0) {
    fail(`TypeScript source oracle failed: ${result.stderr || result.stdout}`);
  }
  const records = [];
  for (const line of result.stdout.split(/\r?\n/)) {
    if (!line.trim()) continue;
    try {
      records.push(JSON.parse(line));
    } catch (error) {
      fail(`TypeScript source oracle emitted invalid JSON: ${error.message}`);
    }
  }
  const header = records.find((record) => record.recordType === "header");
  const footer = records.find((record) => record.recordType === "footer");
  if (!header || !footer) fail("TypeScript source oracle did not emit header/footer");
  const diagnostics = records
    .filter((record) => record.recordType === "diagnostic")
    .map((record) => ({ file: record.file, message: record.message }))
    .filter((record) => typeof record.file === "string" && typeof record.message === "string")
    .sort((left, right) => left.file.localeCompare(right.file, "en") || left.message.localeCompare(right.message, "en"));
  return { records, header, footer, diagnostics, raw: Buffer.from(result.stdout, "utf8") };
}

function scriptKindFor(file) {
  if (file.endsWith(".tsx")) return ts.ScriptKind.TSX;
  if (file.endsWith(".jsx")) return ts.ScriptKind.JSX;
  if (file.endsWith(".mts") || file.endsWith(".mjs")) return ts.ScriptKind.JS;
  if (file.endsWith(".cts") || file.endsWith(".cjs")) return ts.ScriptKind.JS;
  if (file.endsWith(".js")) return ts.ScriptKind.JS;
  return ts.ScriptKind.TS;
}

function byteRange(source, sourceFile, node) {
  return {
    startByte: Buffer.byteLength(source.slice(0, node.getStart(sourceFile)), "utf8"),
    endByte: Buffer.byteLength(source.slice(0, node.getEnd()), "utf8"),
  };
}

function visitAst(node, callback) {
  callback(node);
  ts.forEachChild(node, (child) => visitAst(child, callback));
}

function nodeName(node) {
  if (!node?.name) return null;
  return ts.isIdentifier(node.name) || ts.isStringLiteral(node.name) || ts.isNumericLiteral(node.name)
    ? node.name.text
    : null;
}

function unwrapExpression(node) {
  let current = node;
  while (current && (ts.isParenthesizedExpression(current) || ts.isAsExpression(current) || ts.isTypeAssertionExpression(current))) {
    current = current.expression;
  }
  return current;
}

function propertyName(node) {
  if (!node?.name) return null;
  if (ts.isIdentifier(node.name) || ts.isStringLiteral(node.name) || ts.isNumericLiteral(node.name)) return node.name.text;
  return null;
}

function importedLocals(sourceFile, modules) {
  const names = new Map();
  for (const statement of sourceFile.statements) {
    if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
    if (!modules.has(statement.moduleSpecifier.text)) continue;
    const clause = statement.importClause;
    if (!clause) continue;
    if (clause.name) names.set(clause.name.text, "default");
    if (clause.namedBindings && ts.isNamedImports(clause.namedBindings)) {
      for (const element of clause.namedBindings.elements) names.set(element.name.text, element.propertyName?.text ?? element.name.text);
    }
    if (clause.namedBindings && ts.isNamespaceImport(clause.namedBindings)) {
      names.set(`${clause.namedBindings.name.text}.*`, "*");
    }
  }
  return names;
}

function declarationTargets(source, sourceFile) {
  const targets = new Map();
  const renderableDeclaration = (node) => {
    if (ts.isFunctionDeclaration(node) || ts.isClassDeclaration(node)) return true;
    if (!ts.isVariableDeclaration(node)) return false;
    // Destructured values such as Next's `this.props.Component` are ordinary
    // runtime values, not declarations whose component identity Compass can
    // prove.  Do not turn a binding-pattern spelling into a render target.
    if (!ts.isIdentifier(node.name)) return false;
    const initializer = unwrapExpression(node.initializer);
    if (!initializer) return false;
    if (ts.isArrowFunction(initializer) || ts.isFunctionExpression(initializer)) return true;
    // Styled/template values and other tagged expressions are deliberately
    // outside the JSX render capability.  They remain ordinary language
    // evidence until a framework-specific component contract qualifies them.
    if (ts.isTaggedTemplateExpression(initializer)) return false;
    if (ts.isCallExpression(initializer)) {
      const callee = unwrapExpression(initializer.expression);
      // Context values are uppercase in common React code but are not
      // component targets.
      if (ts.isIdentifier(callee) && callee.text === "createContext") return false;
      if (ts.isPropertyAccessExpression(callee) && ts.isIdentifier(callee.name) && callee.name.text === "createContext") return false;
      // Match the production role boundary: a value factory is renderable for
      // this capability only when its argument contains JSX (for example a
      // proven memo/forwardRef wrapper).  React.lazy/next/dynamic and opaque
      // factories have their own incomplete/dynamic evidence and must not be
      // scored as concrete `react.render.jsx` facts.
      return initializer.arguments.some((argument) => containsJsx(argument));
    }
    // Property aliases, conditional selections, and computed values do not
    // establish a callable component identity without executing user code.
    return false;
  };
  const add = (nameNode, node) => {
    if (!nameNode || !ts.isIdentifier(nameNode)) return;
    targets.set(nameNode.text, {
      name: nameNode.text,
      kind: node.kind,
      renderable: renderableDeclaration(node),
      ...byteRange(source, sourceFile, nameNode),
    });
  };
  const addBinding = (binding, node) => {
    if (!binding) return;
    if (ts.isIdentifier(binding)) {
      add(binding, node);
      return;
    }
    if (ts.isObjectBindingPattern(binding)) {
      for (const element of binding.elements) {
        if (ts.isBindingElement(element)) addBinding(element.name, node);
      }
      return;
    }
    if (ts.isArrayBindingPattern(binding)) {
      for (const element of binding.elements) {
        if (ts.isBindingElement(element)) addBinding(element.name, node);
      }
    }
  };
  visitAst(sourceFile, (node) => {
    if (ts.isFunctionDeclaration(node) || ts.isClassDeclaration(node) || ts.isEnumDeclaration(node) || ts.isInterfaceDeclaration(node)) add(node.name, node);
    else if (ts.isVariableDeclaration(node)) addBinding(node.name, node);
  });
  return targets;
}

function targetForExpression(source, sourceFile, targets, node) {
  const expression = unwrapExpression(node);
  if (!expression) return null;
  if (ts.isIdentifier(expression)) return targets.get(expression.text) ?? null;
  if (ts.isJsxElement(expression) || ts.isJsxSelfClosingElement(expression)) {
    const tag = ts.isJsxElement(expression) ? expression.openingElement.tagName : expression.tagName;
    if (ts.isIdentifier(tag)) return targets.get(tag.text) ?? null;
  }
  return null;
}

function directProperty(object, name) {
  if (!object || !ts.isObjectLiteralExpression(object)) return null;
  for (const property of object.properties) {
    if (ts.isPropertyAssignment(property) && propertyName(property) === name) return property.initializer;
    if (ts.isShorthandPropertyAssignment(property) && propertyName(property) === name) return property.name;
  }
  return null;
}

function routeObjects(node, callback, parentPath = "") {
  const current = unwrapExpression(node);
  if (!current) return;
  if (ts.isObjectLiteralExpression(current)) {
    const pathValue = directProperty(current, "path");
    const routePath = pathValue && ts.isStringLiteral(pathValue) ? `${parentPath}/${pathValue.text}`.replaceAll("//", "/") : parentPath;
    callback(current, routePath);
    for (const property of current.properties) {
      if (ts.isPropertyAssignment(property) && propertyName(property) === "children") routeObjects(property.initializer, callback, routePath);
    }
    return;
  }
  if (ts.isArrayLiteralExpression(current)) {
    for (const element of current.elements) routeObjects(element, callback, parentPath);
  }
}

function defaultExportTarget(source, sourceFile, targets) {
  let target = null;
  visitAst(sourceFile, (node) => {
    if (target || !ts.isExportAssignment(node)) return;
    const expression = unwrapExpression(node.expression);
    target = targetForExpression(source, sourceFile, targets, expression);
    if (!target && expression && (ts.isArrowFunction(expression) || ts.isFunctionExpression(expression) || ts.isClassExpression(expression))) {
      target = {
        name: "<default>",
        kind: expression.kind,
        renderable: true,
        ...byteRange(source, sourceFile, expression),
      };
    }
  });
  if (target) return target;
  visitAst(sourceFile, (node) => {
    if (target || (!ts.isFunctionDeclaration(node) && !ts.isClassDeclaration(node))) return;
    if (node.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.DefaultKeyword)) {
      target = targetForExpression(source, sourceFile, targets, node.name ?? node);
      if (!target) target = { name: "<default>", ...byteRange(source, sourceFile, node) };
    }
  });
  return target;
}

function declarationNode(source, sourceFile, declaration) {
  let found = null;
  visitAst(sourceFile, (node) => {
    if (found || (!ts.isFunctionDeclaration(node) && !ts.isClassDeclaration(node) && !ts.isMethodDeclaration(node) && !ts.isVariableDeclaration(node))) return;
    const name = node.name && ts.isIdentifier(node.name) ? node.name : null;
    if (!name || name.text !== declaration.name) return;
    const range = byteRange(source, sourceFile, name);
    if (range.startByte === declaration.startByte && range.endByte === declaration.endByte) found = node;
  });
  return found;
}

function containsJsx(node) {
  let found = false;
  if (!node) return false;
  visitAst(node, (child) => {
    if (ts.isJsxElement(child) || ts.isJsxSelfClosingElement(child) || ts.isJsxFragment(child)) found = true;
  });
  return found;
}

function containsHookCall(node) {
  let found = false;
  if (!node) return false;
  visitAst(node, (child) => {
    if (!ts.isCallExpression(child)) return;
    const expression = unwrapExpression(child.expression);
    if (ts.isIdentifier(expression) && /^use[A-Z]/u.test(expression.text)) found = true;
  });
  return found;
}

function containsComponentFactory(node) {
  if (!node || !ts.isVariableDeclaration(node)) return false;
  const initializer = unwrapExpression(node.initializer);
  if (!initializer || !ts.isCallExpression(initializer)) return false;
  const callee = unwrapExpression(initializer.expression);
  if (!ts.isIdentifier(callee) || !new Set(["memo", "forwardRef", "lazy"]).has(callee.text)) return false;
  return initializer.arguments.some((argument) => containsJsx(argument));
}

function declarationContainsJsx(node) {
  if (!node) return false;
  if (ts.isVariableDeclaration(node)) {
    const initializer = unwrapExpression(node.initializer);
    if (!initializer) return false;
    if (ts.isArrowFunction(initializer) || ts.isFunctionExpression(initializer)) return containsJsx(initializer);
    if (ts.isCallExpression(initializer)) {
      return initializer.arguments.some((argument) => containsJsx(argument));
    }
    return false;
  }
  if (ts.isFunctionDeclaration(node) || ts.isFunctionExpression(node) || ts.isArrowFunction(node) || ts.isMethodDeclaration(node)) {
    return Boolean(node.body && containsJsx(node.body));
  }
  if (ts.isClassDeclaration(node) || ts.isClassExpression(node)) return containsJsx(node);
  return false;
}

function isImportMetaGlobCall(node) {
  return ts.isCallExpression(node)
    && ts.isPropertyAccessExpression(node.expression)
    && node.expression.name.text === "glob"
    && ts.isMetaProperty(node.expression.expression)
    && node.expression.expression.name.text === "meta";
}

function topLevelDirectiveRange(source, sourceFile, directive) {
  for (const statement of sourceFile.statements) {
    if (!ts.isExpressionStatement(statement) || !ts.isStringLiteral(statement.expression)) continue;
    if (statement.expression.text === directive) return byteRange(source, sourceFile, statement.expression);
  }
  return null;
}

function isRecursiveJsxTarget(node, name) {
  let current = node.parent;
  while (current) {
    if (ts.isFunctionDeclaration(current) || ts.isClassDeclaration(current) || ts.isMethodDeclaration(current)) {
      if (current.name && ts.isIdentifier(current.name) && current.name.text === name) return true;
    }
    if (ts.isVariableDeclaration(current) && ts.isIdentifier(current.name) && current.name.text === name) return true;
    current = current.parent;
  }
  return false;
}

function discoverSourceFiles(root) {
  const files = [];
  const walk = (directory) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true }).sort((left, right) => left.name.localeCompare(right.name, "en"))) {
      if ([".git", "node_modules", "dist", "build", "coverage", ".next", ".turbo"].includes(entry.name)) continue;
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) walk(absolute);
      else if (entry.isFile() && SOURCE_SUFFIXES.has(path.extname(entry.name).toLowerCase())) files.push(relativePath(root, absolute));
    }
  };
  walk(root);
  return files;
}

function sourceFacts(root, framework, records) {
  const declarations = records.filter((record) => record.recordType === "declaration");
  const references = records.filter((record) => record.recordType === "reference");
  const calls = records.filter((record) => record.recordType === "call");
  const constructs = records.filter((record) => record.recordType === "construct");
  const imports = records.filter((record) => record.recordType === "import");
  const sourceFiles = new Set([
    ...records.filter((record) => record.recordType === "file").map((record) => record.file),
    ...discoverSourceFiles(root),
  ]);
  const facts = [];
  const factIds = new Map();
  const add = (fact) => {
    if (facts.length >= MAX_FACTS) fail(`frontend fact limit exceeds ${MAX_FACTS}`);
    const baseId = sha256(Buffer.from(JSON.stringify(fact)));
    // Multiple bounded observation passes can see the same framework
    // declaration (for example, the generic construct stream and the direct
    // config fallback). They are one semantic fact at one exact source
    // anchor, not separate relationship occurrences. Keep genuine repeated
    // source sites distinct while eliminating only byte-for-byte duplicates.
    if (factIds.has(baseId)) return;
    factIds.set(baseId, 1);
    facts.push({
      id: baseId,
      ...fact,
    });
  };
  const sourceDeclarations = (file, names = null) => declarations.filter((declaration) =>
    declaration.sourceFile === file &&
    (!names || names.has(declaration.name)) &&
    declaration.namespace === "value"
  );
  const isComponentSpelling = (spelling) =>
    typeof spelling === "string" && /^[A-Z]/u.test(spelling.split(".")[0]);

  const resolveImport = (sourceFile, moduleSpecifier) => {
    if (typeof moduleSpecifier !== "string") return null;
    const bases = moduleSpecifier.startsWith(".")
      ? [path.posix.normalize(path.posix.join(path.posix.dirname(sourceFile), moduleSpecifier))]
      : [moduleSpecifier, `src/${moduleSpecifier.replace(/^[@~]\//u, "")}`, `app/${moduleSpecifier.replace(/^[@~]\//u, "")}`];
    const candidates = bases.flatMap((base) => [base, ...[".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"].map((suffix) => `${base}${suffix}`), ...["index.ts", "index.tsx", "index.js", "index.jsx"].map((suffix) => `${base}/${suffix}`)]);
    const direct = candidates.find((candidate) => sourceFiles.has(candidate));
    if (direct) return direct;
    // Resolve the common workspace alias forms without treating the first
    // suffix match as truth.  A unique source identity is required; multiple
    // packages with the same alias target remain explicitly unresolved.
    const suffix = moduleSpecifier.replace(/^[@~]\//u, "");
    const matches = [...sourceFiles].filter((file) => {
      const withoutExtension = file.replace(/\.(?:[cm]?[jt]sx?)$/u, "");
      return withoutExtension === suffix
        || withoutExtension.endsWith(`/${suffix}`)
        || withoutExtension === `${suffix}/index`;
    });
    return matches.length === 1 ? matches[0] : null;
  };

  const locallyBound = (reference) => {
    if (sourceDeclarations(reference.sourceFile, new Set([reference.targetSpelling])).length > 0) return true;
    const binding = imports.find((entry) =>
      entry.sourceFile === reference.sourceFile && entry.localName === reference.targetSpelling,
    );
    if (!binding) return false;
    const importedFile = resolveImport(reference.sourceFile, binding.moduleSpecifier);
    if (!importedFile) return false;
    const importedName = binding.importedName === "default" ? null : binding.importedName;
    return declarations.some((declaration) =>
      declaration.sourceFile === importedFile &&
      (importedName === null || declaration.name === importedName),
    );
  };

  const sourceFileNames = new Set(sourceFiles);
  const astFiles = new Map();
  for (const file of sourceFileNames) {
    const absolute = path.join(root, file);
    if (!fs.existsSync(absolute) || !SOURCE_SUFFIXES.has(path.extname(file).toLowerCase())) continue;
    const source = fs.readFileSync(absolute, "utf8");
    astFiles.set(file, {
      source,
      ast: ts.createSourceFile(absolute, source, ts.ScriptTarget.Latest, true, scriptKindFor(file)),
    });
  }
  function importedTargetFromAstFile(file, localName, visited) {
    const entry = astFiles.get(file);
    if (!entry) return null;
    const matches = [];
    for (const statement of entry.ast.statements) {
      if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
      const clause = statement.importClause;
      if (!clause) continue;
      let importedName = null;
      if (clause.name?.text === localName) importedName = "default";
      if (clause.namedBindings && ts.isNamedImports(clause.namedBindings)) {
        const binding = clause.namedBindings.elements.find((element) => element.name.text === localName);
        if (binding) importedName = binding.propertyName?.text ?? binding.name.text;
      }
      if (!importedName) continue;
      const importedFile = resolveImport(file, statement.moduleSpecifier.text);
      if (!importedFile) continue;
      const target = targetFromAstFile(importedFile, importedName, visited);
      if (target) matches.push(target);
    }
    return matches.length === 1 ? matches[0] : null;
  }

  function targetFromAstFile(file, exportedName, visited = new Set()) {
    if (visited.has(`${file}:${exportedName}`)) return null;
    visited.add(`${file}:${exportedName}`);
    const entry = astFiles.get(file);
    if (!entry) return null;
    const targets = declarationTargets(entry.source, entry.ast);
    if (exportedName === "default") {
      const target = defaultExportTarget(entry.source, entry.ast, targets);
      if (target) return { file, target };
      for (const statement of entry.ast.statements) {
        if (!ts.isExportAssignment(statement)) continue;
        const expression = unwrapExpression(statement.expression);
        if (!ts.isIdentifier(expression)) continue;
        const imported = importedTargetFromAstFile(file, expression.text, visited);
        if (imported) return imported;
      }
    }
    const direct = targets.get(exportedName);
    if (direct) return { file, target: direct };
    const imported = importedTargetFromAstFile(file, exportedName, visited);
    if (imported) return imported;
    for (const statement of entry.ast.statements) {
      if (!ts.isExportDeclaration(statement) || !statement.moduleSpecifier || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
      const importedFile = resolveImport(file, statement.moduleSpecifier.text);
      if (!importedFile) continue;
      if (!statement.exportClause) {
        // `export * from` contributes every named export except `default`.
        // Follow only the requested spelling and retain the unique target
        // requirement so ambiguous barrels remain unresolved.
        if (exportedName === "default") continue;
        const target = targetFromAstFile(importedFile, exportedName, visited);
        if (target) return target;
        continue;
      }
      if (!ts.isNamedExports(statement.exportClause)) continue;
      const specifier = statement.exportClause.elements.find((element) => element.name.text === exportedName);
      if (!specifier) continue;
      const localName = specifier.propertyName?.text ?? specifier.name.text;
      const target = targetFromAstFile(importedFile, localName, visited);
      if (target) return target;
    }
    return null;
  }
  const astImportTarget = (file, localName) => {
    const entry = astFiles.get(file);
    if (!entry) return null;
    for (const statement of entry.ast.statements) {
      if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
      const clause = statement.importClause;
      if (!clause) continue;
      let importedName = null;
      if (clause.name?.text === localName) importedName = "default";
      if (clause.namedBindings && ts.isNamedImports(clause.namedBindings)) {
        const binding = clause.namedBindings.elements.find((element) => element.name.text === localName);
        if (binding) importedName = binding.propertyName?.text ?? binding.name.text;
      }
      if (!importedName) continue;
      const importedFile = resolveImport(file, statement.moduleSpecifier.text);
      if (!importedFile) continue;
      const target = targetFromAstFile(importedFile, importedName);
      if (target) return target;
    }
    return null;
  };
  // Mirror the production package-scoped activation boundary for Next route
  // conventions.  A nested package manifest is an ownership boundary: its
  // own dependency/configuration evidence wins and an unrelated repository
  // root dependency must not activate that package's `app` or `pages` tree.
  // This is intentionally source-only and bounded; it never evaluates a
  // configuration file or invokes a package manager.
  const nextActivationCache = new Map();
  const nextProjectActive = (file) => {
    if (nextActivationCache.has(file)) return nextActivationCache.get(file);
    let directory = path.dirname(path.join(root, file));
    let active = false;
    while (directory.startsWith(root)) {
      const packageFile = path.join(directory, "package.json");
      if (fs.existsSync(packageFile)) {
        try {
          const packageDocument = JSON.parse(fs.readFileSync(packageFile, "utf8"));
          const dependencies = [
            packageDocument?.dependencies,
            packageDocument?.devDependencies,
            packageDocument?.optionalDependencies,
            packageDocument?.peerDependencies,
          ];
          active = dependencies.some((section) =>
            section && typeof section === "object" && Object.prototype.hasOwnProperty.call(section, "next"),
          );
          if (!active) {
            active = ["next.config.js", "next.config.mjs", "next.config.ts"].some((name) =>
              fs.existsSync(path.join(directory, name)),
            );
          }
        } catch {
          active = false;
        }
        break;
      }
      if (["next.config.js", "next.config.mjs", "next.config.ts"].some((name) =>
        fs.existsSync(path.join(directory, name)),
      )) {
        active = true;
        break;
      }
      if (directory === root) break;
      directory = path.dirname(directory);
    }
    nextActivationCache.set(file, active);
    return active;
  };
  // The compiler oracle intentionally follows the project tsconfig and can
  // omit JavaScript files when allowJs is disabled.  JSX itself is still a
  // source fact, so inspect every bounded projection file with the pinned
  // compiler AST and resolve only same-project declarations/imports.
  for (const [file, entry] of astFiles) {
    const targets = declarationTargets(entry.source, entry.ast);
    const hasClientDirective = Boolean(topLevelDirectiveRange(entry.source, entry.ast, "use client"));
    for (const declaration of sourceDeclarations(file)) {
      const node = declarationNode(entry.source, entry.ast, declaration);
      const component = /^[A-Z]/u.test(declaration.name) && (declarationContainsJsx(node) || containsComponentFactory(node));
      if (component) {
        add({
          factType: "role",
          capability: "react.component.roles",
          framework,
          role: "ui_component",
          sourceFile: file,
          startByte: declaration.startByte,
          endByte: declaration.endByte,
          targetSpelling: declaration.name,
          origin: "ast",
          anchor: "exact",
        });
        if (framework === "next-app" && hasClientDirective) {
          add({
            factType: "role",
            capability: "next.client-server-directive",
            framework,
            role: "client_boundary",
            sourceFile: file,
            startByte: declaration.startByte,
            endByte: declaration.endByte,
            targetSpelling: declaration.name,
            origin: "ast",
            anchor: "exact",
          });
        }
      }
      if (/^use[A-Z]/u.test(declaration.name) && containsHookCall(node)) {
        add({
          factType: "role",
          capability: "react.hooks",
          framework,
          role: "hook",
          sourceFile: file,
          startByte: declaration.startByte,
          endByte: declaration.endByte,
          targetSpelling: declaration.name,
          origin: "ast",
          anchor: "exact",
        });
      }
    }
    visitAst(entry.ast, (node) => {
      if (
        framework === "vite" &&
        !/(^|\/)vite\.config\.[cm]?[jt]s$/u.test(file) &&
        isImportMetaGlobCall(node)
      ) {
        const range = byteRange(entry.source, entry.ast, node.expression);
        add({
          factType: "configuration",
          capability: "vite.file_set.glob",
          framework,
          relation: "configuration",
          sourceFile: file,
          ...range,
          targetSpelling: "import.meta.glob",
          origin: "ast",
          anchor: "exact",
        });
      }
      const tag = ts.isJsxElement(node)
        ? node.openingElement.tagName
        : ts.isJsxSelfClosingElement(node)
          ? node.tagName
          : null;
      const receiver = tag && ts.isIdentifier(tag)
        ? tag
        : tag && ts.isPropertyAccessExpression(tag) && ts.isIdentifier(tag.expression)
          ? tag.expression
          : null;
      if (!receiver || !isComponentSpelling(receiver.text)) return;
      // Compass's graph contract intentionally rejects non-call self-loops.
      // Keep recursive component uses in the source observation pool while
      // excluding them from the render relation capability score.
      if (isRecursiveJsxTarget(node, receiver.text)) return;
      const target = targets.get(receiver.text) ?? astImportTarget(file, receiver.text)?.target;
      // Uppercase names are only a spelling convention. Context objects and
      // other values also appear as `<Context.Provider>`; they are ordinary
      // references, not renderable component targets. Require a source-proven
      // function/class or known component-factory value before publishing the
      // JSX render capability.
      if (!target || target.renderable !== true) return;
      // The semantic target is the proven receiver, while the resolver's
      // member-use anchor is the property identifier (`Provider`).  Preserve
      // that exact source use-site without inventing a declaration target for
      // the member itself.
      const range = byteRange(
        entry.source,
        entry.ast,
        tag && ts.isPropertyAccessExpression(tag) ? tag.name : receiver,
      );
      add({
        factType: "relationship",
        capability: "react.render.jsx",
        framework,
        relation: "renders",
        sourceFile: file,
        ...range,
        targetSpelling: receiver.text,
        origin: "ast",
        anchor: "exact",
      });
    });
  }
  const routeFile = (file) => {
    const normalized = file.replaceAll("\\", "/");
    return /(^|\/)(app|pages|routes)(\/|$)/u.test(normalized) || normalized.includes("route") || normalized.endsWith("root.tsx") || normalized.endsWith("root.ts");
  };
  const addRouteFact = (capability, file, target, stage = "route_component", targetFile = null, sourceAnchor = null) => {
    if (!file) return;
    add({
      factType: "relationship",
      capability,
      framework,
      relation: "routes_to",
      stage,
      sourceFile: file,
      ...(sourceAnchor ?? {}),
      ...(targetFile ? { targetFile } : {}),
      ...(target ? {
        targetSpelling: target.name,
        targetStartByte: target.startByte,
        targetEndByte: target.endByte,
      } : {}),
      resolution: target ? "exact" : "unresolved",
      origin: "ast",
      anchor: "exact",
    });
  };
  const addRoute = (capability, file, target, stage = "route_component", targetFile = file, sourceAnchor = null) => {
    if (!target || !targetFile) return;
    addRouteFact(capability, file, target, stage, targetFile, sourceAnchor);
  };
  const addHierarchyFact = (capability, parentFile, childFile) => {
    if (!parentFile || !childFile || parentFile === childFile) return;
    add({
      factType: "relationship",
      capability,
      framework,
      relation: "contains",
      sourceFile: parentFile,
      startByte: 0,
      endByte: 1,
      targetFile: childFile,
      targetStartByte: 0,
      targetEndByte: 0,
      origin: "convention",
      anchor: "file",
    });
  };
  const routeFiles = [];
  const routeFileForFramework = (file) => {
    if (framework === "next-app") return /(^|\/)app\/(?:.+\/)?(page|layout|template|error|global-error|loading|not-found|default|route)\.[cm]?[jt]sx?$/u.test(file);
    if (framework === "next-pages") return /(^|\/)pages\/.+\.[cm]?[jt]sx?$/u.test(file);
    if (framework === "react-router") return /(^|\/)(app\/routes|src\/routes)\/.+\.[cm]?[jt]sx?$/u.test(file);
    if (framework === "remix") return /(^|\/)(app\/|src\/)?routes\/.+\.[cm]?[jt]sx?$/u.test(file);
    if (framework === "tanstack-router") return /(^|\/)src\/routes\/.+\.[cm]?[jt]sx?$/u.test(file);
    return false;
  };
  const routeParent = (file, candidates) => {
    const parts = file.replaceAll("\\", "/").split("/");
    for (let index = parts.length - 1; index > 0; index -= 1) {
      const parentDirectory = parts.slice(0, index).join("/");
      // Do not select the child itself as its own parent.  A route module is
      // always a candidate in its own directory, so omitting this identity
      // guard would make the loop stop at the first iteration and silently
      // drop every ancestor hierarchy relationship.
      const parent = candidates.find((candidate) => candidate !== file && candidate.split("/").slice(0, -1).join("/") === parentDirectory);
      if (parent) return parent;
    }
    return null;
  };
  const routeHierarchyScope = (file) => {
    const normalized = file.replaceAll("\\", "/").replace(/^\/+|\/+$/gu, "");
    for (const marker of [
      "src/app/",
      "app/",
      "src/pages/",
      "pages/",
      "app/routes/",
      "src/routes/",
      "routes/",
    ]) {
      const index = normalized.indexOf(marker);
      if (index >= 0 && (index === 0 || normalized[index - 1] === "/")) {
        return `${normalized.slice(0, index)}${marker.slice(0, -1)}`;
      }
    }
    return "";
  };
  const resolveSourceFile = (file, moduleSpecifier) => {
    if (typeof moduleSpecifier !== "string" || !moduleSpecifier || moduleSpecifier.startsWith("@")) return null;
    const clean = moduleSpecifier.replace(/^\.\//u, "");
    const bases = moduleSpecifier.startsWith(".")
      ? [path.posix.normalize(path.posix.join(path.posix.dirname(file), moduleSpecifier))]
      : [clean, path.posix.join(path.posix.dirname(file), clean), path.posix.join(path.posix.dirname(path.posix.dirname(file)), clean)];
    const candidates = bases.flatMap((base) => [base, ...[...SOURCE_SUFFIXES].map((suffix) => `${base}${suffix}`), ...["index.ts", "index.tsx", "index.js", "index.jsx"].map((suffix) => `${base}/${suffix}`)]);
    return candidates.find((candidate) => sourceFileNames.has(candidate)) ?? null;
  };
  const targetInModule = (file, moduleSpecifier) => {
    const targetFile = resolveSourceFile(file, moduleSpecifier);
    if (!targetFile) return null;
    const entry = astFiles.get(targetFile);
    if (!entry) return null;
    const targets = declarationTargets(entry.source, entry.ast);
    const target = defaultExportTarget(entry.source, entry.ast, targets);
    return target ? { targetFile, target } : null;
  };
  const defaultRouteTarget = (file, source, ast, targets) => {
    const resolved = targetFromAstFile(file, "default");
    if (resolved) return resolved;
    const target = defaultExportTarget(source, ast, targets);
    return target ? { file, target } : null;
  };
  const directHandler = (object, targets) => {
    for (const property of ["component", "Component", "element", "ErrorBoundary", "errorElement", "errorComponent", "pendingComponent", "notFoundComponent"]) {
      const value = directProperty(object, property);
      const target = targetForExpression(object.getSourceFile().text, object.getSourceFile(), targets, value);
      if (target) return { property, target };
    }
    return null;
  };
  for (const [file, entry] of astFiles) {
    if (!routeFile(file)) continue;
    const { source, ast } = entry;
    const targets = declarationTargets(source, ast);
    if (framework === "next-app" && nextProjectActive(file) && /(^|\/)app\/(?:.+\/)?(page|layout|template|error|global-error|loading|not-found|default|route)\.[cm]?[jt]sx?$/u.test(file)) {
      routeFiles.push(file);
      const appFileName = file.replace(/\.[cm]?[jt]sx?$/u, "").split("/").pop();
      if (appFileName === "route") {
        for (const name of ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]) {
          const target = targets.get(name);
          if (target) addRoute("next.app.route", file, target, "handler", file);
        }
        continue;
      }
      const resolved = defaultRouteTarget(file, source, ast, targets);
      const target = resolved?.target ?? null;
      const stage = {
        page: "route_component",
        layout: "layout",
        template: "template",
        error: "error_boundary",
        "global-error": "error_boundary",
        loading: "loading",
        "not-found": "not_found",
        default: "default",
      }[appFileName] ?? "route_component";
      if (target) addRoute("next.app.route", file, target, stage, resolved.file);
      else addRouteFact("next.app.route", file, null, stage, file);
      for (const name of ["generateStaticParams", "generateMetadata"]) {
        const generated = targets.get(name);
        if (generated) addRoute("next.app.route", file, generated, "data_loader", file);
      }
    }
    if (framework === "next-pages" && nextProjectActive(file) && /(^|\/)pages\/.+\.[cm]?[jt]sx?$/u.test(file)) {
      routeFiles.push(file);
      const resolved = defaultRouteTarget(file, source, ast, targets);
      const target = resolved?.target ?? null;
      const stage = /(^|\/)pages\/api\//u.test(file)
        ? "handler"
        : /(^|\/)pages\/_document\.[cm]?[jt]sx?$/u.test(file)
          ? "template"
          : "route_component";
      if (target) addRoute("next.pages.route", file, target, stage, resolved.file);
      else addRouteFact("next.pages.route", file, null, stage, file);
      if (/[\[\]]/u.test(file)) {
        if (target) addRoute("next.pages.dynamic", file, target, stage, resolved.file);
        else addRouteFact("next.pages.dynamic", file, null, stage, file);
      }
    }
    if (framework === "react-router" && /(^|\/)(app\/routes|src\/routes)\/.+\.[cm]?[jt]sx?$/u.test(file)) {
      routeFiles.push(file);
      const resolved = defaultRouteTarget(file, source, ast, targets);
      const target = resolved?.target ?? null;
      if (target) addRoute("react-router.route", file, target, "route_component", resolved.file);
      else addRouteFact("react-router.route", file, null, "route_component", file);
      for (const stage of ["loader", "action"]) {
        const handler = targets.get(stage);
        if (handler) addRoute("react-router.loader-action", file, handler, stage);
      }
    }
    if (framework === "remix" && /(^|\/)(app\/|src\/)?routes\//u.test(file)) {
      const exported = new Map();
      const defaultTarget = defaultExportTarget(source, ast, targets);
      if (defaultTarget) exported.set("route_component", defaultTarget);
      for (const [name, stage] of [["loader", "loader"], ["action", "action"]]) {
        const target = targets.get(name);
        if (target) exported.set(stage, target);
      }
      if (exported.size) routeFiles.push(file);
      for (const [stage, target] of exported) addRoute("remix.route", file, target, stage);
      for (const [stage, target] of exported) {
        if (stage === "loader" || stage === "action") addRoute("remix.loader-action", file, target, stage);
      }
    }
    if (framework === "remix" && /(^|\/)(app|src)\/routes\.[cm]?[jt]sx?$/u.test(file)) {
      const importsForFile = importedLocals(ast, new Set(["remix/routes"]));
      const factories = new Set(["route", "get", "post", "put", "del", "form", "resources"]);
      const joinPath = (parent, child) => {
        const clean = String(child ?? "").replace(/^\/+|\/+$/gu, "");
        if (!clean) return parent || "/";
        return parent ? `${parent.replace(/\/$/u, "")}/${clean}` : `/${clean}`;
      };
      const emitValue = (value, parentPath, sourceAnchor) => {
        const expression = unwrapExpression(value);
        if (!expression) return;
        if (ts.isStringLiteral(expression)) {
          addRouteFact("remix.route.config", file, null, "route_component", null, sourceAnchor);
          return;
        }
        if (ts.isObjectLiteralExpression(expression)) {
          for (const property of expression.properties) {
            if (!ts.isPropertyAssignment(property)) continue;
            const key = propertyName(property);
            if (!key) continue;
            emitValue(property.initializer, joinPath(parentPath, key), byteRange(source, ast, property));
          }
          return;
        }
        if (!ts.isCallExpression(expression) || !ts.isIdentifier(expression.expression)) return;
        const factory = importsForFile.get(expression.expression.text);
        if (!factory || !factories.has(factory)) return;
        const first = expression.arguments[0];
        const second = expression.arguments[1];
        if (factory === "route") {
          if (first && ts.isObjectLiteralExpression(unwrapExpression(first))) {
            emitValue(first, parentPath, byteRange(source, ast, expression));
          } else if (first && ts.isStringLiteral(first) && second && ts.isObjectLiteralExpression(unwrapExpression(second))) {
            emitValue(second, joinPath(parentPath, first.text), byteRange(source, ast, expression));
          }
          return;
        }
        if (!first || !ts.isStringLiteral(first)) return;
        const stage = factory === "form" ? "route_component" : "handler";
        addRouteFact("remix.route.config", file, null, stage, null, byteRange(source, ast, expression));
      };
      visitAst(ast, (node) => {
        if (!ts.isCallExpression(node) || !ts.isIdentifier(node.expression)) return;
        if (importsForFile.get(node.expression.text) !== "route" || !node.arguments[0] || !ts.isObjectLiteralExpression(unwrapExpression(node.arguments[0]))) return;
        emitValue(node.arguments[0], "", byteRange(source, ast, node));
      });
    }
    if (framework === "tanstack-router") {
      const importsForFile = importedLocals(ast, new Set(["@tanstack/react-router", "@tanstack/router-core"]));
      let recognizedRoute = false;
      visitAst(ast, (node) => {
        if (!ts.isCallExpression(node)) return;
        const callee = ts.isIdentifier(node.expression) ? node.expression.text : null;
        const factory = callee ? importsForFile.get(callee) : null;
        if (!factory || !["createFileRoute", "createLazyFileRoute", "createRoute", "createRootRoute", "createRootRouteWithContext"].includes(factory)) return;
        const outer = node.parent && ts.isCallExpression(node.parent) && node.parent.expression === node ? node.parent : node;
        const args = outer.arguments;
        const config = [...args].find((argument) => ts.isObjectLiteralExpression(unwrapExpression(argument)));
        if (!config) return;
        recognizedRoute = true;
        const routeStageProperties = [["component", "route_component"], ["pendingComponent", "boundary"], ["errorComponent", "boundary"], ["notFoundComponent", "boundary"]];
        for (const [property, stage] of routeStageProperties) {
          const target = targetForExpression(source, ast, targets, directProperty(unwrapExpression(config), property));
          addRoute("tanstack.route", file, target, stage, file, byteRange(source, ast, node));
        }
        const loaderValue = directProperty(unwrapExpression(config), "loader");
        const loader = targetForExpression(source, ast, targets, loaderValue);
        if (loader) {
          addRoute("tanstack.loader", file, loader, "loader", file, byteRange(source, ast, node));
        } else if (loaderValue && ts.isIdentifier(unwrapExpression(loaderValue))) {
          // Route loaders are often imported from a data module. Resolve that
          // bounded same-project import before deciding that no independent
          // target exists; inline closures remain intentionally unresolved and
          // are not advertised as target relationships.
          const imported = astImportTarget(file, unwrapExpression(loaderValue).text);
          if (imported) addRoute("tanstack.loader", file, imported.target, "loader", imported.file, byteRange(source, ast, node));
        }
      });
      if (!recognizedRoute
        && /(^|\/)src\/routes\/.+\.[cm]?[jt]sx?$/u.test(file)
        && !/\/routeTree\.gen\.[cm]?[jt]s$/u.test(file)
        && targets.has("Route")) {
        routeFiles.push(file);
      }
      if (recognizedRoute) routeFiles.push(file);
    }
    if (framework === "react-router") {
      const importsForFile = importedLocals(ast, new Set(["react-router", "react-router-dom"]));
      visitAst(ast, (node) => {
        if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) {
          const imported = importsForFile.get(node.expression.text);
          if (["createBrowserRouter", "createHashRouter", "createMemoryRouter", "createRouter"].includes(imported)) {
            const first = node.arguments[0];
            routeObjects(first, (object) => {
              const handler = directHandler(object, targets);
              if (handler) addRoute("react-router.route", file, handler.target, "route_component", file, byteRange(source, ast, object));
            });
          }
        }
        if (!ts.isJsxElement(node) && !ts.isJsxSelfClosingElement(node)) return;
        const tag = ts.isJsxElement(node) ? node.openingElement.tagName : node.tagName;
        if (!ts.isIdentifier(tag) || importsForFile.get(tag.text) !== "Route") return;
        const pathAttribute = (ts.isJsxElement(node) ? node.openingElement : node).attributes.properties.find((attribute) => ts.isJsxAttribute(attribute) && attribute.name.text === "path");
        if (!pathAttribute || !ts.isJsxAttribute(pathAttribute) || !pathAttribute.initializer || !ts.isStringLiteral(pathAttribute.initializer)) return;
        const elementAttribute = (ts.isJsxElement(node) ? node.openingElement : node).attributes.properties.find((attribute) => ts.isJsxAttribute(attribute) && ["Component", "element"].includes(attribute.name.text));
        if (!elementAttribute || !ts.isJsxAttribute(elementAttribute) || !elementAttribute.initializer) return;
        const target = targetForExpression(source, ast, targets, ts.isJsxExpression(elementAttribute.initializer) ? elementAttribute.initializer.expression : elementAttribute.initializer);
        addRoute("react-router.route", file, target, "route_component", file, byteRange(source, ast, node));
      });
    }
  }

  // Compass uses bytewise portable-path ordering at the resolver boundary;
  // avoid locale-dependent ordering when selecting a same-directory parent.
  routeFiles.sort();
  const hierarchyCapability = {
    "next-app": "next.app.hierarchy",
    "next-pages": "next.pages.hierarchy",
    "react-router": "react-router.hierarchy",
    remix: "remix.hierarchy",
    "tanstack-router": "tanstack.route.hierarchy",
  }[framework];
  if (hierarchyCapability) {
    const groups = new Map();
    for (const file of routeFiles) {
      const scope = routeHierarchyScope(file);
      if (!groups.has(scope)) groups.set(scope, []);
      groups.get(scope).push(file);
    }
    for (const candidates of groups.values()) {
      candidates.sort();
      for (const file of candidates) {
        const parent = routeParent(file, candidates);
        if (parent) addHierarchyFact(hierarchyCapability, parent, file);
      }
    }
  }

  for (const construct of constructs) {
    if (framework === "vite" && construct.sourceFile && /^.*vite\.config\.[cm]?[jt]s$/u.test(construct.sourceFile)) {
      if (construct.relation === "calls" && construct.targetSpelling === "defineConfig") {
        add({ factType: "configuration", capability: "vite.config.factory", framework, relation: "configuration", sourceFile: construct.sourceFile, startByte: construct.startByte, endByte: construct.endByte, targetSpelling: construct.targetSpelling, origin: "ast", anchor: "exact" });
      }
      if (construct.relation === "imports" && typeof construct.targetSpelling === "string" && construct.targetSpelling.includes("plugin")) {
        add({ factType: "configuration", capability: "vite.plugin.identity", framework, relation: "configuration", sourceFile: construct.sourceFile, startByte: construct.startByte, endByte: construct.endByte, targetSpelling: construct.targetSpelling, origin: "ast", anchor: "exact" });
      }
      if (construct.relation === "accesses" && construct.targetSpelling === "glob") {
        add({ factType: "configuration", capability: "vite.file_set.glob", framework, relation: "configuration", sourceFile: construct.sourceFile, startByte: construct.startByte, endByte: construct.endByte, targetSpelling: construct.targetSpelling, origin: "ast", anchor: "exact" });
      }
    }
  }

  // The generic oracle intentionally leaves JavaScript config files outside a
  // project when a tsconfig does not enable allowJs.  Parse those bounded
  // config files directly with the same pinned compiler API so Vite evidence
  // is not lost merely because a repository's project references are narrow.
  if (framework === "vite") {
    const configFiles = [];
    const walk = (directory) => {
      for (const entry of fs.readdirSync(directory, { withFileTypes: true }).sort((left, right) => left.name.localeCompare(right.name, "en"))) {
        if (entry.name === ".git" || entry.name === "node_modules" || entry.name === "dist" || entry.name === "build") continue;
        const file = path.join(directory, entry.name);
        if (entry.isDirectory()) walk(file);
        else if (entry.isFile() && /^vite\.config\.[cm]?[jt]s$/u.test(entry.name)) configFiles.push(file);
      }
    };
    walk(root);
    for (const file of configFiles) {
      const relative = relativePath(root, file);
      const source = fs.readFileSync(file, "utf8");
      const scriptKind = file.endsWith(".ts") ? ts.ScriptKind.TS : file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.JS;
      const sourceFile = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, scriptKind);
      const pluginLocals = new Set(
        sourceFile.statements
          .filter((statement) => ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier) && statement.moduleSpecifier.text.includes("plugin"))
          .flatMap((statement) => {
            const clause = statement.importClause;
            if (!clause) return [];
            const names = clause.name ? [clause.name.text] : [];
            if (clause.namedBindings && ts.isNamedImports(clause.namedBindings)) names.push(...clause.namedBindings.elements.map((element) => element.name.text));
            return names;
          }),
      );
      const byteRange = (node) => ({
        startByte: Buffer.byteLength(source.slice(0, node.getStart(sourceFile)), "utf8"),
        endByte: Buffer.byteLength(source.slice(0, node.getEnd()), "utf8"),
      });
      // Record the module-specifier anchor independently of the compiler
      // project's import binding facts.  JavaScript config files frequently
      // sit outside a tsconfig's allowJs/include set, and the binding anchor
      // alone loses the identity that makes a plugin import useful to a
      // framework graph.  The fact is deduplicated with the generic stream
      // when that stream already observed the same module specifier.
      for (const statement of sourceFile.statements) {
        if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
        if (!statement.moduleSpecifier.text.includes("plugin")) continue;
        const range = byteRange(statement.moduleSpecifier);
        add({ factType: "configuration", capability: "vite.plugin.identity", framework, relation: "configuration", sourceFile: relative, ...range, targetSpelling: statement.moduleSpecifier.text, origin: "ast", anchor: "exact" });
      }
      const visit = (node) => {
        if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "defineConfig") {
          const range = byteRange(node.expression);
          add({ factType: "configuration", capability: "vite.config.factory", framework, relation: "configuration", sourceFile: relative, ...range, targetSpelling: "defineConfig", origin: "ast", anchor: "exact" });
        }
        if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && pluginLocals.has(node.expression.text)) {
          const range = byteRange(node.expression);
          add({ factType: "configuration", capability: "vite.plugin.identity", framework, relation: "configuration", sourceFile: relative, ...range, targetSpelling: node.expression.text, origin: "ast", anchor: "exact" });
        }
        if (isImportMetaGlobCall(node)) {
          const range = byteRange(node.expression);
          add({ factType: "configuration", capability: "vite.file_set.glob", framework, relation: "configuration", sourceFile: relative, ...range, targetSpelling: "import.meta.glob", origin: "ast", anchor: "exact" });
        }
        ts.forEachChild(node, visit);
      };
      visit(sourceFile);
    }
  }

  // Preserve a stratified compiler-occurrence pool for the scorecard.  These
  // are independently observed source relationships (not Compass output and
  // not framework claims) and keep the accepted-sample floor honest on
  // framework repositories whose config files are intentionally small.
  for (const construct of constructs) {
    if (!["imports", "calls", "accesses", "declares", "references", "reexports"].includes(construct.relation)) continue;
    add({
      factType: "source_observation",
      capability: `source.${construct.relation}`,
      framework,
      relation: "source_observation",
      sourceFile: construct.sourceFile,
      startByte: construct.startByte,
      endByte: construct.endByte,
      targetSpelling: construct.targetSpelling,
      origin: "ast",
      anchor: "exact",
    });
  }
  facts.sort((left, right) => JSON.stringify(left).localeCompare(JSON.stringify(right), "en"));
  return facts;
}

function main() {
  const { root, framework, output } = parseArguments(process.argv.slice(2));
  const oracle = runTypescriptOracle(root);
  const facts = sourceFacts(root, framework, oracle.records);
  const document = {
    schema: SCHEMA,
    provider: PROVIDER,
    toolchain: `node-${process.versions.node.split(".")[0]};typescript-${oracle.header.metadata?.compilerVersion ?? "5.9.3"}`,
    rootRelative: true,
    framework,
    sourceOracle: {
      schema: oracle.header.schema,
      provider: oracle.header.provider,
      sourceDigest: oracle.footer.sourceDigest,
      oracleSha256: sha256(oracle.raw),
      scannedFiles: oracle.header.scannedFiles,
      parsedFiles: oracle.header.parsedFiles,
      diagnosticCount: oracle.header.diagnosticCount,
      diagnostics: oracle.diagnostics.slice(0, 256),
      diagnosticsTruncated: Number(oracle.header.diagnosticCount) > oracle.diagnostics.length,
    },
    facts,
    limits: {
      maxFacts: MAX_FACTS,
      sourceOracleBounded: true,
      execution: "none",
    },
  };
  const encoded = `${JSON.stringify(document)}\n`;
  if (output) fs.writeFileSync(output, encoded, { encoding: "utf8", flag: "wx" });
  else process.stdout.write(encoded);
}

try {
  main();
} catch (error) {
  if (process.exitCode !== 2) fail(error instanceof Error ? error.message : String(error));
}
