# Add dual licensing and community channels to Compass

This design makes Compass legally consistent and gives contributors clear places to ask questions, report bugs, propose changes, report vulnerabilities, and submit pull requests.

## Goal and audience

The goal is to make the repository ready for public participation without inventing governance roles or response promises. The audience includes end users, prospective contributors, maintainers, and security researchers.

## Licensing design

Compass will offer its original work under either the MIT License or Apache License 2.0. Each recipient may choose either license.

The repository will make that choice explicit through these changes:

- Change the workspace SPDX expression to `MIT OR Apache-2.0`
- Add the complete MIT text as `LICENSE-MIT`
- Add the complete Apache License 2.0 text as `LICENSE-APACHE`
- Replace `LICENSE` with a short dual-license notice that links to both complete texts
- Make first-party workspace crates inherit the workspace license
- Preserve vendored and third-party components under their existing licenses
- Add a README section that links to both license texts and `THIRD_PARTY_NOTICES.md`

The existing MIT copyright notice remains unchanged. Contribution guidance will state that intentional contributions use the same dual license unless a contributor says otherwise.

## Community routing design

Each interaction type will have one primary destination:

| Interaction | Destination |
| --- | --- |
| Usage question, design discussion, or open-ended idea | GitHub Discussions |
| Reproducible defect | GitHub bug report form |
| Actionable feature proposal | GitHub feature request form |
| Security vulnerability | GitHub private vulnerability reporting |
| Code or documentation change | GitHub pull request |
| Conduct incident | GitHub content reporting or private contact with repository administrators through GitHub |

Blank GitHub issues will be disabled. The issue chooser will link to Discussions and private vulnerability reporting so reports reach the correct channel.

## Community documents

The repository will add focused documents with cross-links:

- `CONTRIBUTING.md`: contribution flow, development setup, verification commands, pull request expectations, and licensing of contributions
- `CODE_OF_CONDUCT.md`: behavior standards, unacceptable conduct, scope, reporting paths, and enforcement expectations
- `SECURITY.md`: supported code, private reporting steps, useful report contents, disclosure coordination, and relevant security boundaries
- `SUPPORT.md`: boundaries between Discussions, Issues, security reports, and unsupported private support
- `.github/ISSUE_TEMPLATE/bug_report.yml`: structured reproduction, environment, expected behavior, and logs
- `.github/ISSUE_TEMPLATE/feature_request.yml`: problem, proposed outcome, alternatives, and scope
- `.github/ISSUE_TEMPLATE/config.yml`: routes questions and vulnerabilities away from public issues
- `.github/pull_request_template.md`: summary, motivation, verification, compatibility, documentation, and checklist

The README will add a community section near the contributor material. It will link to all four documents, Discussions, Issues, and the pull request page.

## Security policy boundaries

Security reports will use GitHub private vulnerability reporting only. The policy will not publish an email address or promise a fixed response time.

The supported surface will include the latest release and the default branch. The policy will ask reporters to include impact, reproduction steps, affected versions, and suggested mitigations when available.

The policy will distinguish local defaults from opt-in network features. It will name semantic providers, HTTP Model Context Protocol serving, database pushes, URL acquisition, and external workspace integrations as relevant network boundaries.

## Conduct and moderation

The code of conduct will set concise, project-specific expectations. It will prohibit harassment, discrimination, threats, sexualized conduct, deliberate disruption, and publication of private information.

Because the project has no dedicated conduct email, reports will use GitHub's reporting tools or private contact with repository administrators through GitHub. Maintainers may edit or remove content, close interactions, restrict participation, or escalate violations to GitHub.

## Validation

Implementation is complete when these checks pass:

1. `cargo metadata --no-deps --format-version 1` reports `MIT OR Apache-2.0` for first-party workspace packages.
2. Both full license texts exist and the root notice links to them.
3. Every local Markdown link resolves.
4. GitHub issue forms parse as YAML and contain the required issue-form keys.
5. The README routes Discussions, Issues, security reports, and pull requests to distinct destinations.
6. `git diff --check` reports no whitespace errors.

Rust tests are not required because the implementation changes manifests, Markdown, and GitHub configuration only. The manifest metadata check covers the package-level behavior that changes.

## Out of scope

This work will not add a contributor license agreement, Developer Certificate of Origin requirement, maintainer roster, governance model, funding policy, release service-level agreement, or automatic moderation workflow.
