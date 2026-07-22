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
- Neo4j and FalkorDB pushes connect to external databases
- URL acquisition and Google Workspace extraction access configured external services
- PostgreSQL extraction connects to the supplied database server

Include the selected options and endpoint type in reports about these features. Never include live credentials.
