# VS Code Compass CLI Onboarding Design

## Context

The VS Code extension discovers a Compass CLI from `compass.cliPath`, `PATH`,
and common install directories when it activates. When discovery fails, the
Workspace tree reports that the CLI needs attention and routes the user to the
existing binary selector. The startup notification similarly offers the
walkthrough or binary selection. Both paths assume that Compass has already
been installed.

New users need a direct first-run path that installs Compass visibly in the
integrated terminal, verifies the resulting executable, and continues to the
existing repository initialization wizard without reloading VS Code.

## Goals

- Give a user with no Compass CLI a focused onboarding page.
- Install Compass through a visible VS Code terminal with one explicit action.
- Support macOS, Linux, Windows x64, and Windows ARM64.
- Verify the installed executable and required extension capabilities
  automatically.
- Activate the verified CLI in the current extension session without a window
  reload.
- End on a clear ready state with one primary repository-initialization action.
- Preserve manual selection for users who already have a CLI in a nonstandard
  location.

## Non-Goals

- Bundling a Compass native binary in the VSIX.
- Running an installer in a hidden child process.
- Combining machine-wide CLI installation with repository scope configuration.
- Changing Compass graph-building behavior or the repository initialization
  wizard.
- Adding telemetry, provider setup, or automatic graph creation.
- Refreshing the Graphify codegraph for this work, following the project
  owner's explicit direction.

## Entry Points

When discovery reports `missing`, the Workspace tree shows **Set up Compass**
with a **Not installed** description. Selecting it opens the onboarding panel.

The startup notification says that Compass is required and offers:

1. **Install Compass**, which opens the onboarding panel.
2. **Select existing CLI**, which preserves the current binary-selection flow.

The command palette gains **Compass: Install CLI** so the page can be reopened
directly. Incompatible installations continue to prefer binary selection or an
upgrade path; the new-user installer is primarily for a genuinely missing CLI.

## Onboarding Page

The page is a dedicated webview titled **Get started with Compass**. It uses the
same VS Code theme variables, typography, focus treatment, and responsive
layout conventions as the existing initialization wizard.

### Ready to install

The first state explains that Compass runs locally and that the VSIX does not
contain a native executable. It shows:

- the detected workspace-host platform;
- the exact command that will run;
- a primary **Install Compass** button; and
- a secondary **Select an existing CLI** action.

The webview never supplies command text to the extension host. It sends only a
bounded `install` intent, and the host chooses a fixed command for the detected
platform.

### Installing

The primary action creates or reuses a terminal named **Compass Setup**, shows
it, and immediately runs the official platform command. On Windows the
extension creates an explicit PowerShell terminal instead of assuming that the
user's default profile is PowerShell. The webview changes to **Installing
Compass…**, disables duplicate actions, and tells the user that full output is
available in the terminal.

macOS and Linux run the released `install.sh`. Windows runs a new released
`install.ps1`. The integrated terminal is visible before the command is sent.

### Verifying

When the terminal command finishes, the extension:

1. discovers the new executable in configured, `PATH`, and common locations;
2. reads `compass --version`;
3. requests `compass capabilities --format json`;
4. validates the result against the extension's existing capability schema and
   requirements; and
5. activates the verified executable in the current extension runtime.

The page reports **Verifying installation…** during this work. It never shows a
ready state based only on the installer's exit code or the presence of a file.

### Ready

Successful verification shows **Compass is ready**, the version, and the
selected executable path. The only primary action is **Initialize repository**.
It opens the existing initialization wizard and uses the normal repository
picker in a multi-root workspace.

If no repository folder is open, the primary action becomes **Open repository
folder**. Opening a folder returns the user to the normal Compass workspace
flow.

## Live CLI Runtime

CLI discovery is currently captured once during extension activation. A small
runtime controller becomes the single owner of:

- the current discovery result;
- the active executable;
- candidate inspection and capability negotiation; and
- change notifications for CLI-dependent views.

Repository sessions continue to share one process-manager boundary. The
controller verifies a candidate with a temporary process manager before
publishing it. After verification succeeds, it updates the shared executable,
stores the selected absolute path in the machine-scoped `compass.cliPath`
setting, refreshes every repository's capability report, and notifies the
Workspace tree and status bar.

The existing binary selector uses the same activation method. Selecting a valid
CLI therefore also stops requiring a VS Code reload. Runtime activation is
rejected while a Compass write or watch process is active so an executable
cannot change under a running operation.

The Workspace tree reads current discovery state from the controller rather
than retaining the activation-time snapshot.

## Terminal Execution

