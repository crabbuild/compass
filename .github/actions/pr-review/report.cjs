const crypto = require("node:crypto");
const fs = require("node:fs");
const { TextDecoder } = require("node:util");

const MAX_REPORT_BYTES = 16 * 1024 * 1024;
const DIGEST = /^sha256:[0-9a-f]{64}$/;
const PROFILE_DIGEST = /^[0-9a-f]{64}$/;
const OBJECT_ID = /^[0-9a-f]{40}([0-9a-f]{24})?$/;
const FINGERPRINT = /^cmpprv1:[0-9a-f]{64}$/;
const COMPLETENESS = new Set(["local_exact", "downstream_complete", "downstream_partial", "downstream_unavailable"]);
const CONFIDENCE = new Set(["exact", "probable", "inferred", "unknown"]);
const FRESHNESS = new Set(["exact_head", "stale", "unknown"]);
const FINDING_TYPES = new Set(["architecture_delta", "contract_change", "impact", "verification_gap", "dependency_change", "structural_change"]);
const VERIFICATION_STATES = new Set(["unknown", "covered", "gap", "partial", "stale", "failing", "not_run"]);
const FACTOR_KINDS = ["public_contract_change", "affected_consumer", "cross_boundary_impact", "cycle", "weak_confidence_witness", "verification_gap", "incomplete_evidence", "merge_conflict"];
const FACTOR_RUBRIC = [[20, 40], [4, 24], [10, 20], [20, 20], [4, 16], [12, 36], [20, 20], [30, 30]];

function compareUtf8(left, right) {
  return Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8"));
}

