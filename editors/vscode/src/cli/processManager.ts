import { randomUUID } from "node:crypto";
import { spawn as nodeSpawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import type { ZodType } from "zod";
import { ProgressEventSchema, type ProgressEvent } from "./contracts";

const DEFAULT_OUTPUT_LIMIT = 8 * 1024 * 1024;
type Spawn = typeof nodeSpawn;

export type CommandResult = { code: number; stdout: string; stderr: string };
export type OutputLimits = {
  stdoutBytes?: number;
  stderrBytes?: number;
};
export type RunningCommand = {
  operationId: string;
  completed: Promise<CommandResult>;
  cancel(): void;
};

export class CompassProcessManager {
  private executable: string;

  constructor(
    executable: string,
    private readonly spawn: Spawn = nodeSpawn
  ) {
    this.executable = executable;
  }

  get executablePath(): string {
    return this.executable;
  }

  useExecutable(executable: string): void {
    const next = executable.trim();
    if (!next) throw new Error("Compass executable path cannot be empty");
    this.executable = next;
  }

  run(
    cwd: string,
    args: readonly string[],
    signal?: AbortSignal,
    limits?: OutputLimits
  ): Promise<CommandResult> {
    const command = this.startCommand(cwd, args, limits);
    const cancel = () => command.cancel();
    signal?.addEventListener("abort", cancel, { once: true });
    return command.completed.finally(() => signal?.removeEventListener("abort", cancel));
  }

  startCommand(
    cwd: string,
    args: readonly string[],
    limits?: OutputLimits
  ): RunningCommand {
    const child = this.start(cwd, args);
    return {
      operationId: randomUUID(),
      completed: collect(child, true, limits),
      cancel: () => child.kill()
    };
  }

  async runJson<T>(
    cwd: string,
    args: readonly string[],
    schema: ZodType<T>,
    signal?: AbortSignal,
    limits?: OutputLimits
  ): Promise<T> {
    const result = await this.run(cwd, args, signal, limits);
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
        buffered = boundedPartial(buffered, chunk, DEFAULT_OUTPUT_LIMIT, "stdout");
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
  captureStdout = true,
  limits: OutputLimits = {}
): Promise<CommandResult> {
  return new Promise((resolve, reject) => {
    const stdout = new BoundedTextBuffer(
      "stdout",
      limits.stdoutBytes ?? DEFAULT_OUTPUT_LIMIT
    );
    const stderr = new BoundedTextBuffer(
      "stderr",
      limits.stderrBytes ?? DEFAULT_OUTPUT_LIMIT
    );
    let streamError: unknown;
    const append = (buffer: BoundedTextBuffer, chunk: string): void => {
      if (streamError) return;
      try {
        buffer.append(chunk);
      } catch (error) {
        streamError = error;
        child.kill();
      }
    };
    if (captureStdout) {
      child.stdout.setEncoding("utf8");
      child.stdout.on("data", (chunk: string) => {
        append(stdout, chunk);
      });
    }
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      append(stderr, chunk);
    });
    child.once("error", reject);
    child.once("close", (code) => {
      if (streamError) {
        reject(streamError);
      } else {
        resolve({ code: code ?? 1, stdout: stdout.value, stderr: stderr.value });
      }
    });
  });
}

class BoundedTextBuffer {
  private text = "";
  private bytes = 0;

  constructor(
    private readonly stream: "stdout" | "stderr",
    private readonly limit: number
  ) {}

  get value(): string {
    return this.text;
  }

  append(chunk: string): void {
    const nextBytes = Buffer.byteLength(chunk, "utf8");
    if (this.bytes + nextBytes > this.limit) {
      throw outputLimitError(this.stream, this.limit);
    }
    this.text += chunk;
    this.bytes += nextBytes;
  }
}

function boundedPartial(
  current: string,
  chunk: string,
  limit: number,
  stream: "stdout" | "stderr"
): string {
  if (
    Buffer.byteLength(current, "utf8") + Buffer.byteLength(chunk, "utf8") > limit
  ) {
    throw outputLimitError(stream, limit);
  }
  return current + chunk;
}

function outputLimitError(stream: "stdout" | "stderr", limit: number): Error {
  const mib = limit / (1024 * 1024);
  const display = Number.isInteger(mib) ? mib.toFixed(0) : mib.toFixed(2);
  return new Error(`Compass ${stream} exceeded the ${display} MiB safety limit`);
}
