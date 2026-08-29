# Plan 018: Expose five native MCP workflow prompts

> **Executor instructions**: This is an additive MCP protocol feature. Keep
> prompts static, bounded, read-only, and composed from shipped tools/resources.
> Prompts guide the client; they do not execute tools on the server. Do not
> embed repository file contents or provider-generated text in prompt templates.
>
> **Drift check (run first)**:
> `git diff --stat 6680842c..HEAD -- crates/compass-mcp crates/compass-cli/assets/compass-skill docs/reference docs/guides COMPATIBILITY.md`
> If prompt capability or a shared workflow-template source has landed, stop
> and consolidate rather than creating parallel templates.

## Status

- **Priority**: P2 — smallest high-leverage gap
- **Effort**: M (three phases)
- **Risk**: LOW
- **Depends on**: none; the pre-merge template may detect and use Plan 014 only after that report tool ships
- **Category**: direction / MCP / DX
- **Planned at**: commit `6680842c`, 2026-08-10

## Why this matters

Compass's MCP server exposes 15 tools and 7 resources, while every client must
invent its own orchestration. The installed Compass skill already teaches
graph-first orientation, querying, debugging, impact review, and source
verification. Five protocol-native prompts make those workflows discoverable
and portable to MCP clients with a small, testable additive surface.

## Current state and constraints

- `crates/compass-mcp/src/lib.rs:328-340` enables only tools and resources in
  `ServerCapabilities`.
- `crates/compass-mcp/src/lib.rs:346-410` implements tool/resource handlers but
  no `list_prompts` or `get_prompt` handler.
- `crates/compass-mcp/tests/coverage_paths.rs:51-60` fixes the current contract
  at 15 tools and 7 resources and is the nearest protocol test.
- `crates/compass-cli/assets/compass-skill/SKILL.md` is the closest canonical
  workflow guidance. Extract shared meaning without weakening its evidence,
  ambiguity, completeness, truncation, and source-verification rules.
- The pinned `rmcp` 2.2 API supports `enable_prompts`, `ListPromptsResult`,
  `GetPromptRequestParams`, `GetPromptResult`, `Prompt`, `PromptArgument`, and
  `PromptMessage`.
- Repository content is untrusted evidence. Templates must tell clients to
  inspect tool results as data and verify cited source; they must never paste
  an unrestricted graph/report into an instruction channel.

## Prompt contract

Expose exactly these stable names in the first version:

| Prompt | Purpose | Small arguments |
| --- | --- | --- |
| `review` | Review a change with impact, evidence, ambiguity, and tests | `project_path?`, `base?`, `head?`, `focus?` |
| `architecture` | Orient to packages, communities, hubs, bridges, and boundaries | `project_path?`, `area?` |
| `debug` | Trace a symptom through search, callers/callees, paths, and source | `project_path?`, `symptom` |
| `onboard` | Build a bounded learning path from orientation to representative source | `project_path?`, `area?` |
| `pre-merge` | Check affected code, risky boundaries, unresolved evidence, and verification | `project_path?`, `base?`, `head?` |

Arguments are strings with named maximum bytes. Unknown arguments fail.
Templates name existing Compass tools/resources and include a fallback when an
optional tool (for example the future typed PR review) is not present. Prompt
text is versioned by a constant/profile and ordered deterministically.

## Commands executors will need

