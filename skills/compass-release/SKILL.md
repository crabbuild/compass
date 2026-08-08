---
name: compass-release
description: Release Compass from the latest origin/main. Use this skill whenever a user asks to sync Compass, bump a Compass version, create or merge a release PR, build or publish macOS/Linux/Windows artifacts, write release notes, verify release downloads, or build the matching VS Code VSIX. Follow the complete gated workflow rather than treating a release as a local version edit.
compatibility: Requires git, GitHub CLI (gh) with push/release permission, jq, Rust 1.97.1, Cargo, Node/npm, tar, shasum (or sha256sum), and a writable /Volumes/Workspace/crabbuild-target volume for Cargo builds.
---

# Compass release

Use this skill for the full Compass release path. Compass releases are immutable, multi-platform artifacts, so the version bump, PR, tag, CI build, release notes, and download verification must describe one exact commit.

## Inputs and defaults

Extract these from the user request before changing anything:

- `version`: the explicit SemVer requested by the user (for example `0.3.7`). Never guess a version when the request is ambiguous.
- `with_vsix`: build the VS Code extension when requested; do not publish it to a GitHub release unless the user explicitly asks.
- `release_notes`: summarize the user-visible changes since the previous Compass tag. Keep the notes concise and link the changelog and comparison range.
- `repo`: the Compass checkout named by the user. Resolve it with `git rev-parse --show-toplevel`; do not assume a particular worktree path.

Use these shell variables after the version is confirmed:

```sh
version="<requested-version>"
tag="compass-v${version}"
branch="codex/release-${version}"
target_dir="/Volumes/Workspace/crabbuild-target/compass-release-${version}"
```

## Safety gates

Stop and report the blocker instead of improvising when:

1. The worktree has user changes (`git status --short` is non-empty). Preserve them; never use `git reset --hard`, `git clean`, force-push, or overwrite unrelated files.
2. `/Volumes/Workspace` is missing or `$target_dir` cannot be created and written. Cargo must not fall back to a local `target/` directory.
3. The requested tag already exists, points at a different commit, or the GitHub release already exists. Published realizations are immutable.
4. `gh auth status` or repository permissions do not allow fetching, pushing, PR operations, or release inspection.
5. Metadata, validation, PR checks, tag validation, a release job, an archive checksum, or an archive smoke check fails. Do not publish a partial artifact set.

Publishing a PR, tag, or release is an external mutation, but it is in scope when the user explicitly asks to publish. Confirm exact version, branch, base commit, and target repository in commentary before the first push.

## 1. Sync and audit

From the repository root:

```sh
git fetch origin main
git status --short
git rev-parse origin/main
git log -1 --oneline origin/main
gh auth status
```

Start from the exact `origin/main` tree. If the worktree is clean, create or switch to `codex/release-${version}` from `origin/main`; if the branch already exists, verify that it has no unrelated commits before continuing. Do not silently release from a stale branch. Record the base SHA.

Read the repository `AGENTS.md`, `COMPATIBILITY.md`, `docs/implementation/workspace-tour.md`, and `docs/implementation/extending-compass.md` before editing. For a release-only change, the relevant package identity is `compass-cli` (binary `compass`), the shared workspace version in the root `Cargo.toml`, and the VS Code package when the extension is included.

Audit the current version with Cargo metadata, not a single text match:

```sh
RUSTUP_TOOLCHAIN=1.97.1 cargo metadata --no-deps --format-version 1
node -p "require('./editors/vscode/package.json').version"
```

The Compass workspace packages must agree on one release version (excluding the vendored language-pack exception used by the repository). The fuzz harness package itself remains `0.0.0`.

## 2. Bump metadata and changelog

Update only Compass-owned metadata from the previous version to `$version`:

- root workspace version in `Cargo.toml`;
- internal Compass dependency constraints in `crates/*/Cargo.toml` and `fuzz/Cargo.toml`;
- Compass package records in `Cargo.lock` and `fuzz/Cargo.lock`;
- the qualification producer version in `tests/qualification/code-graph-v1-semantic.json` when present;
- `editors/vscode/package.json` and its corresponding root `package-lock.json` workspace entry;
- `CHANGELOG.md`, preserving the empty `Unreleased` section and adding a dated `$version` section.

Do not globally replace arbitrary occurrences of the old number: external dependencies, the fuzz package version, schema versions, and historical changelog entries may legitimately retain other values. Use a controlled edit and inspect `git diff`.

Build release notes from the meaningful commits between the previous `compass-v*` tag and `origin/main`; omit generated noise and internal-only details. Cover behavior users can act on, installation, checksums, the changelog link, and the comparison link.

## 3. Validate before publishing the PR

Verify the build volume and create only this checkout's target directory:

```sh
test -d /Volumes/Workspace
mkdir -p "$target_dir"
test -w "$target_dir"
```

Set `CARGO_TARGET_DIR="$target_dir"` on every Cargo invocation. Run the repository's release gates, using `--locked` and Rust 1.97.1:

