import type { SourceLocation } from "../contracts/graph";

export type StructuredResult = {
  columns: string[];
  rows: string[][];
};

export type ExplanationConnection = {
  direction: "incoming" | "outgoing";
  label: string;
  relation: string;
  confidence: string;
};

export type ExplanationResult =
  | {
    kind: "node";
    label: string;
    id: string;
    source?: SourceLocation | undefined;
    type?: string | undefined;
    community?: string | undefined;
    degree?: number | undefined;
    connections: ExplanationConnection[];
  }
  | {
    kind: "ambiguous";
    title: string;
    candidates: Array<{ id: string; source?: string | undefined }>;
  }
  | { kind: "prose"; text: string };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object"
    && value !== null
    && !Array.isArray(value);
}

function displayValue(value: unknown): string {
  if (value === null) return "null";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean" || typeof value === "bigint") {
    return String(value);
  }
  if (value === undefined) return "";
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

export function normalizeStructuredResult(value: unknown): StructuredResult | undefined {
  if (!isRecord(value) || !Array.isArray(value.rows) || !value.rows.every(isRecord)) {
    return undefined;
  }
  const records = value.rows;
  const columns = [...new Set(records.flatMap((row) => Object.keys(row)))];
  if (columns.length === 0) return undefined;
  return {
    columns,
    rows: records.map((row) => columns.map((column) => displayValue(row[column])))
  };
}

export function parseExplanationResult(text: string): ExplanationResult {
  const lines = text.split(/\r?\n/);
  const first = lines[0]?.trim() ?? "";
  if (first.startsWith("Ambiguous:")) {
    const candidates: Array<{ id: string; source?: string | undefined }> = [];
    for (let index = 1; index < lines.length; index += 1) {
      const source = lines[index]?.trim();
      const id = lines[index + 1]?.trim().match(/^id:\s*(.+)$/)?.[1];
      if (!id) continue;
      candidates.push({ id, ...(source ? { source } : {}) });
      index += 1;
    }
    return { kind: "ambiguous", title: first, candidates };
  }
  const label = first.match(/^Node:\s*(.+)$/)?.[1];
  if (!label) return { kind: "prose", text };
  const fields = new Map<string, string>();
  const connections: ExplanationConnection[] = [];
  for (const rawLine of lines.slice(1)) {
    const line = rawLine.trim();
    const field = line.match(/^([A-Za-z]+):\s*(.*)$/);
    if (field && field[1] !== "Connections") {
      fields.set(field[1]!.toLocaleLowerCase(), field[2]!.trim());
      continue;
    }
    const connection = line.match(/^(-->|<--)\s+(.+?)\s+\[([^\]]+)]\s+\[([^\]]+)]/);
    if (connection) {
      connections.push({
        direction: connection[1] === "-->" ? "outgoing" : "incoming",
        label: connection[2]!,
        relation: connection[3]!,
        confidence: connection[4]!
      });
    }
  }
  const source = explanationSource(fields.get("source"));
  const degreeValue = fields.get("degree");
  const degree = degreeValue && /^\d+$/.test(degreeValue) ? Number(degreeValue) : undefined;
  return {
    kind: "node",
    label,
    id: fields.get("id") ?? label,
    ...(source ? { source } : {}),
    ...(fields.get("type") ? { type: fields.get("type") } : {}),
    ...(fields.get("community") ? { community: fields.get("community") } : {}),
    ...(degree !== undefined ? { degree } : {}),
    connections
  };
}

function explanationSource(value: string | undefined): SourceLocation | undefined {
  if (!value) return undefined;
  const match = value.match(/^(.*?)\s+(L\d+(?::\d+)?(?:-L?\d+(?::\d+)?)?)$/);
  const file = (match?.[1] ?? value).trim();
  if (!file) return undefined;
  const range = match?.[2]?.match(/^L(\d+)(?::\d+)?(?:-L?(\d+)(?::\d+)?)?$/);
  return {
    file,
    ...(range
      ? {
          startLine: Number(range[1]),
          endLine: Number(range[2] ?? range[1])
        }
      : {})
  };
}
