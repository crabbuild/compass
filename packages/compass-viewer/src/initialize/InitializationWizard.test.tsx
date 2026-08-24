import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import {
  InitializationWizard,
  type InitializationHost
} from "./InitializationWizard";

function host(): InitializationHost {
  return {
    start: vi.fn(),
    cancel: vi.fn(),
    reset: vi.fn(),
    openGraph: vi.fn(),
    showOutput: vi.fn()
  };
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const match = Array.from(container.querySelectorAll("button"))
    .find((candidate) => candidate.textContent?.trim() === label);
  if (!match) throw new Error(`button ${label} not found`);
  return match;
}

function setTextarea(container: HTMLElement, label: string, value: string): void {
  const textarea = Array.from(container.querySelectorAll("textarea"))
    .find((candidate) => candidate.getAttribute("aria-label") === label);
  if (!textarea) throw new Error(`textarea ${label} not found`);
  const setter = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    "value"
  )?.set;
  flushSync(() => {
    setter?.call(textarea, value);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("InitializationWizard", () => {
  it("reviews custom scope rules before starting the index", () => {
    const wizardHost = host();
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <InitializationWizard
        repositoryName="compass"
        repositoryRoot="/workspace/compass"
        scopeFiles={[
          "packages/api/src/main.ts",
          "src/commands/init.ts",
          "src/views/graph.ts"
        ]}
        host={wizardHost}
      />
    ));

    const customScope = Array.from(container.querySelectorAll<HTMLInputElement>('input[type="radio"]'))
      .find((candidate) => candidate.closest("label")?.textContent?.includes("Custom scope"));
    if (!customScope) throw new Error("custom scope option not found");
    flushSync(() => customScope.click());
    const srcScope = Array.from(container.querySelectorAll<HTMLInputElement>(
      '.init-scope-tree input[type="checkbox"]'
    )).find((candidate) => candidate.closest("label")?.textContent?.trim() === "src");
    if (!srcScope) throw new Error("src tree scope not found");
    flushSync(() => srcScope.click());
    flushSync(() => button(container, "Continue").click());
    setTextarea(container, "Additional include globs", "packages/**");
    setTextarea(container, "Exclude paths and globs", "**/generated/**\nvendor");
    flushSync(() => button(container, "Review configuration").click());

    expect(container.textContent).toContain("packages/**");
    flushSync(() => button(container, "Build Compass index").click());

    expect(wizardHost.start).toHaveBeenCalledWith({
      includes: ["src", "packages/**"],
      excludes: ["**/generated/**", "vendor"],
      replaceExisting: false
    });
    root.unmount();
  });

  it("requires an explicit acknowledgement before replacing existing configuration", () => {
    const wizardHost = host();
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <InitializationWizard
        repositoryName="compass"
        repositoryRoot="/workspace/compass"
        configurationExists
        host={wizardHost}
      />
    ));

    flushSync(() => button(container, "Continue").click());
    flushSync(() => button(container, "Review configuration").click());
    const build = button(container, "Replace configuration and build");
    expect(build.disabled).toBe(true);
    expect(container.textContent).toContain("Existing configuration");

    const acknowledgement = Array.from(container.querySelectorAll<HTMLInputElement>(
      'input[type="checkbox"]'
    )).find((candidate) => candidate.closest("label")?.textContent?.includes(
      "replace .compass/config.toml"
    ));
    if (!acknowledgement) throw new Error("replacement acknowledgement not found");
    flushSync(() => acknowledgement.click());
    expect(build.disabled).toBe(false);
    flushSync(() => build.click());
    expect(wizardHost.start).toHaveBeenCalledWith({
      includes: [],
      excludes: [],
      replaceExisting: true
    });
    root.unmount();
  });

  it("shows file-level progress from the active build", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <InitializationWizard
        repositoryName="compass"
        repositoryRoot="/workspace/compass"
        host={host()}
        status={{
          kind: "building",
          phase: "indexing",
          current: 37,
          total: 120,
          message: "src/history/store.rs"
        }}
      />
    ));

    expect(container.textContent).toContain("37 of 120 files");
    expect(container.textContent).toContain("src/history/store.rs");
    const progress = container.querySelector('[role="progressbar"]');
    expect(progress?.getAttribute("aria-valuenow")).toBe("37");
    expect(button(container, "Cancel build").disabled).toBe(false);
    root.unmount();
  });

  it("offers a managed OCR model install without blocking the index build", () => {
    const wizardHost = { ...host(), installOcrModel: vi.fn() };
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <InitializationWizard
        repositoryName="compass"
        repositoryRoot="/workspace/compass"
        host={wizardHost}
        ocrModel={{
          kind: "missing",
          profile: "pp-ocrv6-small",
          installCommand: "compass models install pp-ocrv6-small"
        }}
      />
    ));

    flushSync(() => button(container, "Continue").click());
    flushSync(() => button(container, "Review configuration").click());
    expect(container.textContent).toContain("Read text from scans and embedded images");
    expect(container.textContent).toContain("PP-OCRv6 Small");
    flushSync(() => button(container, "Install OCR model").click());
    expect(wizardHost.installOcrModel).toHaveBeenCalledOnce();
    expect(button(container, "Build Compass index").disabled).toBe(false);
    root.unmount();
  });

  it("announces cancellation and keeps recovery actions available", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <InitializationWizard
        repositoryName="compass"
        repositoryRoot="/workspace/compass"
        host={host()}
        status={{ kind: "cancelled" }}
      />
    ));

    expect(container.textContent).toContain("Build cancelled");
    expect(button(container, "Resume build").disabled).toBe(false);
    expect(button(container, "Edit configuration").disabled).toBe(false);
    root.unmount();
  });
});
