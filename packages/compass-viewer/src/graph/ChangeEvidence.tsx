import { ExternalLinkIcon } from "lucide-react";
import {
  SourceLocationSchema,
  type GraphEdge,
  type GraphNode,
  type SourceLocation
} from "../contracts/graph";
import { displayFieldValue } from "../history/recordDiff";

export type GraphSourceRevisions = {
  before: string;
  after: string;
};

export function ChangeEvidence({
  node,
  edges,
  nodes,
  sourceRevisions,
  onFocus,
  onOpenSource
}: {
  node: GraphNode;
  edges: GraphEdge[];
  nodes: ReadonlyMap<string, GraphNode>;
  sourceRevisions?: GraphSourceRevisions | undefined;
  onFocus(nodeId: string): void;
  onOpenSource(source: SourceLocation, revision?: string): void;
}) {
  const beforeSource = evidenceSource(node.evidence?.before);
  const afterSource = evidenceSource(node.evidence?.after);

  return (
    <>
      {node.evidence?.fields.length ? (
        <section className="compass-record-evidence" aria-labelledby="compass-node-changes-title">
          <div className="compass-evidence-heading">
            <h3 id="compass-node-changes-title">{evidenceTitle(node.change)}</h3>
            <span>{node.evidence.fields.length} fields</span>
          </div>
          <FieldTable fields={node.evidence.fields} />
          {node.change === "changed" && (beforeSource || afterSource) && (
            <div className="compass-evidence-source-actions">
              {beforeSource && (
                <button
                  type="button"
                  onClick={() => onOpenSource(beforeSource, sourceRevisions?.before)}
                >
                  <ExternalLinkIcon aria-hidden="true" />
                  Open before
                </button>
              )}
              {afterSource && (
                <button
                  type="button"
                  onClick={() => onOpenSource(afterSource, sourceRevisions?.after)}
                >
                  <ExternalLinkIcon aria-hidden="true" />
                  Open after
                </button>
              )}
            </div>
          )}
        </section>
      ) : null}

      <section className="compass-relationship-evidence" aria-labelledby="compass-relationships-title">
        <div className="compass-neighbors-heading">
          <span id="compass-relationships-title">Relationships</span>
          <strong>{edges.length}</strong>
        </div>
        {edges.length ? (
          <div className="compass-relationship-list">
            {edges.map((edge) => {
              const otherId = edge.source === node.id ? edge.target : edge.source;
              const other = nodes.get(otherId);
              return (
                <details key={edge.id}>
                  <summary>
                    <span className="compass-change-badge" data-change={edge.change}>
                      {changeLabel(edge.change)}
                    </span>
                    <span className="compass-relationship-target">
                      {other?.label ?? otherId}
                    </span>
                    <small>{edge.relation}</small>
                    {edge.confidence && <i>{edge.confidence}</i>}
                  </summary>
                  <div className="compass-relationship-detail">
                    {edge.change === "changed" && edge.evidence?.fields.length ? (
                      <FieldTable fields={edge.evidence.fields} />
                    ) : (
                      <p>
                        {changeLabel(edge.change)} relationship: {edge.source} → {edge.target}
                      </p>
                    )}
                    <button type="button" onClick={() => onFocus(otherId)}>
                      Focus {other?.label ?? otherId}
                    </button>
                  </div>
                </details>
              );
            })}
          </div>
        ) : (
          <span className="compass-empty">No connected relationships</span>
        )}
      </section>
    </>
  );
}

function FieldTable({
  fields
}: {
  fields: NonNullable<GraphNode["evidence"]>["fields"];
}) {
  return (
    <div className="compass-field-table-wrap">
      <table className="compass-field-table">
        <thead>
          <tr><th>Field</th><th>Before</th><th>After</th></tr>
        </thead>
        <tbody>
          {fields.map((field) => (
            <tr key={field.field}>
              <th scope="row">{field.field}</th>
              <FieldValue value={field.before} />
              <FieldValue value={field.after} />
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function FieldValue({ value }: { value: unknown }) {
  const display = displayFieldValue(value);
  return (
    <td>
      <code>{display.text}</code>
      {display.truncated && <span className="compass-value-truncated">Value shortened</span>}
    </td>
  );
}

function evidenceSource(
  snapshot: Record<string, unknown> | undefined
): SourceLocation | undefined {
  const parsed = SourceLocationSchema.safeParse(snapshot?.source);
  return parsed.success ? parsed.data : undefined;
}

function changeLabel(change: GraphEdge["change"]): string {
  if (!change || change === "unchanged") return "Context";
  return `${change[0]?.toLocaleUpperCase()}${change.slice(1)}`;
}

function evidenceTitle(change: GraphNode["change"]): string {
  if (change === "added") return "Added symbol metadata";
  if (change === "removed") return "Removed symbol metadata";
  return "What changed";
}
