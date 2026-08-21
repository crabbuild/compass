type JsonRecord = Record<string, unknown>;

export function queryFailureMessage(raw: string, exitCode: number): string {
  const trimmed = raw.trim();
  if (!trimmed) return `Compass query exited with code ${exitCode}`;
  const decoded = decodeJson(trimmed);
  const message = decoded === undefined ? undefined : nestedMessage(decoded);
  if (message) return sentence(message);
  if (decoded !== undefined) {
    return "Compass returned an error the extension could not interpret. Update Compass and the extension, then try again.";
  }
  return sentence(trimmed.replace(/^error:\s*/i, ""));
}

function decodeJson(value: string): unknown | undefined {
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return undefined;
  }
}

function nestedMessage(value: unknown): string | undefined {
  if (typeof value === "string") return value.trim() || undefined;
  if (!isRecord(value)) return undefined;
  for (const key of ["message", "detail", "reason"]) {
    const candidate = value[key];
    if (typeof candidate === "string" && candidate.trim()) return candidate.trim();
  }
  if (value.error !== undefined) {
    const candidate = nestedMessage(value.error);
    if (candidate) return candidate;
  }
  if (Array.isArray(value.diagnostics)) {
    for (const diagnostic of value.diagnostics) {
      const candidate = nestedMessage(diagnostic);
      if (candidate) return candidate;
    }
  }
  return undefined;
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sentence(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return trimmed;
  return trimmed[0]!.toLocaleUpperCase() + trimmed.slice(1);
}
