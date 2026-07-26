import {
  AlertTriangle,
  ArrowLeft,
  ArrowRight,
  Check,
  CheckCircle2,
  Circle,
  FileCode2,
  FolderTree,
  Gauge,
  GitBranch,
  LoaderCircle,
  Settings2,
  SquareTerminal,
  XCircle
} from "lucide-react";
import { useMemo, useState } from "react";
import type { ReactNode } from "react";

export type InitializationRequest = {
  includes: string[];
  excludes: string[];
  replaceExisting: boolean;
};

export type InitializationStatus =
  | {
    kind: "building";
    phase: string;
    current?: number;
    total?: number;
    message: string;
  }
  | { kind: "success"; message?: string }
  | { kind: "error"; message: string }
  | { kind: "cancelled" };

export type InitializationHost = {
  start(request: InitializationRequest): void;
  cancel(): void;
  reset(): void;
  openGraph(): void;
  showOutput(): void;
};

type Props = {
  repositoryName: string;
  repositoryRoot: string;
  configurationExists?: boolean;
  host: InitializationHost;
  status?: InitializationStatus;
};

const steps = [
  { label: "Repository scope", icon: FolderTree },
  { label: "Index rules", icon: Settings2 },
  { label: "Review and build", icon: Gauge }
];

export function InitializationWizard({
  repositoryName,
  repositoryRoot,
  configurationExists = false,
  host,
  status
}: Props) {
  const [step, setStep] = useState(0);
  const [scope, setScope] = useState<"all" | "custom">("all");
  const [includeText, setIncludeText] = useState("");
  const [excludeText, setExcludeText] = useState("");
  const [replaceExisting, setReplaceExisting] = useState(false);
  const includes = useMemo(
    () => scope === "custom" ? splitRules(includeText) : [],
    [includeText, scope]
  );
  const excludes = useMemo(() => splitRules(excludeText), [excludeText]);
  const customScopeMissing = scope === "custom" && includes.length === 0;
  const request = {
    includes,
    excludes,
    replaceExisting: configurationExists && replaceExisting
  };

  if (status?.kind === "building") {
    return <BuildProgress repositoryName={repositoryName} status={status} host={host} />;
  }
  if (status?.kind === "success") {
    return (
      <main className="init-shell init-result-shell">
        <section className="init-result-card" aria-labelledby="init-success-title">
          <span className="init-result-icon init-result-icon-success">
            <CheckCircle2 aria-hidden="true" />
          </span>
          <p className="init-eyebrow">Initialization complete</p>
          <h1 id="init-success-title">Your Compass index is ready</h1>
          <p>
            {status.message
              ?? `${repositoryName} is indexed and ready for graph exploration.`}
          </p>
          <div className="init-result-actions">
            <button className="init-button init-button-primary" onClick={host.openGraph}>
              Open code graph
              <ArrowRight aria-hidden="true" />
            </button>
            <button className="init-button init-button-secondary" onClick={host.showOutput}>
              <SquareTerminal aria-hidden="true" />
              View build output
            </button>
          </div>
        </section>
      </main>
    );
  }
  if (status?.kind === "cancelled") {
    return (
      <main className="init-shell init-result-shell">
        <section
          className="init-result-card"
          aria-labelledby="init-cancelled-title"
          role="status"
        >
          <span className="init-result-icon init-result-icon-cancelled">
            <XCircle aria-hidden="true" />
          </span>
          <p className="init-eyebrow">Initialization stopped</p>
          <h1 id="init-cancelled-title">Build cancelled</h1>
          <p>No index was published. Resume with the reviewed scope or edit it first.</p>
          <div className="init-result-actions">
            <button
              className="init-button init-button-primary"
              onClick={configurationExists ? host.reset : () => host.start(request)}
            >
              {configurationExists ? "Review saved configuration" : "Resume build"}
              <ArrowRight aria-hidden="true" />
            </button>
            {!configurationExists && (
              <button className="init-button init-button-secondary" onClick={host.reset}>
                Edit configuration
              </button>
            )}
            <button className="init-button init-button-quiet" onClick={host.showOutput}>
              View output
            </button>
          </div>
        </section>
      </main>
    );
  }
  if (status?.kind === "error") {
    return (
      <main className="init-shell init-result-shell">
        <section className="init-result-card" aria-labelledby="init-error-title">
          <span className="init-result-icon init-result-icon-error">
            <AlertTriangle aria-hidden="true" />
          </span>
          <p className="init-eyebrow">Build stopped</p>
          <h1 id="init-error-title">Compass could not build the index</h1>
          <p role="alert">{status.message}</p>
          <div className="init-result-actions">
            <button
              className="init-button init-button-primary"
              onClick={configurationExists ? host.reset : () => host.start(request)}
            >
              {configurationExists ? "Review saved configuration" : "Try build again"}
            </button>
            {!configurationExists && (
              <button className="init-button init-button-secondary" onClick={host.reset}>
                Edit configuration
              </button>
            )}
            <button className="init-button init-button-quiet" onClick={host.showOutput}>
              View output
            </button>
          </div>
        </section>
      </main>
    );
  }

  return (
    <main className="init-shell">
      <header className="init-header">
        <div>
          <p className="init-eyebrow">Compass initialization</p>
          <h1>Build a map of {repositoryName}</h1>
          <p className="init-header-copy">
            Choose the files Compass should understand, review the saved rules, then build
            the first local code index.
          </p>
        </div>
        <div className="init-repository-badge" title={repositoryRoot}>
          <GitBranch aria-hidden="true" />
          <span>
            <small>Repository</small>
            <strong>{repositoryName}</strong>
          </span>
        </div>
      </header>

      <div className="init-layout">
        <nav className="init-step-nav" aria-label="Initialization steps">
          <ol>
            {steps.map((item, index) => {
              const Icon = item.icon;
              const state = index < step ? "complete" : index === step ? "active" : "pending";
              return (
                <li key={item.label} data-state={state}>
                  <button
                    type="button"
                    onClick={() => index <= step && setStep(index)}
                    disabled={index > step}
                    aria-current={index === step ? "step" : undefined}
                  >
                    <span className="init-step-marker">
                      {index < step ? <Check aria-hidden="true" /> : <Icon aria-hidden="true" />}
                    </span>
                    <span>
                      <small>Step {index + 1}</small>
                      <strong>{item.label}</strong>
                    </span>
                  </button>
                </li>
              );
            })}
          </ol>
          <div className="init-local-note">
            <FileCode2 aria-hidden="true" />
            <p>
              <strong>Local by default</strong>
              Source indexing runs on this machine and follows your existing
              <code>.gitignore</code>.
            </p>
          </div>
        </nav>

        <section className="init-stage" aria-live="polite">
          {step === 0 && (
            <>
              <StageHeading
                step="01"
                title="What should Compass index?"
                copy="Start with the whole repository or narrow the initial graph to specific paths."
              />
              <div className="init-scope-grid">
                <label className="init-choice" data-selected={scope === "all"}>
                  <input
                    type="radio"
                    name="scope"
                    checked={scope === "all"}
                    onChange={() => setScope("all")}
                  />
                  <span className="init-choice-radio"><Circle aria-hidden="true" /></span>
                  <span>
                    <strong>All eligible files</strong>
                    <small>
                      Recommended. Index supported source and document files while honoring
                      ignored paths.
                    </small>
                  </span>
                </label>
                <label className="init-choice" data-selected={scope === "custom"}>
                  <input
                    type="radio"
                    name="scope"
                    checked={scope === "custom"}
                    onChange={() => setScope("custom")}
                  />
                  <span className="init-choice-radio"><Circle aria-hidden="true" /></span>
                  <span>
                    <strong>Custom scope</strong>
                    <small>
                      Focus the graph on selected packages, services, folders, files, or glob
                      patterns.
                    </small>
                  </span>
                </label>
              </div>
              <RepositoryFacts root={repositoryRoot} />
              <StageActions>
                <span />
                <button className="init-button init-button-primary" onClick={() => setStep(1)}>
                  Continue
                  <ArrowRight aria-hidden="true" />
                </button>
              </StageActions>
            </>
          )}

          {step === 1 && (
            <>
              <StageHeading
                step="02"
                title="Set the index rules"
                copy="Use project-relative paths or globs. Put one rule on each line."
              />
              <div className="init-fields">
                <label>
                  <span>
                    <strong>Include paths and globs</strong>
                    <small>{scope === "all" ? "The full eligible repository is included." : "At least one include rule is required."}</small>
                  </span>
                  <textarea
                    aria-label="Include paths and globs"
                    value={includeText}
                    disabled={scope === "all"}
                    onChange={(event) => setIncludeText(event.target.value)}
                    placeholder={"src\npackages/api/**"}
                    rows={5}
                  />
                </label>
                <label>
                  <span>
                    <strong>Exclude paths and globs</strong>
                    <small>Optional. These rules are applied after includes.</small>
                  </span>
                  <textarea
                    aria-label="Exclude paths and globs"
                    value={excludeText}
                    onChange={(event) => setExcludeText(event.target.value)}
                    placeholder={"**/generated/**\n**/fixtures/**"}
                    rows={5}
                  />
                </label>
              </div>
              {customScopeMissing && (
                <p className="init-validation" role="alert">
                  Add at least one path or glob for a custom scope.
                </p>
              )}
              <StageActions>
                <button className="init-button init-button-secondary" onClick={() => setStep(0)}>
                  <ArrowLeft aria-hidden="true" />
                  Back
                </button>
                <button
                  className="init-button init-button-primary"
                  disabled={customScopeMissing}
                  onClick={() => setStep(2)}
                >
                  Review configuration
                  <ArrowRight aria-hidden="true" />
                </button>
              </StageActions>
            </>
          )}

          {step === 2 && (
            <>
              <StageHeading
                step="03"
                title="Review and build"
                copy="Compass saves this scope in .compass/config.toml, then builds the local index."
              />
              <div className="init-review">
                <ReviewRow
                  label="Repository"
                  value={<code title={repositoryRoot}>{repositoryRoot}</code>}
                />
                <ReviewRow
                  label="Include"
                  value={includes.length === 0
                    ? <span className="init-rule-default">All eligible files</span>
                    : <RuleList rules={includes} />}
                />
                <ReviewRow
                  label="Exclude"
                  value={excludes.length === 0
                    ? <span className="init-rule-default">No additional exclusions</span>
                    : <RuleList rules={excludes} />}
                />
                <ReviewRow
                  label="Ignore policy"
                  value={<span className="init-rule-default">Honor .gitignore</span>}
                />
                <ReviewRow
                  label="Output"
                  value={<code>compass-out/</code>}
                />
              </div>
              <div className="init-build-callout">
                <Gauge aria-hidden="true" />
                <div>
                  <strong>Ready to index</strong>
                  <p>
                    You can close other Compass views while this runs. File-level progress
                    will appear here.
                  </p>
                </div>
              </div>
              {configurationExists && (
                <label className="init-replace-confirmation">
                  <input
                    type="checkbox"
                    checked={replaceExisting}
                    onChange={(event) => setReplaceExisting(event.target.checked)}
                  />
                  <span>
                    <strong>Existing configuration</strong>
                    <small>
                      I understand this scope will replace .compass/config.toml.
                    </small>
                  </span>
                </label>
              )}
              <StageActions>
                <button className="init-button init-button-secondary" onClick={() => setStep(1)}>
                  <ArrowLeft aria-hidden="true" />
                  Back
                </button>
                <button
                  className="init-button init-button-primary init-button-build"
                  disabled={configurationExists && !replaceExisting}
                  onClick={() => host.start(request)}
                >
                  {configurationExists
                    ? "Replace configuration and build"
                    : "Build Compass index"}
                  <ArrowRight aria-hidden="true" />
                </button>
              </StageActions>
            </>
          )}
        </section>
      </div>
    </main>
  );
}

