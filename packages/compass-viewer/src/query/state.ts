import type { SourceLocation } from "../contracts/graph";

export type StructuredResult = {
  columns: string[];
  rows: string[][];
};

export type NaturalQuerySummary = {
  strategy: string;
  depth: number;
  starts: string[];
  total: number;
};

export type NaturalQueryEntry = {
  kind: string;
  label: string;
  community?: string | undefined;
  source?: SourceLocation | undefined;
};

export type NaturalQueryResult = {
  summary?: NaturalQuerySummary | undefined;
  entries: NaturalQueryEntry[];
  prose?: string | undefined;
};

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

export function parseNaturalQueryResult(text: string): NaturalQueryResult {
  const entries: NaturalQueryEntry[] = [];
  const prose: string[] = [];
  let summary: NaturalQuerySummary | undefined;

  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) continue;
    const parsedSummary = parseTraversalSummary(line);
    if (parsedSummary) {
      summary = parsedSummary;
      continue;
    }
    const entry = parseTraversalEntry(line);
    if (entry) {
      entries.push(entry);
    } else {
      prose.push(line);
    }
  }

  return {
    summary,
    entries,
    ...(prose.length > 0 ? { prose: prose.join("\n") } : {})
  };
}

function parseTraversalSummary(line: string): NaturalQuerySummary | undefined {
  const match = line.match(
    /^Traversal:\s*(\S+)\s+depth=(\d+)\s*\|\s*Start:\s*\[(.*)]\s*\|\s*(\d+)\s+nodes?\s+found$/i
  );
  if (!match) return undefined;
  const starts = [...match[3]!.matchAll(/['"]([^'"]+)['"]/g)]
    .map((candidate) => candidate[1]!)
    .filter(Boolean);
  return {
    strategy: match[1]!,
    depth: Number(match[2]),
    starts,
    total: Number(match[4])
  };
}

function parseTraversalEntry(line: string): NaturalQueryEntry | undefined {
  const match = line.match(/^([A-Z][A-Z_-]*)\s+(.+?)\s+\[(.*)]$/);
  if (!match) return undefined;
  const attributes = match[3]!;
  const file = attribute(attributes, "src");
  const location = attribute(attributes, "loc");
  const community = attribute(attributes, "community");
  const lineRange = location?.match(/^L(\d+)(?:[-:](?:L)?(\d+))?$/i);
  const source = file
    ? {
        file,
        ...(lineRange
          ? {
              startLine: Number(lineRange[1]),
              endLine: Number(lineRange[2] ?? lineRange[1])
            }
          : {})
      }
    : undefined;

  return {
    kind: match[1]!,
    label: match[2]!,
    ...(community ? { community } : {}),
    ...(source ? { source } : {})
  };
}

function attribute(attributes: string, name: string): string | undefined {
  const match = attributes.match(
    new RegExp(`(?:^|\\s)${name}=(.*?)(?=\\s+[A-Za-z_][\\w-]*=|$)`)
  );
  const value = match?.[1]?.trim();
  return value || undefined;
}
