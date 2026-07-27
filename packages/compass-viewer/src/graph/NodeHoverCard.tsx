import type { CSSProperties } from "react";
import type { GraphNode } from "../contracts/graph";

export type GraphHover = {
  nodeId: string;
  x: number;
  y: number;
};

function sourceRange(node: GraphNode): string | undefined {
  const start = node.source?.startLine;
  const end = node.source?.endLine;
  if (start === undefined) return undefined;
  return end !== undefined && end !== start ? `${start}–${end}` : String(start);
}

export function NodeHoverCard({
  node,
  hover
}: {
  node: GraphNode;
  hover: GraphHover;
}) {
  const style = {
    "--compass-hover-x": `${hover.x + 18}px`,
    "--compass-hover-y": `${hover.y - 42}px`
  } as CSSProperties;
  const range = sourceRange(node);
  return (
    <div className="compass-node-hover-card" role="tooltip" style={style}>
      <div className="compass-hover-heading">
        <strong>{node.label}</strong>
        <span>{(node.kind ?? "symbol").toLocaleUpperCase()}</span>
      </div>
      {node.change && (
        <span className="compass-change-badge" data-change={node.change}>
          {node.change === "unchanged"
            ? "Context"
            : `${node.change[0]?.toLocaleUpperCase()}${node.change.slice(1)}`}
        </span>
      )}
      {node.memberCount !== undefined ? (
        <>
          <p>{node.memberCount.toLocaleString()} symbols</p>
          {node.change && node.change !== "unchanged" && (
            <p className="compass-hover-hint">Select to inspect exact changes</p>
          )}
        </>
      ) : (
        <>
          {node.language && <p>Language: {node.language}</p>}
          {node.source?.file && <p className="compass-hover-source">{node.source.file}</p>}
          {range && <p>Lines: {range}</p>}
          {node.signature && <code>{node.signature}</code>}
        </>
      )}
    </div>
  );
}