```sh
RUSTUP_TOOLCHAIN=1.97.1 CARGO_TARGET_DIR="$target_dir" cargo metadata --no-deps --format-version 1
RUSTUP_TOOLCHAIN=1.97.1 CARGO_TARGET_DIR="$target_dir" cargo check --workspace --all-targets --locked
RUSTUP_TOOLCHAIN=1.97.1 CARGO_TARGET_DIR="$target_dir" cargo fmt --all -- --check
RUSTUP_TOOLCHAIN=1.97.1 CARGO_TARGET_DIR="$target_dir" cargo clippy --workspace --lib --bins --locked -- -D warnings
RUSTUP_TOOLCHAIN=1.97.1 CARGO_TARGET_DIR="$target_dir" cargo test --workspace --lib --bins --locked
RUSTUP_TOOLCHAIN=1.97.1 CARGO_TARGET_DIR="$target_dir" cargo test -p compass-cli --test compass_product --locked
sh scripts/check_product_boundary.sh
sh scripts/test_release_scripts.sh
RUSTUP_TOOLCHAIN=1.97.1 CARGO_TARGET_DIR="$target_dir" ./scripts/qualify_code_graph_v1.sh --fixtures-only
```

For a release that changes JavaScript, the VS Code extension, or viewer assets, also run:

```sh
npm ci
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs
```

If a required gate cannot run, state why and do not claim the release is fully verified.

## 4. Commit, PR, and merge

Inspect all changes before staging:

```sh
git diff --check
git diff --stat
git status --short
```

Commit the intentional release metadata and changelog as `Release Compass $version`. Push `codex/release-$version`, then open a draft PR into `main` with a short summary, validation list, and release-note summary. Mark it ready only after the diff is reviewed. Wait for every required check, including the six platform build checks, to pass. Merge with the repository's normal merge strategy and delete the remote branch only after checks are green.

After merging:

```sh
git fetch origin main
merge_sha="$(git rev-parse origin/main)"
gh pr view <number> --repo <owner>/<repo> --json state,mergeCommit
```

The PR merge commit becomes the only valid release source. Do not tag the pre-merge branch tip.

## 5. Tag, build, publish, and verify the CLI release

Before tagging, confirm the tag is absent and the merged tree has one uniform Compass version. Create an annotated tag on the merge SHA and push it:

```sh
git show-ref --verify --quiet "refs/tags/$tag" && {
  echo "tag already exists: $tag" >&2
  exit 1
}
git tag -a "$tag" "$merge_sha" -m "Compass v${version}"
git push origin "$tag"
```

The tag triggers `.github/workflows/compass-release.yml`. It builds and verifies these six targets:

- `x86_64-apple-darwin` and `aarch64-apple-darwin`;
- `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`;
- `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`.

Monitor the exact workflow run until it completes successfully:

```sh
run_id="$(gh run list --repo <owner>/<repo> --workflow compass-release.yml --limit 10 --json databaseId,headBranch,headSha,status,conclusion | jq -r --arg tag "$tag" '.[] | select(.headBranch == $tag) | .databaseId' | head -n 1)"
test -n "$run_id"
gh run watch "$run_id" --repo <owner>/<repo> --interval 60 --exit-status
```

Inspect the published release and replace autogenerated notes with curated notes titled `Compass v$version`. The expected asset set is 15 files: six `.tar.gz` archives, six matching `.sha256` files, `compass-release.json`, `install.sh`, and `install.ps1`.

Download into a unique temporary directory and verify every checksum and archive listing:

```sh
verify_dir="$(mktemp -d "/tmp/compass-release-${version}.XXXXXX")"
gh release download "$tag" --repo <owner>/<repo> --dir "$verify_dir"
test "$(find "$verify_dir" -maxdepth 1 -type f | wc -l | tr -d ' ')" = 15
test "$(jq -r '.schema' "$verify_dir/compass-release.json")" = "compass.release/1"
test "$(jq -r '.version' "$verify_dir/compass-release.json")" = "$version"
test "$(jq -r '.tag' "$verify_dir/compass-release.json")" = "$tag"
test "$(jq -r '.artifacts | length' "$verify_dir/compass-release.json")" = 6
for checksum_file in "$verify_dir"/*.sha256; do
  (cd "$verify_dir" && shasum -a 256 -c "$(basename "$checksum_file")")
done
for archive in "$verify_dir"/compass-*.tar.gz; do
  tar -tzf "$archive" >/dev/null
done
```

Confirm the release is not draft/prerelease, the tag resolves to `origin/main`, and the worktree has no generated or unrelated changes. Report the release URL, workflow URL, tag SHA, all target names, asset count, and verification result. Include convenient exact-version install commands:

```sh
curl -fsSL "https://github.com/<owner>/<repo>/releases/download/${tag}/install.sh" | sh
```

```powershell
irm "https://github.com/<owner>/<repo>/releases/download/${tag}/install.ps1" | iex
```

## 6. Optional VS Code VSIX

When the user asks to build the extension, first ensure `editors/vscode/package.json` and the root lockfile workspace entry equal `$version`, then run:

```sh
npm ci
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs
npm run package -w editors/vscode
npm run smoke:vsix -w editors/vscode
shasum -a 256 "editors/vscode/crabbuild-compass-vscode-${version}.vsix"
```

The local artifact is `editors/vscode/crabbuild-compass-vscode-$version.vsix`. The manual `.github/workflows/compass-vscode-release.yml` workflow can package a CI artifact when the user explicitly asks; it requires the version input and confirmation `package`. Do not attach the VSIX to the CLI release unless requested.

## Final handoff

Lead with the outcome. State:

- version, release URL, PR URL, tag, merge SHA, and workflow URL;
- six target archives, six checksums, `compass-release.json`, and the two installer scripts (or the VSIX path/checksum when requested);
- curated release-note highlights;
- gates run and any checks not run or non-fatal warnings;
- checksum/archive verification and worktree status.

Never describe a release as published until the GitHub release is live and all expected downloads have passed verification.
