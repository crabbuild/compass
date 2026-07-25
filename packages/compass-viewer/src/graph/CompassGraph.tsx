import { useCallback, useMemo, useReducer, useRef } from "react";
import {
  EyeIcon,
  EyeOffIcon,
  Maximize2Icon,
  PauseIcon,
  PlayIcon,
  RotateCcwIcon,
  SearchIcon
} from "lucide-react";
import type { GraphViewModel, SourceLocation } from "@/contracts/graph";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { graphReducer, initialGraphState } from "./state";
import { VisNetworkCanvas, type GraphCanvasHandle } from "./VisNetworkCanvas";

export type GraphHost = {
  openSource(source: SourceLocation): void;
};

type Props = {
  model: GraphViewModel;
  host: GraphHost;
};

export function CompassGraph({ model, host }: Props) {
  const [state, dispatch] = useReducer(graphReducer, initialGraphState);
  const canvasRef = useRef<GraphCanvasHandle>(null);
  const selected = model.nodes.find((node) => node.id === state.focusedNodeId);
  const neighbors = useMemo(() => {
    if (!selected) return [];
    const ids = new Set<string>();
    for (const edge of model.edges) {
      if (edge.source === selected.id) ids.add(edge.target);
      if (edge.target === selected.id) ids.add(edge.source);
    }
    return [...ids]
      .map((id) => model.nodes.find((node) => node.id === id))
      .filter((node) => node !== undefined)
      .sort((left, right) => left.label.localeCompare(right.label));
  }, [model.edges, model.nodes, selected]);
  const matches = useMemo(() => {
    const query = state.query.trim().toLocaleLowerCase();
    if (!query) return [];
    return model.nodes
      .filter((node) => node.label.toLocaleLowerCase().includes(query)
        || node.source?.file.toLocaleLowerCase().includes(query))
      .slice(0, 20);
  }, [model.nodes, state.query]);
  const focus = useCallback((nodeId: string) => {
    dispatch({ type: "focus", nodeId });
  }, []);
  const clear = useCallback(() => dispatch({ type: "clearFocus" }), []);
  const stabilized = useCallback(() => dispatch({ type: "stabilized" }), []);
  const status = selected
    ? `Inspecting ${selected.label}`
    : state.physicsRunning ? "Layout settling" : "Layout paused";

  return (
    <TooltipProvider>
      <div className="compass-workspace">
        <VisNetworkCanvas
          ref={canvasRef}
          model={model}
          focusedNodeId={state.focusedNodeId}
          physicsRunning={state.physicsRunning}
          forceLabels={state.forceLabels}
          hiddenCommunities={state.hiddenCommunities}
          onFocus={focus}
          onClear={clear}
          onStabilized={stabilized}
        />

        <div className="compass-toolbar" role="toolbar" aria-label="Graph controls">
          <div className="flex min-w-0 items-center gap-2">
            <div className="relative min-w-48 max-w-80 flex-1">
              <SearchIcon
                aria-hidden="true"
                className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted-foreground"
              />
              <Input
                className="pl-8"
                type="search"
                value={state.query}
                placeholder="Search nodes and files"
                aria-label="Search graph nodes"
                aria-controls="compass-search-results"
                aria-expanded={matches.length > 0}
                onChange={(event) => dispatch({
                  type: "search",
                  query: event.target.value
                })}
              />
              {matches.length > 0 && (
                <div
                  id="compass-search-results"
                  role="listbox"
                  className="absolute left-0 right-0 top-10 max-h-72 overflow-auto rounded-md border bg-popover p-1 text-popover-foreground shadow-xl"
                >
                  {matches.map((node) => (
                    <button
                      key={node.id}
                      type="button"
                      role="option"
                      aria-selected={node.id === selected?.id}
                      className="flex w-full flex-col rounded-sm px-2 py-1.5 text-left hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                      onClick={() => {
                        focus(node.id);
                        dispatch({ type: "search", query: "" });
                      }}
                    >
                      <span className="truncate text-sm">{node.label}</span>
                      {node.source && (
                        <span className="truncate font-mono text-xs text-muted-foreground">
                          {node.source.file}
                        </span>
                      )}
                    </button>
                  ))}
                </div>
              )}
            </div>
            <Badge variant="outline">{model.stats.nodes.toLocaleString()} nodes</Badge>
          </div>

          <div className="flex items-center gap-1">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant={state.physicsRunning ? "secondary" : "outline"}
                  size="sm"
                  onClick={() => dispatch({
                    type: "setPhysics",
                    running: !state.physicsRunning
                  })}
                >
                  {state.physicsRunning
                    ? <PauseIcon data-icon="inline-start" />
                    : <PlayIcon data-icon="inline-start" />}
                  {state.physicsRunning ? "Pause layout" : "Resume layout"}
                </Button>
              </TooltipTrigger>
              <TooltipContent>Control force-directed layout motion</TooltipContent>
            </Tooltip>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Fit graph in view"
              onClick={() => canvasRef.current?.fit()}
            >
              <Maximize2Icon />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Reset graph view"
              onClick={() => {
                clear();
                canvasRef.current?.reset();
              }}
            >
              <RotateCcwIcon />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={state.forceLabels ? "Hide labels" : "Show labels"}
              aria-pressed={state.forceLabels}
              onClick={() => dispatch({
                type: "setLabels",
                visible: !state.forceLabels
              })}
            >
              {state.forceLabels ? <EyeOffIcon /> : <EyeIcon />}
            </Button>
          </div>
        </div>

        <aside className="compass-inspector" aria-label="Graph inspector">
          <ScrollArea className="h-full">
            <div className="flex flex-col gap-4 p-4">
              <header className="flex flex-col gap-1">
                <span className="font-mono text-xs uppercase tracking-wider text-muted-foreground">
                  {model.stats.aggregated ? "Community overview" : "Code graph"}
                </span>
                <h1 className="truncate text-base font-semibold">{model.title}</h1>
              </header>

              {selected ? (
                <>
                  <section className="flex flex-col gap-2">
                    <div className="flex items-start gap-2">
                      <span
                        aria-hidden="true"
                        className="mt-1 size-3 shrink-0 rounded-full"
                        style={{
                          background: model.communities.find(
                            (community) => community.id === selected.community
                          )?.color
                        }}
                      />
                      <div className="min-w-0">
                        <h2 className="break-words text-sm font-semibold">{selected.label}</h2>
                        <p className="font-mono text-xs text-muted-foreground">
                          {selected.kind ?? "Node"} · degree {selected.degree ?? neighbors.length}
                        </p>
                      </div>
                    </div>
                    {selected.source ? (
                      <Button
                        variant="outline"
                        size="sm"
                        className="justify-start"
                        onClick={() => host.openSource(selected.source!)}
                      >
                        Open source
                        <span className="ml-auto truncate font-mono text-xs text-muted-foreground">
                          {selected.source.file}
                        </span>
                      </Button>
                    ) : (
                      <Alert>
                        <AlertTitle>No source location</AlertTitle>
                        <AlertDescription>
                          This node represents derived or external graph knowledge.
                        </AlertDescription>
                      </Alert>
                    )}
                  </section>

                  <Separator />

                  <section className="flex flex-col gap-2">
                    <div className="flex items-center justify-between">
                      <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                        Connected nodes
                      </h3>
                      <Badge variant="secondary">{neighbors.length}</Badge>
                    </div>
                    <div className="flex flex-col gap-1">
                      {neighbors.map((neighbor) => (
                        <Button
                          key={neighbor.id}
                          variant="ghost"
                          size="sm"
                          className="justify-start truncate"
                          aria-label={`Inspect neighbor ${neighbor.label}`}
                          onClick={() => focus(neighbor.id)}
                        >
                          <span className="truncate">{neighbor.label}</span>
                        </Button>
                      ))}
                    </div>
                  </section>
                </>
              ) : (
                <Alert>
                  <SearchIcon />
                  <AlertTitle>Select a node</AlertTitle>
                  <AlertDescription>
                    Search or choose a node to spotlight its immediate relationships.
                  </AlertDescription>
                </Alert>
              )}

              <Separator />

              <section className="flex flex-col gap-2">
                <div className="flex items-center justify-between">
                  <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                    Communities
                  </h3>
                  <Badge variant="outline">{model.stats.communities}</Badge>
                </div>
                <div className="flex flex-wrap gap-1.5">
                  {model.communities.map((community) => {
                    const hidden = state.hiddenCommunities.has(community.id);
                    return (
                      <Button
                        key={community.id}
                        variant={hidden ? "outline" : "secondary"}
                        size="xs"
                        aria-pressed={!hidden}
                        onClick={() => dispatch({
                          type: "toggleCommunity",
                          communityId: community.id
                        })}
                      >
                        <span
                          aria-hidden="true"
                          className="size-2 rounded-full"
                          style={{ background: community.color }}
                        />
                        <span className="max-w-40 truncate">{community.label}</span>
                      </Button>
                    );
                  })}
                </div>
              </section>

              <Separator />

              <dl className="grid grid-cols-2 gap-x-3 gap-y-2 text-xs">
                <dt className="text-muted-foreground">Nodes</dt>
                <dd className="text-right font-mono">{model.stats.nodes.toLocaleString()}</dd>
                <dt className="text-muted-foreground">Edges</dt>
                <dd className="text-right font-mono">{model.stats.edges.toLocaleString()}</dd>
                <dt className="text-muted-foreground">Communities</dt>
                <dd className="text-right font-mono">{model.stats.communities.toLocaleString()}</dd>
              </dl>
            </div>
          </ScrollArea>
        </aside>

        <div className="compass-statusbar" role="status" aria-live="polite">
          <span className="compass-status-dot" aria-hidden="true" />
          <span>{status}</span>
        </div>
      </div>
    </TooltipProvider>
  );
}
