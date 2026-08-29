import { describe, expect, it } from "vitest"

import CompassPlugin from "../src/index"

type PluginContext = Parameters<typeof CompassPlugin>[0]

describe("installed Compass OpenCode bridge", () => {
  it("loads through the plugin API and emits the native HTTP MCP entry", async () => {
    const context = {
      $: () => {
        throw new Error("the HTTP configuration path must not spawn a process")
      },
    } as unknown as PluginContext
    const hooks = await CompassPlugin(context)
    const registered = hooks.tool?.compass_mcp_config
    expect(registered).toBeDefined()
    const rendered = await registered?.execute(
      { transport: "http" },
      {
        directory: ".",
        worktree: ".",
        sessionID: "integration",
        messageID: "integration",
        agent: "integration",
        abort: AbortSignal.timeout(1_000),
        metadata: () => undefined,
        ask: async () => undefined,
      },
    )
    expect(JSON.parse(String(rendered))).toEqual({
      mcp: {
        compass: {
          type: "remote",
          url: "http://127.0.0.1:8080/mcp",
          enabled: true,
        },
      },
    })
  })
})
