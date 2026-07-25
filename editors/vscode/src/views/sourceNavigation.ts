import { realpath } from "node:fs/promises";
import path from "node:path";
import * as vscode from "vscode";
import type { SourceLocation } from "@compass/viewer/contracts/graph";

export type SourceResolution =
  | { kind: "ok"; file: string }
  | { kind: "repository-mismatch" }
  | { kind: "outside-repository" };

export async function resolveSource(
  repository: { id: string; root: string },
  repositoryId: string,
  sourceFile: string
): Promise<SourceResolution> {
  if (repository.id !== repositoryId) return { kind: "repository-mismatch" };
  const root = await realpath(repository.root);
  const target = await realpath(path.resolve(root, sourceFile));
  if (target !== root && !target.startsWith(`${root}${path.sep}`)) {
    return { kind: "outside-repository" };
  }
  return { kind: "ok", file: target };
}

export async function openGraphSource(
  repository: { id: string; root: string },
  repositoryId: string,
  source: SourceLocation
): Promise<void> {
  const resolved = await resolveSource(repository, repositoryId, source.file);
  if (resolved.kind !== "ok") {
    throw new Error(
      resolved.kind === "repository-mismatch"
        ? "The source request belongs to another repository."
        : "Compass refused to open a path outside the repository."
    );
  }
  const document = await vscode.workspace.openTextDocument(resolved.file);
  const editor = await vscode.window.showTextDocument(document, { preview: true });
  const start = source.startByte !== undefined
    ? document.positionAt(source.startByte)
    : new vscode.Position(Math.max(0, (source.startLine ?? 1) - 1), 0);
  const end = source.endByte !== undefined
    ? document.positionAt(source.endByte)
    : new vscode.Position(Math.max(start.line, (source.endLine ?? source.startLine ?? 1) - 1), 0);
  editor.revealRange(new vscode.Range(start, end), vscode.TextEditorRevealType.InCenterIfOutsideViewport);
  editor.selection = new vscode.Selection(start, end);
}
