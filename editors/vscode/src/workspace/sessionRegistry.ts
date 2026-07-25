import { stat } from "node:fs/promises";
import type { TextEditor, WorkspaceFolder } from "vscode";
import type { CompassProcessManager } from "../cli/processManager";
import { RepositorySession } from "./repositorySession";
import type { GraphState } from "./repositorySession";

export class SessionRegistry {
  private readonly sessions = new Map<string, RepositorySession>();

  constructor(
    folders: readonly WorkspaceFolder[],
    processes: CompassProcessManager
  ) {
    for (const folder of folders) {
      const root = folder.uri.fsPath;
      this.sessions.set(root, new RepositorySession(folder.uri.toString(), root, processes));
    }
  }

  all(): RepositorySession[] {
    return [...this.sessions.values()];
  }

  byId(id: string | undefined): RepositorySession | undefined {
    if (!id) return undefined;
    return this.all().find((session) => session.id === id);
  }

  forEditor(editor: TextEditor | undefined): RepositorySession | undefined {
    if (!editor) return this.all()[0];
    return this.all()
      .filter((session) => editor.document.uri.fsPath.startsWith(`${session.root}/`))
      .sort((left, right) => right.root.length - left.root.length)[0];
  }

  async refresh(): Promise<void> {
    await Promise.all(this.all().map(async (session) => {
      session.graphState = refreshedGraphState(
        session.graphState,
        await exists(session.graphPath),
        session.activeWriter !== undefined
      );
    }));
  }
}

export function refreshedGraphState(
  current: GraphState,
  materialized: boolean,
  hasActiveWriter: boolean
): GraphState {
  if (hasActiveWriter) return "building";
  if (current === "failed") return "failed";
  return materialized ? "available" : "not-materialized";
}

async function exists(file: string): Promise<boolean> {
  try {
    return (await stat(file)).isFile();
  } catch {
    return false;
  }
}
