# MCP conformance and named-client interoperability

This page records the qualification boundary for Compass MCP 2026-07-28, the
latest published MCP revision at the decision date. It describes current test
evidence and named-client compatibility without downgrading the server for
clients that still implement an older revision.

## Reference conformance

HTTP qualification uses `modelcontextprotocol/conformance` at commit
`74edef34d674f563537be8c6587cebaa58e830ca` with the frozen `2026-07-28`
requirements. `scripts/qualify_mcp_conformance.sh` builds a strict graph from a
checked-in source fixture, starts the Compass HTTP server, and runs the named
stateless, listing, resource-error, DNS-rebinding, and header scenarios. CI runs
that script with the native stdio integration test
`protocol_conformance::stdio_conformance_discovers_lists_invokes_reads_and_closes`.

The expected-failure file is check-granular. Its entries cover only reference
runner diagnostics that require fixture-only tools Compass does not advertise,
plus a header check whose runner calls the first real Compass tool with invalid
arguments. Transport, wire-schema, discovery, routing, cache metadata, error
identity, resource errors, and security failures are not baselined.

## Named-client run on 2026-08-28

All runs used the installed client version, isolated configuration, the local
Compass binary, and a real `graph_stats` call. A pass required discovery and a
returned Compass payload; configuration or process startup alone did not count.

| Client | Version | Stdio | HTTP |
| --- | --- | --- | --- |
| Codex CLI | `0.150.1` | PASS — `Nodes: 120215` | PASS — `Nodes: 120215` |
| Claude Code | `2.1.251` | PASS — `Nodes: 120215` | PASS — `Nodes: 120215` |
| OpenCode | `1.18.25` | PASS — `Nodes: 120215` | INCOMPATIBLE — sends `initialize` for `2025-11-25`, then attempts the removed GET event stream |

Codex stdio and HTTP were exercised without a model through the installed
app-server's `mcpServerStatus/list` and `mcpServer/tool/call` interfaces using
`scripts/qualify_codex_mcp_client.py`. Claude Code used `--strict-mcp-config`
and an allowlist containing only `mcp__compass__graph_stats`. OpenCode used
`OPENCODE_CONFIG_CONTENT`, `--pure`, and an explicit project path for the tool
call. No credentials or unsanitized client logs are stored in the repository.

Codex 0.146.0 initially failed the HTTP cell, but the isolated official 0.150.1
release passed both cells with its `mcp_2026_07_28` feature enabled. The
OpenCode HTTP failure was reproduced in the installed 1.18.23 build, latest
stable 1.18.25 build, and the 2026-08-28 dev build. The stable client sent:

```text
POST /mcp
User-Agent: opencode/1.18.25

initialize(protocolVersion = "2025-11-25")
```

It omitted the MCP 2026 request headers, received Compass's typed
`Unsupported protocol version` response, and fell back to `GET /mcp`, which
Compass correctly answers with `405 Method Not Allowed`. MCP 2026-07-28 removed
both `initialize` and the independent GET stream.

## Gate status

On 2026-08-29 the user selected the latest published MCP revision as the
governing requirement. Both required 2026-07-28 conformance legs pass, so C-010
may close with the OpenCode HTTP cell recorded as incompatible. Compass does not
make that cell appear green and does not accept the removed 2025 initialize/GET
lifecycle on either transport. A future OpenCode release can add a passing
matrix entry without a
Compass compatibility mode; a Compass conformance regression or a regression
in a previously passing cell remains blocking.

## Reproduction

Build Compass and run the deterministic server checks:

```bash
cargo build -p compass-cli --bin compass --locked
cargo test -p compass-mcp --test protocol_conformance --locked
scripts/qualify_mcp_conformance.sh
```

Run the model-free Codex client harness for either transport:

```bash
scripts/qualify_codex_mcp_client.py \
  --codex "$(command -v codex)" \
  --expected-version 0.150.1 \
  --compass "$PWD/target/debug/compass" \
  --graph "$PWD/compass-out/graph.json" \
  --transport stdio
```

Replace `stdio` with `http` to reproduce the recorded Codex 0.150.1 HTTP pass.
