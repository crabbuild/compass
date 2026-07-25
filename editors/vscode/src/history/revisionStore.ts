import { randomUUID } from "node:crypto";
import { chmod, mkdir, readFile, readdir, rm } from "node:fs/promises";
import path from "node:path";
import { HistoricalGraphSchema, type HistoricalGraph } from "@compass/viewer/contracts/history";
import type { RepositorySession } from "../workspace/repositorySession";
import { historicalGraphExportArgs } from "../views/communityArguments";
import { LruCache } from "./lru";

export class RevisionStore {
  private readonly decoded = new LruCache<string, HistoricalGraph>(3);
  private readonly communities = new LruCache<string, HistoricalGraph>(12);

  constructor(
    private readonly directory: string,
    private readonly session: RepositorySession
  ) {}

  async initialize(): Promise<void> {
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    await chmod(this.directory, 0o700).catch(() => undefined);
    const files = await readdir(this.directory);
    await Promise.all(files
      .filter((file) => file.endsWith(".tmp"))
      .map((file) => rm(path.join(this.directory, file), { force: true })));
  }

  async load(
    commit: string,
    nodeLimit: number,
    expected?: Pick<HistoricalGraph, "realization" | "fingerprint">
  ): Promise<HistoricalGraph> {
    const key = `${commit}:${nodeLimit}`;
    const cached = this.decoded.get(key);
    if (cached && this.matchesIdentity(cached, expected)) return cached;
    if (cached) this.decoded.delete(key);
    const decoded = await this.export(commit, nodeLimit);
    if (!this.matchesIdentity(decoded, expected)) {
      throw new Error(
        "The preferred historical realization changed while loading this revision. Refresh the timeline and try again."
      );
    }
    this.decoded.set(key, decoded);
    return decoded;
  }

  async loadCommunity(
    commit: string,
    communityId: number,
    nodeLimit: number,
    expected: Pick<HistoricalGraph, "realization" | "fingerprint">
  ): Promise<HistoricalGraph> {
    const key = [
      commit,
      expected.realization,
      expected.fingerprint,
      communityId,
      nodeLimit
    ].join(":");
    const cached = this.communities.get(key);
    if (cached) return cached;
    const decoded = await this.export(commit, nodeLimit, communityId);
    if (decoded.realization !== expected.realization
      || decoded.fingerprint !== expected.fingerprint) {
      this.decoded.delete(`${commit}:${nodeLimit}`);
      throw new Error(
        "The preferred historical realization changed while loading this community. Reopen the revision and try again."
      );
    }
    this.communities.set(key, decoded);
    return decoded;
  }

  private async export(
    commit: string,
    nodeLimit: number,
    communityId?: number
  ): Promise<HistoricalGraph> {
    const file = path.join(this.directory, `${randomUUID()}.tmp`);
    try {
      const args = historicalGraphExportArgs(commit, file, nodeLimit, communityId);
      const result = await this.session.processes.run(this.session.root, args);
      if (result.code !== 0) throw new Error(result.stderr || `Compass exited with ${result.code}`);
      const decoded = HistoricalGraphSchema.parse(JSON.parse(await readFile(file, "utf8")));
      if (decoded.commit !== commit) {
        throw new Error(`Historical export returned ${decoded.commit}; expected revision ${commit}`);
      }
      return decoded;
    } finally {
      await rm(file, { force: true });
    }
  }

  private matchesIdentity(
    graph: HistoricalGraph,
    expected?: Pick<HistoricalGraph, "realization" | "fingerprint">
  ): boolean {
    return !expected
      || (graph.realization === expected.realization
        && graph.fingerprint === expected.fingerprint);
  }
}
