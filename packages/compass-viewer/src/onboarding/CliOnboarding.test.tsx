import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import {
  CliOnboarding,
  type CliOnboardingHost,
  type CliOnboardingState
} from "./CliOnboarding";

function host(): CliOnboardingHost {
  return {
    install: vi.fn(),
    verifyAgain: vi.fn(),
    selectExisting: vi.fn(),
    initializeRepository: vi.fn(),
    openRepository: vi.fn(),
    showTerminal: vi.fn()
  };
}

function render(state: CliOnboardingState, onboardingHost = host()) {
  const container = document.createElement("div");
  const root = createRoot(container);
  flushSync(() => root.render(
    <CliOnboarding state={state} host={onboardingHost} />
  ));
  return { container, root, host: onboardingHost };
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const match = Array.from(container.querySelectorAll("button"))
    .find((candidate) => candidate.textContent?.trim() === label);
  if (!match) throw new Error(`button ${label} not found`);
  return match;
}

describe("CliOnboarding", () => {
  it("shows the exact command before running the installer", () => {
    const command = "curl https://example.invalid/install.sh | sh";
    const mounted = render({
      kind: "ready-to-install",
      platform: "macOS",
      command
    });

    expect(mounted.container.textContent).toContain(command);
    flushSync(() => button(mounted.container, "Install Compass").click());
    expect(mounted.host.install).toHaveBeenCalledOnce();
    expect(mounted.container.querySelectorAll("button")).toHaveLength(2);
    mounted.root.unmount();
  });

  it("announces installation and verification without another install action", () => {
    for (const state of [
      {
        kind: "installing" as const,
        platform: "Linux",
        command: "curl https://example.invalid/install.sh | sh"
      },
      { kind: "verifying" as const }
    ]) {
      const mounted = render(state);
      expect(mounted.container.querySelector('[role="status"]')).not.toBeNull();
      expect(mounted.container.textContent).not.toContain("Install Compass");
      expect(button(mounted.container, "View terminal")).not.toBeNull();
      mounted.root.unmount();
    }
  });

  it("continues to initialization when a workspace is open", () => {
    const mounted = render({
      kind: "ready",
      version: "0.1.7",
      executable: "/home/dev/.local/bin/compass",
      hasWorkspace: true
    });

    expect(mounted.container.textContent).toContain("Compass is ready");
    expect(mounted.container.textContent).toContain("/home/dev/.local/bin/compass");
    flushSync(() => button(mounted.container, "Initialize repository").click());
    expect(mounted.host.initializeRepository).toHaveBeenCalledOnce();
    mounted.root.unmount();
  });

  it("opens a folder when installation finishes without a workspace", () => {
    const mounted = render({
      kind: "ready",
      version: "0.1.7",
      executable: "C:\\Users\\dev\\.local\\bin\\compass.exe",
      hasWorkspace: false
    });

    flushSync(() => button(mounted.container, "Open repository folder").click());
    expect(mounted.host.openRepository).toHaveBeenCalledOnce();
    mounted.root.unmount();
  });

  it("keeps bounded recovery actions and searched locations visible", () => {
    const mounted = render({
      kind: "error",
      title: "Compass was not found",
      message: "The installer finished but verification could not find Compass.",
      searched: ["/usr/bin/compass", "/home/dev/.local/bin/compass"],
      canVerifyAgain: true
    });

    expect(mounted.container.querySelector('[role="alert"]')?.textContent)
      .toContain("verification");
    expect(mounted.container.textContent).toContain("/usr/bin/compass");
    flushSync(() => button(mounted.container, "Verify again").click());
    flushSync(() => button(mounted.container, "View terminal").click());
    flushSync(() => button(mounted.container, "Select an existing CLI").click());
    expect(mounted.host.verifyAgain).toHaveBeenCalledOnce();
    expect(mounted.host.showTerminal).toHaveBeenCalledOnce();
    expect(mounted.host.selectExisting).toHaveBeenCalledOnce();
    mounted.root.unmount();
  });

  it("retries the installer after an installer failure", () => {
    const mounted = render({
      kind: "error",
      title: "Installation failed",
      message: "The installer exited with code 1.",
      canVerifyAgain: false
    });

    flushSync(() => button(mounted.container, "Try again").click());
    expect(mounted.host.install).toHaveBeenCalledOnce();
    mounted.root.unmount();
  });

  it("does not offer automatic installation on an unsupported host", () => {
    const mounted = render({
      kind: "unsupported",
      platform: "freebsd",
      message: "Install Compass from a release archive."
    });

    expect(Array.from(mounted.container.querySelectorAll("button"))
      .some((candidate) => candidate.textContent?.trim() === "Install Compass"))
      .toBe(false);
    expect(button(mounted.container, "Select an existing CLI")).not.toBeNull();
    mounted.root.unmount();
  });
});
