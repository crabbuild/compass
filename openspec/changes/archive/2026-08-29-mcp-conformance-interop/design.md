# Design: MCP conformance and client interoperability

## Qualification boundary

The independent reference is `modelcontextprotocol/conformance` at commit
`74edef34d674f563537be8c6587cebaa58e830ca` (package version
`0.2.0-alpha.11`). The released npm package `0.1.16` is deliberately not used:
it predates the 2026-07-28 scenarios. CI fetches the exact commit and runs the
applicable stateless, discovery/listing, resource-error, DNS-rebinding, and
standard-header scenarios at `--spec-version 2026-07-28`; moving the pin or the
scenario set is a reviewed change.

The official runner accepts an HTTP URL but has no stdio-server mode. HTTP is
therefore checked directly by that runner. Stdio is checked by a native Rust
conformance test using the same server handler through a bounded duplex stream;
it verifies initialization/discovery, the exact ordered tool and resource lists, one
real typed Compass invocation, protocol errors, and clean shutdown. The two
legs are named separately in CI so neither can disappear behind the other.
The production stdio and HTTP services advertise only 2026-07-28; the stdio
leg also proves that a legacy initialize request is rejected.

The official suite contains diagnostic fixture checks whose named tools are
not product requirements. Compass does not expose test-only protocol methods
in production merely to impersonate the suite's everything-server fixture.
Those individual checks are recorded in a narrow expected-failure file; no
whole scenario is baselined. Any transport, wire-schema, routing, or advertised
capability failure remains a blocking failure.

Four expected failures are matched by check identifier because they require
diagnostic tools or streamed behaviors Compass does not advertise, so they are
not evidence for product capabilities. The fifth calls the first real Compass
tool with `{}`; after the run, the qualification script requires that exact
entry to be HTTP 400 / JSON-RPC `-32602` from invalid tool arguments. A changed
cause, including a header-mismatch regression, fails the gate. None is claimed
as independent evidence for the behavior named by its runner check. Native
coverage pins every behavior Compass does claim, including end-to-end optional-
whitespace normalization; the remaining reference-runner coverage gaps stay
explicit until it can supply valid product arguments.

## Named-client matrix

Interop runs against these installed, exact client versions:

| Client | Version | Stdio | HTTP |
| --- | --- | --- | --- |
| Codex CLI | `0.150.1` | discover + invoke | discover + invoke |
| Claude Code | `2.1.251` | discover + invoke | discover + invoke |
| OpenCode | `1.18.25` | discover + invoke | discover + invoke |

Each run uses a temporary home/config directory and the local Compass graph.
Configuration-only success is insufficient: the receipt must identify a real
Compass tool and contain evidence from its returned payload. Credentials are
inherited only through the client's supported authentication boundary and are
never copied into repository artifacts. Logs are sanitized before a stable
matrix is recorded.

## Gate behavior

The latest published MCP revision and both CI conformance legs are the normative
merge gate. The named-client matrix is required release evidence, but a client
that implements an older MCP revision is recorded as incompatible rather than
forcing Compass to reintroduce removed protocol behavior. A regression in a
previously passing client cell or a failure caused by Compass deviating from the
latest protocol remains blocking. Unavailable credentials or a missing named
binary are reported as not measured, never as a pass.

## Observed gate status

The 2026-08-28 run passed both transports in Codex CLI 0.150.1 and Claude Code
2.1.251, and passed stdio in OpenCode 1.18.25. OpenCode's HTTP cell is
incompatible because that client sends `initialize` with protocol `2025-11-25`
before falling back to the removed GET stream. The same behavior was reproduced
in the day's dev build, whose source still depends on MCP SDK 1.29.0 and contains
no 2026 protocol opt-in. Compass correctly rejects that lifecycle under the
C-008 2026-only contract. On 2026-08-29 the user explicitly chose the latest MCP
revision as the governing requirement, so this observed older-client limitation
does not hold C-010 or Wave 3.