function object(value, name) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${name} must be an object`);
  }
  return value;
}

function exactKeys(value, required, optional, name) {
  const allowed = new Set([...required, ...optional]);
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) throw new Error(`${name} has unknown field ${key}`);
  }
  for (const key of required) {
    if (!Object.hasOwn(value, key)) throw new Error(`${name} is missing ${key}`);
  }
}

function boundedString(value, name, allowEmpty = false) {
  if (typeof value !== "string" || (!allowEmpty && value.length === 0) || value.length > 16 * 1024 || /[\u0000-\u001f\u007f-\u009f]/u.test(value)) {
    throw new Error(`${name} must be a bounded non-empty string`);
  }
}

function nonempty(value, name) {
  boundedString(value, name, false);
}

function strictStrings(values, name, allowEmpty = false) {
  if (!Array.isArray(values)) throw new Error(`${name} must be an array`);
  let previous;
  for (const value of values) {
    boundedString(value, name, allowEmpty);
    if (previous !== undefined && compareUtf8(value, previous) <= 0) throw new Error(`${name} must be strictly ordered`);
    previous = value;
  }
}

function validateShape(report) {
  object(report, "report");
  exactKeys(
    report,
    ["schema", "identity", "completeness", "findings", "risk_factors", "advisory_risk", "gates", "omissions", "report_digest"],
    [],
    "report",
  );
  const identity = object(report.identity, "report identity");
  exactKeys(
    identity,
    ["repository", "revisions", "graph_schema", "extractor_version", "configuration_digest", "policy_pack_digest", "evidence_manifest_digest"],
    ["pull_request_number"],
    "report identity",
  );
  const repository = object(identity.repository, "repository identity");
  exactKeys(repository, ["forge", "host", "owner", "name"], [], "repository identity");
  for (const key of ["forge", "host", "owner", "name"]) nonempty(repository[key], `repository ${key}`);
  if (identity.pull_request_number !== undefined && (!Number.isSafeInteger(identity.pull_request_number) || identity.pull_request_number <= 0)) {
    throw new Error("pull-request identity must be a positive integer");
  }
  const revisions = object(identity.revisions, "revision identity");
  exactKeys(revisions, ["merge_base", "pull_request_head", "target_head", "merge_result"], [], "revision identity");
  for (const key of ["merge_base", "pull_request_head", "target_head"]) {
    if (!OBJECT_ID.test(revisions[key])) throw new Error(`${key} is not a full object ID`);
  }
  const merge = object(revisions.merge_result, "merge result");
  if (merge.state === "clean") {
    exactKeys(merge, ["state", "object_id"], [], "clean merge result");
    if (!OBJECT_ID.test(merge.object_id)) throw new Error("merge result is not a full object ID");
  } else if (merge.state === "conflicted") {
    exactKeys(merge, ["state", "evidence_digest"], [], "conflicted merge result");
    if (!DIGEST.test(merge.evidence_digest)) throw new Error("conflict evidence digest is invalid");
  } else if (merge.state === "unavailable") {
    exactKeys(merge, ["state", "reason"], [], "unavailable merge result");
    nonempty(merge.reason, "merge unavailable reason");
  } else {
    throw new Error("merge result state is invalid");
  }
  for (const key of ["graph_schema", "extractor_version"]) {
    nonempty(identity[key], key);
  }
  if (!PROFILE_DIGEST.test(identity.configuration_digest)) throw new Error("configuration digest is invalid");
  if (!DIGEST.test(identity.policy_pack_digest)) throw new Error("policy pack digest is invalid");
  if (!DIGEST.test(identity.evidence_manifest_digest)) throw new Error("evidence manifest digest is invalid");
  if (!COMPLETENESS.has(report.completeness)) {
    throw new Error("report completeness is invalid");
  }
  if (!Array.isArray(report.findings) || !Array.isArray(report.risk_factors) || !Array.isArray(report.gates) || !Array.isArray(report.omissions)) {
    throw new Error("report collections must be arrays");
  }
  let previous = "";
  for (const finding of report.findings) {
    object(finding, "finding");
    exactKeys(
      finding,
      ["fingerprint", "finding_type", "classifier_version", "statement", "source_entities", "target_entities", "witness", "locations", "verification", "source_revision", "evidence_source", "evidence_digest", "confidence", "completeness", "freshness", "remediation", "deterministic"],
      [],
      "finding",
    );
    if (!FINGERPRINT.test(finding.fingerprint) || finding.fingerprint <= previous) throw new Error("finding fingerprints are invalid or unordered");
    previous = finding.fingerprint;
    if (!Number.isSafeInteger(finding.classifier_version) || finding.classifier_version <= 0) throw new Error("finding classifier version is invalid");
    nonempty(finding.statement, "finding statement");
    strictStrings(finding.source_entities, "finding source entities");
    strictStrings(finding.target_entities, "finding target entities");
    if (finding.source_entities.length === 0 || !Array.isArray(finding.witness) || !Array.isArray(finding.locations)) {
      throw new Error("finding evidence collections are invalid");
    }
    if (!FINDING_TYPES.has(finding.finding_type) || !CONFIDENCE.has(finding.confidence) || !COMPLETENESS.has(finding.completeness) || !FRESHNESS.has(finding.freshness)) {
      throw new Error("finding typed state is invalid");
    }
    let priorTarget;
    for (const hop of finding.witness) {
      object(hop, "witness hop");
      exactKeys(hop, ["source", "relation", "target", "confidence"], [], "witness hop");
      for (const key of ["source", "relation", "target"]) nonempty(hop[key], `witness ${key}`);
      if (!CONFIDENCE.has(hop.confidence) || (priorTarget !== undefined && hop.source !== priorTarget)) throw new Error("witness path is invalid");
      priorTarget = hop.target;
    }
    let previousLocation;
    for (const location of finding.locations) {
      object(location, "finding location");
      exactKeys(location, ["path"], ["start_byte", "end_byte"], "finding location");
      nonempty(location.path, "finding location path");
      for (const key of ["start_byte", "end_byte"]) {
        if (location[key] !== undefined && (!Number.isSafeInteger(location[key]) || location[key] < 0)) throw new Error("finding location offset is invalid");
      }
      if (location.start_byte !== undefined && location.end_byte !== undefined && location.end_byte < location.start_byte) throw new Error("finding location range is invalid");
      if (previousLocation !== undefined) {
        const pathOrder = compareUtf8(location.path, previousLocation.path);
        const optionalOrder = (left, right) => left === undefined ? (right === undefined ? 0 : -1) : right === undefined ? 1 : left - right;
        const order = pathOrder || optionalOrder(location.start_byte, previousLocation.start_byte) || optionalOrder(location.end_byte, previousLocation.end_byte);
        if (order <= 0) throw new Error("finding locations are not strictly ordered");
      }
      previousLocation = location;
    }
    if (!OBJECT_ID.test(finding.source_revision) || !DIGEST.test(finding.evidence_digest)) throw new Error("finding evidence identity is invalid");
    nonempty(finding.evidence_source, "finding evidence source");
    boundedString(finding.remediation, "finding remediation", true);
    if (typeof finding.deterministic !== "boolean") throw new Error("finding deterministic flag is invalid");
    const verification = object(finding.verification, "finding verification");
    exactKeys(verification, ["state", "exact_tests", "recommended_tests", "gap", "reason"], [], "finding verification");
    if (!VERIFICATION_STATES.has(verification.state) || typeof verification.gap !== "boolean") throw new Error("finding verification state is invalid");
    strictStrings(verification.exact_tests, "exact tests");
    strictStrings(verification.recommended_tests, "recommended tests");
    nonempty(verification.reason, "verification reason");
    const expectedGap = new Set(["gap", "partial", "failing", "not_run"]).has(verification.state);
    if (verification.gap !== expectedGap) throw new Error("verification gap contradicts verification state");
    if (finding.completeness !== report.completeness || finding.evidence_digest !== identity.evidence_manifest_digest) throw new Error("finding evidence contradicts report identity");
    const expectedRevision = merge.state === "clean" ? merge.object_id : revisions.pull_request_head;
    if (finding.source_revision !== expectedRevision) throw new Error("finding source revision contradicts report identity");
    if (finding.deterministic && (finding.confidence !== "exact" || finding.freshness !== "exact_head" || new Set(["downstream_partial", "downstream_unavailable"]).has(finding.completeness) || merge.state !== "clean")) {
      throw new Error("deterministic finding lacks exact complete merge evidence");
    }
  }
  const knownFingerprints = new Set(report.findings.map((finding) => finding.fingerprint));
  let previousFactor = -1;
  for (const factor of report.risk_factors) {
    object(factor, "risk factor");
    exactKeys(factor, ["kind", "points", "explanation", "finding_fingerprints"], [], "risk factor");
    const factorIndex = FACTOR_KINDS.indexOf(factor.kind);
    if (factorIndex < 0 || factorIndex <= previousFactor) throw new Error("risk factor kinds are invalid or unordered");
    previousFactor = factorIndex;
    nonempty(factor.explanation, "risk factor explanation");
    if (!Number.isSafeInteger(factor.points) || factor.points < 0 || factor.points > 100 || !Array.isArray(factor.finding_fingerprints)) {
      throw new Error("risk factor contract is invalid");
    }
    strictStrings(factor.finding_fingerprints, "risk factor finding fingerprints");
    if (factor.finding_fingerprints.some((fingerprint) => !knownFingerprints.has(fingerprint))) throw new Error("risk factor references an unknown finding");
    if (factor.finding_fingerprints.length === 0 && !new Set(["incomplete_evidence", "merge_conflict"]).has(factor.kind)) throw new Error("risk factor has no finding evidence");
    const [points, cap] = FACTOR_RUBRIC[factorIndex];
    if (factor.points !== Math.min(cap, points * Math.max(1, factor.finding_fingerprints.length))) throw new Error("risk factor points contradict rubric version 1");
  }
  let previousGate;
  for (const gate of report.gates) {
    object(gate, "gate");
    exactKeys(gate, ["id", "rule_version", "state", "statement", "finding_fingerprints"], [], "gate");
    nonempty(gate.id, "gate id");
    if (previousGate !== undefined && gate.id <= previousGate) throw new Error("gate IDs are not strictly ordered");
    previousGate = gate.id;
    nonempty(gate.statement, "gate statement");
    if (!Number.isSafeInteger(gate.rule_version) || gate.rule_version <= 0 || !new Set(["pass", "fail", "indeterminate", "error"]).has(gate.state) || !Array.isArray(gate.finding_fingerprints)) {
      throw new Error("gate contract is invalid");
    }
    strictStrings(gate.finding_fingerprints, "gate finding fingerprints");
    if (gate.finding_fingerprints.some((fingerprint) => !knownFingerprints.has(fingerprint))) throw new Error("gate references an unknown finding");
    if (gate.state === "fail" && gate.finding_fingerprints.length === 0) throw new Error("failing gate has no finding evidence");
  }
  object(report.advisory_risk, "advisory risk");
  exactKeys(report.advisory_risk, ["rubric_version", "score", "band", "explanation"], [], "advisory risk");
  if (!Number.isSafeInteger(report.advisory_risk.rubric_version) || report.advisory_risk.rubric_version <= 0) throw new Error("advisory rubric version is invalid");
  nonempty(report.advisory_risk.explanation, "advisory risk explanation");
  const bands = new Set(["low", "moderate", "high", "critical", "unavailable"]);
  const score = report.advisory_risk.score;
  if (!bands.has(report.advisory_risk.band) || (score !== null && (!Number.isSafeInteger(score) || score < 0 || score > 100)) || (report.advisory_risk.band === "unavailable") !== (score === null)) {
    throw new Error("advisory risk state is invalid");
  }
  if (report.advisory_risk.rubric_version !== 1) throw new Error("unsupported advisory rubric version");
  const expectedScore = merge.state === "clean"
    ? Math.min(100, report.risk_factors.reduce((total, factor) => total + factor.points, 0))
    : null;
  const expectedBand = expectedScore === null ? "unavailable" : expectedScore <= 19 ? "low" : expectedScore <= 44 ? "moderate" : expectedScore <= 69 ? "high" : "critical";
  if (score !== expectedScore || report.advisory_risk.band !== expectedBand) throw new Error("advisory risk contradicts risk factors or merge state");
  const initialGate = report.gates.find((gate) => gate.id === "proven-contract-break");
  if (!initialGate || initialGate.rule_version !== 1) throw new Error("initial deterministic gate is missing or unsupported");
  const provenBreaks = report.findings.filter((finding) => finding.finding_type === "contract_change" && finding.deterministic).map((finding) => finding.fingerprint);
  const expectedGate = merge.state !== "clean" || new Set(["downstream_partial", "downstream_unavailable"]).has(report.completeness)
    ? "indeterminate"
    : provenBreaks.length === 0 ? "pass" : "fail";
  if (initialGate.state !== expectedGate || stable(initialGate.finding_fingerprints) !== stable(provenBreaks)) throw new Error("initial deterministic gate contradicts findings");
  const omissionCategories = new Set();
  for (const omission of report.omissions) {
    object(omission, "omission");
    exactKeys(omission, ["category", "count", "reason"], [], "omission");
    nonempty(omission.category, "omission category");
    nonempty(omission.reason, "omission reason");
    if (!Number.isSafeInteger(omission.count) || omission.count <= 0 || omissionCategories.has(omission.category)) throw new Error("omission contract is invalid");
    omissionCategories.add(omission.category);
  }
  if (!DIGEST.test(report.report_digest)) throw new Error("report digest is invalid");
}

function stable(value) {
  if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stable(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function readBounded(path, maximum, name) {
  const descriptor = fs.openSync(path, "r");
  try {
    const metadata = fs.fstatSync(descriptor);
    if (!metadata.isFile()) throw new Error(`${name} is not a regular file`);
    if (metadata.size > maximum) throw new Error(`${name} exceeds ${maximum} bytes`);
    const bytes = Buffer.alloc(maximum + 1);
    let length = 0;
    while (length < bytes.length) {
      const count = fs.readSync(descriptor, bytes, length, bytes.length - length, null);
      if (count === 0) break;
      length += count;
    }
    if (length > maximum) throw new Error(`${name} exceeds ${maximum} bytes`);
    return bytes.subarray(0, length);
  } finally {
    fs.closeSync(descriptor);
  }
}

function decodeUtf8(bytes, name) {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new Error(`${name} is not valid UTF-8`);
  }
}

function readReport(path) {
  const bytes = readBounded(path, MAX_REPORT_BYTES, "report");
  const report = JSON.parse(decodeUtf8(bytes, "report"));
  if (report.schema !== "compass.pr_intelligence.report/1") {
    throw new Error(`unsupported report schema ${JSON.stringify(report.schema)}`);
  }
  validateShape(report);
  const unsigned = { ...report };
  delete unsigned.report_digest;
  const digest = `sha256:${crypto.createHash("sha256").update(stable(unsigned)).digest("hex")}`;
  if (digest !== report.report_digest) throw new Error("report digest mismatch");
  return report;
}

function validateIdentity(report, { repository, host, pullRequestNumber, expectedBase, expectedHead }) {
  if (!/^[A-Za-z0-9_.-]{1,255}\/[A-Za-z0-9_.-]{1,255}$/.test(repository)) {
    throw new Error("expected repository identity is invalid");
  }
  if (typeof host !== "string" || host.length === 0 || host.length > 253 || host.includes("/") || host.includes(":")) throw new Error("expected repository host is invalid");
  const identity = report.identity;
  const [owner, name] = repository.split("/");
  if (identity.repository.forge !== "github" || identity.repository.host !== host || identity.repository.owner !== owner || identity.repository.name !== name) throw new Error("stale report repository identity");
  const expectedPullRequest = pullRequestNumber || undefined;
  if (identity.pull_request_number !== expectedPullRequest) {
    throw new Error("stale report pull-request identity");
  }
  if (identity.revisions.target_head !== expectedBase || identity.revisions.pull_request_head !== expectedHead) {
    throw new Error("stale report revision identity");
  }
}

module.exports = { MAX_REPORT_BYTES, decodeUtf8, readBounded, readReport, stable, validateIdentity, validateShape };
