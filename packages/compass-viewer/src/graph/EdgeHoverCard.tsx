import { ArrowRightIcon, FileCode2Icon, GitBranchIcon } from "lucide-react";
import type { CSSProperties } from "react";
import type { GraphEdge, GraphNode } from "../contracts/graph";
import { formatGraphRelation, formatRelationshipSite } from "./edgeLabels";
import { navigableRelationshipSource } from "./sourceNavigation";

export type GraphEdgeHover = {
  edgeId: string;
  x: number;
  y: number;
};

function displayConfidence(edge: GraphEdge): string {
  const confidence = edge.confidence;
  if (!confidence) return "Unspecified";
  return `${confidence[0]?.toLocaleUpperCase()}${confidence.slice(1)}`;
}

function displayChange(change: GraphEdge["change"]): string | undefined {
  if (!change) return undefined;
  return change === "unchanged"
    ? "Context"
    : `${change[0]?.toLocaleUpperCase()}${change.slice(1)}`;
}

export function EdgeHoverCard({
  edge,
  sourceNode,
  targetNode,
  hover
}: {
  edge: GraphEdge;
  sourceNode: GraphNode;
  targetNode: GraphNode;
  hover: GraphEdgeHover;
}) {
  const style = {
    "--compass-hover-x": `${hover.x + 18}px`,
    "--compass-hover-y": `${hover.y - 52}px`
  } as CSSProperties;
  const relationshipSite = formatRelationshipSite(edge.relationshipSite);
  const evidence = edge.codeEvidence?.[0];
  const change = displayChange(edge.change);
  const navigable = navigableRelationshipSource(edge) !== undefined;

  return (
    <div className="compass-edge-hover-card" role="tooltip" style={style}>
      <div className="compass-edge-hover-heading">
        <span className="compass-edge-hover-eyebrow">
          <GitBranchIcon aria-hidden="true" />
          Relationship
        </span>
        <span
          className="compass-edge-confidence"
          data-confidence={edge.confidence ?? "unspecified"}
        >
          {displayConfidence(edge)}
        </span>
      </div>

      <div className="compass-edge-trace" aria-label={`${sourceNode.label} to ${targetNode.label}`}>
        <strong>{sourceNode.label}</strong>
        <span aria-hidden="true"><ArrowRightIcon /></span>
        <strong>{targetNode.label}</strong>
      </div>

      <div className="compass-edge-relation-row">
        <strong>{formatGraphRelation(edge) || "Relationship"}</strong>
        {change && (
          <span className="compass-change-badge" data-change={edge.change}>{change}</span>
        )}
      </div>

      {(evidence || relationshipSite) && (
        <div className="compass-edge-metadata">
          {evidence && (
            <p>
              <span>Evidence</span>
              <code>{evidence.rule ?? evidence.origin}</code>
            </p>
          )}
          {relationshipSite && (
            <p>
              <span>Source</span>
              <code>{relationshipSite}</code>
            </p>
          )}
        </div>
      )}

      {navigable && (
        <p className="compass-hover-hint">
          <FileCode2Icon aria-hidden="true" />
          <span><strong>Double-click</strong> to open relationship source</span>
        </p>
      )}
    </div>
  );
}
