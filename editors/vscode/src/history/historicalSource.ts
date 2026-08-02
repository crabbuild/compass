import { execFile } from "node:child_process";
import path from "node:path";
import * as vscode from "vscode";
import {
  SourceLocationSchema,
  type SourceLocation
} from "@compass/viewer/contracts/graph";
import { fullLineSourceRange } from "../views/sourceRange";

const MAX_SOURCE_BYTES = 8 * 1024 * 1024;

export type GitSourceReader = (
  cwd: string,
  args: readonly string[]
) => Promise<string>;

export class HistoricalSourceProvider implements vscode.TextDocumentContentProvider {
  private readonly entries = new Map<string, {
    commit: string;
    relativePath: string;
  }>();

  constructor(
    readonly scheme: string,
    private readonly repositoryRoot: string,
    private readonly readGitSource: GitSourceReader = defaultGitSourceReader
  ) {}

  async provideTextDocumentContent(uri: vscode.Uri): Promise<string> {
    const token = new URLSearchParams(uri.query).get("token");
    const entry = token ? this.entries.get(token) : undefined;
    if (!entry) throw new Error("This historical source request is no longer available.");
    return this.readGitSource(
      this.repositoryRoot,
      historicalSourceArgs(entry.commit, entry.relativePath)
    );
  }

  async open(commit: string, source: SourceLocation): Promise<void> {
    if (!/^[0-9a-f]{7,64}$/i.test(commit)) {
      throw new Error("Compass refused an invalid historical revision.");
    }
    const relativePath = repositoryRelativePath(this.repositoryRoot, source.file);
    const token = crypto.randomUUID();
    this.entries.set(token, { commit, relativePath });
    const uri = vscode.Uri.from({
      scheme: this.scheme,
      path: `/${commit.slice(0, 9)}/${relativePath}`,
      query: new URLSearchParams({ token }).toString()
    });
    const document = await vscode.workspace.openTextDocument(uri);
    const editor = await vscode.window.showTextDocument(document, { preview: true });
    revealSource(editor, document, source);
  }

  dispose(): void {
    this.entries.clear();
  }
}

export function repositoryRelativePath(root: string, sourceFile: string): string {
  const resolved = path.resolve(root, sourceFile);
  const relative = path.relative(path.resolve(root), resolved);
  if (!relative
    || relative === ".."
    || relative.startsWith(`..${path.sep}`)
    || path.isAbsolute(relative)) {
    throw new Error("Compass refused to open a path outside the repository.");
  }
  return relative.split(path.sep).join("/");
}

export function historicalSourceArgs(
  commit: string,
  relativePath: string
): string[] {
  return ["show", "--no-textconv", `${commit}:${relativePath}`];
}

export function parseHistoricalSourceLocation(value: unknown): SourceLocation {
  const parsed = SourceLocationSchema.safeParse(value);
  if (!parsed.success) {
    throw new Error("Compass refused an invalid historical source location.");
  }
  return parsed.data;
}

export function revealSource(
  editor: vscode.TextEditor,
  document: vscode.TextDocument,
  source: SourceLocation
): void {
  const range = fullLineSourceRange(document, source);
  editor.revealRange(range, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
  editor.selection = new vscode.Selection(range.start, range.end);
}

function defaultGitSourceReader(
  cwd: string,
  args: readonly string[]
): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(
      "git",
      [...args],
      {
        cwd,
        encoding: "utf8",
        maxBuffer: MAX_SOURCE_BYTES,
        windowsHide: true
      },
      (error, stdout, stderr) => {
        if (error) {
          reject(new Error(stderr.trim() || error.message));
        } else {
          resolve(stdout);
        }
      }
    );
  });
}