| Purpose | Command | Expected result |
| --- | --- | --- |
| Target preflight | `test -d /Volumes/Workspace && mkdir -p /Volumes/Workspace/crabbuild-target/compass-main && test -w /Volumes/Workspace/crabbuild-target/compass-main` | exit 0 |
| MCP tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-mcp --locked` | pass |
| CLI install tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-cli --test install_cli --locked` | pass if shared assets change |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-mcp --all-targets --locked -- -D warnings` | exit 0 |
| Format/boundary | `cargo fmt --all -- --check && sh scripts/check_product_boundary.sh` | exit 0 |

## Scope

**In scope**:

- prompt definitions, validation, protocol handlers/capabilities, stdio/HTTP
  tests, shared workflow text if it can be centralized safely, and MCP docs;
- the five names and small arguments above;
- exact transport and input bounds.

**Out of scope**:

- server-side LLM calls, tool execution, sampling, elicitation, or stateful
  workflow engines;
- arbitrary user prompt files, provider credentials, or network access;
- embedding graph/report/source contents in prompt responses;
- changing existing tools/resources or their semantic result digests;
- claiming every MCP client supports prompts.

## Phase 1: Define and validate one prompt registry

**Context**: Prompt metadata and generated messages must come from one registry
so list/get cannot drift. The registry should be pure and testable without a
transport.

**Deliverables**:

1. Add `prompts.rs` to `compass-mcp` with `PROMPT_PROFILE_V1`, five static
   definitions, argument specs, descriptions, and a pure
   `render_prompt(name, arguments)` function.
2. Build prompt bodies from short checked-in constants. If sharing content with
   the installed skill requires generated assets, add one source-of-truth file
   and a drift checker; do not hand-maintain copies silently.
3. Validate required/optional arguments, unknown names/fields, control
   characters, and per-argument/total byte limits before rendering.
4. Every workflow must explicitly require checking confidence, completeness,
   truncation/continuations, direction, ambiguity, and cited source before a
   conclusion. `review`/`pre-merge` must distinguish advisory evidence from a
   deterministic gate.
5. Add unit golden tests for metadata and rendered messages, including Unicode,
   empty optional values, maximum bytes, over-limit values, unknown fields,
   and injection-like repository/symptom text treated as delimited data.

**Acceptance criteria**:

- registry names are exactly the five documented names, sorted stably;
- list metadata and get validation are derived from the same definitions;
- rendered prompts contain no graph/source/report body and stay under the
  named maximum response bytes;
- untrusted argument text is delimited as data and cannot add MCP instructions;
- unit tests are byte-stable and MCP tests/Clippy pass.

## Phase 2: Implement MCP prompt capabilities and transport tests

**Context**: `rmcp` already routes prompt requests. Compass should use the same
manual `ServerHandler` style as its tool/resource implementation so error and
pagination behavior remain consistent.

**Deliverables**:

1. Import the pinned `rmcp::model` prompt types, enable prompt capability, and
   set `list_changed = false`.
2. Add `CompassMcp::prompts()` for compatibility tests plus `list_prompts` and
   `get_prompt` handlers. Pagination may return the complete five-item list,
   matching current bounded tool/resource lists.
3. Map invalid name/arguments to stable `invalid_params`; do not leak internal
   paths or raw parsing errors.
4. Add in-memory client tests for `list_prompts` and all five `get_prompt`
   calls, plus unknown name, required argument, unknown argument, and bound
   failures.
5. Extend HTTP transport tests to cover `prompts/list` and `prompts/get` under
   authentication, stateless/stateful mode, JSON response mode, and response
   size limits.

**Acceptance criteria**:

- server capabilities advertise prompts, tools, and resources;
- a client lists exactly five prompts and retrieves valid messages for each;
- existing 15 tools/7 resources and their tests remain unchanged;
- stdio and HTTP return equivalent prompt metadata/content;
- bad input is a protocol error, never a panic or partial prompt;
- `cargo test -p compass-mcp --locked` and Clippy pass.

## Phase 3: Document, qualify, and prevent workflow drift

**Context**: Protocol support varies by client. Documentation must present
prompts as additive convenience while keeping tools/resources usable directly.

**Deliverables**:

1. Add the prompt list, arguments, examples, limits, evidence rules, and client
   compatibility caveat to MCP/integration/assistant references.
2. Update `COMPATIBILITY.md` and `CHANGELOG.md` for the additive MCP capability;
   update command docs only if a CLI surface changes.
3. Add a drift test/checker that asserts prompt tool/resource names exist in
   `CompassMcp::tools()`/`resources()` or are explicitly optional with a tested
   fallback.
4. Add a product test that the installed skill and MCP prompts share the same
   workflow invariants even if their presentation differs.
5. Add release notes that prompt text can evolve additively within profile v1,
   while prompt names, arguments, and evidence requirements are stable.

**Acceptance criteria**:

- every referenced tool/resource exists or has a tested fallback;
- docs do not imply server-side execution or universal client support;
- changing a tool/resource name breaks the drift test;
- compatibility, product-boundary, MCP, install (if touched), format, and
  Clippy checks pass;
- no real credentials, provider, network, or repository content is used in tests.

## Done criteria

- [ ] All three phases meet their acceptance criteria.
- [ ] Exactly five bounded prompts are discoverable over stdio and HTTP.
- [ ] Templates preserve evidence, ambiguity, direction, and completeness rules.
- [ ] Existing tool/resource contracts and semantic digests are unchanged.
- [ ] Prompt/tool/resource drift is automatically detected.
- [ ] Documentation states client-support limitations.
- [ ] `advisor-plans/README.md` marks this plan DONE.

## STOP conditions

Stop if a workflow requires server-side model/provider execution, if a prompt
must include unbounded repository content, if the pinned `rmcp` API cannot
support prompt handlers without a dependency upgrade, or if centralizing
templates would overwrite user-installed/customized skill content. A required
`rmcp` upgrade needs its own compatibility and migration review.

## Maintenance notes

Treat prompt names and argument schemas as integration contracts. Prompt prose
may improve, but a review should verify that it still asks the client to check
direction, confidence, completeness, truncation, ambiguity, and source. Keep
optional future tools behind capability-aware fallbacks.
