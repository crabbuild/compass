import { copyFile, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

export class CurrentGraphSnapshot {
  private directory: string | undefined;
  private disposed = false;
  graphPath: string | undefined;

  async replace(sourceGraphPath: string): Promise<string> {
    if (this.disposed) throw new Error("The graph snapshot has been disposed.");
    const directory = await mkdtemp(path.join(tmpdir(), "compass-vscode-graph-"));
    const graphPath = path.join(directory, "graph.json");
    try {
      await copyFile(sourceGraphPath, graphPath);
      const sourceDirectory = path.dirname(sourceGraphPath);
      await Promise.all([
        this.copyOptional(
          path.join(sourceDirectory, ".compass_analysis.json"),
          path.join(directory, ".compass_analysis.json")
        ),
        this.copyOptional(
          path.join(sourceDirectory, ".compass_labels.json"),
          path.join(directory, ".compass_labels.json")
        )
      ]);
    } catch (error) {
      await rm(directory, { recursive: true, force: true });
      throw error;
    }
    if (this.disposed) {
      await rm(directory, { recursive: true, force: true });
      throw new Error("The graph snapshot was disposed while it was being created.");
    }
    const previous = this.directory;
    this.directory = directory;
    this.graphPath = graphPath;
    if (previous) await rm(previous, { recursive: true, force: true });
    return graphPath;
  }

  async dispose(): Promise<void> {
    this.disposed = true;
    const directory = this.directory;
    this.directory = undefined;
    this.graphPath = undefined;
    if (directory) await rm(directory, { recursive: true, force: true });
  }

  private async copyOptional(source: string, destination: string): Promise<void> {
    try {
      await copyFile(source, destination);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
  }
}
