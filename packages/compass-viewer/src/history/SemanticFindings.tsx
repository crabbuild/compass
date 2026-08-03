import { ChevronRightIcon, FileDiffIcon, SparklesIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { Badge } from "../components/ui/badge";
import type { SemanticDiffReport } from "../contracts/history";
import { SourceChanges, type SourceChange } from "./SourceChanges";

const MAX_RENDERED_FINDINGS = 500;
const MAX_RENDERED_VALUES = 100;
const MAX_STRUCTURED_VALUE_CHARACTERS = 20_000;

export type SemanticEvidence = {
  sourceChanges: SourceChange[];
  findings: SemanticDiffReport["findings"];
};

export function semanticEvidence(report: SemanticDiffReport | undefined): SemanticEvidence {
  if (report === undefined) return { findings: [], sourceChanges: [] };
  return {
    findings: report.findings,
    sourceChanges: report.source_changes.map((change) => ({
      ...(change.old_path === null ? {} : { old_path: change.old_path }),
      ...(change.new_path === null ? {} : { new_path: change.new_path }),
      status: change.status,
      patch: change.patch
    }))
  };
}

export function SourceChangeEvidence({ report }: { report: SemanticDiffReport | undefined }) {
  const { sourceChanges } = semanticEvidence(report);
  return (
    <section className="history-evidence-panel" aria-labelledby="history-source-title">
      <header className="history-evidence-header">
        <div className="history-diff-heading">
          <FileDiffIcon aria-hidden="true" />
          <h2 id="history-source-title">Source changes</h2>
          <Badge variant="secondary">{sourceChanges.length}</Badge>
        </div>
        <p>Review the exact files and lines changed between these revisions.</p>
      </header>
      {sourceChanges.length > 0 ? (
        <SourceChanges changes={sourceChanges} />
      ) : (
        <p className="history-diff-empty">No source patch was reported for this comparison.</p>
      )}
    </section>
  );
}

export function SemanticFindings({ report }: { report: SemanticDiffReport | undefined }) {
  const { findings } = semanticEvidence(report);
  const visibleFindings = findings.slice(0, MAX_RENDERED_FINDINGS);
  const [openFindings, setOpenFindings] = useState<ReadonlySet<number>>(
    () => new Set(visibleFindings.length ? [0] : [])
  );

  useEffect(() => {
    setOpenFindings((current) => new Set(
      [...current].filter((index) => index < visibleFindings.length)
    ));
  }, [visibleFindings.length]);

  const allOpen = visibleFindings.length > 0 && openFindings.size === visibleFindings.length;

  return (
    <section className="history-evidence-panel" aria-labelledby="history-findings-title">
      <header className="history-evidence-header history-findings-header">
        <div>
          <div className="history-diff-heading">
            <SparklesIcon aria-hidden="true" />
            <h2 id="history-findings-title">Semantic findings</h2>
            <Badge variant="secondary">{findings.length}</Badge>
          </div>
          <p>Inspect behavior and relationship changes inferred from the graph comparison.</p>
        </div>
        {findings.length > 1 && (
          <button
            type="button"
            className="history-findings-expand"
            onClick={() => setOpenFindings(
              allOpen ? new Set() : new Set(visibleFindings.map((_, index) => index))
            )}
          >
            {allOpen ? "Collapse all" : "Expand all"}
          </button>
        )}
      </header>
      {visibleFindings.length > 0 ? (
        <div className="history-finding-list">
          {visibleFindings.map((finding, index) => {
            const details = findingDetails(finding);
            return (
              <details
                key={index}
                open={openFindings.has(index)}
                onToggle={(event) => {
                  const nextOpen = event.currentTarget.open;
                  setOpenFindings((current) => {
                    const next = new Set(current);
                    if (nextOpen) next.add(index);
                    else next.delete(index);
                    return next;
                  });
                }}
              >
                <summary>
                  <span className="history-finding-index" aria-hidden="true">
                    {index + 1}
                  </span>
                  <span className="history-finding-summary">
                    <strong>{findingSummary(finding, index)}</strong>
                    <small>
                      {details
                        ? `${Object.keys(details).length} evidence ${Object.keys(details).length === 1 ? "field" : "fields"}`
                        : "Summary only"}
                    </small>
                  </span>
                  <ChevronRightIcon aria-hidden="true" />
                </summary>
                <div className="history-finding-body">
                  {details ? (
                    <dl>
                      {Object.entries(details).map(([key, value]) => (
                        <div key={key}>
                          <dt>{humanize(key)}</dt>
                          <dd>{renderFindingValue(value)}</dd>
                        </div>
                      ))}
                    </dl>
                  ) : (
                    <p>No additional structured evidence was reported for this finding.</p>
                  )}
                </div>
              </details>
            );
          })}
          {findings.length > visibleFindings.length && (
            <p className="history-diff-empty" role="note">
              Showing the first {visibleFindings.length.toLocaleString()} of {findings.length.toLocaleString()} findings. Export the HTML or JSON semantic diff for the exhaustive report.
            </p>
          )}
        </div>
      ) : (
        <div className="history-findings-empty">
          <SparklesIcon aria-hidden="true" />
          <div>
            <strong>No semantic findings</strong>
            <p>The comparison did not report relationship or behavior-level changes.</p>
          </div>
        </div>
      )}
    </section>
  );
}

function findingSummary(finding: unknown, index: number): string {
  if (finding && typeof finding === "object") {
    const record = finding as Record<string, unknown>;
    for (const key of ["headline", "summary", "title", "message"]) {
      if (typeof record[key] === "string") return record[key];
    }
  }
  return `Finding ${index + 1}`;
}

function findingDetails(finding: unknown): Record<string, unknown> | undefined {
  if (!finding || typeof finding !== "object" || Array.isArray(finding)) return undefined;
  const details = Object.fromEntries(
    Object.entries(finding as Record<string, unknown>)
      .filter(([key]) => !["headline", "summary", "title", "message"].includes(key))
  );
  return Object.keys(details).length > 0 ? details : undefined;
}

function humanize(value: string): string {
  const words = value
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replaceAll("_", " ")
    .replaceAll("-", " ");
  return words.charAt(0).toLocaleUpperCase() + words.slice(1);
}

function renderFindingValue(value: unknown) {
  if (value === null || value === undefined) {
    return <span className="history-finding-muted">Not reported</span>;
  }
  if (Array.isArray(value)) {
    if (value.length === 0) return <span className="history-finding-muted">None</span>;
    const visible = value.slice(0, MAX_RENDERED_VALUES);
    return (
      <ul>
        {visible.map((item, index) => (
          <li key={index}>{renderFindingValue(item)}</li>
        ))}
        {value.length > visible.length && (
          <li className="history-finding-muted">
            {value.length - visible.length} additional values omitted from this bounded preview
          </li>
        )}
      </ul>
    );
  }
  if (typeof value === "object") {
    const json = JSON.stringify(value, null, 2);
    const bounded = json.length > MAX_STRUCTURED_VALUE_CHARACTERS
      ? `${json.slice(0, MAX_STRUCTURED_VALUE_CHARACTERS)}\n… structured value shortened`
      : json;
    return <pre>{bounded}</pre>;
  }
  if (typeof value === "boolean") return <code>{value ? "Yes" : "No"}</code>;
  if (typeof value === "number") return <code>{value.toLocaleString()}</code>;
  return <span>{String(value)}</span>;
}
