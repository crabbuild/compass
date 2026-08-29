import { type Plugin, tool } from "@opencode-ai/plugin"

const MCP_URL = "http://127.0.0.1:8080/mcp"

/** Thin OpenCode bridge. Compass owns every graph and query operation. */
export const CompassPlugin: Plugin = async ({ $ }) => ({
  tool: {
    compass_mcp_config: tool({
      description: "Emit a credential-free Compass MCP configuration for OpenCode.",
      args: {
        transport: tool.schema.enum(["stdio", "http"]).default("stdio"),
      },
      async execute({ transport }) {
        if (transport === "http") {
          return JSON.stringify({
            mcp: {
              compass: { type: "remote", url: MCP_URL, enabled: true },
            },
          }, null, 2)
        }
        const result = await $`compass agent mcp-config --platform opencode --transport stdio`.quiet()
        return result.text()
      },
    }),
  },
})

export default CompassPlugin
