import type { RepositorySession } from "../workspace/repositorySession";

export async function loadSemanticDiff(
  session: RepositorySession,
  parent: string,
  commit: string,
  signal?: AbortSignal
): Promise<unknown> {
  const result = await session.processes.run(
    session.root,
    ["diff", parent, commit, "--format", "json"],
    signal
  );
  if (result.code !== 0) throw new Error(result.stderr || `Compass exited with ${result.code}`);
  return JSON.parse(result.stdout);
}
