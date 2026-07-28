# VS Code Compass CLI Onboarding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give users without Compass a one-click VS Code onboarding page that installs the official CLI in a visible integrated terminal, verifies and activates it without reloading, then opens repository initialization.

**Architecture:** Add a first-party PowerShell installer, a mutable CLI runtime that safely activates verified executables, a pure React onboarding view, and a VS Code panel that owns terminal execution and verification. Keep installer command selection, runtime activation, presentation, and terminal orchestration behind separate typed interfaces so each boundary is independently testable.

**Tech Stack:** TypeScript 5.9, VS Code Extension API 1.95, React 19, Zod 4, Vitest 3, PowerShell 5.1+/PowerShell 7, POSIX shell, GitHub Actions

## Global Constraints

- Follow implementation-first ordering: add production behavior before its focused regression tests. Do not use red/green TDD sequencing.
- The VSIX must not bundle a native Compass binary.
- Installers must use HTTPS, verify SHA-256, use user-writable destinations, and request no elevation.
- The extension must show the exact fixed command and run it in a visible VS Code terminal.
- Webview input must never provide command text, paths, URLs, or shell fragments.
- Support macOS and Linux on x64/ARM64 plus Windows x64/ARM64.
- Activate a verified CLI without reloading VS Code.
- Preserve the existing repository initialization wizard and open it only after the user selects **Initialize repository**.
- Preserve the user's existing uncommitted `editors/vscode/package.json` version change from `0.1.6` to `0.1.7`; do not include that version hunk in feature commits.
- Do not run `graphify update .`, following the project owner's explicit direction.
- Do not modify or stage the unrelated untracked plans already present in `docs/superpowers/plans/`.

## File Structure

### New files

- `scripts/install.ps1` — official Windows release downloader, checksum verifier, extractor, and user-local installer.
- `scripts/test_install_ps1.ps1` — fixture-backed Windows installer integration checks.
- `editors/vscode/src/cli/runtime.ts` — verified live CLI activation and discovery state.
- `editors/vscode/src/cli/runtime.test.ts` — runtime activation regression coverage.
- `editors/vscode/src/install/command.ts` — pure platform command and PowerShell discovery helpers.
- `editors/vscode/src/install/command.test.ts` — command selection coverage.
- `editors/vscode/src/install/messages.ts` — bounded onboarding webview schemas and host state types.
- `editors/vscode/src/install/messages.test.ts` — message rejection coverage.
- `editors/vscode/src/views/cliOnboardingPanel.ts` — webview lifecycle, visible terminal execution, completion observation, and verification.
- `editors/vscode/src/views/cliOnboardingPanel.test.ts` — terminal completion and fallback polling coverage through injected host dependencies.
- `editors/vscode/src/webviews/onboarding.tsx` — VS Code-to-React onboarding adapter.
- `packages/compass-viewer/src/onboarding/CliOnboarding.tsx` — pure onboarding presentation.
- `packages/compass-viewer/src/onboarding/CliOnboarding.test.tsx` — rendering, action, and accessibility coverage.
- `packages/compass-viewer/src/onboarding.css` — onboarding-specific responsive styles.

### Modified files

- `scripts/test_release_scripts.sh` — retain POSIX installer coverage and assert both installer assets exist.
- `.github/workflows/compass-ci.yml` — run PowerShell installer checks on Windows.
- `.github/workflows/compass-release.yml` — publish `scripts/install.ps1`.
- `README.md` — document the Windows one-line installer.
- `docs/getting-started.md` — document the Windows one-line installer and override variables.
- `editors/vscode/src/cli/processManager.ts` — allow a verified executable to replace the inactive runtime executable.
- `editors/vscode/src/cli/processManager.test.ts` — cover live executable replacement.
- `editors/vscode/src/views/workspaceTree.ts` — read current discovery through a getter.
- `editors/vscode/src/views/treeModel.ts` — route missing-CLI users to onboarding.
- `editors/vscode/src/views/treeModel.test.ts` — cover the new tree entry point.
- `editors/vscode/src/extension.ts` — create the runtime, register onboarding, and activate selected CLIs without reload.
- `editors/vscode/src/test/suite/extension.integration.ts` — assert the install command is registered.
- `editors/vscode/esbuild.mjs` — bundle the onboarding webview.
- `editors/vscode/package.json` — contribute **Compass: Install CLI** and update walkthrough copy.
- `editors/vscode/scripts/smoke-vsix.mjs` — require the onboarding bundle and continue rejecting native binaries.
- `editors/vscode/README.md` — explain first-run installation and recovery.
- `editors/vscode/CHANGELOG.md` — add an `0.1.7` onboarding entry without changing the user-owned version line.
- `packages/compass-viewer/src/index.ts` — export the onboarding component and types.
- `packages/compass-viewer/src/theme.css` — import onboarding styles.

---

### Task 1: Official Windows Installer and Release Asset

