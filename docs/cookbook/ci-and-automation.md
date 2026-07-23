# Cookbook: CI and automation

Use these patterns to generate, query, and publish Compass artifacts in CI
without depending on human text or hiding failures.

> **Who this page is for:** CI/platform engineers and maintainers adding graph
> checks to pull requests or scheduled jobs.
>
> **You will learn:** deterministic build jobs, exact query policies, artifact
> retention, cache keys, and historical comparison.
>
> **Prerequisites:** Compass installed in the runner and repository checkout.
>
> **Completion time:** 15–30 minutes.

## Recipe 1: build and upload a structural graph

```bash
set -eu
compass --version
compass update . --no-viz
test -s compass-out/graph.json
test -s compass-out/GRAPH_REPORT.md
test -s compass-out/manifest.json
```

Upload the three artifacts plus:

```bash
git rev-parse HEAD > compass-out/source-revision.txt
compass --version > compass-out/compass-version.txt
```

Treat graph artifacts with source-code confidentiality.

## Recipe 2: exact policy query

Create `.compass/no-ambiguous-auth.cypher`:

```cypher
MATCH (source)-[edge]->(target)
WHERE edge.confidence = 'AMBIGUOUS'
  AND (source.label CONTAINS 'Auth' OR target.label CONTAINS 'Auth')
RETURN source.id, type(edge), target.id
ORDER BY source.id, target.id
LIMIT 500
```

Run:

```bash
compass query --cql \
  --file .compass/no-ambiguous-auth.cypher \
  --format json \
  --output compass-out/no-ambiguous-auth.json
```

Decide policy in a separate script that validates
`compass.cql.result/1` and counts rows. This keeps query execution and
organization-specific pass/fail rules separate.

## Recipe 3: parameterized reusable query

`.compass/callers.cypher`:

```cypher
MATCH (caller)-[edge:CALLS]->(target)
WHERE target.label = $target
RETURN caller.id, edge.confidence, target.id
ORDER BY caller.id
LIMIT 500
```

Run:

```bash
compass query --cql \
  --file .compass/callers.cypher \
  --param target="$TARGET_LABEL" \
  --format json \
  --output compass-out/callers.json
```

Do not inject `TARGET_LABEL` into source text.

## Recipe 4: compare base and head

```bash
set -eu
compass history build "$BASE_SHA" --code-only
compass history build "$HEAD_SHA" --profile-from "$BASE_SHA"
compass diff "$BASE_SHA" "$HEAD_SHA" \
  --topology-only \
  --format json > compass-out/topology-diff.json
```

Use full commit SHAs supplied by the CI provider. Avoid fetch-on-demand inside
historical materialization; make required commits available during checkout.

## Cache strategy

Cache keys should include:

```text
operating-system / target
Compass version or binary hash
Cargo.lock or release artifact identity
build profile / code-only vs semantic
relevant parser/extractor version
repository content key
```

Do not restore:

- a semantic cache under another model/prompt/profile;
- query plan cache under an incompatible graph schema;
- a manifest from another output/root;
- a live SQLite/WAL history copy assembled from partial files.

History is a live durable store. Prefer supported backup or rebuild workflows
over naive cache archiving.

## Failure policy

```text
Compass nonzero
  -> job fails; retain diagnostic

Query limit
  -> job fails or reports inconclusive; never "zero matches"

Missing provider key
  -> fail semantic job or explicitly run code-only

Unknown result major version
  -> fail closed

Graph policy rows found
  -> organization policy decides warn/fail
```

## Security

- Pin or verify downloaded release artifacts.
- Do not echo provider or database keys.
- Avoid semantic providers for unapproved repositories.
- Do not expose `graph.html` publicly by default.
- Sanitize CI artifacts according to repository classification.
- Do not execute untrusted checkout build scripts merely to build a graph.

## Suggested artifact bundle

```text
compass-out/
├── graph.json
├── GRAPH_REPORT.md
├── manifest.json
├── source-revision.txt
├── compass-version.txt
├── policy-result.json
└── topology-diff.json
```

HTML is optional and can be omitted for size or security.

## Related pages

- [Integrating Compass](../guides/integrating-compass.md)
- [Output reference](../reference/outputs.md)
- [Security and privacy](../design/security-and-privacy.md)

**Next step:** implement the build-only recipe first, inspect its artifacts,
then add one exact policy query with explicit schema validation.
