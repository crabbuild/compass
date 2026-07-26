import { createHash } from "node:crypto";
import { mkdir, readFile, rename, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import {
  GraphViewModelSchema,
  type GraphViewModel
} from "@compass/viewer/contracts/graph";
import { z } from "zod";

export const GRAPH_OVERVIEW_SCHEMA = "compass.graph-overview/1" as const;

const GraphOverviewArtifactSchema = z.object({
  schema: z.literal(GRAPH_OVERVIEW_SCHEMA),
  sourceGraphBytes: z.number().int().nonnegative(),
  sourceGraphModifiedMs: z.number().nonnegative().optional(),
  nodeLimit: z.number().int().positive(),
  model: GraphViewModelSchema
});

export type GraphSourceInfo = {
  bytes: number;
  modifiedMs: number;
};

export async function graphSourceInfo(graphPath: string): Promise<GraphSourceInfo> {
  const metadata = await stat(graphPath);
  return {
    bytes: metadata.size,
    modifiedMs: metadata.mtimeMs
  };
}

export async function loadPreparedGraphOverview(
  graphPath: string,
  nodeLimit: number
): Promise<GraphViewModel | undefined> {
  const overviewPath = path.join(path.dirname(graphPath), "graph-overview.json");
  try {
    const [graph, overviewMetadata, artifact] = await Promise.all([
      graphSourceInfo(graphPath),
      stat(overviewPath),
      readArtifact(overviewPath)
    ]);
    if (
      overviewMetadata.mtimeMs < graph.modifiedMs ||
      artifact.sourceGraphBytes !== graph.bytes ||
      artifact.nodeLimit !== nodeLimit
    ) {
      return undefined;
    }
    return artifact.model;
  } catch (error) {
    if (isMissing(error)) return undefined;
    return undefined;
  }
}

export function graphOverviewCachePath(storageRoot: string, repositoryId: string): string {
  const key = createHash("sha256").update(repositoryId).digest("hex").slice(0, 24);
  return path.join(storageRoot, "graph-overviews", `${key}.json`);
}

export async function loadCachedGraphOverview(
  cachePath: string,
  graphPath: string,
  nodeLimit: number
): Promise<GraphViewModel | undefined> {
  try {
    const [graph, artifact] = await Promise.all([
      graphSourceInfo(graphPath),
      readArtifact(cachePath)
    ]);
    if (
      artifact.sourceGraphBytes !== graph.bytes ||
      artifact.sourceGraphModifiedMs !== graph.modifiedMs ||
      artifact.nodeLimit !== nodeLimit
    ) {
      return undefined;
    }
    return artifact.model;
  } catch {
    return undefined;
  }
}

export async function writeCachedGraphOverview(
  cachePath: string,
  graphPath: string,
  nodeLimit: number,
  model: GraphViewModel
): Promise<void> {
  const graph = await graphSourceInfo(graphPath);
  await mkdir(path.dirname(cachePath), { recursive: true });
  const temporary = `${cachePath}.${process.pid}.${Date.now()}.tmp`;
  await writeFile(temporary, JSON.stringify({
    schema: GRAPH_OVERVIEW_SCHEMA,
    sourceGraphBytes: graph.bytes,
    sourceGraphModifiedMs: graph.modifiedMs,
    nodeLimit,
    model
  }));
  await rename(temporary, cachePath);
}

async function readArtifact(filePath: string) {
  const parsed = GraphOverviewArtifactSchema.safeParse(
    JSON.parse(await readFile(filePath, "utf8"))
  );
  if (!parsed.success) throw new Error(`Invalid Compass graph overview: ${filePath}`);
  return parsed.data;
}

function isMissing(error: unknown): boolean {
  return (error as NodeJS.ErrnoException).code === "ENOENT";
}
