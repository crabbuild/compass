# Report a Compass security vulnerability

Report suspected vulnerabilities privately so maintainers can investigate before exploit details become public. Don't open a public issue, pull request, or discussion for a vulnerability.

## Supported code

Security fixes target these versions:

| Version | Support |
| --- | --- |
| Default branch | Fixes are developed and verified here |
| Latest release | Supported with security updates |
| Older releases | Upgrade to the latest release before requesting a fix |

## Submit a private report

Use [GitHub private vulnerability reporting](https://github.com/crabbuild/compass/security/advisories/new). If the private reporting form isn't available, wait for the repository owner to enable it rather than publishing exploit details.

Include the information that helps reproduce and assess the problem:

- Affected Compass version or commit
- Operating system and installation method
- Vulnerable command, input, or integration
- Reproduction steps or a minimal proof of concept
- Expected and observed behavior
- Potential impact and affected data
- Suggested mitigation, if you have one

Remove API keys, database passwords, private source code, and unrelated personal data from the report.

## Coordinate disclosure

Give maintainers time to reproduce, assess, fix, and publish an advisory before sharing details elsewhere. The project doesn't promise a fixed response or release time because severity and remediation scope vary.

Maintainers will use the private advisory to coordinate questions, credit, affected versions, mitigations, and publication. A report may be closed when it doesn't cross a security boundary or can't be reproduced from the supplied information.

## Understand the security boundary

Compass parses untrusted project content and graph files. Its default structural build and query workflow stays local, but these opt-in features cross process or network boundaries:

- Semantic extraction sends selected content to the configured model provider
- Streamable HTTP exposes the Model Context Protocol server on the configured interface
- Agent Graph mutation is a separate opt-in capability. HTTP deployments must
  use distinct non-empty read and write API keys, canonical project allowlists,
  and server-owned principals/permissions. The write tool is absent when
  disabled; mask capability must be enabled separately. Never place prompts,
  responses, chain-of-thought, credentials, or source excerpts in audit
  metadata.
- Neo4j and FalkorDB pushes connect to external databases
- URL acquisition and Google Workspace extraction access configured external services
- PostgreSQL extraction connects to the supplied database server
- Assistant setup can register local command hooks. Review generated hook files
  before trusting them in the host, and reinstall after moving the Compass
  executable so the managed command path remains accurate.
- `compass upgrade` downloads the bounded `compass.release/1` manifest and the
  selected archive from official GitHub release URLs. It validates the schema,
  stable tag/version binding, bounded unique targets, selected archive name and
  size, SHA-256 digest, archive path, and staged executable version before
  replacing the running binary.
- `compass review` parses exact untrusted Git objects without checking out or
  executing their code. Its reusable Action must run in a dedicated job that
  has not executed contributor-controlled scripts; the write token belongs
  only in the pinned comment-delivery step. Fork reviews publish read-only
  evidence and suppress comments. Never use `pull_request_target` to execute a
  contributor head.

Include the selected options and endpoint type in reports about these features. Never include live credentials.

## Store boundary

`graph.json`, `store.ref`, backup manifests, SQLite files, and future adapter
objects are untrusted input. Compass validates schema majors, bounded sizes,
content digests, active selectors, canonical JSON export, and reference
bindings before exposing a store snapshot. `compass store restore` is
fail-closed, restores only into a new destination, and removes an incomplete
destination on validation failure. Stop writers before copying a redb file;
the SQLite backup command checkpoints WAL for this purpose.

The namespace is an isolation and lifecycle key, not an authorization
mechanism. A future hosted adapter must add authentication, authorization,
TLS, audit logging, quotas, and tenant-scoped GC outside the common contract.
The local release has no cloud endpoint, credential, or TLS setting and does
not link cloud SDKs into the CLI. Never attach a store database or raw backup
to a public issue: it can disclose repository names, paths, source anchors,
and graph structure. Share a sanitized `compass store status --format json`
response instead.
