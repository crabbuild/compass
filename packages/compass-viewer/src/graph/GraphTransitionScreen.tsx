import { useEffect, useState } from "react";
import { CompassBrandMark } from "./CompassBrandMark";

export const SLOW_LAYOUT_DELAY_MS = 10_000;

export type GraphTransitionScreenProps =
  | {
    kind: "layout";
    onShowGraph(): void;
  }
  | {
    kind: "community";
    communityLabel: string;
  };

export function GraphTransitionScreen(props: GraphTransitionScreenProps) {
  const [slowLayout, setSlowLayout] = useState(false);

  useEffect(() => {
    if (props.kind !== "layout") {
      setSlowLayout(false);
      return;
    }

    const timeout = window.setTimeout(() => {
      setSlowLayout(true);
    }, SLOW_LAYOUT_DELAY_MS);
    return () => window.clearTimeout(timeout);
  }, [props.kind]);

  const isCommunity = props.kind === "community";
  const title = isCommunity
    ? `Opening ${props.communityLabel}`
    : slowLayout
      ? "Still arranging this graph"
      : "Arranging graph layout";
  const description = isCommunity
    ? "Fetching symbols and relationships for this community."
    : slowLayout
      ? "This graph is taking longer than expected. You can open the current layout now."
      : "Positioning nodes and relationships for a readable first view.";

  return (
    <section
      className="compass-graph-transition"
      data-kind={props.kind}
      role="status"
      aria-live="polite"
      aria-atomic="true"
    >
      <div
        className="compass-load-visual"
        data-state="loading"
        aria-hidden="true"
      >
        <span className="compass-load-mark">
          <CompassBrandMark />
        </span>
        <span className="compass-load-progress"><i /></span>
      </div>
      <div className="compass-load-copy">
        <span className="compass-load-eyebrow">Compass graph</span>
        <h1>{title}</h1>
        <p className="compass-graph-transition-description">{description}</p>
        {!isCommunity && slowLayout && (
          <div className="compass-load-actions">
            <button
              className="compass-load-action compass-load-action-primary"
              type="button"
              onClick={props.onShowGraph}
            >
              Show graph now
            </button>
          </div>
        )}
      </div>
    </section>
  );
}
