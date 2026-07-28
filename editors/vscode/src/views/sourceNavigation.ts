import * as vscode from "vscode";
import type { SourceLocation } from "@compass/viewer/contracts/graph";
import { resolveSource } from "./sourceResolution";
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
  const start = source.startByte !== undefined
    ? document.positionAt(source.startByte)
    : new vscode.Position(Math.max(0, (source.startLine ?? 1) - 1), 0);
  const recordedEndLine = source.endLine ?? source.startLine;
  const end = recordedEndLine !== undefined
    ? document.validatePosition(
      new vscode.Position(Math.max(start.line, recordedEndLine), 0)
    )
    : source.endByte !== undefined
      ? document.positionAt(source.endByte)
      : start;
  editor.revealRange(new vscode.Range(start, end), vscode.TextEditorRevealType.InCenterIfOutsideViewport);
  editor.selection = new vscode.Selection(start, end);
}
