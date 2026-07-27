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
  activeStep?: number;
};

const DEFAULT_LOADING_COPY: GraphLoadingCopy = {
  eyebrow: "Compass graph",
  title: "Mapping your codebase",
  steps: ["Reading graph", "Arranging relationships", "Preparing inspector"],
  activeStep: 0
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
        {loading && (
          <svg className="compass-load-graph" viewBox="0 0 180 112">
            <path
              className="compass-load-edge compass-load-edge-a"
              d="M18 74 58 28 90 56"
            />
            <path
              className="compass-load-edge compass-load-edge-b"
              d="M90 56 132 20 162 62"
            />
            <path
              className="compass-load-edge compass-load-edge-c"
              d="M42 94 90 56 138 94"
            />
            <circle className="compass-load-node compass-load-node-a" cx="18" cy="74" r="4" />
            <circle className="compass-load-node compass-load-node-b" cx="58" cy="28" r="4" />
            <circle className="compass-load-node compass-load-node-c" cx="132" cy="20" r="4" />
            <circle className="compass-load-node compass-load-node-d" cx="162" cy="62" r="4" />
            <circle className="compass-load-node compass-load-node-e" cx="42" cy="94" r="4" />
            <circle className="compass-load-node compass-load-node-f" cx="138" cy="94" r="4" />
            <circle className="compass-load-tracer" r="3">
              <animateMotion
                dur="2.8s"
                repeatCount="indefinite"
                path="M18 74 58 28 90 56 132 20 162 62"
              />
            </circle>
          </svg>
        )}
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
            {loadingCopy.steps.map((step, index) => {
              const activeStep = loadingCopy.activeStep ?? 0;
              const stepState = index < activeStep
                ? "complete"
                : index === activeStep
                  ? "active"
                  : "pending";
              return (
                <span key={step} className="compass-load-step" data-state={stepState}>
                  <i aria-hidden="true" />
                  {step}
                </span>
              );
            })}
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
