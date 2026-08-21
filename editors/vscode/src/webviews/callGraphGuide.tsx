import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent
} from "react";
import { createRoot } from "react-dom/client";
import {
  ArrowDownToLineIcon,
  ArrowRightIcon,
  ArrowUpFromLineIcon,
  BookOpenIcon,
  ChevronRightIcon,
  FileCode2Icon,
  GitForkIcon,
  MousePointer2Icon,
  NetworkIcon,
  SearchIcon
} from "lucide-react";
import type { CallDirection } from "@compass/viewer/contracts/callGraph";
import type { QueryCompletion } from "@compass/viewer";
import {
  MAX_CALL_GRAPH_SYMBOL_LENGTH,
  callGraphCompletionTerm,
  parseCallGraphCompletionItems
} from "../views/callGraphGuideMessages";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };

type GuideSource = {
  fileLabel: string;
  languageId: string;
};

type CompletionStatus = "idle" | "waiting" | "loading" | "ready" | "error";

const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass call graph guide root is missing");

function CallGraphGuide() {
  const lookupRef = useRef<HTMLFormElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const completionGeneration = useRef(0);
  const completionRequestId = useRef<string | undefined>(undefined);
  const completionTimeout = useRef<number | undefined>(undefined);
  const [source, setSource] = useState<GuideSource | null | undefined>(undefined);
  const [symbol, setSymbol] = useState("");
  const [symbolDirection, setSymbolDirection] = useState<CallDirection>("both");
  const [lookupPending, setLookupPending] = useState(false);
  const [completionFocused, setCompletionFocused] = useState(false);
  const [completionStatus, setCompletionStatus] = useState<CompletionStatus>("idle");
  const [completionError, setCompletionError] = useState("");
  const [completionRetry, setCompletionRetry] = useState(0);
  const [suggestions, setSuggestions] = useState<QueryCompletion[]>([]);
  const [activeSuggestion, setActiveSuggestion] = useState(0);
  const term = callGraphCompletionTerm(symbol);
  const showSuggestions = completionFocused && suggestions.length > 0;
  const showCompletionStatus = completionFocused && term !== undefined
    && (completionStatus === "loading"
      || completionStatus === "error"
      || (completionStatus === "ready" && suggestions.length === 0));

  useEffect(() => {
    const receive = (event: MessageEvent) => {
      const requestId = typeof event.data?.requestId === "string"
        ? event.data.requestId
        : undefined;
      if (requestId && ["completions", "completionError", "completionCancelled"]
        .includes(event.data?.type)) {
        if (requestId !== completionRequestId.current) return;
        completionRequestId.current = undefined;
        if (completionTimeout.current !== undefined) {
          window.clearTimeout(completionTimeout.current);
          completionTimeout.current = undefined;
        }
        if (event.data.type === "completions") {
          const items = parseCallGraphCompletionItems(event.data.items);
          if (items) {
            setSuggestions(items);
            setCompletionStatus("ready");
            setCompletionError("");
          } else {
            setSuggestions([]);
            setCompletionStatus("error");
            setCompletionError("Compass returned suggestions this extension could not read.");
          }
        } else if (event.data.type === "completionError") {
          setSuggestions([]);
          setCompletionStatus("error");
          setCompletionError(typeof event.data.message === "string"
            ? event.data.message
            : "Code graph suggestions are unavailable.");
        } else {
          setCompletionStatus("idle");
        }
        return;
      }
      if (event.data?.type === "openSymbolFailed") {
        setLookupPending(false);
        return;
      }
      if (event.data?.type !== "hydrate") return;
      const next = event.data.source;
      if (next === null) {
        setSource(null);
      } else if (
        typeof next?.fileLabel === "string"
        && typeof next?.languageId === "string"
      ) {
        setSource(next);
      }
    };
    window.addEventListener("message", receive);
    vscode.postMessage({ type: "ready" });
    return () => window.removeEventListener("message", receive);
  }, []);

  useEffect(() => {
    const generation = ++completionGeneration.current;
    setSuggestions([]);
    setActiveSuggestion(0);
    setCompletionError("");
    if (!term || !completionFocused || lookupPending) {
      setCompletionStatus("idle");
      return;
    }
    setCompletionStatus("waiting");
    const timer = window.setTimeout(() => {
      if (generation !== completionGeneration.current) return;
      const requestId = `call-completion-${generation}`;
      completionRequestId.current = requestId;
      setCompletionStatus("loading");
      vscode.postMessage({ type: "completeSymbol", requestId, term });
      completionTimeout.current = window.setTimeout(() => {
        if (completionRequestId.current !== requestId) return;
        completionRequestId.current = undefined;
        completionTimeout.current = undefined;
        vscode.postMessage({ type: "cancelCompletion", requestId });
        setCompletionStatus("error");
        setCompletionError("Code graph search timed out. You can still trace the typed symbol.");
      }, 5000);
    }, 180);
    return () => {
      window.clearTimeout(timer);
      if (completionTimeout.current !== undefined) {
        window.clearTimeout(completionTimeout.current);
        completionTimeout.current = undefined;
      }
      const requestId = completionRequestId.current;
      if (requestId) {
        completionRequestId.current = undefined;
        vscode.postMessage({ type: "cancelCompletion", requestId });
      }
    };
  }, [completionFocused, completionRetry, lookupPending, term]);

  const open = (direction: CallDirection) => {
    vscode.postMessage({ type: "openDirection", direction });
  };
  const traceSymbol = (value: string) => {
    const normalized = value.trim();
    if (!normalized || lookupPending) return;
    setSymbol(normalized);
    setCompletionFocused(false);
    setSuggestions([]);
    setLookupPending(true);
    vscode.postMessage({
      type: "openSymbol",
      symbol: normalized,
      direction: symbolDirection
    });
  };
  const openSymbol = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    traceSymbol(symbol);
  };
  const chooseSuggestion = (suggestion: QueryCompletion) => {
    setSymbol(suggestion.insertText);
    setCompletionFocused(false);
    setSuggestions([]);
    setCompletionStatus("idle");
    requestAnimationFrame(() => inputRef.current?.focus());
  };
  const handleSymbolKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape" && showSuggestions) {
      event.preventDefault();
      setSuggestions([]);
      setCompletionStatus("idle");
      return;
    }
    if ((event.key === "ArrowDown" || event.key === "ArrowUp")
      && suggestions.length > 0) {
      event.preventDefault();
      setActiveSuggestion((current) => event.key === "ArrowDown"
        ? (current + 1) % suggestions.length
        : (current - 1 + suggestions.length) % suggestions.length);
      return;
    }
    if (event.key === "Tab" && showSuggestions) {
      event.preventDefault();
      const suggestion = suggestions[activeSuggestion];
      if (suggestion) chooseSuggestion(suggestion);
      return;
    }
    if (event.key === "Enter" && showSuggestions) {
      const suggestion = suggestions[activeSuggestion];
      if (!suggestion) return;
      event.preventDefault();
      traceSymbol(suggestion.insertText);
    }
  };
  const ready = source !== null && source !== undefined;

  return (
    <main className="call-guide-shell">
      <div className="call-guide-grid" aria-hidden="true" />

      <header className="call-guide-hero">
        <section className="call-guide-intro">
          <div className="call-guide-kicker">
            <NetworkIcon aria-hidden="true" />
            <span>Compass / Call trace</span>
          </div>
          <h1>Trace how this function connects.</h1>
          <p className="call-guide-lead">
            Start from a symbol name or the cursor. Follow what calls in, what
            calls out, or map the full neighborhood in one view.
          </p>
          <form
            ref={lookupRef}
            className="call-guide-lookup"
            onSubmit={openSymbol}
            onBlur={(event) => {
              const next = event.relatedTarget;
              if (next instanceof Node && lookupRef.current?.contains(next)) return;
              setCompletionFocused(false);
            }}
          >
            <label htmlFor="call-guide-symbol">Trace by symbol</label>
            <div className="call-guide-lookup-row">
              <span className="call-guide-lookup-input">
                <SearchIcon aria-hidden="true" />
                <input
                  ref={inputRef}
                  id="call-guide-symbol"
                  type="text"
                  value={symbol}
                  maxLength={MAX_CALL_GRAPH_SYMBOL_LENGTH}
                  onChange={(event) => {
                    setSymbol(event.target.value);
                    setCompletionFocused(true);
                  }}
                  onFocus={() => setCompletionFocused(true)}
                  onKeyDown={handleSymbolKeyDown}
                  placeholder="Function, qualified name, or symbol ID"
                  autoComplete="off"
                  autoFocus
                  spellCheck={false}
                  role="combobox"
                  aria-autocomplete="list"
                  aria-controls={showSuggestions
                    ? "call-guide-symbol-suggestions"
                    : undefined}
                  aria-expanded={showSuggestions}
                  aria-activedescendant={showSuggestions
                    ? `call-guide-symbol-suggestion-${activeSuggestion}`
                    : undefined}
                />
              </span>
              <button
                type="submit"
                disabled={!symbol.trim() || lookupPending}
              >
                {lookupPending ? "Opening…" : "Show graph"}
                <ArrowRightIcon aria-hidden="true" />
              </button>
            </div>
            {showSuggestions && (
              <div
                id="call-guide-symbol-suggestions"
                className="call-guide-suggestions"
                role="listbox"
                aria-label="Callable symbols from the active code graph"
              >
                <span className="call-guide-suggestions-label">
                  Callable code graph symbols
                </span>
                {suggestions.map((suggestion, index) => (
                  <button
                    id={`call-guide-symbol-suggestion-${index}`}
                    key={`${suggestion.nodeId}:${suggestion.insertText}`}
                    type="button"
                    role="option"
                    aria-selected={index === activeSuggestion}
                    onMouseDown={(event) => event.preventDefault()}
                    onMouseEnter={() => setActiveSuggestion(index)}
                    onClick={() => chooseSuggestion(suggestion)}
                  >
                    <span>{suggestion.label}</span>
                    <small>{suggestion.detail}</small>
                  </button>
                ))}
                <span className="call-guide-suggestions-hint">
                  ↑↓ choose · Tab complete · Enter trace
                </span>
              </div>
            )}
            {showCompletionStatus && (
              <div className="call-guide-completion-status" role="status">
                <span>{completionStatus === "loading"
                  ? "Searching callable symbols in the active code graph…"
                  : completionStatus === "error"
                    ? completionError
                    : `No callable graph symbols match “${term}”.`}</span>
                {completionStatus === "error" && (
                  <button
                    type="button"
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => setCompletionRetry((current) => current + 1)}
                  >
                    Retry
                  </button>
                )}
              </div>
            )}
            <fieldset className="call-guide-lookup-directions">
              <legend>Direction</legend>
              {(["callers", "both", "callees"] as const).map((direction) => (
                <button
                  key={direction}
                  type="button"
                  aria-pressed={symbolDirection === direction}
                  onClick={() => setSymbolDirection(direction)}
                >
                  {direction === "both"
                    ? "Both"
                    : direction === "callers" ? "Callers" : "Callees"}
                </button>
              ))}
              <span>Press Enter to trace</span>
            </fieldset>
          </form>
          <div
            className="call-guide-source"
            data-ready={ready ? "true" : "false"}
            role="status"
          >
            <span className="call-guide-source-light" aria-hidden="true" />
            <FileCode2Icon aria-hidden="true" />
            <span>
              {source === undefined
                ? "Reading the active editor…"
                : source
                  ? source.fileLabel
                  : "Open a source file to enable cursor trace actions"}
            </span>
            {source && <code>{source.languageId}</code>}
          </div>
        </section>

        <CallRoute />
      </header>

      <section className="call-guide-section" aria-labelledby="call-guide-how">
        <div className="call-guide-heading">
          <div>
            <span className="call-guide-overline">Editor workflow</span>
            <h2 id="call-guide-how">Three moves from code to context</h2>
          </div>
          <span className="call-guide-shortcut">
            Right-click <ArrowRightIcon aria-hidden="true" /> Compass
          </span>
        </div>

        <ol className="call-guide-steps">
          <li>
            <span className="call-guide-step-number">01</span>
            <MousePointer2Icon aria-hidden="true" />
            <div>
              <h3>Place the cursor</h3>
              <p>Click anywhere inside an indexed function or method body.</p>
            </div>
          </li>
          <li className="call-guide-step-menu">
            <span className="call-guide-step-number">02</span>
            <div>
              <h3>Open Compass</h3>
              <p>Right-click in the editor and open the Compass submenu.</p>
            </div>
            <ContextMenuPreview />
          </li>
          <li>
            <span className="call-guide-step-number">03</span>
            <GitForkIcon aria-hidden="true" />
            <div>
              <h3>Choose a direction</h3>
              <p>Inspect incoming callers, outgoing callees, or both.</p>
            </div>
          </li>
        </ol>
      </section>

      <section className="call-guide-section call-guide-actions-section" aria-labelledby="call-guide-actions">
        <div className="call-guide-heading">
          <div>
            <span className="call-guide-overline">Start now</span>
            <h2 id="call-guide-actions">Trace from the captured cursor</h2>
          </div>
          <p>
            {ready
              ? "Compass will return to your source before opening the graph."
              : "Open a source file, then reopen this guide from the side panel."}
          </p>
        </div>

        <div className="call-guide-actions">
          <DirectionAction
            direction="callers"
            icon={<ArrowDownToLineIcon aria-hidden="true" />}
            title="Show Callers"
            description="Who reaches this function"
            onOpen={open}
            disabled={!ready}
          />
          <DirectionAction
            direction="both"
            icon={<GitForkIcon aria-hidden="true" />}
            title="Show Both"
            description="See the full call neighborhood"
            onOpen={open}
            disabled={!ready}
            primary
          />
          <DirectionAction
            direction="callees"
            icon={<ArrowUpFromLineIcon aria-hidden="true" />}
            title="Show Callees"
            description="What this function reaches"
            onOpen={open}
            disabled={!ready}
          />
        </div>
      </section>

      <footer className="call-guide-footer">
        <div className="call-guide-coverage">
          <span className="call-guide-coverage-badge">Structural graph</span>
          <p>
            Works across every call-capable language Compass indexes. Program IR
            adds exact call evidence when available, and partial coverage is
            labeled in the graph.
          </p>
        </div>
        <button
          className="call-guide-walkthrough"
          type="button"
          onClick={() => vscode.postMessage({ type: "openWalkthrough" })}
        >
          <BookOpenIcon aria-hidden="true" />
          Open full walkthrough
          <ChevronRightIcon aria-hidden="true" />
        </button>
      </footer>
    </main>
  );
}