**Files:**
- Create: `scripts/install.ps1`
- Create: `scripts/test_install_ps1.ps1`
- Modify: `scripts/test_release_scripts.sh`
- Modify: `.github/workflows/compass-ci.yml`
- Modify: `.github/workflows/compass-release.yml`
- Modify: `README.md`
- Modify: `docs/getting-started.md`

**Interfaces:**
- Consumes: release archives named `compass-<target>.tar.gz` and matching `.sha256` files.
- Produces: `scripts/install.ps1`, honoring `COMPASS_RELEASE_BASE_URL` and `COMPASS_INSTALL_DIR`, and installing `compass.exe` to the reported absolute path.

- [ ] **Step 1: Implement the PowerShell installer**

Create `scripts/install.ps1` with strict error behavior, deterministic
architecture mapping, checksum validation, and cleanup:

```powershell
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Repository = if ($env:COMPASS_REPOSITORY) {
    $env:COMPASS_REPOSITORY
} else {
    "crabbuild/compass"
}
$ReleaseBaseUrl = if ($env:COMPASS_RELEASE_BASE_URL) {
    $env:COMPASS_RELEASE_BASE_URL.TrimEnd("/")
} else {
    "https://github.com/$Repository/releases/latest/download"
}
$InstallDir = if ($env:COMPASS_INSTALL_DIR) {
    $env:COMPASS_INSTALL_DIR
} else {
    Join-Path $HOME ".local\bin"
}

$Architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
$Target = switch ($Architecture) {
    ([System.Runtime.InteropServices.Architecture]::X64) {
        "x86_64-pc-windows-msvc"
    }
    ([System.Runtime.InteropServices.Architecture]::Arm64) {
        "aarch64-pc-windows-msvc"
    }
    default {
        throw "unsupported Windows architecture: $Architecture"
    }
}

$Name = "compass-$Target"
$Archive = "$Name.tar.gz"
$Checksum = "$Archive.sha256"
$Temporary = Join-Path ([System.IO.Path]::GetTempPath()) "compass-install-$([guid]::NewGuid())"

try {
    New-Item -ItemType Directory -Path $Temporary | Out-Null
    Invoke-WebRequest -UseBasicParsing -Uri "$ReleaseBaseUrl/$Archive" `
        -OutFile (Join-Path $Temporary $Archive)
    Invoke-WebRequest -UseBasicParsing -Uri "$ReleaseBaseUrl/$Checksum" `
        -OutFile (Join-Path $Temporary $Checksum)

    $Expected = ((Get-Content (Join-Path $Temporary $Checksum) -Raw).Trim() `
        -split "\s+")[0].ToLowerInvariant()
    if ($Expected -notmatch "^[0-9a-f]{64}$") {
        throw "invalid SHA-256 file for $Archive"
    }
    $Actual = (Get-FileHash (Join-Path $Temporary $Archive) -Algorithm SHA256) `
        .Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) {
        throw "checksum verification failed for $Archive"
    }

    tar -xzf (Join-Path $Temporary $Archive) -C $Temporary
    if ($LASTEXITCODE -ne 0) {
        throw "failed to extract $Archive"
    }
    $Source = Join-Path $Temporary "$Name\compass.exe"
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        throw "release archive does not contain $Name\compass.exe"
    }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $Destination = Join-Path $InstallDir "compass.exe"
    $Staged = Join-Path $InstallDir "compass.exe.new"
    Copy-Item -LiteralPath $Source -Destination $Staged -Force
    Move-Item -LiteralPath $Staged -Destination $Destination -Force
    Write-Output "Installed Compass to $Destination"
} finally {
    Remove-Item -LiteralPath $Temporary -Recurse -Force -ErrorAction SilentlyContinue
}
```

- [ ] **Step 2: Publish the PowerShell asset**

Add `scripts/install.ps1` to the `files:` list in
`.github/workflows/compass-release.yml`:

```yaml
files: |
  dist/*
  scripts/install.sh
  scripts/install.ps1
```

Add a POSIX release assertion after the existing installer tests:

```sh
test -f "$repo_root/scripts/install.sh"
test -f "$repo_root/scripts/install.ps1"
```

- [ ] **Step 3: Document the one-line Windows installation command**

Replace the manual Windows-only archive paragraph in `README.md` and
`docs/getting-started.md` with:

```powershell
irm https://github.com/crabbuild/compass/releases/latest/download/install.ps1 | iex
```

Keep the manual archive and checksum instructions immediately below it as the
offline/fallback path. Document:

```powershell
$env:COMPASS_INSTALL_DIR = "$PWD\bin"
.\install.ps1
```

- [ ] **Step 4: Add fixture-backed PowerShell installer checks**

Create `scripts/test_install_ps1.ps1`. The script must:

