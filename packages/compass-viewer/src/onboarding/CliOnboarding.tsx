import {
  AlertTriangle,
  ArrowRight,
  CheckCircle2,
  CloudDownload,
  FolderOpen,
  LoaderCircle,
  Map,
  Search,
  SquareTerminal
} from "lucide-react";
import type { ReactNode } from "react";

export type CliOnboardingState =
  | {
    kind: "ready-to-install";
    platform: string;
    command: string;
  }
  | {
    kind: "installing";
    platform: string;
    command: string;
  }
  | { kind: "verifying" }
  | {
    kind: "ready";
    version: string;
    executable: string;
    hasWorkspace: boolean;
  }
  | {
    kind: "error";
    title: string;
    message: string;
    searched?: string[] | undefined;
    canVerifyAgain: boolean;
  }
  | {
    kind: "unsupported";
    platform: string;
    message: string;
  };

export type CliOnboardingHost = {
  install(): void;
  verifyAgain(): void;
  selectExisting(): void;
  initializeRepository(): void;
  openRepository(): void;
  showTerminal(): void;
};

type Props = {
  state: CliOnboardingState;
  host: CliOnboardingHost;
};

export function CliOnboarding({ state, host }: Props) {
  if (state.kind === "installing") {
    return (
      <OnboardingCard icon={<LoaderCircle className="cli-onboarding-spinner" />}>
        <p className="init-eyebrow">Compass setup</p>
        <h1>Installing Compass…</h1>
        <p role="status" aria-live="polite">
          The official installer is running in the visible Compass Setup terminal.
        </p>
        <Command command={state.command} platform={state.platform} />
        <Actions>
          <button className="init-button init-button-secondary" onClick={host.showTerminal}>
            <SquareTerminal aria-hidden="true" />
            View terminal
          </button>
        </Actions>
      </OnboardingCard>
    );
  }

  if (state.kind === "verifying") {
    return (
      <OnboardingCard icon={<Search className="cli-onboarding-spinner" />}>
        <p className="init-eyebrow">Compass setup</p>
        <h1>Verifying installation…</h1>
        <p role="status" aria-live="polite">
          Checking the executable, version, and capabilities required by this extension.
        </p>
        <Actions>
          <button className="init-button init-button-secondary" onClick={host.showTerminal}>
            <SquareTerminal aria-hidden="true" />
            View terminal
          </button>
        </Actions>
      </OnboardingCard>
    );
  }

  if (state.kind === "ready") {
    return (
      <OnboardingCard icon={<CheckCircle2 /> } success>
        <p className="init-eyebrow">Installation complete</p>
        <h1>Compass is ready</h1>
        <p>Compass {state.version} is verified and active in this VS Code window.</p>
        <dl className="cli-onboarding-facts">
          <div>
            <dt>Version</dt>
            <dd>{state.version}</dd>
          </div>
          <div>
            <dt>Executable</dt>
            <dd title={state.executable}>{state.executable}</dd>
          </div>
        </dl>
        <Actions>
          {state.hasWorkspace ? (
            <button
              className="init-button init-button-primary"
              onClick={host.initializeRepository}
            >
              Initialize repository
              <ArrowRight aria-hidden="true" />
            </button>
          ) : (
            <button className="init-button init-button-primary" onClick={host.openRepository}>
              <FolderOpen aria-hidden="true" />
              Open repository folder
            </button>
          )}
        </Actions>
      </OnboardingCard>
    );
  }

  if (state.kind === "error") {
    return (
      <OnboardingCard icon={<AlertTriangle />}>
        <p className="init-eyebrow">Setup stopped</p>
        <h1>{state.title}</h1>
        <p role="alert">{state.message}</p>
        {state.searched && state.searched.length > 0 && (
          <div className="cli-onboarding-searched">
            <strong>Searched locations</strong>
            <ul>
              {state.searched.map((location) => (
                <li key={location}><code>{location}</code></li>
              ))}
            </ul>
          </div>
        )}
        <Actions>
          {state.canVerifyAgain ? (
            <button className="init-button init-button-primary" onClick={host.verifyAgain}>
              Verify again
            </button>
          ) : (
            <button className="init-button init-button-primary" onClick={host.install}>
              Try again
            </button>
          )}
          <button className="init-button init-button-secondary" onClick={host.showTerminal}>
            <SquareTerminal aria-hidden="true" />
            View terminal
          </button>
          <button className="init-button init-button-quiet" onClick={host.selectExisting}>
            Select an existing CLI
          </button>
        </Actions>
      </OnboardingCard>
    );
  }

  if (state.kind === "unsupported") {
    return (
      <OnboardingCard icon={<AlertTriangle />}>
        <p className="init-eyebrow">Compass setup</p>
        <h1>Automatic installation is unavailable</h1>
        <p>{state.message}</p>
        <p className="cli-onboarding-platform">Workspace host: {state.platform}</p>
        <Actions>
          <button className="init-button init-button-primary" onClick={host.selectExisting}>
            Select an existing CLI
          </button>
        </Actions>
      </OnboardingCard>
    );
  }

  return (
    <OnboardingCard icon={<Map />}>
      <p className="init-eyebrow">Compass for VS Code</p>
      <h1>Get started with Compass</h1>
      <p>
        Install the local Compass CLI to map this codebase. The extension does not
        bundle a native executable or send telemetry.
      </p>
      <div className="cli-onboarding-assurance">
        <CloudDownload aria-hidden="true" />
        <span>
          <strong>Visible, verified installation</strong>
          <small>
            VS Code opens a terminal, runs the official installer, then checks the
            installed version and capabilities.
          </small>
        </span>
      </div>
      <Command command={state.command} platform={state.platform} />
      <Actions>
        <button className="init-button init-button-primary" onClick={host.install}>
          <SquareTerminal aria-hidden="true" />
          Install Compass
        </button>
        <button className="init-button init-button-secondary" onClick={host.selectExisting}>
          Select an existing CLI
        </button>
      </Actions>
    </OnboardingCard>
  );
}

function OnboardingCard({
  children,
  icon,
  success = false
}: {
  children: ReactNode;
  icon: ReactNode;
  success?: boolean;
}) {
  return (
    <main className="init-shell init-result-shell cli-onboarding-shell">
      <section className="init-result-card cli-onboarding-card">
        <span
          className={`init-result-icon${success ? " init-result-icon-success" : ""}`}
          aria-hidden="true"
        >
          {icon}
        </span>
        {children}
      </section>
    </main>
  );
}

function Command({ command, platform }: { command: string; platform: string }) {
  return (
    <div className="cli-onboarding-command">
      <span>Command for {platform}</span>
      <pre><code>{command}</code></pre>
    </div>
  );
}

function Actions({ children }: { children: ReactNode }) {
  return <div className="init-result-actions cli-onboarding-actions">{children}</div>;
}
