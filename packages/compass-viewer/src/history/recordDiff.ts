import type { GraphRecordEvidence } from "../contracts/graph";

const PRESENTATION_FIELDS = new Set(["color", "change", "evidence"]);
const STRUCTURAL_DERIVED_FIELDS = new Set([
  "anchor",
  "anchors",
  "columnEnd",
  "columnStart",
  "community",
  "communityName",
  "community_name",
  "degree",
  "endByte",
  "endColumn",
  "endLine",
  "lineEnd",
  "lineStart",
  "line_end",
  "line_start",
  "location",
  "normLabel",
  "norm_label",
  "relationshipSite",
  "relationship_site",
  "sourceDigest",
  "sourceFile",
  "sourceHash",
  "source_digest",
  "source_file",
  "source_hash",
  "startByte",
  "startColumn",
  "startLine",
  "wiringSite",
  "wiring_site"
]);

export type StructuralRecordKind = "node" | "edge";

export function compareRecord(
  before: Record<string, unknown> | undefined,
  after: Record<string, unknown> | undefined
): GraphRecordEvidence {
  const canonicalBefore = canonicalRecord(before);
  const canonicalAfter = canonicalRecord(after);
  return {
    ...(canonicalBefore ? { before: canonicalBefore } : {}),
    ...(canonicalAfter ? { after: canonicalAfter } : {}),
    fields: collectChanges(canonicalBefore, canonicalAfter)
  };
}

/**
 * Compare graph meaning rather than storage or layout metadata. Exact record
 * diffs remain available through compareRecord and the history diff command.
 */
export function compareStructuralRecord(
  before: Record<string, unknown> | undefined,
  after: Record<string, unknown> | undefined,
  kind: StructuralRecordKind
): GraphRecordEvidence {
  const exactBefore = canonicalRecord(before);
  const exactAfter = canonicalRecord(after);
  const structuralBefore = canonicalStructuralRecord(before, kind);
  const structuralAfter = canonicalStructuralRecord(after, kind);
  return {
    ...(exactBefore ? { before: exactBefore } : {}),
    ...(exactAfter ? { after: exactAfter } : {}),
    fields: collectChanges(structuralBefore, structuralAfter)
  };
}

export function structuralRecordProjection(
  record: Record<string, unknown>,
  kind: StructuralRecordKind
): Record<string, unknown> {
  return canonicalStructuralRecord(record, kind) ?? {};
}

export function displayFieldValue(
  value: unknown,
  maxLength = 240
): { text: string; truncated: boolean } {
  const complete = value === undefined
    ? "Not recorded"
    : typeof value === "string"
      ? value
      : JSON.stringify(value, null, isStructured(value) ? 2 : undefined) ?? String(value);
  if (complete.length <= maxLength) return { text: complete, truncated: false };
  return {
    text: `${complete.slice(0, Math.max(0, maxLength - 1)).trimEnd()}…`,
    truncated: true
  };
}

function canonicalRecord(
  record: Record<string, unknown> | undefined
): Record<string, unknown> | undefined {
  if (!record) return undefined;
  return canonicalObject(record);
}

function canonicalStructuralRecord(
  record: Record<string, unknown> | undefined,
  kind: StructuralRecordKind
): Record<string, unknown> | undefined {
  if (!record) return undefined;
  const rootIgnored = kind === "node" ? new Set(["source"]) : new Set(["id", "key"]);
  return canonicalStructuralObject(record, rootIgnored);
}

function canonicalStructuralValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalStructuralValue);
  if (isStructured(value)) return canonicalStructuralObject(value);
  return value;
}

function canonicalStructuralObject(
  value: Record<string, unknown>,
  rootIgnored: ReadonlySet<string> = new Set()
): Record<string, unknown> {
  return Object.fromEntries(
    Object.keys(value)
      .filter((key) =>
        !PRESENTATION_FIELDS.has(key)
        && !STRUCTURAL_DERIVED_FIELDS.has(key)
        && !rootIgnored.has(key))
      .sort((left, right) => left.localeCompare(right))
      .map((key) => [key, canonicalStructuralValue(value[key])])
  );
}

function canonicalValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalValue);
  if (isStructured(value)) return canonicalObject(value);
  return value;
}

function canonicalObject(value: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(
    Object.keys(value)
      .filter((key) => !PRESENTATION_FIELDS.has(key))
      .sort((left, right) => left.localeCompare(right))
      .map((key) => [key, canonicalValue(value[key])])
  );
}

function collectChanges(
  before: Record<string, unknown> | undefined,
  after: Record<string, unknown> | undefined,
  prefix = ""
): GraphRecordEvidence["fields"] {
  const keys = new Set([
    ...Object.keys(before ?? {}),
    ...Object.keys(after ?? {})
  ]);
  const changes: GraphRecordEvidence["fields"] = [];
  for (const key of [...keys].sort((left, right) => left.localeCompare(right))) {
    const field = prefix ? `${prefix}.${key}` : key;
    const previous = before?.[key];
    const next = after?.[key];
    if (isStructured(previous) && isStructured(next)) {
      changes.push(...collectChanges(previous, next, field));
    } else if (!sameValue(previous, next)) {
      changes.push({
        field,
        ...(previous !== undefined ? { before: previous } : {}),
        ...(next !== undefined ? { after: next } : {})
      });
    }
  }
  return changes;
}

function sameValue(left: unknown, right: unknown): boolean {
  return JSON.stringify(canonicalValue(left)) === JSON.stringify(canonicalValue(right));
}

function isStructured(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
