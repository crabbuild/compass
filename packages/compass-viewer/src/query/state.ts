export type StructuredResult = {
  columns: string[];
  rows: string[][];
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
