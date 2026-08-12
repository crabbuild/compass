import {
  ArrowLeftIcon,
  FocusIcon,
  LayoutGridIcon,
  Maximize2Icon,
  PauseIcon,
  PlayIcon,
  RotateCcwIcon,
  RouteIcon,
  TagsIcon,
  ZoomInIcon,
  ZoomOutIcon
} from "lucide-react";
import type { GraphLayoutStyle } from "./renderingProfile";

export function GraphToolbar({
  status,
  physicsRunning,
  layoutStyle,
  forceLabels,
  showEdgeLabels,
  hasSelection,
  onTogglePhysics,
  onLayoutChange,
  onZoomOut,
  onResetZoom,
  onZoomIn,
  onFit,
  onFitSelection,
  onReset,
  onToggleLabels,
  onToggleEdgeLabels,
  onBack
}: {
  status: string;
  physicsRunning: boolean;
  layoutStyle: GraphLayoutStyle;
  forceLabels: boolean;
  showEdgeLabels: boolean;
  hasSelection: boolean;
  onTogglePhysics(): void;
  onLayoutChange(layout: GraphLayoutStyle): void;
  onZoomOut(): void;
  onResetZoom(): void;
  onZoomIn(): void;
  onFit(): void;
  onFitSelection(): void;
  onReset(): void;
  onToggleLabels(): void;
  onToggleEdgeLabels(): void;
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
            <option value="hierarchical">Depth layers</option>
            <option value="circle">Circle</option>
            <option value="concentric">Concentric</option>
            <option value="spiral">Spiral</option>
            <option value="grid">Square grid</option>
          </select>
        </label>
        <button
          className="compass-tool-button compass-tool-icon-button"
          type="button"
          aria-label={physicsRunning
            ? "Pause layout"
            : layoutStyle === "automatic" ? "Resume layout" : "Fixed layout"}
          aria-pressed={physicsRunning}
          disabled={layoutStyle !== "automatic"}
          title={layoutStyle === "automatic"
            ? physicsRunning ? "Pause layout" : "Resume layout"
            : "Physics is available in Automatic layout"}
          onClick={onTogglePhysics}
        >
          {physicsRunning
            ? <PauseIcon />
            : layoutStyle === "automatic" ? <PlayIcon /> : <PauseIcon />}
        </button>
        <span className="compass-toolbar-separator" aria-hidden="true" />
        <div className="compass-zoom-controls" role="group" aria-label="Zoom controls">
          <button
            className="compass-tool-button compass-tool-icon-button"
            type="button"
            aria-label="Zoom out"
            title="Zoom out"
            onClick={onZoomOut}
          >
            <ZoomOutIcon />
          </button>
          <button
            className="compass-tool-button compass-zoom-reset"
            type="button"
            aria-label="Reset zoom to 100%"
            title="Reset zoom to 100%"
            onClick={onResetZoom}
          >
            100%
          </button>
          <button
            className="compass-tool-button compass-tool-icon-button"
            type="button"
            aria-label="Zoom in"
            title="Zoom in"
            onClick={onZoomIn}
          >
            <ZoomInIcon />
          </button>
        </div>
        <button
          className="compass-tool-button compass-tool-icon-button"
          type="button"
          aria-label="Fit graph in view"
          title="Fit graph in view"
          onClick={onFit}
        >
          <Maximize2Icon />
        </button>
        <button
          className="compass-tool-button compass-tool-icon-button"
          type="button"
          aria-label="Fit selected neighborhood"
          title={hasSelection
            ? "Fit the selected node and its immediate neighbors"
            : "Select a node to fit its neighborhood"}
          disabled={!hasSelection}
          onClick={onFitSelection}
        >
          <FocusIcon />
        </button>
        <button
          className="compass-tool-button compass-tool-icon-button"
          type="button"
          aria-label="Reset graph view"
          title="Reset graph view"
          onClick={onReset}
        >
          <RotateCcwIcon />
        </button>
        <button
          className="compass-tool-button compass-tool-icon-button"
          type="button"
          aria-label={forceLabels ? "Hide labels" : "Show labels"}
          aria-pressed={forceLabels}
          title={forceLabels ? "Hide node labels" : "Show all node labels"}
          onClick={onToggleLabels}
        >
          <TagsIcon />
        </button>
        <button
          className="compass-tool-button compass-tool-icon-button"
          type="button"
          aria-label={showEdgeLabels
            ? "Hide relationship labels"
            : "Show relationship labels"}
          aria-pressed={showEdgeLabels}
          title={showEdgeLabels
            ? "Hide relationship labels"
            : "Show relationship labels"}
          onClick={onToggleEdgeLabels}
        >
          <RouteIcon />
        </button>
      </div>
    </div>
  );
}
