# Review pull requests with Compass

Compass can publish one evidence-qualified PR report as JSON, Markdown, SARIF,
an artifact, a job summary, and an optional sticky comment. Advisory risk
helps reviewers prioritize attention. Only typed deterministic gates can fail
the reusable Action.

## Review exact local revisions

Make both commits available locally, then run:

```bash
compass review \
  --base "$BASE_SHA" \
  --head "$HEAD_SHA" \
  --format json \
  --output compass-pr-review.json
```

Use full SHAs in automation. Compass resolves them once, creates a
deterministic synthetic merge without changing the checkout, materializes
comparable graph history when needed, and rejects profile mismatch. Local mode
never fetches missing objects.

If the checkout has no existing history profile and contains non-code files,
select the local structural profile explicitly before the first review:

```bash
compass history build "$BASE_SHA" --code-only
```

This keeps the review fully local and does not install hooks. A semantic review
must use a configured semantic history profile instead; missing credentials are
never silently downgraded. The reusable GitHub Action performs the explicit
code-only preparation automatically.

To bind a frozen GitHub event without a second API read:

```bash
compass review \
  --base "$BASE_SHA" \
  --head "$HEAD_SHA" \
  --repo crabbuild/compass \
  --host github.com \
  --pull-request-number 42 \
  --format json
```

Use `--pr 42 --repo crabbuild/compass` when you intentionally want the GitHub
adapter to freeze current API metadata and paginated changed files through the
authenticated `gh` CLI.

## Use the reusable Action safely

Pin the Action to a full commit SHA. Put review in a dedicated job that checks
out source but does not run repository scripts, build hooks, tests, package
installers, or other contributor-controlled executables:

```yaml
name: Compass PR review

on:
  pull_request:
    types: [opened, synchronize, reopened]

permissions:
  contents: read
  pull-requests: write

jobs:
  compass-review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@<full-commit-sha>
        with:
          fetch-depth: 0
          persist-credentials: false
      - uses: crabbuild/compass@<full-commit-sha>
        with:
          compass-version: <exact-compatible-release>
          fail-on: none
          github-token: ${{ secrets.GITHUB_TOKEN }}
```

Replace the commit placeholders with reviewed immutable Action commits and set
`compass-version` to an exact released version that contains `compass review`.
The Action has no binary-version default, so it cannot silently install an
older release that lacks the command. It downloads the exact configured
Compass release and checksum, validates the
archive layout, runs analysis without the token in its environment, uploads
the evidence, writes the job summary, and only then passes the token to the
pinned comment-delivery step.

Do not add `pull_request_target` to check out or execute the contributor head.
Do not place this Action after a step that executes untrusted PR code in the
same job. If your CI must run contributor code, keep that work in another job
without write permissions or secrets.

## Forks and permissions

For a fork PR, the Action still freezes revisions, analyzes, writes the job
summary, and uploads the artifact. It suppresses comment delivery even if a
token input was configured. A missing token or absent PR number also disables
delivery.

If GitHub returns a normal permission denial, the Action records
`permission-denied` and keeps the already-published report truthful. Rate-limit
and server failures use bounded retries; unresolved delivery failures remain
errors. The marker includes repository, PR, report schema, and Action identity.
Reruns update one owned comment and remove duplicate owned markers without
touching other comments.

## Choose gate policy

```yaml
with:
  fail-on: none
```

`none` never converts advisory risk or a failing typed gate into Action
failure. The output is `advisory` when a gate reports fail.

```yaml
with:
  fail-on: deterministic
```

`deterministic` fails only when at least one typed gate state is `fail`.
`indeterminate` remains a distinct successful outcome so a conflict or missing
evidence is not mislabeled as clean. Analysis and gate `error` states fail with
`analysis-error`. There is deliberately no `fail-on-risk` input.

## Consume outputs

The Action exposes `report-path`, `sarif-path`, `outcome`, and
`comment-outcome`. The uploaded artifact is the durable cross-job boundary;
paths point into the current runner's temporary directory and are not portable
to another job.

Treat the JSON, Markdown, SARIF, and logs as source-sensitive. They can expose
internal symbols, paths, dependencies, and verification gaps. Apply the same
artifact access and retention policy as the repository.

For an additive readiness summary, run `compass review` with `--readiness`.
This preserves the canonical report and emits a separate digest-linked
`compass.pr-readiness/1` envelope. A read-only workflow example is available at
[PR readiness Action example](../examples/pr-readiness-action.yml). Pin all
production actions and Compass versions to reviewed immutable releases.

## Related pages

- [PR Intelligence contract](../reference/pr-intelligence.md)
- [CI and automation cookbook](../cookbook/ci-and-automation.md)
- [Security and privacy](../design/security-and-privacy.md)

**Next step:** start with `fail-on: none`, inspect several complete and
conflicted reports, then enable `deterministic` only after your branch policy
recognizes the typed gate contract.
