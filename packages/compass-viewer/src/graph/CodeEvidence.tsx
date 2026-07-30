import {
  BadgeCheckIcon,
  CircleHelpIcon,
  ExternalLinkIcon,
  FileCogIcon,
  GitForkIcon,
  SignpostIcon,
  TriangleAlertIcon
} from "lucide-react";
import type {
  CodeEvidenceRecord,
  CodeQueryDiagnostic,
  CodeSourceAnchor
} from "../contracts/codeQuery";

export function CodeEvidence({
  evidence,
  diagnostics = [],
  truncated = false,
  title = "Trust evidence",
  onOpenSource
}: {
  evidence: readonly CodeEvidenceRecord[];
  diagnostics?: readonly CodeQueryDiagnostic[] | undefined;
  truncated?: boolean | undefined;
  title?: string | undefined;
  onOpenSource?: ((source: CodeSourceAnchor) => void) | undefined;
}) {
  if (!evidence.length && !diagnostics.length && !truncated) return null;
  return (
    <section className="compass-code-evidence" aria-label={title}>
      <div className="compass-evidence-heading">
        <h3>{title}</h3>
        <span>{evidence.length} {evidence.length === 1 ? "record" : "records"}</span>
      </div>
      {truncated && (
        <p className="compass-code-diagnostic" data-severity="warning" role="status">
          <TriangleAlertIcon aria-hidden="true" />
          Results reached a configured limit. Narrow the query or raise one bound.
        </p>
      )}
      {diagnostics.map((diagnostic) => (
        <p
          className="compass-code-diagnostic"
          data-severity={diagnostic.code === "no_match" ? "info" : "warning"}
          key={`${diagnostic.code}:${diagnostic.message}`}
          role="status"
        >
          {diagnostic.code === "no_match"
            ? <CircleHelpIcon aria-hidden="true" />
            : <TriangleAlertIcon aria-hidden="true" />}
          <span><strong>{diagnosticLabel(diagnostic.code)}</strong>{diagnostic.message}</span>
        </p>
      ))}
      <div className="compass-code-evidence-ledger">
        {evidence.map((item, index) => {
          const status = evidenceStatus(item);
          const source = item.wiringSite ?? item.anchor;
          return (
            <article
              className="compass-code-evidence-entry"
              data-status={status.key}
              key={`${item.layer}:${item.extractor}:${index}`}
            >
              <span className="compass-code-evidence-icon" aria-hidden="true">
                <EvidenceIcon item={item} />
              </span>
              <div className="compass-code-evidence-copy">
                <div className="compass-code-evidence-status">
                  <strong>{status.label}</strong>
                  <span>{item.layer === "program_ir" ? "Program IR" : "Code graph"}</span>
                </div>
                <dl>
                  <div><dt>Extractor</dt><dd>{item.extractor}</dd></div>
                  <div><dt>Confidence</dt><dd>{item.confidence}</dd></div>
                  {item.rule && <div><dt>Rule</dt><dd>{item.rule}</dd></div>}
                </dl>
                {source && (
                  onOpenSource ? (
                    <button
                      className="compass-code-evidence-source"
                      type="button"
                      onClick={() => onOpenSource(source)}
                    >
                      <ExternalLinkIcon aria-hidden="true" />
                      {sourceLabel(item, source)}
                    </button>
                  ) : (
                    <span className="compass-code-evidence-source">
                      {sourceLabel(item, source)}
                    </span>
                  )
                )}
                {item.candidates.length > 0 && (
                  <div className="compass-code-candidates">
                    <span>Retained candidates</span>
                    <ul>
                      {item.candidates.map((candidate) => (
                        <li key={`${candidate.nodeId}:${candidate.reason}`}>
                          <code>{candidate.nodeId}</code>
                          <span>{candidate.reason}</span>
                          <small>{candidate.confidence}</small>
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}

function EvidenceIcon({ item }: { item: CodeEvidenceRecord }) {
  if (item.resolution === "unresolved") return <CircleHelpIcon />;
  if (item.resolution === "ambiguous" || item.confidence === "ambiguous") {
    return <TriangleAlertIcon />;
  }
  if (item.origin === "heuristic") return <GitForkIcon />;
  if (item.origin === "config") return <FileCogIcon />;
  if (item.origin === "convention") return <SignpostIcon />;
  return <BadgeCheckIcon />;
}

function evidenceStatus(item: CodeEvidenceRecord): { key: string; label: string } {
  if (item.resolution === "unresolved") return { key: "unresolved", label: "Unresolved" };
  if (item.resolution === "ambiguous" || item.confidence === "ambiguous") {
    return { key: "ambiguous", label: "Ambiguous" };
  }
  if (item.origin === "heuristic") return { key: "heuristic", label: "Heuristic" };
  if (item.origin === "config") return { key: "config", label: "Configuration" };
  if (item.origin === "convention") return { key: "convention", label: "Convention" };
  return { key: "exact", label: "Exact" };
}

function sourceLabel(item: CodeEvidenceRecord, source: CodeSourceAnchor): string {
  const prefix = item.origin === "heuristic" ? "Wired at" : "Evidence at";
  return `${prefix} ${source.file}:${source.startLine}`;
}

function diagnosticLabel(code: CodeQueryDiagnostic["code"]): string {
  return `${code.replaceAll("_", " ")}: `;
}