function BuildProgress({
  repositoryName,
  status,
  host
}: {
  repositoryName: string;
  status: Extract<InitializationStatus, { kind: "building" }>;
  host: InitializationHost;
}) {
  const current = status.current ?? 0;
  const total = status.total ?? 0;
  const percentage = total > 0 ? Math.min(100, Math.round((current / total) * 100)) : 0;
  const indexing = status.phase === "indexing";
  return (
    <main className="init-shell init-progress-shell">
      <header className="init-progress-header">
        <div>
          <p className="init-eyebrow">Building Compass index</p>
          <h1>{indexing ? `Mapping ${repositoryName}` : "Preparing repository"}</h1>
          <p>
            {indexing
              ? "Compass is extracting symbols and relationships from each eligible file."
              : status.message}
          </p>
        </div>
        <span className="init-running-badge">
          <LoaderCircle aria-hidden="true" />
          Running
        </span>
      </header>

      <section className="init-progress-card" aria-labelledby="init-progress-title">
        <div className="init-progress-summary">
          <div>
            <small id="init-progress-title">Indexing progress</small>
            <strong>{indexing && total > 0 ? `${current} of ${total} files` : "Discovering files"}</strong>
          </div>
          <span>{indexing && total > 0 ? `${percentage}%` : "—"}</span>
        </div>
        <div
          className="init-progress-track"
          role="progressbar"
          aria-label="Compass indexing progress"
          aria-valuemin={0}
          aria-valuenow={current}
          aria-valuemax={total || undefined}
        >
          <span
            className={total > 0 ? "" : "is-indeterminate"}
            style={total > 0 ? { width: `${percentage}%` } : undefined}
          />
        </div>
        <div className="init-current-file">
          <FileCode2 aria-hidden="true" />
          <span>
            <small>Current file</small>
            <code>{indexing ? status.message : "Scanning repository scope…"}</code>
          </span>
        </div>

        <ol className="init-runway" aria-label="Index build phases">
          <BuildPhase
            state="complete"
            title="Configuration"
            detail="Scope rules validated and saved"
          />
          <BuildPhase
            state="active"
            title="File index"
            detail={indexing && total > 0 ? `${current} of ${total} processed` : "Finding eligible files"}
          />
          <BuildPhase state="pending" title="Graph assembly" detail="Resolve and connect relationships" />
        </ol>
      </section>

      <footer className="init-progress-footer">
        <p>Large repositories can take a few minutes. Progress updates as each file finishes.</p>
        <button className="init-button init-button-secondary" onClick={host.cancel}>
          Cancel build
        </button>
      </footer>
    </main>
  );
}