function CallRoute() {
  return (
    <div
      className="call-guide-route"
      role="img"
      aria-label="A caller flows into the selected function, which flows out to callees"
    >
      <div className="call-guide-route-label">
        <span>Live call trace</span>
        <code>target → graph</code>
      </div>
      <div className="call-guide-route-stage">
        <div className="call-guide-route-line call-guide-route-line-in" aria-hidden="true">
          <i />
        </div>
        <div className="call-guide-route-line call-guide-route-line-out" aria-hidden="true">
          <i />
        </div>
        <div className="call-guide-route-node" data-kind="caller">
          <span>IN</span>
          <strong>caller()</strong>
          <small>incoming edge</small>
        </div>
        <div className="call-guide-route-node call-guide-route-current" data-kind="current">
          <MousePointer2Icon aria-hidden="true" />
          <strong>target()</strong>
          <small>selected function</small>
        </div>
        <div className="call-guide-route-node" data-kind="callee">
          <span>OUT</span>
          <strong>callee()</strong>
          <small>outgoing edge</small>
        </div>
      </div>
      <div className="call-guide-route-readout" aria-hidden="true">
        <span>01 locate</span>
        <span>02 resolve</span>
        <span>03 render</span>
      </div>
    </div>
  );
}

function ContextMenuPreview() {
  return (
    <div
      className="call-guide-menu-preview"
      role="img"
      aria-label="Compass editor context menu with callers, callees, and both"
    >
      <div className="call-guide-menu-parent">
        <NetworkIcon aria-hidden="true" />
        <span>Compass</span>
        <ChevronRightIcon aria-hidden="true" />
      </div>
      <div className="call-guide-submenu">
        <span><ArrowDownToLineIcon aria-hidden="true" /> Show Callers</span>
        <span><ArrowUpFromLineIcon aria-hidden="true" /> Show Callees</span>
        <span className="is-active"><GitForkIcon aria-hidden="true" /> Show Callers &amp; Callees</span>
      </div>
    </div>
  );
}

function DirectionAction({
  direction,
  icon,
  title,
  description,
  onOpen,
  disabled,
  primary = false
}: {
  direction: CallDirection;
  icon: React.ReactNode;
  title: string;
  description: string;
  onOpen(direction: CallDirection): void;
  disabled: boolean;
  primary?: boolean;
}) {
  return (
    <button
      className="call-guide-action"
      data-primary={primary ? "true" : "false"}
      type="button"
      disabled={disabled}
      onClick={() => onOpen(direction)}
    >
      <span className="call-guide-action-icon">{icon}</span>
      <span>
        <strong>{title}</strong>
        <small>{description}</small>
      </span>
      <ArrowRightIcon className="call-guide-action-arrow" aria-hidden="true" />
      {primary && <em>Recommended</em>}
    </button>
  );
}

createRoot(element).render(<CallGraphGuide />);