1. accept `-CompassBinary` as an optional path to a built `compass.exe`;
2. create x64 and ARM64 fixture archives with the exact release layout;
3. create valid `.sha256` files with `Get-FileHash`;
4. serve the fixture directory from `python -m http.server` on a dynamically
   reserved loopback port;
5. invoke `install.ps1` with fixture `COMPASS_RELEASE_BASE_URL` and a temporary
   `COMPASS_INSTALL_DIR`;
6. assert `compass.exe` exists and byte-matches the fixture;
7. replace one checksum with 64 zeroes and assert installation fails without
   publishing `compass.exe`; and
8. stop the server and delete all temporary directories in `finally`.

Use this architecture override inside the installer solely for deterministic
tests:

```powershell
$ArchitectureName = if ($env:COMPASS_INSTALL_ARCH) {
    $env:COMPASS_INSTALL_ARCH
} else {
    [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
}
$Target = switch ($ArchitectureName.ToLowerInvariant()) {
    "x64" { "x86_64-pc-windows-msvc" }
    "arm64" { "aarch64-pc-windows-msvc" }
    default { throw "unsupported Windows architecture: $ArchitectureName" }
}
```

- [ ] **Step 5: Run the Windows installer check in CI**

Add this step to the `native-platforms` job after the native tests:

```yaml
- name: Test PowerShell installer
  if: runner.os == 'Windows'
  shell: pwsh
  run: |
    cargo build --release --locked -p compass-cli --bin compass --target ${{ matrix.target }}
    ./scripts/test_install_ps1.ps1 `
      -CompassBinary "target/${{ matrix.target }}/release/compass.exe"
```

- [ ] **Step 6: Run focused verification**

Run on macOS/Linux:

```bash
sh -n scripts/install.sh
sh -n scripts/test_release_scripts.sh
sh scripts/test_release_scripts.sh
git diff --check -- scripts .github/workflows README.md docs/getting-started.md
```

Run on Windows or a host with `pwsh`:

```powershell
./scripts/test_install_ps1.ps1
```

Expected: the shell release tests pass; PowerShell tests install both fixture
targets and reject the corrupt checksum.

- [ ] **Step 7: Commit the installer deliverable**

```bash
git add scripts/install.ps1 scripts/test_install_ps1.ps1 \
  scripts/test_release_scripts.sh .github/workflows/compass-ci.yml \
  .github/workflows/compass-release.yml README.md docs/getting-started.md
git commit -m "feat: add official Windows Compass installer"
```

### Task 2: Live Verified CLI Runtime

**Files:**
- Create: `editors/vscode/src/cli/runtime.ts`
- Create: `editors/vscode/src/cli/runtime.test.ts`
- Modify: `editors/vscode/src/cli/processManager.ts`
- Modify: `editors/vscode/src/cli/processManager.test.ts`
- Modify: `editors/vscode/src/views/workspaceTree.ts`

**Interfaces:**
- Consumes: `CompassDiscovery`, `CompassInstallation`,
  `CompassProcessManager`, `CapabilityReportSchema`, `COMPASS_REQUIREMENTS`,
  repository sessions, and a persistence callback.
- Produces:
  - `CompassRuntime.discovery: CompassDiscovery`
  - `CompassRuntime.onDidChange(listener: () => void): { dispose(): void }`
  - `CompassRuntime.activate(installation: CompassInstallation): Promise<ActivatedCompass>`
  - `CompassProcessManager.useExecutable(executable: string): void`

- [ ] **Step 1: Make the shared process manager switchable only by explicit activation**

Change the executable field in `processManager.ts` from constructor-private
readonly state to guarded mutable state:

```ts
export class CompassProcessManager {
  private executable: string;

  constructor(executable: string, private readonly spawn: Spawn = nodeSpawn) {
    this.executable = executable;
  }

  get executablePath(): string {
    return this.executable;
  }

  useExecutable(executable: string): void {
    const next = executable.trim();
    if (!next) throw new Error("Compass executable path cannot be empty");
    this.executable = next;
  }
}
```

Do not add automatic discovery or validation to this low-level class. The
runtime controller is the only caller that publishes a verified executable.

- [ ] **Step 2: Implement the runtime controller**

Create `runtime.ts` with these public contracts:

```ts
export type ActivatedCompass = {
  installation: CompassInstallation;
  capabilities: CapabilityReport;
};

export type CompassRuntimeDependencies = {
  processes: CompassProcessManager;
  sessions(): readonly RepositorySession[];
  persistCliPath(executable: string): Promise<void>;
  createCandidateProcesses?(executable: string): CompassProcessManager;
};

export class CompassRuntime {
  private current: CompassDiscovery;
  private readonly listeners = new Set<() => void>();

  constructor(
    discovery: CompassDiscovery,
    private readonly dependencies: CompassRuntimeDependencies
  ) {
    this.current = discovery;
  }

  get discovery(): CompassDiscovery {
    return this.current;
  }