The extension uses the workspace extension host's `process.platform`, so Remote
SSH, WSL, and Dev Containers install Compass on the host where extension
processes run.

VS Code terminal shell integration supplies the install command's exit event
when available. The extension waits a bounded interval for shell integration,
runs the command through `executeCommand`, and filters the completion event by
the exact terminal and execution instance before acting on it.

Some shells do not expose shell integration. In that case, the extension sends
the command as terminal text and starts bounded discovery polling. Polling
stops when a compatible executable is found, the setup terminal closes, the
panel closes, or the timeout expires. A timeout is a retryable verification
failure; it is not reported as a successful install.

Windows prefers `pwsh.exe` when it is available and otherwise uses the
in-box `powershell.exe`. If neither executable can be launched, the page shows
manual release guidance and binary selection rather than falling back to an
unknown command shell.

The command is chosen from a pure, platform-specific helper so command
construction and unsupported-platform behavior can be unit tested without a
terminal.

## Official PowerShell Installer

`scripts/install.ps1` becomes a first-party release asset beside
`scripts/install.sh`. It:

1. requires a supported Windows host;
2. detects x64 or ARM64;
3. selects `x86_64-pc-windows-msvc` or `aarch64-pc-windows-msvc`;
4. downloads the matching release archive and `.sha256` file over HTTPS;
5. verifies the archive with `Get-FileHash -Algorithm SHA256`;
6. extracts `compass.exe` into a temporary directory;
7. copies it atomically into a user-writable install directory; and
8. prints the installed path and any PATH guidance.

The default directory is a location already searched by VS Code discovery and
does not require elevation. `COMPASS_RELEASE_BASE_URL` and
`COMPASS_INSTALL_DIR` overrides mirror the shell installer so tests and
controlled deployments can use local fixtures.

The release workflow publishes both installer scripts. The release-script test
suite exercises PowerShell architecture selection, successful checksum
verification, checksum rejection, extraction, and the final installed
executable on Windows CI.

## Failure and Recovery

- A nonzero shell-integration exit shows **Installation failed**, retains and
  focuses the setup terminal, and offers **Try again** and **Select existing
  CLI**.
- A successful command with no discovered executable shows every searched
  location and offers **Verify again**.
- A discovered executable that fails capability negotiation shows its path and
  version with a concise compatibility error.
- Closing the setup terminal before verification returns the page to a
  retryable stopped state.
- Closing the onboarding panel cancels timers and listeners but does not close
  the user's terminal.
- If a compatible Compass installation appears while the page is open, the
  page skips installation and advances through verification to ready.
- Unsupported platforms show manual release guidance and binary selection
  without offering a command that cannot succeed.

Installer and verification failures never change `compass.cliPath` or the
active runtime.

## Security and Privacy

- The exact fixed command is displayed before execution.
- The installer always runs in a visible user terminal.
- No command, path, URL, or shell fragment is accepted from webview input.
- Installers use HTTPS and require a matching SHA-256 release checksum before
  installing the binary.
- Installation uses user-writable paths and does not request elevation.
- The extension still runs only in trusted workspaces.
- The VSIX does not gain a native binary or remote webview asset.
- No telemetry is added.

## Component Boundaries

- `@compass/viewer` owns the reusable React onboarding presentation and state
  rendering.
- The VS Code webview entry adapts extension messages into viewer props and
  host intents.
- The onboarding panel owns terminal lifecycle observation and panel-specific
  state.
- A CLI runtime controller owns discovery, verification, activation, and
  change notification.
- A pure install-command module maps supported platforms to display and
  execution commands.
- A bounded message parser rejects malformed webview intents.
- `scripts/install.ps1` owns Windows release download, verification, and
  installation.

These units communicate through typed messages and narrow methods so terminal,
runtime, and presentation behavior can be tested independently.

## Verification

- Unit-test install command selection for macOS, Linux, Windows, and unsupported
  hosts.
- Unit-test CLI runtime activation, capability rejection, persistence, and
  active-operation refusal.
- Unit-test Workspace tree behavior for missing, installing, verified, and
  incompatible CLI states.
- Unit-test strict onboarding message parsing.
- Component-test every onboarding state, error recovery, disabled controls,
  focus order, keyboard activation, and accessible status announcements.
- Test shell-integration completion filtering and the bounded polling fallback.
- Test `install.ps1` against fixture release assets for both Windows
  architectures and for checksum rejection.
- Validate that the release workflow publishes `install.sh` and `install.ps1`.
- Run the VS Code typecheck, unit tests, integration tests, production build,
  VSIX packaging, and VSIX smoke test.
- Run the existing release-script tests and `git diff --check`.
