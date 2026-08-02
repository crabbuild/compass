# Get help with Compass

Compass uses public, searchable support channels so answers can help other users. Choose a channel based on the kind of help you need.

## Ask a question in GitHub Discussions

Use [GitHub Discussions](https://github.com/crabbuild/compass/discussions) for:

- Installation and configuration questions
- Help understanding commands or graph output
- Integration and workflow advice
- Open-ended ideas and design exploration
- Examples you want to share with the community

Include your Compass version, operating system, installation method, command, and sanitized output. Remove credentials and private project content.

## Open a GitHub Issue

Use the [issue chooser](https://github.com/crabbuild/compass/issues/new/choose) for:

- Reproducible defects
- Actionable feature requests with a defined problem and outcome
- Documentation errors with a specific location and correction

Search existing issues before filing a report. A focused reproduction helps maintainers distinguish a Compass defect from project-specific behavior.

## Report security vulnerabilities privately

Follow [SECURITY.md](SECURITY.md) and use [GitHub private vulnerability reporting](https://github.com/crabbuild/compass/security/advisories/new). Don't disclose exploit details in Discussions or Issues.

## Support boundaries

The project doesn't offer guaranteed response times, private implementation consulting, or support for modified binaries and unsupported releases. Community members may still help when a question includes enough public information to reproduce the behavior.

## Store troubleshooting

For a local output, collect:

```bash
compass store status compass-out --format json
compass store validate compass-out --format json
```

If the graph is valid and only the sidecar fails, use the JSON engine while
rebuilding:

```bash
compass search "symbol" --graph compass-out/graph.json --engine json --format json
scripts/rebuild_compass_store.sh . --out compass-out --compass compass
```

Back up before any repair and restore into a new directory. Do not edit SQLite
tables, copy a redb file while it is open for writing, or report a failed
validation as an empty graph. Include the Compass version, platform, schema
identifiers, sanitized status/validation output, and exact command. Remove
databases, backup bundles, private source, credentials, `.env` files, and
machine-specific paths from public reports. PostgreSQL, DynamoDB, and cloud
store endpoints are not supported in this local release.
