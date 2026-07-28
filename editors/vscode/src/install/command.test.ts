import { describe, expect, it, vi } from "vitest";
import { resolveInstallCommand } from "./command";

describe("resolveInstallCommand", () => {
  it.each([
    ["darwin", "macOS"],
    ["linux", "Linux"]
  ] as const)("returns the official shell installer for %s", async (platform, label) => {
    await expect(resolveInstallCommand(platform)).resolves.toEqual({
      kind: "supported",
      platformLabel: label,
      command:
        "curl --proto '=https' --tlsv1.2 -LsSf " +
        "https://github.com/crabbuild/compass/releases/latest/download/install.sh | sh"
    });
  });

  it("prefers PowerShell 7 on Windows", async () => {
    const canExecute = vi.fn(async (candidate: string) =>
      candidate === "C:\\Tools\\pwsh.exe"
    );

    const result = await resolveInstallCommand(
      "win32",
      { PATH: "C:\\Tools;C:\\Windows\\System32", SystemRoot: "C:\\Windows" },
      canExecute
    );

    expect(result).toMatchObject({
      kind: "supported",
      platformLabel: "Windows",
      shellPath: "C:\\Tools\\pwsh.exe"
    });
    expect(result.kind === "supported" ? result.command : "")
      .toContain("install.ps1");
    expect(canExecute.mock.calls[0]?.[0]).toBe("C:\\Tools\\pwsh.exe");
  });

  it("falls back to in-box Windows PowerShell", async () => {
    const expected =
      "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe";
    const result = await resolveInstallCommand(
      "win32",
      { PATH: "", SystemRoot: "C:\\Windows" },
      async (candidate) => candidate === expected
    );

    expect(result).toMatchObject({
      kind: "supported",
      shellPath: expected
    });
  });

  it("does not run an unknown shell when PowerShell is unavailable", async () => {
    await expect(resolveInstallCommand(
      "win32",
      { PATH: "" },
      async () => false
    )).resolves.toMatchObject({
      kind: "unsupported",
      platformLabel: "Windows"
    });
  });

  it("returns manual guidance for unsupported hosts", async () => {
    await expect(resolveInstallCommand("freebsd")).resolves.toMatchObject({
      kind: "unsupported",
      platformLabel: "freebsd"
    });
  });
});