  onDidChange(listener: () => void): { dispose(): void } {
    this.listeners.add(listener);
    return { dispose: () => this.listeners.delete(listener) };
  }

  async activate(installation: CompassInstallation): Promise<ActivatedCompass> {
    const sessions = this.dependencies.sessions();
    if (sessions.some((session) => session.activeWriter || session.watch)) {
      throw new Error("Stop active Compass builds and watchers before changing the CLI.");
    }
    const candidate = this.dependencies.createCandidateProcesses?.(
      installation.executable
    ) ?? new CompassProcessManager(installation.executable);
    const cwd = sessions[0]?.root ?? process.cwd();
    const capabilities = await candidate.runJson(
      cwd,
      ["capabilities", "--format", "json"],
      CapabilityReportSchema
    );
    const issue = Object.values(COMPASS_REQUIREMENTS)
      .map((requirement) => compatibilityIssue(capabilities, undefined, requirement))
      .find((value) => value !== undefined);
    if (issue) throw new Error(issue);

    await this.dependencies.persistCliPath(installation.executable);
    this.dependencies.processes.useExecutable(installation.executable);
    for (const session of sessions) {
      session.capabilities = capabilities;
      session.capabilityError = undefined;
    }
    this.current = activeDiscovery(installation, this.current);
    for (const listener of this.listeners) listener();
    return { installation, capabilities };
  }
}
```

Implement `activeDiscovery` so the activated installation is first, duplicate
paths are removed, and the original `searched` list remains available for
diagnostics.

- [ ] **Step 3: Make the Workspace tree read current discovery**

Change `WorkspaceTree` to consume a getter:

```ts
constructor(
  private readonly registry: SessionRegistry,
  private readonly discovery: () => CompassDiscovery
) {}

getChildren(node?: TreeNode): TreeNode[] {
  if (node) return node.children ?? [];
  return buildWorkspaceTree(this.discovery(), this.registry.all());
}
```

The extension will pass `() => runtime.discovery` in Task 5.

- [ ] **Step 4: Add runtime and process-manager regression tests**

Add implementation-following tests covering:

```ts
it("switches future process launches to the activated executable", async () => {
  const processes = new CompassProcessManager("compass", spawn as never);
  processes.useExecutable("/home/dev/.local/bin/compass");
  processes.startCommand("/repo", ["--version"]);
  expect(spawn).toHaveBeenCalledWith(
    "/home/dev/.local/bin/compass",
    ["--version"],
    expect.any(Object)
  );
});
```

In `runtime.test.ts`, use an injected candidate process manager and assert:

- all required capabilities are checked before `persistCliPath`;
- the shared process manager and sessions update only after verification;
- incompatible reports reject without mutation;
- active writers and watchers reject without candidate execution;
- listeners fire once after successful activation; and
- duplicate installation paths are removed from the published discovery.

- [ ] **Step 5: Run focused verification**

```bash
npm run test -w editors/vscode -- \
  src/cli/processManager.test.ts src/cli/runtime.test.ts
npm run typecheck -w editors/vscode
git diff --check -- editors/vscode/src/cli editors/vscode/src/views/workspaceTree.ts
```

Expected: focused Vitest files and the extension typecheck pass.

- [ ] **Step 6: Commit the runtime deliverable**

```bash
git add editors/vscode/src/cli/processManager.ts \
  editors/vscode/src/cli/processManager.test.ts \
  editors/vscode/src/cli/runtime.ts \
  editors/vscode/src/cli/runtime.test.ts \
  editors/vscode/src/views/workspaceTree.ts
git commit -m "feat(vscode): activate verified Compass CLIs live"
```

### Task 3: Reusable Onboarding Presentation

**Files:**
- Create: `packages/compass-viewer/src/onboarding/CliOnboarding.tsx`
- Create: `packages/compass-viewer/src/onboarding/CliOnboarding.test.tsx`
- Create: `packages/compass-viewer/src/onboarding.css`
- Modify: `packages/compass-viewer/src/index.ts`
- Modify: `packages/compass-viewer/src/theme.css`

**Interfaces:**
- Consumes: a serializable `CliOnboardingState` and an action-only
  `CliOnboardingHost`.
- Produces:
  - `CliOnboarding`
  - `CliOnboardingState`
  - `CliOnboardingHost`

- [ ] **Step 1: Define the presentation contract**

Create `CliOnboarding.tsx` with these exported types:

```ts
export type CliOnboardingState =
  | {
      kind: "ready-to-install";
      platform: string;
      command: string;
    }
  | {
      kind: "installing";
      platform: string;
      command: string;
    }
  | { kind: "verifying" }
  | {
      kind: "ready";
      version: string;
      executable: string;
      hasWorkspace: boolean;
    }
  | {
      kind: "error";
      title: string;
      message: string;
      searched?: string[];
      canVerifyAgain: boolean;
    }
  | {
      kind: "unsupported";
      platform: string;
      message: string;
    };

