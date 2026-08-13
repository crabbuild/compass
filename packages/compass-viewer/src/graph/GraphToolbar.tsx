import {
  ArrowLeftIcon,
  FocusIcon,
  KeyboardIcon,
  LayoutGridIcon,
  MapIcon,
  Maximize2Icon,
  PauseIcon,
  PlayIcon,
  RotateCcwIcon,
  RouteIcon,
  ScanSearchIcon,
  SettingsIcon,
  TagsIcon,
  ZoomInIcon,
  ZoomOutIcon
} from "lucide-react";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import type { GraphEdgeDirection } from "./neighborhood";
import type { GraphLayoutStyle } from "./renderingProfile";
import type { GraphLayoutSpacing } from "./state";

function isEditableTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLElement
    && (target.isContentEditable
      || target.tagName === "INPUT"
      || target.tagName === "TEXTAREA"
      || target.tagName === "SELECT");
}

export function GraphToolbar({
  status,
  physicsRunning,
  layoutStyle,
  forceLabels,
  showEdgeLabels,
  hasSelection,
  isolateSelection,
  neighborhoodDepth,
  edgeDirection,
  layoutSpacing,
  showMinimap,
  leadingControls,
  leadingPanel,
  leadingPanelOpen,
  onLeadingPanelClose,
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
  onToggleIsolation,
  onNeighborhoodDepthChange,
  onEdgeDirectionChange,
  onLayoutSpacingChange,
  onToggleMinimap,
  onBack
}: {
  status: string;
  physicsRunning: boolean;
  layoutStyle: GraphLayoutStyle;
  forceLabels: boolean;
  showEdgeLabels: boolean;
  hasSelection: boolean;
  isolateSelection: boolean;
  neighborhoodDepth: number;
  edgeDirection: GraphEdgeDirection;
  layoutSpacing: GraphLayoutSpacing;
  showMinimap: boolean;
  leadingControls?: ReactNode;
  leadingPanel?: ReactNode;
  leadingPanelOpen?: boolean | undefined;
  onLeadingPanelClose?: (() => void) | undefined;
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
  onToggleIsolation(): void;
  onNeighborhoodDepthChange(depth: number): void;
  onEdgeDirectionChange(direction: GraphEdgeDirection): void;
  onLayoutSpacingChange(spacing: GraphLayoutSpacing): void;
  onToggleMinimap(): void;
  onBack?: (() => void) | undefined;
}) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const explorationId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  const settingsButtonRef = useRef<HTMLButtonElement>(null);
  const restoreSettingsFocusRef = useRef(false);
  useEffect(() => {
    if (leadingPanelOpen) setSettingsOpen(false);
  }, [leadingPanelOpen]);
  useEffect(() => {
    if (settingsOpen || !restoreSettingsFocusRef.current) return;
    restoreSettingsFocusRef.current = false;
    settingsButtonRef.current?.focus();
  }, [settingsOpen]);
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (settingsOpen) {
          restoreSettingsFocusRef.current = true;
          setSettingsOpen(false);
        }
        return;
      }
      if (event.key !== "?" || isEditableTarget(event.target)) return;
      event.preventDefault();
      onLeadingPanelClose?.();
      setSettingsOpen(true);
    };
    const handlePointerDown = (event: PointerEvent) => {
      if (panelRef.current?.contains(event.target as Node)) return;
      setSettingsOpen(false);
    };
    document.addEventListener("keydown", handleKeyDown);
    document.addEventListener("pointerdown", handlePointerDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      document.removeEventListener("pointerdown", handlePointerDown);
    };
  }, [onLeadingPanelClose, settingsOpen]);

  return (
    <div
      ref={panelRef}
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
          className="compass-tool-button compass-physics-button"
          type="button"
          aria-label={physicsRunning
            ? "Stop layout"
            : layoutStyle === "automatic" ? "Run layout" : "Fixed layout"}
          aria-pressed={physicsRunning}
          disabled={layoutStyle !== "automatic"}
          title={layoutStyle === "automatic"
            ? physicsRunning ? "Stop layout" : "Run layout"
            : "Physics is available in Automatic layout"}
          onClick={onTogglePhysics}
        >
          {physicsRunning
            ? <PauseIcon />
            : layoutStyle === "automatic" ? <PlayIcon /> : <PauseIcon />}
          <span>{physicsRunning
            ? "Stop"
            : layoutStyle === "automatic" ? "Layout" : "Fixed"}</span>
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
            ? `Fit the selected node and its ${neighborhoodDepth}-hop neighborhood`
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
        <button
          ref={settingsButtonRef}
          className="compass-tool-button compass-tool-icon-button"
          type="button"
          aria-label="Graph settings"
          aria-expanded={settingsOpen}
          aria-controls={explorationId}
          title="Graph settings (?)"
          onClick={() => setSettingsOpen((open) => {
            const next = !open;
            if (next) onLeadingPanelClose?.();
            return next;
          })}
        >
          <SettingsIcon />
        </button>
        {leadingControls ? (
          <div className="compass-toolbar-leading">{leadingControls}</div>
        ) : null}
      </div>
      {leadingPanel}
      {settingsOpen ? (
        <div
          id={explorationId}
          className="compass-explore-panel compass-glass-panel"
          role="region"
          aria-label="Graph exploration controls"
        >
          <div className="compass-explore-heading">
            <div>
              <strong>Graph settings</strong>
              <span>{hasSelection
                ? "Tune the selected neighborhood and canvas"
                : "Set the next neighborhood, then select a node"}</span>
            </div>
            <kbd>?</kbd>
          </div>

          {!hasSelection ? (
            <div className="compass-selection-prompt" role="note">
              <ScanSearchIcon aria-hidden="true" />
              <span>
                <strong>Select a node to isolate it</strong>
                <small>Use the canvas or node search. Depth and direction can be set now.</small>
              </span>
            </div>
          ) : null}

          <button
            className="compass-explore-toggle"
            type="button"
            aria-label="Isolate selection"
            aria-pressed={isolateSelection}
            disabled={!hasSelection}
            onClick={onToggleIsolation}
          >
            <ScanSearchIcon aria-hidden="true" />
            <span>
              <strong>Isolate selection</strong>
              <small>Hide everything outside the traversal</small>
            </span>
            <i aria-hidden="true" />
          </button>

          <fieldset className="compass-explore-field">
            <legend>Neighborhood depth</legend>
            <div className="compass-segmented-control" aria-label="Neighborhood depth">
              {[1, 2, 3, 4].map((depth) => (
                <button
                  key={depth}
                  type="button"
                  aria-label={`${depth} hop${depth === 1 ? "" : "s"}`}
                  aria-pressed={neighborhoodDepth === depth}
                  onClick={() => onNeighborhoodDepthChange(depth)}
                >
                  {depth}
                </button>
              ))}
            </div>
          </fieldset>

          <fieldset className="compass-explore-field">
            <legend>Edge direction</legend>
            <div className="compass-segmented-control" aria-label="Edge direction">
              {([
                ["both", "Both"],
                ["outgoing", "Out"],
                ["incoming", "In"]
              ] as const).map(([direction, label]) => (
                <button
                  key={direction}
                  type="button"
                  aria-label={`${label === "Both" ? "Both directions" : label === "Out" ? "Outgoing edges" : "Incoming edges"}`}
                  aria-pressed={edgeDirection === direction}
                  onClick={() => onEdgeDirectionChange(direction)}
                >
                  {label}
                </button>
              ))}
            </div>
          </fieldset>

          <label className="compass-explore-select">
            <span>Layout spacing</span>
            <select
              aria-label="Layout spacing"
              value={layoutSpacing}
              onChange={(event) => onLayoutSpacingChange(
                Number(event.target.value) as GraphLayoutSpacing
              )}
            >
              <option value={0.75}>Compact · 75%</option>
              <option value={1}>Default · 100%</option>
              <option value={1.25}>Airy · 125%</option>
              <option value={1.5}>Wide · 150%</option>
            </select>
          </label>

          <button
            className="compass-explore-toggle"
            type="button"
            aria-label="Show minimap"
            aria-pressed={showMinimap}
            onClick={onToggleMinimap}
          >
            <MapIcon aria-hidden="true" />
            <span>
              <strong>Show minimap</strong>
              <small>Track and reposition the visible viewport</small>
            </span>
            <i aria-hidden="true" />
          </button>

          <div className="compass-shortcut-guide" aria-label="Graph keyboard shortcuts">
            <div className="compass-shortcut-title">
              <KeyboardIcon aria-hidden="true" />
              <strong>Keyboard</strong>
            </div>
            <dl>
              <div><dt><kbd>F</kbd></dt><dd>Fit graph</dd></div>
              <div><dt><kbd>⇧ F</kbd></dt><dd>Fit selection</dd></div>
              <div><dt><kbd>+</kbd> <kbd>−</kbd></dt><dd>Zoom</dd></div>
              <div><dt><kbd>0</kbd></dt><dd>100% zoom</dd></div>
              <div><dt><kbd>I</kbd></dt><dd>Isolate</dd></div>
              <div><dt><kbd>[</kbd> <kbd>]</kbd></dt><dd>Depth</dd></div>
              <div><dt><kbd>D</kbd></dt><dd>Direction</dd></div>
              <div><dt><kbd>M</kbd></dt><dd>Minimap</dd></div>
            </dl>
          </div>
        </div>
      ) : null}
    </div>
  );
}
