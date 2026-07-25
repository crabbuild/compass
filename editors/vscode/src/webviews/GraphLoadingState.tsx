import { Fragment } from "react";
import {
  AlertTriangleIcon,
  CompassIcon,
  RotateCcwIcon,
  SquareTerminalIcon
} from "lucide-react";

export type GraphLoadState =
  | { kind: "loading" }
  | { kind: "error"; message: string };

export type GraphLoadingCopy = {
  eyebrow: string;
  title: string;
  steps: readonly string[];
};

const DEFAULT_LOADING_COPY: GraphLoadingCopy = {
  eyebrow: "Compass graph",
  title: "Mapping your codebase",
  steps: ["Reading graph", "Arranging relationships", "Preparing inspector"]
};

export function GraphLoadingState({
  state,
  onRetry,
  onShowOutput,
  loadingCopy = DEFAULT_LOADING_COPY,
  variant = "graph"
}: {
  state: GraphLoadState;
  onRetry(): void;
  onShowOutput(): void;
  loadingCopy?: GraphLoadingCopy;
  variant?: "graph" | "architecture";
}) {
  const loading = state.kind === "loading";
  return (
    <main className="compass-load-shell" data-variant={variant}>
      <div
        className="compass-load-constellation"
        data-testid="graph-constellation"
        data-state={state.kind}
        aria-hidden="true"
      >
        <span className="compass-load-mark">
          {loading ? <CompassIcon /> : <AlertTriangleIcon />}
        </span>
        {loading && <span className="compass-load-progress"><i /></span>}
      </div>

      <section
        className="compass-load-copy"
        role={loading ? "status" : "alert"}
        aria-live="polite"
      >
        <span className="compass-load-eyebrow">
          {loading ? loadingCopy.eyebrow : "Compass graph"}
        </span>
        <h1>{loading ? loadingCopy.title : "Compass could not load this graph"}</h1>
        {loading ? (
          <p className="compass-load-steps">
            {loadingCopy.steps.map((step, index) => (
              <Fragment key={step}>
                {index > 0 && <b aria-hidden="true">·</b>}
                <span>{step}</span>
              </Fragment>
            ))}
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
      {variant === "architecture" && loading && (
        <div className="architecture-load-skeleton" aria-hidden="true">
          <span className="architecture-load-rail" />
          <span className="architecture-load-flow" />
          <span className="architecture-load-content" />
        </div>
      )}
    </main>
  );
}
