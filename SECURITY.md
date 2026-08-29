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
- Stateless MCP 2026-07-28 Streamable HTTP exposes the server on the configured
  interface; every request remains independently subject to host validation,
  authentication when configured, and body limits
- Neo4j and FalkorDB pushes connect to external databases
- URL acquisition and Google Workspace extraction access configured external services
- PostgreSQL extraction connects to the supplied database server
- Assistant setup can register local command hooks. Review generated hook files
  before trusting them in the host, and reinstall after moving the Compass
  executable so the managed command path remains accurate.
- `compass agent export` publishes only the embedded seven-skill collection and
  credential-free native MCP configuration. `compass agent validate` treats
  bundles and managed installations as untrusted: it bounds files and bytes,
  rejects symlinks, escaping paths and common machine-specific roots, verifies
  manifests and
  checksums, accepts only the documented Compass stdio command or loopback HTTP
  endpoint, and reports likely literal credentials without echoing their values.
- `compass upgrade` downloads the bounded `compass.release/1` manifest and the
  selected archive from official GitHub release URLs. It validates the schema,
  stable tag/version binding, bounded unique targets, selected archive name and
  size, SHA-256 digest, archive path, and staged executable version before
  replacing the running binary.

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