export type CliOnboardingHost = {
  install(): void;
  verifyAgain(): void;
  selectExisting(): void;
  initializeRepository(): void;
  openRepository(): void;
  showTerminal(): void;
};
```

- [ ] **Step 2: Implement the six visual states**

Implement one `main` landmark with:

- **Get started with Compass** and local-first copy in ready-to-install;
- a read-only `<code>` block containing the exact command;
- **Install Compass** as the sole primary install action;
- `role="status"` and `aria-live="polite"` around installing/verifying state;
- **Compass is ready** with version and executable path;
- **Initialize repository** when `hasWorkspace` is true;
- **Open repository folder** when `hasWorkspace` is false;
- focused recovery copy and bounded searched-location rendering for errors; and
- **Select an existing CLI** on ready-to-install, error, and unsupported states.

Use the existing `init-button`, `init-eyebrow`, and result-card conventions
where they match. Add onboarding-specific class names only for command display,
platform detail, and searched paths.

- [ ] **Step 3: Add responsive theme styles and exports**

Import the new stylesheet from `theme.css`:

```css
@import "./initialize.css";
@import "./onboarding.css";
```

Export the public component:

```ts
export * from "./onboarding/CliOnboarding";
```

Keep the page usable at 320 CSS pixels and use VS Code variables already
defined in `theme.css`; add no fixed light/dark palette.

- [ ] **Step 4: Add component regression tests**

Add tests after the component implementation that render each union member and
assert:

- the exact command is visible before install;
- `host.install` fires only from **Install Compass**;
- installing and verifying announce status and expose no duplicate install;
- ready shows version/path and selects the correct repository action;
- error invokes **Verify again**, **View terminal**, and binary selection;
- unsupported does not render an install action; and
- every interactive control is a native button with a visible label.

Use the existing `createRoot` plus `flushSync` pattern from
`InitializationWizard.test.tsx`.

- [ ] **Step 5: Run focused verification**

```bash
npm run test -w @compass/viewer -- \
  src/onboarding/CliOnboarding.test.tsx
npm run typecheck -w @compass/viewer
npm run build -w @compass/viewer
git diff --check -- packages/compass-viewer/src
```

Expected: component tests, typecheck, and deterministic viewer build pass.

- [ ] **Step 6: Commit the viewer deliverable**

```bash
git add packages/compass-viewer/src/onboarding \
  packages/compass-viewer/src/onboarding.css \
  packages/compass-viewer/src/index.ts \
  packages/compass-viewer/src/theme.css
git commit -m "feat(viewer): add Compass CLI onboarding states"
```

### Task 4: Visible Terminal Installation and Verification Panel

**Files:**
- Create: `editors/vscode/src/install/command.ts`
- Create: `editors/vscode/src/install/command.test.ts`
- Create: `editors/vscode/src/install/messages.ts`
- Create: `editors/vscode/src/install/messages.test.ts`
- Create: `editors/vscode/src/views/cliOnboardingPanel.ts`
- Create: `editors/vscode/src/views/cliOnboardingPanel.test.ts`
- Create: `editors/vscode/src/webviews/onboarding.tsx`
- Modify: `editors/vscode/esbuild.mjs`

**Interfaces:**
- Consumes: `CompassRuntime`, `discoverCompass`, VS Code terminal shell
  integration events, and `CliOnboarding`.
- Produces:
  - `resolveInstallCommand(platform, environment): Promise<InstallCommand>`
  - `openCliOnboardingPanel(context, dependencies): Promise<void>`
  - the bundled `dist/webviews/onboarding.js`

- [ ] **Step 1: Implement fixed platform commands**

Create `command.ts`:

```ts
export type InstallCommand =
  | {
      kind: "supported";
      platformLabel: string;
      command: string;
      shellPath?: string;
    }
  | {
      kind: "unsupported";
      platformLabel: string;
      message: string;
    };

const POSIX_INSTALL =
  "curl --proto '=https' --tlsv1.2 -LsSf " +
  "https://github.com/crabbuild/compass/releases/latest/download/install.sh | sh";
const WINDOWS_INSTALL =
  "Invoke-RestMethod " +
  "'https://github.com/crabbuild/compass/releases/latest/download/install.ps1' " +
  "| Invoke-Expression";
```

For `darwin` and `linux`, return `POSIX_INSTALL` without a forced shell. For
`win32`, call an injected executable-access helper over this ordered candidate
list:

1. each `pwsh.exe` found on `PATH`;
2. `%ProgramFiles%\PowerShell\7\pwsh.exe`;
3. each `powershell.exe` found on `PATH`;
4. `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`.

Return the first executable candidate as `shellPath`. If none is executable,
return `kind: "unsupported"` with manual release guidance. For all other
platforms, return unsupported.

- [ ] **Step 2: Implement strict onboarding messages**

Create `messages.ts`:

```ts
export const OnboardingToHostMessageSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("ready") }),
  z.object({ type: z.literal("install") }),
  z.object({ type: z.literal("verifyAgain") }),
  z.object({ type: z.literal("selectExisting") }),
  z.object({ type: z.literal("initializeRepository") }),
  z.object({ type: z.literal("openRepository") }),
  z.object({ type: z.literal("showTerminal") })
]);

