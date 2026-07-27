# Compass Upgrade Command Design

**Date:** 2026-07-27

**Status:** Approved for implementation planning

## Goal

Add a public `compass upgrade` command that upgrades the running Compass
executable to the newest stable GitHub release. When the installed version is
current, the command exits successfully and clearly tells the user that it is
already the latest version.

## User Experience

The command has no required arguments:

```text
compass upgrade
```

When an update is available, Compass reports the version transition, downloads
and verifies the matching release, replaces the running executable, and prints:

```text
Upgraded Compass from 0.1.8 to 0.1.9.
```

When the running version equals the latest stable release, Compass makes no
filesystem changes, exits with code 0, and prints:

```text
Compass 0.1.9 is already the latest version.
```

If the running version is newer than the latest stable release, Compass does
not downgrade it. It exits successfully and reports that the running version
is already newer than the latest release.

`compass upgrade --help` is a dedicated public help page. Unexpected arguments
are rejected before any network or filesystem activity.

## Supported Installations and Platforms

The upgrade operates on the currently running executable, regardless of
whether it was originally installed by the shell installer, extracted
manually, or installed with Cargo. The executable must be writable by the
current user; otherwise the command fails with a permissions-oriented error and
installation guidance.

The command supports every target currently published by the Compass release
workflow:

- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

Other targets fail before downloading an archive and identify the unsupported
target.

## Release Discovery and Version Policy

Compass queries the GitHub latest-release endpoint for
`crabbuild/compass`. The response must identify a non-draft, non-prerelease
release whose tag has the exact form `compass-v<semver>`.

Only a stable release newer than `CARGO_PKG_VERSION` is eligible. Prereleases,
malformed tags, and downgrades are never installed. Version comparison uses
semantic-version ordering rather than string ordering.

The release must contain both assets for the current target:

```text
compass-<target>.tar.gz
compass-<target>.tar.gz.sha256
```

Missing or duplicate matching assets are treated as release errors.

## Components

The CLI dispatch remains in `compass-cli`, following the existing
hand-written command architecture.

### Command adapter

`upgrade_commands` owns argument validation, human-readable outcomes, and the
top-level upgrade sequence. `lib.rs` dispatches `upgrade` to this module, while
`help.rs` registers the command in the public help catalog.

### Release client

The release client fetches and parses only the fields needed from GitHub:
release tag, draft/prerelease flags, asset names, and asset download URLs. It
uses bounded response handling, explicit HTTP status checks, and a Compass
user-agent.

### Artifact verifier

The verifier downloads the archive and checksum into a temporary directory,
requires a strict SHA-256 checksum entry for the selected archive, computes the
archive digest, and rejects any mismatch before extraction.

The archive must contain the expected packaged path:

```text
compass-<target>/compass
compass-<target>/compass.exe
```

depending on the platform. Path traversal, unexpected executable paths, and
missing executables are rejected.

### Binary validator and replacer

Before replacement, Compass runs the staged executable with `--version` and
requires it to report exactly the selected release version. The staged binary
is then passed to a cross-platform self-replacement boundary backed by
`self-replace`.

Replacement is the only operation that touches the installed executable. On
Unix the executable is atomically swapped. On Windows the replacement boundary
handles the running executable lock and deferred cleanup. Failures before the
swap leave the installed executable unchanged.

The release client, target resolver, artifact staging, process validation, and
replacement operation use narrow interfaces so they can be tested
independently without replacing the test runner.

## Data Flow

1. Parse `compass upgrade` and reject unsupported arguments.
2. Resolve the compile target to one of the six published targets.
3. Fetch and validate the latest stable release metadata.
4. Compare the latest version with the running package version.
5. Return the no-op message immediately when no upgrade is needed.
6. Resolve exactly one archive and checksum asset for the target.
7. Download both assets into a temporary directory.
8. Verify the archive SHA-256 checksum.
9. Safely extract the expected Compass executable.
10. Run the staged executable with `--version` and validate the result.
11. Replace the running executable.
12. Report the completed version transition.

## Error Handling

Errors are concise and actionable:

- Network and HTTP failures say that release metadata or an asset could not be
  downloaded.
- Invalid release metadata identifies the malformed tag or missing asset.
- Checksum mismatches explicitly say verification failed and do not extract or
  replace anything.
- Invalid archives and staged-version mismatches fail before replacement.
- Unsupported targets name the current target and list the supported families.
- Permission or replacement failures identify the executable path and suggest
  rerunning through the installation mechanism with sufficient permissions.

Temporary downloads are removed on both success and failure. Error messages do
not include response bodies, tokens, or unrelated environment values.

## Testing

Implementation follows test-driven development.

Unit tests cover:

- semantic-version comparison for older, equal, and newer running versions;
- exact `compass-v<semver>` tag parsing and prerelease rejection;
- target selection for all six published targets and unsupported targets;
- exact asset selection, including missing and duplicate assets;
- strict checksum parsing and mismatch rejection;
- archive path validation and traversal rejection;
- staged executable version validation;
- replacement not being invoked on any validation failure;
- replacement being invoked exactly once after all validations succeed;
- successful no-op output for an already-current version;
- successful no-downgrade output for a newer development build.

CLI tests cover:

- root help lists `upgrade` with a description;
- `compass upgrade --help` has usage, examples, and standard help options;
- unexpected arguments fail without invoking upgrade dependencies.

An end-to-end updater test uses a local HTTP fixture with synthetic release
metadata, archives, and checksums. Replacement is redirected to a temporary
executable path through the test boundary, so tests never access GitHub or
replace the test binary.

The release workflow continues to build and package all six targets. CI builds
the new dependencies and runs the Compass CLI test suite on macOS, Linux, and
Windows, exercising the platform-specific replacement implementation at
compile time.

## Documentation

The README command reference will list `compass upgrade`, describe the
latest-version no-op, and state that the command installs official stable
GitHub release binaries in place. The release asset naming and checksum
contract remain unchanged.

## Non-Goals

This feature does not:

- check for updates automatically during unrelated commands;
- install prerelease, nightly, or arbitrary versions;
- manage the Compass VS Code extension;
- modify shell `PATH`;
- preserve multiple installed versions or provide rollback;
- delegate upgrading to Homebrew, Cargo, or another package manager;
- change the existing release archive layout.
