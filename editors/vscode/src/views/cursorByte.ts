import * as vscode from "vscode";

export function utf8ByteAt(document: vscode.TextDocument, position: vscode.Position): number {
  const prefix = document.getText(new vscode.Range(new vscode.Position(0, 0), position));
  return Buffer.byteLength(prefix, "utf8");
}
