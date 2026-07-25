import { randomUUID } from "node:crypto";
import { chmod, mkdir, readFile, readdir, rm } from "node:fs/promises";
import path from "node:path";
import { HistoricalGraphSchema, type HistoricalGraph } from "@compass/viewer/contracts/history";
import type { RepositorySession } from "../workspace/repositorySession";
import { LruCache } from "./lru";

export class RevisionStore {
  private readonly decoded = new LruCache<string, HistoricalGraph>(3);

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

  async load(commit: string): Promise<HistoricalGraph> {
    const cached = this.decoded.get(commit);
    if (cached) return cached;
    const file = path.join(this.directory, `${randomUUID()}.tmp`);
    try {
      const result = await this.session.processes.run(this.session.root, [
        "history", "export", commit,
        "--format", "viewer-json",
        "--output", file
      ]);
      if (result.code !== 0) throw new Error(result.stderr || `Compass exited with ${result.code}`);
      const decoded = HistoricalGraphSchema.parse(JSON.parse(await readFile(file, "utf8")));
      if (decoded.commit !== commit) {
        throw new Error(`Historical export returned ${decoded.commit}; expected revision ${commit}`);
      }
      this.decoded.set(commit, decoded);
      return decoded;
    } finally {
      await rm(file, { force: true });
    }
  }
}
