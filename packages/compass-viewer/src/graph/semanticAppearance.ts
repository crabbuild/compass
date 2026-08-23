export type NodeSemanticCategory = "callable" | "type" | "module" | "boundary" | "other";

export type EdgeSemanticCategory = "execution" | "dependency" | "structure" | "flow" | "other";

export const NODE_SEMANTIC_CATEGORIES: readonly NodeSemanticCategory[] = [
  "callable",
  "type",
  "module",
  "boundary",
  "other"
];

export const EDGE_SEMANTIC_CATEGORIES: readonly EdgeSemanticCategory[] = [
  "execution",
  "dependency",
  "structure",
  "flow",
  "other"
];

const CALLABLE_KINDS = new Set([
  "closure",
  "constructor",
  "function",
  "lambda",
  "macro",
  "method",
  "procedure",
  "subroutine",
  "database_procedure"
]);
const TYPE_KINDS = new Set([
  "class",
  "component",
  "enum",
  "interface",
  "annotation",
  "protocol",
  "struct",
  "trait",
  "type",
  "type_alias",
  "union"
]);
const MODULE_KINDS = new Set([
  "directory",
  "document",
  "export",
  "file",
  "folder",
  "import",
  "migration",
  "module",
  "namespace",
  "package"
]);
const BOUNDARY_KINDS = new Set([
  "command",
  "config_key",
  "database",
  "database_column",
  "database_constraint",
  "database_index",
  "database_schema",
  "database_table",
  "database_trigger",
  "database_view",
  "endpoint",
  "event",
  "job",
  "message",
  "query",
  "queue",
  "resource",
  "route",
  "schema",
  "table",
  "topic"
]);

const EXECUTION_RELATIONS = new Set([
  "awaits",
  "calls",
  "dispatches",
  "executes",
  "handles",
  "instantiates",
  "invokes",
  "schedules",
  "tests",
  "triggers",
  "spawns"
]);
const DEPENDENCY_RELATIONS = new Set([
  "depends_on",
  "exports",
  "imports",
  "imports_from",
  "links",
  "re_exports",
  "references",
  "returns",
  "type_of",
  "aliases",
  "uses"
]);
const STRUCTURE_RELATIONS = new Set([
  "contains",
  "decorates",
  "declares",
  "documents",
  "embeds",
  "extends",
  "implements",
  "inherits",
  "member_of",
  "mixes_in",
  "overrides",
  "renders",
  "owns"
]);
const FLOW_RELATIONS = new Set([
  "consumes",
  "emits",
  "maps_to",
  "produces",
  "publishes",
  "reads",
  "receives",
  "registers",
  "routes_to",
  "sends",
  "subscribes",
  "writes"
]);

function normalizeSemanticName(value: string | undefined): string {
  return (value ?? "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

export function nodeSemanticCategory(kind: string | undefined): NodeSemanticCategory {
  const normalized = normalizeSemanticName(kind);
  if (CALLABLE_KINDS.has(normalized)) return "callable";
  if (TYPE_KINDS.has(normalized)) return "type";
  if (MODULE_KINDS.has(normalized)) return "module";
  if (BOUNDARY_KINDS.has(normalized)) return "boundary";
  return "other";
}

export function edgeSemanticCategory(relation: string): EdgeSemanticCategory {
  const normalized = normalizeSemanticName(relation);
  if (EXECUTION_RELATIONS.has(normalized)) return "execution";
  if (DEPENDENCY_RELATIONS.has(normalized)) return "dependency";
  if (STRUCTURE_RELATIONS.has(normalized)) return "structure";
  if (FLOW_RELATIONS.has(normalized)) return "flow";
  return "other";
}

export function nodeSemanticShape(category: NodeSemanticCategory) {
  if (category === "type") return "diamond" as const;
  if (category === "module") return "square" as const;
  if (category === "boundary") return "triangle" as const;
  return "dot" as const;
}

export function nodeSemanticCssColor(category: NodeSemanticCategory): string {
  if (category === "callable") {
    return "var(--vscode-symbolIcon-functionForeground, #5fa8ff)";
  }
  if (category === "type") {
    return "var(--vscode-symbolIcon-classForeground, #e3b341)";
  }
  if (category === "module") {
    return "var(--vscode-symbolIcon-moduleForeground, #56d4b4)";
  }
  if (category === "boundary") {
    return "var(--vscode-symbolIcon-eventForeground, #ff9b87)";
  }
  return "var(--vscode-descriptionForeground, #8b949e)";
}

export function edgeSemanticCssColor(category: EdgeSemanticCategory): string {
  if (category === "execution") {
    return "var(--vscode-symbolIcon-functionForeground, #5fa8ff)";
  }
  if (category === "dependency") {
    return "var(--vscode-symbolIcon-moduleForeground, #56d4b4)";
  }
  if (category === "structure") {
    return "var(--vscode-symbolIcon-classForeground, #e3b341)";
  }
  if (category === "flow") {
    return "var(--vscode-symbolIcon-eventForeground, #ff9b87)";
  }
  return "var(--vscode-descriptionForeground, #8b949e)";
}
