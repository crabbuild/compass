import {
  Maximize2Icon,
  PauseIcon,
  PlayIcon,
  RotateCcwIcon,
  TagsIcon
} from "lucide-react";

export function GraphToolbar({
  status,
  physicsRunning,
  forceLabels,
  onTogglePhysics,
  onFit,
  onReset,
  onToggleLabels
}: {
  status: string;
  physicsRunning: boolean;
  forceLabels: boolean;
  onTogglePhysics(): void;
  onFit(): void;
  onReset(): void;
  onToggleLabels(): void;
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
        <button
          className="compass-tool-button"
          type="button"
          aria-label={physicsRunning ? "Pause layout" : "Resume layout"}
          aria-pressed={physicsRunning}
          onClick={onTogglePhysics}
        >
          {physicsRunning ? <PauseIcon /> : <PlayIcon />}
          <span>{physicsRunning ? "Pause layout" : "Resume layout"}</span>
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
