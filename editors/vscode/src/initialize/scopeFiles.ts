import path from "node:path";
import * as vscode from "vscode";

export const MAX_SCOPE_FILES = 5_000;
const SCOPE_SEARCH_EXCLUDE = "**/{.git,.compass,compass-out,node_modules,target}/**";

export type ScopeFileDiscovery = {
  files: string[];
  truncated: boolean;
};

export async function discoverScopeFiles(root: string): Promise<ScopeFileDiscovery> {
  const uris = await vscode.workspace.findFiles(
    new vscode.RelativePattern(root, "**/*"),
    SCOPE_SEARCH_EXCLUDE,
    MAX_SCOPE_FILES + 1
  );
  return normalizeScopeFiles(root, uris.map((uri) => uri.fsPath), MAX_SCOPE_FILES);
}

export function normalizeScopeFiles(
  root: string,
  candidates: readonly string[],
  limit: number
): ScopeFileDiscovery {
  const files = Array.from(new Set(candidates.flatMap((candidate) => {
    const relative = path.relative(root, candidate);
    if (
      !relative
      || path.isAbsolute(relative)
      || relative === ".."
      || relative.startsWith(`..${path.sep}`)
    ) {
      return [];
    }
    return [relative.split(path.sep).join("/")];
  }))).sort();
  return {
    files: files.slice(0, limit),
    truncated: files.length > limit
  };
}
