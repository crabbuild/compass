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

  it("keeps the ordinary stdout ceiling at 8 MiB", async () => {
    const { child, stdout } = childProcess();
    const processes = new CompassProcessManager(
      "compass",
      vi.fn(() => child) as never
    );
    const command = processes.startCommand("/repo", ["export", "json"]);

    stdout.write("x".repeat(8 * 1024 * 1024 + 1));

    await expect(command.completed).rejects.toThrow(
      "Compass stdout exceeded the 8 MiB safety limit"
    );
    expect(child.kill).toHaveBeenCalledOnce();
  });

  it("allows architecture stdout between 8 MiB and 128 MiB", async () => {
    const { child, stdout } = childProcess();
    const processes = new CompassProcessManager(
      "compass",
      vi.fn(() => child) as never
    );
    const command = processes.startCommand(
      "/repo",
      ["export", "callflow-json"],
      { stdoutBytes: 128 * 1024 * 1024 }
    );
    const payload = "x".repeat(9 * 1024 * 1024);

    stdout.write(payload);
    child.emit("close", 0);

    await expect(command.completed).resolves.toEqual({
      code: 0,
      stdout: payload,
      stderr: ""
    });
  });

  it("measures multibyte output as UTF-8 bytes", async () => {
    const { child, stdout } = childProcess();
    const processes = new CompassProcessManager(
      "compass",
      vi.fn(() => child) as never
    );
    const command = processes.startCommand(
      "/repo",
      ["capabilities"],
      { stdoutBytes: 3 }
    );

    stdout.write("éé");

    await expect(command.completed).rejects.toThrow(
      "Compass stdout exceeded the 0.00 MiB safety limit"
    );
    expect(child.kill).toHaveBeenCalledOnce();
  });

  it("keeps stderr at 8 MiB when stdout has the architecture ceiling", async () => {
    const { child, stderr } = childProcess();
    const processes = new CompassProcessManager(
      "compass",
      vi.fn(() => child) as never
    );
    const command = processes.startCommand(
      "/repo",
      ["export", "callflow-json"],
      { stdoutBytes: 128 * 1024 * 1024 }
    );

    stderr.write("x".repeat(8 * 1024 * 1024 + 1));

    await expect(command.completed).rejects.toThrow(
      "Compass stderr exceeded the 8 MiB safety limit"
    );
    expect(child.kill).toHaveBeenCalledOnce();
  });

  it("switches future launches to an activated executable", () => {
    const first = childProcess();
    const second = childProcess();
    const spawn = vi.fn()
      .mockReturnValueOnce(first.child)
      .mockReturnValueOnce(second.child);
    const processes = new CompassProcessManager("compass", spawn as never);

    processes.startCommand("/repo", ["--version"]);
    processes.useExecutable("/home/dev/.local/bin/compass");
    processes.startCommand("/repo", ["capabilities", "--format", "json"]);

    expect(processes.executablePath).toBe("/home/dev/.local/bin/compass");
    expect(spawn.mock.calls.map(([file]) => file)).toEqual([
      "compass",
      "/home/dev/.local/bin/compass"
    ]);
  });

  it("rejects an empty executable path", () => {
    const processes = new CompassProcessManager("compass");
    expect(() => processes.useExecutable("   "))
      .toThrow("Compass executable path cannot be empty");
    expect(processes.executablePath).toBe("compass");
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
