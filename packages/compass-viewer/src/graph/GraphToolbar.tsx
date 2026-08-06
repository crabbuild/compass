import {
  ArrowLeftIcon,
  LayoutGridIcon,
  Maximize2Icon,
  PauseIcon,
  PlayIcon,
  RotateCcwIcon,
  TagsIcon
} from "lucide-react";
import type { GraphLayoutStyle } from "./renderingProfile";

export function GraphToolbar({
  status,
  physicsRunning,
  layoutStyle,
  forceLabels,
  onTogglePhysics,
  onLayoutChange,
  onFit,
  onReset,
  onToggleLabels,
  onBack
}: {
  status: string;
  physicsRunning: boolean;
  layoutStyle: GraphLayoutStyle;
  forceLabels: boolean;
  onTogglePhysics(): void;
  onLayoutChange(layout: GraphLayoutStyle): void;
  onFit(): void;
  onReset(): void;
  onToggleLabels(): void;
  onBack?: (() => void) | undefined;
}) {
  return (
    <div
      className="compass-graph-toolbar compass-glass-panel"
      role="toolbar"
      aria-label="Graph controls"
    >
      <div
        className="compass-viewer-status"
        data-state={physicsRunning ? "running" : "paused"}
        role="status"
        aria-live="polite"
      >
        <span className="compass-viewer-status-dot" aria-hidden="true" />
        <span className="compass-viewer-status-text">{status}</span>
      </div>
      <div className="compass-toolbar-actions">
        {onBack && (
          <button
            className="compass-tool-button"
            type="button"
            aria-label="Back to community overview"
            onClick={onBack}
          >
            <ArrowLeftIcon />
            <span>Overview</span>
          </button>
        )}
        <label className="compass-layout-picker">
          <LayoutGridIcon aria-hidden="true" />
          <span className="sr-only">Graph layout</span>
          <select
            aria-label="Graph layout"
            value={layoutStyle}
            onChange={(event) => onLayoutChange(event.target.value as GraphLayoutStyle)}
          >
            <option value="automatic">Automatic</option>
            <option value="circle">Circle</option>
            <option value="concentric">Concentric</option>
            <option value="spiral">Spiral</option>
            <option value="grid">Square grid</option>
          </select>
        </label>
        <button
          className="compass-tool-button"
          type="button"
          aria-label={physicsRunning
            ? "Pause layout"
            : layoutStyle === "automatic" ? "Resume layout" : "Fixed layout"}
          aria-pressed={physicsRunning}
          disabled={layoutStyle !== "automatic"}
          title={layoutStyle === "automatic"
            ? undefined
            : "Physics is available in Automatic layout"}
          onClick={onTogglePhysics}
        >
          {physicsRunning
            ? <PauseIcon />
            : layoutStyle === "automatic" ? <PlayIcon /> : <PauseIcon />}
          <span>{physicsRunning
            ? "Pause layout"
            : layoutStyle === "automatic" ? "Resume layout" : "Fixed layout"}</span>
        </button>
        <button
          className="compass-tool-button"
          type="button"
          aria-label="Fit graph in view"
          onClick={onFit}
        >
          <Maximize2Icon />
          <span>Fit graph</span>
        </button>
        <button
          className="compass-tool-button"
          type="button"
          aria-label="Reset graph view"
          onClick={onReset}
        >
          <RotateCcwIcon />
          <span>Reset view</span>
        </button>
        <button
          className="compass-tool-button"
          type="button"
          aria-label={forceLabels ? "Hide labels" : "Show labels"}
          aria-pressed={forceLabels}
          onClick={onToggleLabels}
        >
          <TagsIcon />
          <span>{forceLabels ? "Hide labels" : "Show labels"}</span>
        </button>
      </div>
    </div>
  );
}
