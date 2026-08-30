const REBUILD_REQUIRED_MARKER =
  "rebuild this revision graph with the current Compass version";

export class RevisionGraphRebuildRequired extends Error {
  constructor(
    public readonly commit: string,
    public readonly detail: string
  ) {
    super(rebuildRequiredMessage(commit));
    this.name = "RevisionGraphRebuildRequired";
  }
}

export async function withRevisionGraphContext<T>(
  commit: string,
  load: () => Promise<T>
): Promise<T> {
  try {
    return await load();
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    if (detail.includes(REBUILD_REQUIRED_MARKER)) {
      throw new RevisionGraphRebuildRequired(commit, detail);
    }
    throw error;
  }
}

export function rebuildRequiredMessage(commit: string): string {
  return `The stored graph for ${commit.slice(0, 9)} uses an unsupported format. Rebuild it with the current Compass version, then try again.`;
}
