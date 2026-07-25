import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import type { ChildProcessWithoutNullStreams, SpawnOptionsWithoutStdio } from "node:child_process";
import { describe, expect, it, vi } from "vitest";
import { CompassProcessManager } from "./processManager";

function childProcess(): {
  child: ChildProcessWithoutNullStreams;
  stdout: PassThrough;
  stderr: PassThrough;
} {
  const child = new EventEmitter() as ChildProcessWithoutNullStreams;
  const stdout = new PassThrough();
  const stderr = new PassThrough();
  Object.assign(child, {
    stdin: new PassThrough(),
    stdout,
    stderr,
    kill: vi.fn(() => {
      queueMicrotask(() => child.emit("close", 143));
      return true;
    })
  });
  return { child, stdout, stderr };
}

describe("CompassProcessManager", () => {
  it("spawns without a shell and exposes real cancellation", async () => {
    const { child } = childProcess();
    const spawn = vi.fn(
      (_file: string, _args: readonly string[], _options: SpawnOptionsWithoutStdio) => child
    );
    const processes = new CompassProcessManager("compass", spawn as never);

    const command = processes.startCommand("/repo", ["watch", ".", "--poll"]);
    command.cancel();
    const result = await command.completed;

    expect(child.kill).toHaveBeenCalledOnce();
    expect(result.code).toBe(143);
    expect(spawn).toHaveBeenCalledWith(
      "compass",
      ["watch", ".", "--poll"],
      expect.objectContaining({ cwd: "/repo", shell: false, stdio: "pipe" })
    );
  });

  it("captures bounded stdout and stderr", async () => {
    const { child, stdout, stderr } = childProcess();
    const processes = new CompassProcessManager(
      "compass",
      vi.fn(() => child) as never
    );
    const command = processes.startCommand("/repo", ["capabilities"]);

    stdout.write('{"schema":"compass.capabilities/1"}');
    stderr.write("note");
    child.emit("close", 0);

    await expect(command.completed).resolves.toEqual({
      code: 0,
      stdout: '{"schema":"compass.capabilities/1"}',
      stderr: "note"
    });
  });

  it("rejects malformed JSONL progress instead of throwing outside the command", async () => {
    const { child, stdout } = childProcess();
    const processes = new CompassProcessManager(
      "compass",
      vi.fn(() => child) as never
    );
    const command = processes.startJsonl("/repo", ["history", "build"], vi.fn());

    stdout.write("{not-json}\n");

    await expect(command.completed).rejects.toBeInstanceOf(Error);
    expect(child.kill).toHaveBeenCalledOnce();
  });
});
