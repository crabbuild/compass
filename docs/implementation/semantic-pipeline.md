# Semantic pipeline implementation

The semantic pipeline turns untrusted non-code content and provider responses
into validated graph facts. It is optional and explicitly separate from the
fully local structural code path.

> **Who this page is for:** contributors adding providers, media formats,
> semantic relations, validation, or completeness behavior.
>
> **You will learn:** source classification, extraction/chunking, backend
> selection, request safety, response validation, evidence binding, partial
> results, caching, and test requirements.
>
> **Prerequisites:** [Extraction pipeline](extraction-pipeline.md) and
> [Security and privacy](../design/security-and-privacy.md).
>
> **Reading time:** 12–15 minutes.

## Boundary

```text
code files ----------------> deterministic structural extraction

docs/media/integration text -> semantic orchestration -> validated fragments
                                                      |
                                                      v
                                               shared graph merge
```

The semantic path must not make code-only builds depend on credentials or
network availability.

## Source preparation

Input can arrive as:

- plain text/Markdown;
- PDF;
- DOCX/XLSX;
- images and image references;
- decoded audio/video transcript;
- Google Workspace export;
- remote ingested corpus file.

`compass-media` enforces raw and archive-expansion limits before extracting
text. `compass-transcribe` owns bounded audio/video orchestration.

The semantic layer represents prepared content as bounded units with source
identity and evidence metadata.

## Chunk packing

Large inputs are sliced under character/token/image budgets:

- per-file character caps;
- bounded source slices;
- image byte and per-chunk counts;
- estimated token cost;
- deterministic source/evidence ordering.

Chunking should keep enough context for coherent extraction without sending an
entire repository blindly.

Changes to chunk boundaries can change semantic meaning and cache identity.

## Prompt construction

Compass embeds extraction and deep-mode prompt templates.

Prompt construction:

- identifies the requested structured fragment format;
- separates instructions from untrusted corpus text;
- neutralizes injection sentinels;
- includes source/evidence identity;
- sets mode-specific requirements;
- remains bounded.

Repository content is data, not trusted provider instruction.

## Backend selection

Built-in backends include provider families such as:

- Anthropic/Claude;
- Gemini;
- OpenAI and compatible endpoints;
- Azure OpenAI;
- Ollama-compatible endpoints;
- Bedrock;
- other compatibility backends represented in current CLI/source.

A custom provider registry records:

- provider name;
- base URL;
- default model;
- environment-variable name for the key.

Selection combines explicit CLI configuration and documented environment
fallbacks. History captures provider/model meaning in the extraction profile.

## Endpoint safety

Before a request:

- parse and validate the base URL;
- require an expected scheme;
- reject link-local/metadata addresses where required;
- warn when an allegedly local endpoint is non-loopback;
- apply timeout and response-byte bounds;
- keep keys out of diagnostics;
- avoid uncontrolled redirects or shell execution.

An OpenAI-compatible endpoint can receive the full selected corpus. The
compatibility label does not make it trustworthy.

## Request/response normalization

Provider APIs have different envelopes. Backend helpers normalize:

- content/messages;
- image forms;
- model and temperature rules;
- authentication headers;
- completion text;
- provider errors;
- context-limit signals;
- retryable versus permanent failures.

Normalization should preserve enough provider context for diagnostics without
including secret headers or entire sensitive responses.

## Adaptive retry

When a response indicates context overflow, the pipeline can split or reduce
work and retry under bounded policy.

It must avoid:

- infinite retries;
- retrying permanent authentication/usage errors;
- publishing only the chunks that happened to succeed unless partial policy is
  explicit;
- multiplying concurrency after a limit failure.

Callers configure total concurrency and API timeout.

## Parse untrusted fragments

Provider output is untrusted JSON-like content. The semantic crate enforces
limits such as:

- maximum fragment bytes;
- maximum nodes;
- maximum edges;
- maximum hyperedges;
- maximum nodes per hyperedge;
- maximum ID length;
- maximum provider response bytes.

Parsing/validation checks:

- JSON structure and depth;
- record shapes;
- endpoint identity;
- attribute types and sizes;
- safe IDs;
- relation/evidence fields;
- duplicate and malformed records;
- prompt-injection residue;
- fragment completeness.

Malformed output is a provider/extraction failure, not graph data to repair
silently.

## Evidence binding

Validated nodes and edges are associated with their source evidence. IDs are
normalized and source references retained so users can trace semantic facts
back to a document or media segment.

Provenance must distinguish semantic results from deterministic structural
resolution in the attributes available to consumers.

## Partial builds

The pipeline tracks which semantic sources are partial or missing.

Default:

```text
one required semantic source fails
  -> build is incomplete
  -> no complete semantic publication
```

Explicit partial policy:

```text
--allow-partial
  -> successful fragments may publish
  -> partial source metadata must remain visible
```

An integration must not present partial coverage as a complete repository
graph.

## Cache

Semantic cache identity can include:

- source hash/slice;
- prompt fingerprint;
- mode;
- backend/model;
- relevant parser/semantic version;
- image/evidence inputs.

It excludes secret key values.

A cache hit must undergo compatibility/integrity checks. Old unvalidated
provider content should not bypass new validation solely because it exists.

## Community labeling and deduplication

Semantic providers can optionally help label communities or break genuine
entity-deduplication ties. These are separate explicit roles:

- community labeling changes display/navigation metadata;
- semantic extraction adds content facts;
- a tiebreaker resolves a bounded ambiguity in graph construction.

Do not let a labeling call rewrite structural topology.

## Native media

`compass-whisper` provides bounded CPU inference internals. Device selection
currently exposes CPU only so released behavior remains portable and
dependency-free.

`compass-transcribe` keeps model inference behind a trait and owns file/download
orchestration. No separate public transcription command is exposed merely
because internals exist.

## Tests for provider work

Use local mock servers. Tests should cover:

- exact request path, method, headers with secrets redacted in assertions;
- response normalization;
- non-2xx status;
- malformed JSON;
- oversized body;
- timeout;
- context-overflow adaptive retry;
- authentication failure;
- endpoint scheme/host/link-local checks;
- model/config precedence;
- partial/completeness behavior;
- stable evidence IDs;
- cache hit/miss/invalidation;
- no provider call in code-only mode.

Never require a real paid provider or real secret for the normal test suite.

## Tests for new semantic records

Assert:

- node and edge counts;
- stable IDs;
- source evidence;
- direction and relation;
- provenance;
- hyperedge participants;
- size/depth rejection;
- merge with structural facts;
- round trip into `graph.json` and history artifacts.

## Related pages

- [Security and privacy](../design/security-and-privacy.md)
- [Configuration reference](../reference/configuration.md)
- [Extraction pipeline](extraction-pipeline.md)
- [Provenance](../concepts/provenance.md)

**Next step:** trace one semantic fixture from a bounded source unit through
provider normalization, fragment validation, evidence binding, and graph merge.
