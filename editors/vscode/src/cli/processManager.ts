import { randomUUID } from "node:crypto";
import { spawn as nodeSpawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import type { ZodType } from "zod";
import { ProgressEventSchema, type ProgressEvent } from "./contracts";

const OUTPUT_LIMIT = 8 * 1024 * 1024;
type Spawn = typeof nodeSpawn;

export type CommandResult = { code: number; stdout: string; stderr: string };
export type RunningCommand = {
  operationId: string;
  completed: Promise<CommandResult>;
  cancel(): void;
};

export class CompassProcessManager {
  constructor(
    private readonly executable: string,
    private readonly spawn: Spawn = nodeSpawn
  ) {}

  run(cwd: string, args: readonly string[], signal?: AbortSignal): Promise<CommandResult> {
    const command = this.startCommand(cwd, args);
    const cancel = () => command.cancel();
    signal?.addEventListener("abort", cancel, { once: true });
    return command.completed.finally(() => signal?.removeEventListener("abort", cancel));
  }

  startCommand(cwd: string, args: readonly string[]): RunningCommand {
    const child = this.start(cwd, args);
    return {
      operationId: randomUUID(),
      completed: collect(child),
      cancel: () => child.kill()
    };
  }

  async runJson<T>(
    cwd: string,
    args: readonly string[],
    schema: ZodType<T>,
    signal?: AbortSignal
  ): Promise<T> {
    const result = await this.run(cwd, args, signal);
    if (result.code !== 0) throw new Error(result.stderr || `Compass exited with ${result.code}`);
    return schema.parse(JSON.parse(result.stdout));
  }

  startJsonl(
    cwd: string,
    args: readonly string[],
    onEvent: (event: ProgressEvent) => void
  ): RunningCommand {
    const operationId = randomUUID();
    const child = this.start(cwd, args);
    let buffered = "";
    let terminals = 0;
    let progressError: unknown;
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      if (progressError) return;
      try {
        buffered = bounded(buffered, chunk);
        const lines = buffered.split(/\r?\n/);
        buffered = lines.pop() ?? "";
        for (const line of lines.filter(Boolean)) {
          const event = ProgressEventSchema.parse(JSON.parse(line));
          if (event.terminal) terminals += 1;
          onEvent(event);
        }
      } catch (error) {
        progressError = error;
        child.kill();
      }
    });
    const completed = collect(child, false).then((result) => {
      if (progressError) throw progressError;
      if (terminals !== 1) {
        throw new Error(`Compass emitted ${terminals} terminal progress events`);
      }
      return result;
    });
    return { operationId, completed, cancel: () => child.kill() };
  }

  private start(cwd: string, args: readonly string[]): ChildProcessWithoutNullStreams {
    return this.spawn(this.executable, [...args], {
      cwd,
      shell: false,
      windowsHide: true,
      stdio: "pipe"
    });
  }
}

function collect(
  child: ChildProcessWithoutNullStreams,
  captureStdout = true
): Promise<CommandResult> {
  return new Promise((resolve, reject) => {
    let stdout = "";
    let stderr = "";
    let streamError: unknown;
    const append = (current: string, chunk: string): string => {
      if (streamError) return current;
      try {
        return bounded(current, chunk);
      } catch (error) {
        streamError = error;
        child.kill();
        return current;
      }
    };
    if (captureStdout) {
      child.stdout.setEncoding("utf8");
      child.stdout.on("data", (chunk: string) => {
        stdout = append(stdout, chunk);
      });
    }
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      stderr = append(stderr, chunk);
    });
    child.once("error", reject);
    child.once("close", (code) => {
      if (streamError) {
        reject(streamError);
      } else {
        resolve({ code: code ?? 1, stdout, stderr });
      }
    });
  });
}

function bounded(current: string, chunk: string): string {
  if (current.length + chunk.length > OUTPUT_LIMIT) {
    throw new Error("Compass output exceeded the 8 MiB safety limit");
  }
  return current + chunk;
}
