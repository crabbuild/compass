import {
  AlertTriangleIcon,
  CompassIcon,
  RotateCcwIcon,
  SquareTerminalIcon
} from "lucide-react";

export type GraphLoadState =
  | { kind: "loading" }
  | { kind: "error"; message: string };

export function GraphLoadingState({
  state,
  onRetry,
  onShowOutput
}: {
  state: GraphLoadState;
  onRetry(): void;
  onShowOutput(): void;
}) {
  const loading = state.kind === "loading";
  return (
    <main className="compass-load-shell">
      <div
        className="compass-load-constellation"
        data-testid="graph-constellation"
        data-state={state.kind}
        aria-hidden="true"
      >
        <svg viewBox="0 0 240 180" focusable="false">
          <path className="compass-load-edge compass-load-edge-a" d="M28 112 73 73 119 90 169 49 211 77" />
          <path className="compass-load-edge compass-load-edge-b" d="M53 143 94 124 119 90 157 130 203 116" />
          <path className="compass-load-edge compass-load-edge-c" d="M73 73 94 124M169 49 157 130" />
          <circle className="compass-load-node compass-load-node-a" cx="28" cy="112" r="5" />
          <circle className="compass-load-node compass-load-node-b" cx="73" cy="73" r="7" />
          <circle className="compass-load-node compass-load-node-c" cx="53" cy="143" r="4" />
          <circle className="compass-load-node compass-load-node-d" cx="169" cy="49" r="5" />
          <circle className="compass-load-node compass-load-node-e" cx="211" cy="77" r="7" />
          <circle className="compass-load-node compass-load-node-f" cx="203" cy="116" r="4" />
          <circle className="compass-load-node compass-load-node-g" cx="157" cy="130" r="6" />
        </svg>
        <span className="compass-load-orbit" />
        <span className="compass-load-mark">
          {loading ? <CompassIcon /> : <AlertTriangleIcon />}
        </span>
      </div>

      <section
        className="compass-load-copy"
        role={loading ? "status" : "alert"}
        aria-live="polite"
      >
        <span className="compass-load-eyebrow">Compass graph</span>
        <h1>{loading ? "Mapping your codebase" : "Compass could not load this graph"}</h1>
        {loading ? (
          <p className="compass-load-steps">
            <span>Reading graph</span><b>·</b>
            <span>Arranging relationships</span><b>·</b>
            <span>Preparing inspector</span>
          </p>
        ) : (
          <>
            <p className="compass-load-error">{state.message}</p>
            <div className="compass-load-actions">
              <button
                className="compass-load-action compass-load-action-primary"
                type="button"
                onClick={onRetry}
              >
                <RotateCcwIcon aria-hidden="true" />
                Retry
              </button>
              <button
                className="compass-load-action"
                type="button"
                onClick={onShowOutput}
              >
                <SquareTerminalIcon aria-hidden="true" />
                Show Compass output
              </button>
            </div>
          </>
        )}
      </section>
    </main>
  );
}