function BuildPhase({
  state,
  title,
  detail
}: {
  state: "complete" | "active" | "pending";
  title: string;
  detail: string;
}) {
  return (
    <li data-state={state}>
      <span className="init-runway-marker">
        {state === "complete"
          ? <Check aria-hidden="true" />
          : state === "active"
            ? <LoaderCircle aria-hidden="true" />
            : <Circle aria-hidden="true" />}
      </span>
      <span>
        <strong>{title}</strong>
        <small>{detail}</small>
      </span>
    </li>
  );
}

function StageHeading({ step, title, copy }: { step: string; title: string; copy: string }) {
  return (
    <header className="init-stage-heading">
      <span>{step}</span>
      <div>
        <h2>{title}</h2>
        <p>{copy}</p>
      </div>
    </header>
  );
}

function StageActions({ children }: { children: ReactNode }) {
  return <footer className="init-stage-actions">{children}</footer>;
}

function RepositoryFacts({ root }: { root: string }) {
  return (
    <dl className="init-facts">
      <div>
        <dt>Root</dt>
        <dd><code title={root}>{root}</code></dd>
      </div>
      <div>
        <dt>Ignore rules</dt>
        <dd>.gitignore enabled</dd>
      </div>
      <div>
        <dt>Index type</dt>
        <dd>Local structural graph</dd>
      </div>
    </dl>
  );
}

function ReviewRow({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="init-review-row">
      <span>{label}</span>
      <div>{value}</div>
    </div>
  );
}

function RuleList({ rules }: { rules: string[] }) {
  return (
    <ul className="init-rule-list">
      {rules.map((rule) => <li key={rule}><code>{rule}</code></li>)}
    </ul>
  );
}

function splitRules(value: string): string[] {
  return Array.from(new Set(
    value
      .split(/[\n,]/)
      .map((rule) => rule.trim())
      .filter(Boolean)
  ));
}
