import type { GraphRecordEvidence } from "../contracts/graph";

const PRESENTATION_FIELDS = new Set(["color", "change", "evidence"]);

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
