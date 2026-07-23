# Serve the graph over MCP

Load this reference when an editor or agent needs live Model Context Protocol
access to a graph.

Standard input/output is the local default:

```bash
compass serve compass-out/graph.json
```

HTTP serving is network-visible according to its bind address:

```bash
compass serve \
  --transport http \
  --host 127.0.0.1 \
  --port 8080 \
  --api-key "$COMPASS_MCP_TOKEN"
```

Run `compass serve --help` for the graph selector, path, JSON response mode,
stateful/stateless behavior, and session timeout. Prefer loopback unless the user
explicitly needs remote clients. Require an API key for non-loopback exposure.

Serving is long-lived. Report the chosen graph and endpoint, keep secrets out of
logs, and stop the process when requested. Starting a server does not refresh
the graph; update or extract first when freshness matters.
