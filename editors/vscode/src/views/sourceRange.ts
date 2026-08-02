import * as vscode from "vscode";
import type { SourceLocation } from "@compass/viewer/contracts/graph";

export function fullLineSourceRange(
  document: vscode.TextDocument,
  source: SourceLocation
): vscode.Range {
  // Compass line locations are 1-based and inclusive; VS Code ranges are
  // 0-based and end-exclusive.
  const byteStart = source.startByte !== undefined
    ? document.positionAt(source.startByte)
    : undefined;
  const startLine = source.startLine !== undefined
    ? source.startLine - 1
    : byteStart?.line ?? 0;

  const byteEnd = source.endByte !== undefined
    ? document.positionAt(source.endByte)
    : undefined;
  const byteEndLineExclusive = byteEnd !== undefined
    ? byteEnd.line + (
      byteEnd.character === 0 && source.endByte !== source.startByte ? 0 : 1
    )
    : undefined;
  const endLineExclusive = source.endLine
    ?? source.startLine
    ?? byteEndLineExclusive
    ?? startLine + 1;

  const start = document.validatePosition(
    new vscode.Position(Math.max(0, startLine), 0)
  );
  const end = document.validatePosition(
    new vscode.Position(Math.max(start.line + 1, endLineExclusive), 0)
  );
  return new vscode.Range(start, end);
}
