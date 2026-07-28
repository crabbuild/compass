import { constants } from "node:fs";
import { access } from "node:fs/promises";
import path from "node:path";

export type InstallCommand =
  | {
    kind: "supported";
    platformLabel: string;
    command: string;
    shellPath?: string;
  }
  | {
    kind: "unsupported";
    platformLabel: string;
    message: string;
  };

type CanExecute = (executable: string) => Promise<boolean>;

const POSIX_INSTALL =
  "curl --proto '=https' --tlsv1.2 -LsSf " +
  "https://github.com/crabbuild/compass/releases/latest/download/install.sh | sh";
const WINDOWS_INSTALL =
  "Invoke-RestMethod " +
  "'https://github.com/crabbuild/compass/releases/latest/download/install.ps1' " +
  "| Invoke-Expression";

export async function resolveInstallCommand(
  platform: NodeJS.Platform = process.platform,
  environment: NodeJS.ProcessEnv = process.env,
  canExecute: CanExecute = executableAvailable
): Promise<InstallCommand> {
  if (platform === "darwin") {
    return { kind: "supported", platformLabel: "macOS", command: POSIX_INSTALL };
  }
  if (platform === "linux") {
    return { kind: "supported", platformLabel: "Linux", command: POSIX_INSTALL };
  }
  if (platform !== "win32") {
    return {
      kind: "unsupported",
      platformLabel: platform,
      message: "Install Compass from a supported release archive, then select the executable."
    };
  }

  for (const candidate of powershellCandidates(environment)) {
    if (await canExecute(candidate)) {
      return {
        kind: "supported",
        platformLabel: "Windows",
        command: WINDOWS_INSTALL,
        shellPath: candidate
      };
    }
  }
  return {
    kind: "unsupported",
    platformLabel: "Windows",
    message: "PowerShell was not found. Install Compass from a release archive, then select compass.exe."
  };
}

function powershellCandidates(environment: NodeJS.ProcessEnv): string[] {
  const windowsPath = path.win32;
  const pathValue = environment.PATH ?? environment.Path ?? "";
  const directories = pathValue.split(windowsPath.delimiter).filter(Boolean);
  return deduplicateWindowsPaths([
    ...directories.map((directory) => windowsPath.join(directory, "pwsh.exe")),
    ...(environment.ProgramFiles
      ? [windowsPath.join(environment.ProgramFiles, "PowerShell", "7", "pwsh.exe")]
      : []),
    ...directories.map((directory) => windowsPath.join(directory, "powershell.exe")),
    ...(environment.SystemRoot
      ? [
        windowsPath.join(
          environment.SystemRoot,
          "System32",
          "WindowsPowerShell",
          "v1.0",
          "powershell.exe"
        )
      ]
      : [])
  ]);
}

function deduplicateWindowsPaths(candidates: string[]): string[] {
  const seen = new Set<string>();
  return candidates.filter((candidate) => {
    const key = candidate.toLocaleLowerCase();
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

async function executableAvailable(executable: string): Promise<boolean> {
  try {
    await access(executable, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}