export const HostToOnboardingMessageSchema = z.object({
  type: z.literal("state"),
  state: CliOnboardingStateSchema
});
```

Mirror every `CliOnboardingState` branch in `CliOnboardingStateSchema`. Apply
`.max(8192)` to messages and paths, `.max(256)` to searched arrays, and reject
unknown properties with `.strict()`.

- [ ] **Step 3: Implement panel lifecycle and installation orchestration**

Create a singleton `CliOnboardingPanel` that reveals an existing page instead
of creating parallel installers. Its injected dependencies must include:

```ts
export type CliOnboardingDependencies = {
  runtime: CompassRuntime;
  configuration(): vscode.WorkspaceConfiguration;
  selectExisting(): Promise<void>;
  initializeRepository(): Promise<void>;
  refresh(): Promise<void>;
  discover(): Promise<CompassDiscovery>;
  resolveCommand(): Promise<InstallCommand>;
  pollIntervalMs?: number;
  pollTimeoutMs?: number;
};
```

The production defaults are 750 ms polling and 120 seconds timeout. Implement:

1. initial discovery; if found, activate it and show ready;
2. fixed command resolution and ready-to-install/unsupported hydration;
3. terminal creation as `{ name: "Compass Setup", shellPath }`;
4. `terminal.show(false)` before execution;
5. a bounded wait for `terminal.shellIntegration`;
6. `shellIntegration.executeCommand(command)` plus exact execution-identity
   completion when available;
7. `terminal.sendText(command, true)` plus discovery polling otherwise;
8. verification through `discover()` followed by `runtime.activate()`;
9. terminal-close, panel-dispose, timeout, nonzero exit, no-executable, and
   incompatible-executable recovery states; and
10. listener/timer disposal without closing the terminal.

Never accept a command from `OnboardingToHostMessageSchema`. The parsed
`install` intent always uses the already resolved host command.

- [ ] **Step 4: Implement the webview adapter**

Create `webviews/onboarding.tsx` following `webviews/initialize.tsx`. Render
`CliOnboarding`, translate host callbacks into the seven bounded message types,
validate incoming host state with `HostToOnboardingMessageSchema`, and send
`{ type: "ready" }` after the first render.

Add the entry point in `esbuild.mjs`:

```js
entryPoints: {
  graph: "src/webviews/graph.tsx",
  callGraph: "src/webviews/callGraph.tsx",
  callGraphGuide: "src/webviews/callGraphGuide.tsx",
  architecture: "src/webviews/architecture.tsx",
  query: "src/webviews/query.tsx",
  history: "src/webviews/history.tsx",
  initialize: "src/webviews/initialize.tsx",
  onboarding: "src/webviews/onboarding.tsx"
}
```

Use the existing local `viewer.css` and nonce-only CSP in the panel HTML.

- [ ] **Step 5: Add implementation-following tests**

Cover command selection for macOS, Linux, Windows `pwsh`, Windows PowerShell
fallback, Windows without PowerShell, and an unsupported platform.

Cover message schemas by accepting exactly the seven intents and every host
state, then rejecting extra fields, oversized values, shell text, URLs, and
unbounded searched arrays.

In `cliOnboardingPanel.test.ts`, inject a fake terminal/event host and assert:

- the terminal is shown before command execution;
- shell-integration completion is filtered by terminal and execution identity;
- exit code zero enters discovery and activation;
- nonzero exit preserves the terminal and shows retry;
- no shell integration sends text and polls until discovery succeeds;
- timeout and terminal closure dispose polling;
- panel disposal does not dispose the terminal; and
- duplicate install intents create only one active execution.

- [ ] **Step 6: Run focused verification**

```bash
npm run test -w editors/vscode -- \
  src/install/command.test.ts \
  src/install/messages.test.ts \
  src/views/cliOnboardingPanel.test.ts
npm run typecheck -w editors/vscode
npm run build -w editors/vscode
test -s editors/vscode/dist/webviews/onboarding.js
git diff --check -- editors/vscode/src/install \
  editors/vscode/src/views/cliOnboardingPanel.ts \
  editors/vscode/src/webviews/onboarding.tsx \
  editors/vscode/esbuild.mjs
```

Expected: focused tests and typecheck pass; the onboarding webview bundle is
nonempty.

- [ ] **Step 7: Commit the terminal onboarding deliverable**

```bash
git add editors/vscode/src/install \
  editors/vscode/src/views/cliOnboardingPanel.ts \
  editors/vscode/src/views/cliOnboardingPanel.test.ts \
  editors/vscode/src/webviews/onboarding.tsx \
  editors/vscode/esbuild.mjs
