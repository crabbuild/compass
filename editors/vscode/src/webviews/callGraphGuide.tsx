import { useEffect, useState } from "react";
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
  NetworkIcon
} from "lucide-react";
import type { CallDirection } from "@compass/viewer/contracts/callGraph";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };

type GuideSource = {
  fileLabel: string;
  languageId: string;
};

const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass call graph guide root is missing");

function CallGraphGuide() {
  const [source, setSource] = useState<GuideSource | null | undefined>(undefined);

  useEffect(() => {
    const receive = (event: MessageEvent) => {
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

  const open = (direction: CallDirection) => {
    vscode.postMessage({ type: "openDirection", direction });
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
            Start from the cursor. Follow what calls in, what calls out, or map
            the full neighborhood in one view.
          </p>
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
                  : "Open a source file to enable trace actions"}
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
      aria-label="A caller flows into the current function, which flows out to callees"
    >
      <div className="call-guide-route-label">
        <span>Live call trace</span>
        <code>cursor → graph</code>
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
          <strong>cursor()</strong>
          <small>active function</small>
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
