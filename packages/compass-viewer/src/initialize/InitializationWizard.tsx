import {
  AlertTriangle,
  ArrowLeft,
  ArrowRight,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  File,
  FileCode2,
  Folder,
  FolderTree,
  Gauge,
  GitBranch,
  LoaderCircle,
  Search,
  Settings2,
  SquareTerminal,
  X,
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
  scopeFiles?: string[];
  scopeFilesTruncated?: boolean;
  host: InitializationHost;
  status?: InitializationStatus;
};

const steps = [
  { label: "Repository scope", icon: FolderTree },
  { label: "Glob rules", icon: Settings2 },
  { label: "Review and build", icon: Gauge }
];

const MAX_SCOPE_RULES = 256;

export function InitializationWizard({
  repositoryName,
  repositoryRoot,
  configurationExists = false,
  scopeFiles = [],
  scopeFilesTruncated = false,
  host,
  status
}: Props) {
  const [step, setStep] = useState(0);
  const [scope, setScope] = useState<"all" | "custom">("all");
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [includeGlobText, setIncludeGlobText] = useState("");
  const [excludeText, setExcludeText] = useState("");
  const [replaceExisting, setReplaceExisting] = useState(false);
  const includes = useMemo(
    () => scope === "custom"
      ? mergeRules(selectedPaths, splitRules(includeGlobText))
      : [],
    [includeGlobText, scope, selectedPaths]
  );
  const excludes = useMemo(() => splitRules(excludeText), [excludeText]);
  const customScopeMissing = scope === "custom" && includes.length === 0;
  const tooManyRules = includes.length > MAX_SCOPE_RULES
    || excludes.length > MAX_SCOPE_RULES;
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
                copy="Index the full repository or choose folders and files from the workspace tree."
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
                      Select packages, services, folders, or individual files from the repository.
                    </small>
                  </span>
                </label>
              </div>
              {scope === "custom" && (
                <ScopeTree
                  files={scopeFiles}
                  truncated={scopeFilesTruncated}
                  selected={selectedPaths}
                  onChange={setSelectedPaths}
                />
              )}
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
                title="Refine with glob rules"
                copy="Keep the tree selection as-is or add project-relative globs for precise inclusion and exclusion."
              />
              <div className="init-fields">
                <label>
                  <span>
                    <strong>Additional include globs</strong>
                    <small>
                      {scope === "all"
                        ? "Optional only when using a custom scope."
                        : `${selectedPaths.length} tree selection${selectedPaths.length === 1 ? "" : "s"}; add patterns if needed.`}
                    </small>
                  </span>
                  <textarea
                    aria-label="Additional include globs"
                    value={includeGlobText}
                    disabled={scope === "all"}
                    onChange={(event) => setIncludeGlobText(event.target.value)}
                    placeholder={"packages/*/src/**\napps/**/routes/**"}
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
              {(customScopeMissing || tooManyRules) && (
                <p className="init-validation" role="alert">
                  {tooManyRules
                    ? "Use no more than 256 include rules and 256 exclude rules."
                    : "Select a folder or file from the tree, or add an include glob."}
                </p>
              )}
              <StageActions>
                <button className="init-button init-button-secondary" onClick={() => setStep(0)}>
                  <ArrowLeft aria-hidden="true" />
                  Back
                </button>
                <button
                  className="init-button init-button-primary"
                  disabled={customScopeMissing || tooManyRules}
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

type ScopeNode = {
  name: string;
  path: string;
  kind: "folder" | "file";
  children: ScopeNode[];
};

function ScopeTree({
  files,
  truncated,
  selected,
  onChange
}: {
  files: string[];
  truncated: boolean;
  selected: string[];
  onChange(paths: string[]): void;
}) {
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const nodes = useMemo(() => buildScopeTree(files), [files]);
  const visible = useMemo(() => filterScopeTree(nodes, query), [nodes, query]);
  const toggleSelected = (path: string) => {
    if (selected.includes(path)) {
      onChange(selected.filter((candidate) => candidate !== path));
    } else if (selected.length < MAX_SCOPE_RULES) {
      onChange([...selected, path].sort());
    }
  };
  const toggleExpanded = (path: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  return (
    <section className="init-scope-browser" aria-labelledby="init-scope-browser-title">
      <header>
        <div>
          <strong id="init-scope-browser-title">Repository files</strong>
          <small>Selecting a folder includes every eligible file beneath it.</small>
        </div>
        <span>
          {selected.length} selected{selected.length === MAX_SCOPE_RULES ? " · limit" : ""}
        </span>
      </header>
      <div className="init-scope-toolbar">
        <Search aria-hidden="true" />
        <input
          type="search"
          aria-label="Filter repository files"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Filter folders and files"
        />
        {query && (
          <button type="button" aria-label="Clear file filter" onClick={() => setQuery("")}>
            <X aria-hidden="true" />
          </button>
        )}
      </div>
      <div className="init-scope-body">
        <div className="init-scope-tree" role="tree" aria-label="Repository scope">
          {visible.length > 0 ? visible.map((node) => (
            <ScopeTreeNode
              key={node.path}
              node={node}
              depth={0}
              expanded={expanded}
              forceExpanded={Boolean(query)}
              selected={selected}
              onToggleExpanded={toggleExpanded}
              onToggleSelected={toggleSelected}
            />
          )) : (
            <p className="init-scope-empty">
              {files.length === 0
                ? "No workspace files are available. Use an include glob in the next step."
                : "No folders or files match this filter."}
            </p>
          )}
        </div>
        <aside className="init-scope-ledger" aria-label="Selected repository paths">
          <div>
            <strong>Included from tree</strong>
            <small>Project-relative paths</small>
          </div>
          {selected.length > 0 ? (
            <ul>
              {selected.map((path) => (
                <li key={path}>
                  <code>{path}</code>
                  <button
                    type="button"
                    aria-label={`Remove ${path}`}
                    onClick={() => toggleSelected(path)}
                  >
                    <X aria-hidden="true" />
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <p>Choose a folder or file to begin a focused scope.</p>
          )}
        </aside>
      </div>
      {truncated && (
        <p className="init-scope-notice">
          Showing the first 5,000 workspace files. Use glob rules for paths not shown.
        </p>
      )}
    </section>
  );
}

function ScopeTreeNode({
  node,
  depth,
  expanded,
  forceExpanded,
  selected,
  onToggleExpanded,
  onToggleSelected
}: {
  node: ScopeNode;
  depth: number;
  expanded: Set<string>;
  forceExpanded: boolean;
  selected: string[];
  onToggleExpanded(path: string): void;
  onToggleSelected(path: string): void;
}) {
  const isFolder = node.kind === "folder";
  const isExpanded = forceExpanded || expanded.has(node.path);
  return (
    <div
      role="treeitem"
      aria-label={node.path}
      aria-expanded={isFolder ? isExpanded : undefined}
    >
      <div className="init-scope-row" style={{ paddingLeft: `${depth * 18 + 7}px` }}>
        {isFolder ? (
          <button
            type="button"
            className="init-scope-disclosure"
            aria-label={`${isExpanded ? "Collapse" : "Expand"} ${node.path}`}
            onClick={() => onToggleExpanded(node.path)}
          >
            {isExpanded
              ? <ChevronDown aria-hidden="true" />
              : <ChevronRight aria-hidden="true" />}
          </button>
        ) : <span className="init-scope-disclosure" />}
        <label title={node.path}>
          <input
            type="checkbox"
            checked={selected.includes(node.path)}
            onChange={() => onToggleSelected(node.path)}
          />
          {isFolder ? <Folder aria-hidden="true" /> : <File aria-hidden="true" />}
          <span>{node.name}</span>
        </label>
      </div>
      {isFolder && isExpanded && (
        <div role="group">
          {node.children.map((child) => (
            <ScopeTreeNode
              key={child.path}
              node={child}
              depth={depth + 1}
              expanded={expanded}
              forceExpanded={forceExpanded}
              selected={selected}
              onToggleExpanded={onToggleExpanded}
              onToggleSelected={onToggleSelected}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function buildScopeTree(files: string[]): ScopeNode[] {
  const root: ScopeNode = { name: "", path: "", kind: "folder", children: [] };
  for (const file of files) {
    const parts = file.split("/").filter(Boolean);
    let parent = root;
    parts.forEach((part, index) => {
      const path = parts.slice(0, index + 1).join("/");
      const kind = index === parts.length - 1 ? "file" : "folder";
      let child = parent.children.find((candidate) => candidate.name === part);
      if (!child) {
        child = { name: part, path, kind, children: [] };
        parent.children.push(child);
      }
      parent = child;
    });
  }
  sortScopeNodes(root.children);
  return root.children;
}

function sortScopeNodes(nodes: ScopeNode[]): void {
  nodes.sort((left, right) => {
    if (left.kind !== right.kind) return left.kind === "folder" ? -1 : 1;
    return left.name < right.name ? -1 : left.name > right.name ? 1 : 0;
  });
  nodes.forEach((node) => sortScopeNodes(node.children));
}

function filterScopeTree(nodes: ScopeNode[], rawQuery: string): ScopeNode[] {
  const query = rawQuery.trim().toLowerCase();
  if (!query) return nodes;
  return nodes.flatMap((node) => {
    const children = filterScopeTree(node.children, query);
    if (node.path.toLowerCase().includes(query) || children.length > 0) {
      return [{ ...node, children }];
    }
    return [];
  });
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

function mergeRules(...groups: string[][]): string[] {
  return Array.from(new Set(groups.flat()));
}