git commit -m "feat(vscode): install Compass in the integrated terminal"
```

### Task 5: Extension Entry Points and No-Reload Handoff

**Files:**
- Modify: `editors/vscode/src/extension.ts`
- Modify: `editors/vscode/src/views/treeModel.ts`
- Modify: `editors/vscode/src/views/treeModel.test.ts`
- Modify: `editors/vscode/src/test/suite/extension.integration.ts`
- Modify: `editors/vscode/package.json`
- Modify: `editors/vscode/README.md`
- Modify: `editors/vscode/CHANGELOG.md`

**Interfaces:**
- Consumes: `CompassRuntime`, `openCliOnboardingPanel`, the existing repository
  selector, and the existing initialization command.
- Produces: `compass.installCli`, missing-CLI onboarding entry points, and
  no-reload activation for manual CLI selection.

- [ ] **Step 1: Instantiate and subscribe the runtime**

In `activate`, keep the initial discovery and shared process manager, then add:

```ts
const runtime = new CompassRuntime(discovery, {
  processes,
  sessions: () => registry.all(),
  persistCliPath: async (path) => {
    await vscode.workspace.getConfiguration("compass").update(
      "cliPath",
      path,
      vscode.ConfigurationTarget.Global
    );
  }
});
const workspaceTree = new WorkspaceTree(registry, () => runtime.discovery);
context.subscriptions.push(runtime.onDidChange(() => {
  workspaceTree.refresh();
  statusBar.refresh();
}));
```

Move `statusBar` creation before registering the subscription, or register the
listener immediately after both view objects exist.

- [ ] **Step 2: Activate selected CLIs without reloading**

Replace the current `compass.cliPath` update plus **Reload Window** prompt in
`selectCompassBinary` with:

```ts
try {
  const activated = await runtime.activate(installation);
  await refresh();
  void vscode.window.showInformationMessage(
    `Compass ${activated.installation.version ?? activated.capabilities.compass_version} ` +
    `is ready at ${activated.installation.executable}.`
  );
} catch (error) {
  void vscode.window.showErrorMessage(
    `Compass CLI could not be activated: ${message(error)}`
  );
}
```

Keep browse/manual inspection and the explicit unverified-binary warning, but
do not publish any executable that fails runtime capability validation.

- [ ] **Step 3: Register and route the onboarding command**

Register:

```ts
vscode.commands.registerCommand("compass.installCli", () =>
  openCliOnboardingPanel(context, {
    runtime,
    configuration: () => vscode.workspace.getConfiguration("compass"),
    selectExisting: selectCompassBinary,
    initializeRepository: async () => {
      await vscode.commands.executeCommand("compass.initialize");
    },
    refresh,
    discover: () => discoverCompass(vscode.workspace.getConfiguration("compass")),
    resolveCommand: () => resolveInstallCommand(process.platform, process.env)
  })
)
```

Change the missing startup notification to:

```ts
void vscode.window.showInformationMessage(
  "Install Compass to build and explore a local code graph.",
  "Install Compass",
  "Select existing CLI"
).then(async (action) => {
  if (action === "Install Compass") {
    await vscode.commands.executeCommand("compass.installCli");
  } else if (action === "Select existing CLI") {
    await vscode.commands.executeCommand("compass.selectCli");
  }
});
```

- [ ] **Step 4: Update the Workspace tree entry**

Change the missing branch in `cliAttentionNodes` to:

```ts
return [{
  id: "cli-setup",
  label: "Set up Compass",
  description: "Not installed",
  tooltip: "Install Compass or select an existing CLI to continue.",
  icon: "rocket",
  command: "compass.installCli"
}];
```

Leave the incompatible branch routed to `compass.selectCli`.

- [ ] **Step 5: Contribute the command and onboarding copy**

Add this command to `package.json`:

```json
{
  "command": "compass.installCli",
  "title": "Compass: Install CLI",
  "icon": "$(cloud-download)"
}
```

Change the walkthrough's CLI step to:

```json
{
  "id": "compass.cli",
  "title": "Install the Compass CLI",
  "description": "Open **Compass: Install CLI** to install Compass in the integrated terminal, or select an existing executable."
}
```

Because the file already contains the user's uncommitted `0.1.7` version
change, preserve that worktree value. After applying the command and
walkthrough changes, temporarily change only the version line back to the HEAD
value with `apply_patch`:

```diff
-  "version": "0.1.7",
+  "version": "0.1.6",
```

Stage `editors/vscode/package.json`, then immediately restore the worktree
version with `apply_patch`:

```diff
-  "version": "0.1.6",
+  "version": "0.1.7",
```

The index will contain the feature changes with the original `0.1.6` version,
while the worktree retains the user's `0.1.7` change. Confirm the cached diff
contains no version hunk before committing.

- [ ] **Step 6: Update user-facing extension documentation**

Add an `0.1.7` changelog section:

```markdown
## 0.1.7

