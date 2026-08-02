import * as vscode from "vscode";
import type { SourceLocation } from "@compass/viewer/contracts/graph";
import { resolveSource } from "./sourceResolution";
import { fullLineSourceRange } from "./sourceRange";
export { resolveSource, type SourceResolution } from "./sourceResolution";

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
  const range = fullLineSourceRange(document, source);
  editor.revealRange(range, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
  editor.selection = new vscode.Selection(range.start, range.end);
}
