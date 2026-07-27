import {
  AlertTriangleIcon,
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

function CompassBrandMark() {
  return (
    <svg
      className="compass-load-logo"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        fill="currentColor"
        fillRule="evenodd"
        clipRule="evenodd"
        d="M3.554 21.529c1.797 1.221 4.943-.038 11.236-2.554 1.342-.537 2.013-.806 2.54-1.267q.201-.177.378-.378c.461-.527.73-1.198 1.267-2.54 2.515-6.293 3.775-9.44 2.554-11.236a4.1 4.1 0 0 0-1.083-1.083c-1.797-1.221-4.944.037-11.236 2.554-1.342.537-2.013.806-2.54 1.267q-.201.177-.378.378c-.461.527-.73 1.198-1.267 2.54-2.517 6.292-3.775 9.439-2.554 11.236.29.426.657.793 1.083 1.083M8.25 12a3.75 3.75 0 1 1 7.5 0 3.75 3.75 0 0 1-7.5 0m1.5 0a2.25 2.25 0 1 1 4.5 0 2.25 2.25 0 0 1-4.5 0"
      />
    </svg>
  );
}

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
        className="compass-load-visual"
        data-testid="compass-loading-logo"
        data-state={state.kind}
        aria-hidden="true"
      >
        <span className="compass-load-mark">
          {loading ? <CompassBrandMark /> : <AlertTriangleIcon />}
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
