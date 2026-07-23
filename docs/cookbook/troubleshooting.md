# Cookbook: troubleshooting

This page is a symptom-to-diagnosis map. Start with the narrowest category and
preserve the original diagnostic before deleting or rebuilding anything.

> **Who this page is for:** all Compass users and operators.
>
> **You will learn:** how to diagnose installation, build, graph, query,
> semantic, service, assistant, and history problems safely.
>
> **Prerequisites:** access to the failing command, stderr, and working
> directory.
>
> **Completion time:** 5–20 minutes for common failures.

## First five checks

```bash
pwd
command -v compass
compass --version
git status --short
df -h .
```

Then rerun the exact command without suppressing stderr.

Record:

- command with secrets removed;
- exit code;
- Compass version;
- current directory;
- graph/output path;
- source revision/profile;
- first complete diagnostic.

## Installation

| Symptom | Likely cause | Diagnosis | Remedy |
| --- | --- | --- | --- |
| command not found | install dir absent from `PATH` | `command -v compass`, inspect `PATH` | add install dir and open new shell |
| source build uses wrong compiler | rustup override/toolchain | `rustup show` | use pinned `rust-toolchain.toml` |
| macOS blocks binary | unsigned/not notarized release policy | inspect release and OS message | follow organization policy; build from source if approved |
| checksum fails | incomplete/tampered/wrong archive | retain installer output | do not run archive; download from official release |
| build runs out of disk | large Rust dependencies/artifacts | `df -h .`, `du -sh target` | free space or remove only rebuildable Cargo artifacts |

## Build and update

| Symptom | Likely cause | Diagnosis | Remedy |
| --- | --- | --- | --- |
| no `compass-out/` | command failed before publication | inspect stderr/exit | fix stage error and rerun |
| graph stale | update not rerun/watcher failed | compare source times/revision; run manual update | `compass update .` |
| symbol missing | ignored/unsupported/generated/outside root | inspect report, ignore/exclude, source path | adjust scope/support; rebuild |
| provider requested unexpectedly | semantic sources present | inspect command and file classes | configure intentionally or use code-only |
| graph changed on no-op | config/version/input changed | compare manifest/version/options | clean qualification and investigate fingerprint |
| `graph.html` missing | disabled or graph too large | inspect command/report | query JSON; HTML is optional |

## Query

| Symptom | Likely cause | Diagnosis | Remedy |
| --- | --- | --- | --- |
| graph not found | wrong cwd/path | locate `graph.json` | run from root or `--graph PATH` |
| graph must be JSON | wrong input/export | inspect suffix/content | use canonical graph JSON |
| node not found | label mismatch/duplicates | broad query by file/domain | use stable ID or more context |
| path not found | wrong endpoint/direction/missing edge | explain both endpoints | verify graph coverage and relation |
| too many results | generic phrase/hub | inspect anchors | add behavior/domain terms or CompassQL |
| limit/timeout | query expansion too broad | run `EXPLAIN`, reduce path/labels | narrow query; approved budget only |
| JSON consumer breaks | schema mismatch or parsed human text | inspect version tag | consume documented JSON/JSONL and reject unknown major |

## CompassQL

Diagnostic families:

```text
CQL1xxx  source/syntax/unsupported
CQL2xxx  scope/type/projection/path shape
CQL3xxx  source/token/path/row/memory/time limit
CQL4xxx  parameter/runtime/regex/arithmetic/invariant
```

Response:

- `CQL1xxx/2xxx`: fix query;
- `CQL3xxx`: narrow work or limits deliberately;
- `CQL4xxx`: validate parameters/types and report invariant failures.

No successful partial result is produced on execution failure.

## Semantic providers

| Symptom | Diagnosis | Remedy |
| --- | --- | --- |
| missing key | check documented environment variable is present in process, not print value | configure approved secret or code-only |
| unsafe endpoint | inspect scheme/host warning | use approved HTTPS/loopback endpoint |
| context exceeded | inspect adaptive-retry diagnostic | reduce source/chunk/mode; do not loop manually |
| malformed provider output | retain redacted response metadata | retry bounded; provider/model/prompt may be incompatible |
| partial sources | inspect partial metadata and command policy | rerun failed sources or surface partial status |
| rate limit | inspect provider status | bounded backoff, lower concurrency |

Never paste secret headers or sensitive corpus content into an issue.

## Watch and hooks

```bash
compass hook status
compass update .
```

If manual update succeeds but watch/hook fails:

- stop duplicate watchers;
- inspect hook log;
- confirm installed binary path;
- check rebase/merge/worktree guards;
- reinstall managed hooks after binary relocation;
- uninstall hooks before hand-editing their managed sections.

## Assistant integration

| Symptom | Remedy |
| --- | --- |
| skill not discovered | verify platform, scope, generated destination, assistant restart |
| duplicate managed content | rerun idempotent installer, inspect diff, uninstall/reinstall managed section |
| strict mode surprises user | `COMPASS_HOOK_STRICT=0`, then review project hook |
| assistant ignores graph | confirm `compass-out/`, skill discovery, and explicit repository instructions |
| assistant trusts graph blindly | require source verification and provenance qualification |

## History

| Symptom | Response |
| --- | --- |
| profile mismatch | build comparable realization with `--profile-from` |
| realization missing | build explicitly or allow lazy `--at` materialization |
| preferred corrupt | inspect with list/show; explicit `rebuild --replace-corrupt` |
| lease/live work | wait/join/inspect; do not delete live lock |
| disk file does not shrink after GC | expected; logical reclamation is not `VACUUM` |
| copied DB will not open coherently | restore SQLite and WAL as a consistent resource |
| submodule/LFS/filter limitation | follow diagnostic; history refuses unsafe/implicit expansion |

Useful commands:

```bash
compass history status HEAD
compass history list HEAD --format json
compass history show REALIZATION_ID
compass history gc
```

## Disk-full recovery

Stop writers first. Identify generated space:

```bash
df -h .
du -sh target compass-out 2>/dev/null
```

Safe categories differ:

- Cargo `target/` is rebuildable but deleting it discards compilation cache;
- `compass-out/` is rebuildable current output;
- history SQLite/WAL is durable data and must not be deleted as cache;
- user source and untracked files are never cleanup targets.

Use explicit, validated paths. Report what was removed.

## When to report a bug

Create a minimal reproduction that includes:

- supported platform and Compass version;
- exact command/exit;
- tiny source fixture;
- expected nodes/edges/output;
- actual redacted graph fragment/diagnostic;
- whether cold, incremental, current, or historical;
- profile/backend name without credentials.

For vulnerabilities, follow [SECURITY.md](../../SECURITY.md), not a public
issue.

## Related pages

- [Getting started](../getting-started.md)
- [Operations](../guides/operations.md)
- [Support](../../SUPPORT.md)
- [Security policy](../../SECURITY.md)

**Next step:** capture the failing command, first diagnostic, version, path, and
profile before trying the smallest recovery listed above.
