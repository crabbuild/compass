import {
  TaskContextSchema,
  type TaskContext
} from "@compass/viewer/contracts/taskContext";
import type { RepositorySession } from "../workspace/repositorySession";

export type TaskContextIntent = "explain" | "modify" | "debug" | "test";

export function taskContextArguments(
  intent: TaskContextIntent,
  target: string,
  graphPath: string,
  repositoryRoot: string
): string[] {
  return [
    "context",
    intent,
    operand(target),
    "--graph",
    graphPath,
    "--root",
    repositoryRoot,
    "--format",
    "json"
  ];
}

export async function runTaskContext(
  session: RepositorySession,
  intent: TaskContextIntent,
  target: string,
  signal?: AbortSignal
): Promise<TaskContext> {
  return session.processes.runJson(
    session.root,
    taskContextArguments(intent, target, session.graphPath, session.root),
    TaskContextSchema,
    signal
  );
}

function operand(value: string): string {
  const normalized = value.trim();
  if (!normalized || normalized.startsWith("--")) {
    throw new Error("Compass task-context values must be non-empty and cannot begin with '--'");
  }
  return normalized;
}
