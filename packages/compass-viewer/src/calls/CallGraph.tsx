import { useState } from "react";
import {
  ArrowDownIcon,
  ArrowUpIcon,
  GitForkIcon,
  PlusIcon,
  TriangleAlertIcon
} from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "../components/ui/alert";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import type { SourceLocation } from "../contracts/graph";
import type { CallDirection, CallGraphResponse } from "../contracts/callGraph";
import { CallCanvas } from "./CallCanvas";
import { CoverageNotice } from "./CoverageNotice";

export type CallGraphHost = {
  openSource(source: SourceLocation): void;
  expand(symbol: string, direction: CallDirection, depth: number): void;
};

export function CallGraph({
  graph,
  host
}: {
  graph: CallGraphResponse;
  host: CallGraphHost;
}) {
  const [showAllContinuations, setShowAllContinuations] = useState(false);
  const visibleContinuations = showAllContinuations
    ? graph.continuations
    : graph.continuations.slice(0, 20);
  return (
    <div className="relative h-screen">
      <CallCanvas graph={graph} host={host} />
      <section className="absolute bottom-3 left-3 z-20 flex max-w-[min(44rem,calc(100%-1.5rem))] flex-col gap-2 rounded-md border bg-popover/95 p-3 text-popover-foreground shadow-xl backdrop-blur">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="outline"><GitForkIcon /> depth {graph.depth}</Badge>
          <Badge variant="outline">
            {graph.nodes.length} {graph.nodes.length === 1 ? "node" : "nodes"}
          </Badge>
          <Badge variant="outline">
            {graph.edges.length} {graph.edges.length === 1 ? "edge" : "edges"}
          </Badge>
          <Badge variant="secondary">{graph.coverage.resolved} resolved</Badge>
          <Badge variant="outline">{graph.coverage.inferred} inferred</Badge>
          <Badge variant="outline">{graph.coverage.ambiguous} ambiguous</Badge>
          <Badge variant="destructive">{graph.coverage.unresolved} unresolved</Badge>
        </div>
        {graph.truncated && (
          <Alert>
            <TriangleAlertIcon />
            <AlertTitle>Partial call graph</AlertTitle>
            <AlertDescription>
              Compass reached the configured graph limit. Counts and paths may be incomplete.
            </AlertDescription>
          </Alert>
        )}
        <CoverageNotice coverage={graph.coverage} />
        {graph.continuations.length > 0 && (
          <div>
            <div className="flex max-h-24 flex-wrap gap-1 overflow-auto" aria-label="Call graph continuations">
              {visibleContinuations.map((continuation) => (
                <Button
                  key={`${continuation.symbol}:${continuation.direction}:${continuation.nextDepth}`}
                  size="xs"
                  variant="outline"
                  onClick={() => host.expand(
                    continuation.symbol,
                    continuation.direction,
                    continuation.nextDepth
                  )}
                >
                  {continuation.direction === "callers"
                    ? <ArrowUpIcon /> : continuation.direction === "callees"
                      ? <ArrowDownIcon /> : <PlusIcon />}
                  Expand {continuation.direction}
                </Button>
              ))}
            </div>
            {!showAllContinuations
              && graph.continuations.length > visibleContinuations.length && (
              <div className="mt-2 flex flex-wrap items-center justify-between gap-2">
                <p className="text-xs text-muted-foreground" role="status">
                  Showing {visibleContinuations.length} of {graph.continuations.length} continuations
                </p>
                <Button
                  size="xs"
                  variant="outline"
                  onClick={() => setShowAllContinuations(true)}
                >
                  Show all {graph.continuations.length} continuations
                </Button>
              </div>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