- Add first-run Compass CLI installation in a visible VS Code terminal on
  macOS, Linux, and Windows.
- Verify and activate installed or manually selected CLIs without reloading the
  editor.
```

Update `editors/vscode/README.md` requirements and Workspace sections to
describe **Set up Compass**, the exact-command preview, visible terminal,
automatic verification, and **Initialize repository** handoff.

- [ ] **Step 7: Add entry-point regression tests**

Update `treeModel.test.ts` to expect:

```ts
expect(missing[0]).toMatchObject({
  label: "Set up Compass",
  description: "Not installed",
  command: "compass.installCli"
});
```

Add `"compass.installCli"` to the integration test's required command list.
Retain the incompatible assertion for `compass.selectCli`.

- [ ] **Step 8: Run focused verification**

```bash
npm run test -w editors/vscode -- \
  src/views/treeModel.test.ts
npm run typecheck -w editors/vscode
npm run test:integration -w editors/vscode
git diff --check -- editors/vscode
```

Expected: tree, typecheck, and extension-host command registration pass.

- [ ] **Step 9: Commit the extension wiring**

Stage all task files plus the already prepared package manifest index entry:

```bash
git add editors/vscode/src/extension.ts \
  editors/vscode/src/views/treeModel.ts \
  editors/vscode/src/views/treeModel.test.ts \
  editors/vscode/src/test/suite/extension.integration.ts \
  editors/vscode/README.md editors/vscode/CHANGELOG.md
git diff --cached --check
git diff --cached -- editors/vscode/package.json
git commit -m "feat(vscode): guide missing CLI users through setup"
```

Expected before the commit: the cached package diff contains the command and
walkthrough changes but no version change.

After commit:

```bash
git diff -- editors/vscode/package.json
```

Expected: only the pre-existing `0.1.6` to `0.1.7` version change remains
unstaged.

### Task 6: Packaging, Full Regression, and Release Readiness

**Files:**
- Modify: `editors/vscode/scripts/smoke-vsix.mjs`

**Interfaces:**
- Consumes: all previous task deliverables.
- Produces: a smoke-verified VSIX containing the onboarding page and no native
  Compass executable.

- [ ] **Step 1: Require the onboarding bundle in VSIX smoke verification**

Add this path to the `required` array:

```js
"extension/dist/webviews/onboarding.js",
```

Keep the existing rejection:

```js
if (entries.some((entry) => entry === "extension/compass" || entry === "extension/compass.exe")) {
  throw new Error("VSIX must not bundle the native Compass CLI");
}
```

- [ ] **Step 2: Run all JavaScript and extension checks**

```bash
npm run typecheck:js
npm run test:js
npm run test:integration -w editors/vscode
node scripts/check_viewer_assets.mjs
npm run build:vscode
npm run package -w editors/vscode
npm run smoke:vsix -w editors/vscode
```

Expected: every command exits zero and the VSIX smoke output names the current
package filename.

- [ ] **Step 3: Run installer and repository hygiene checks**

```bash
sh scripts/test_release_scripts.sh
git diff --check
git status --short
```

Expected:

- release script tests pass;
- no whitespace errors;
- the only unrelated worktree entries are the pre-existing package version
  hunk and unrelated untracked plans;
- no Compass binary appears in the VSIX; and
- no Graphify command has been run.

- [ ] **Step 4: Inspect the user journey in an Extension Development Host**

Launch the extension:

```bash
code --extensionDevelopmentPath="$PWD/editors/vscode"
```

With `compass.cliPath` cleared and Compass absent from the test host:

1. confirm the Workspace row says **Set up Compass — Not installed**;
2. open the onboarding page and confirm the exact command is visible;
3. select **Install Compass** and confirm **Compass Setup** is visible before
   execution;
4. confirm installing and verifying states announce progress;
5. confirm ready shows the actual version and absolute path;
6. select **Initialize repository** and confirm the existing scope wizard
   opens;
7. repeat with a deliberately failing release URL and confirm terminal-preserved
   recovery;
8. repeat binary selection and confirm no VS Code reload prompt appears; and
9. confirm light, dark, high-contrast, and 320-pixel-wide layouts remain usable.

- [ ] **Step 5: Commit smoke coverage**

```bash
git add editors/vscode/scripts/smoke-vsix.mjs
git commit -m "test(vscode): smoke-check CLI onboarding assets"
```

- [ ] **Step 6: Review the complete implementation diff**

```bash
git log --oneline --decorate -8
git diff HEAD~6..HEAD --stat
git status --short
```

Confirm every design requirement maps to a committed task, the uncommitted
`0.1.7` version change remains owned by the user, unrelated untracked plans
remain untouched, and no required work is left.
