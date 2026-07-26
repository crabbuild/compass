import { FileDiffIcon, SparklesIcon } from "lucide-react";
import { Badge } from "../components/ui/badge";
import { SourceChanges, type SourceChange } from "./SourceChanges";

export function SemanticFindings({ report }: { report: unknown }) {
  const value = report && typeof report === "object"
    ? report as Record<string, unknown>
    : {};
  const findings = Array.isArray(value.findings) ? value.findings : [];
  const sourceChanges = Array.isArray(value.source_changes)
    ? value.source_changes.filter(isSourceChange)
    : [];
  return (
    <section className="history-diff-evidence">
      <div className="history-diff-heading">
        <FileDiffIcon aria-hidden="true" />
        <h2>Source changes</h2>
        <Badge variant="secondary">{sourceChanges.length}</Badge>
      </div>
      {sourceChanges.length > 0 ? (
        <SourceChanges changes={sourceChanges} />
      ) : (
        <p className="history-diff-empty">No source patch was reported for this comparison.</p>
      )}
      <div className="history-diff-heading history-semantic-heading">
        <SparklesIcon aria-hidden="true" />
        <h2>Semantic findings</h2>
        <Badge variant="secondary">{findings.length}</Badge>
      </div>
      {findings.length > 0 ? (
        <div className="history-finding-list">
          {findings.map((finding, index) => (
            <article key={index}>
              <strong>{findingSummary(finding, index)}</strong>
              {findingDetails(finding) && (
                <details>
                  <summary>Finding details</summary>
                  <pre>{JSON.stringify(findingDetails(finding), null, 2)}</pre>
                </details>
              )}
            </article>
          ))}
        </div>
      ) : (
        <p className="history-diff-empty">No semantic graph findings for this comparison.</p>
      )}
    </section>
  );
}

function isSourceChange(value: unknown): value is SourceChange {
  if (value === null || typeof value !== "object") return false;
  const change = value as Record<string, unknown>;
  return ["old_path", "new_path", "status", "patch"].every(
    (key) => change[key] === undefined || typeof change[key] === "string"
  );
}

function findingSummary(finding: unknown, index: number): string {
  if (finding && typeof finding === "object") {
    const record = finding as Record<string, unknown>;
    for (const key of ["summary", "title", "message"]) {
      if (typeof record[key] === "string") return record[key];
    }
  }
  return `Finding ${index + 1}`;
}

function findingDetails(finding: unknown): Record<string, unknown> | undefined {
  if (!finding || typeof finding !== "object" || Array.isArray(finding)) return undefined;
  const details = Object.fromEntries(
    Object.entries(finding as Record<string, unknown>)
      .filter(([key]) => !["summary", "title", "message"].includes(key))
  );
  return Object.keys(details).length > 0 ? details : undefined;
}
