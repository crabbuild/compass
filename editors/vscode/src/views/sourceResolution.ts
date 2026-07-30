import { realpath } from "node:fs/promises";
import path from "node:path";

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
