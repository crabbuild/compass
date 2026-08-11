const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const deliver = require("./delivery.cjs");
const { stable, validateShape } = require("./report.cjs");

const BASE = "1".repeat(40);
const HEAD = "2".repeat(40);
const RESULT = "3".repeat(40);
const REPOSITORY = "crabbuild/compass";
const PR = 42;

function report(gateState = "pass") {
  const fingerprint = `cmpprv1:${"a".repeat(64)}`;
  const completeness = gateState === "indeterminate" ? "downstream_partial" : "local_exact";
  const findings = gateState === "fail"
    ? [{
        fingerprint,
        finding_type: "contract_change",
        classifier_version: 1,
        statement: "Exact public contract break",
        source_entities: ["symbol:api"],
        target_entities: [],
        witness: [],
        locations: [],
        verification: { state: "covered", exact_tests: ["test_api"], recommended_tests: [], gap: false, reason: "covered" },
        source_revision: RESULT,
        evidence_source: "compass-semantic-diff",
        evidence_digest: `sha256:${"6".repeat(64)}`,
        confidence: "exact",
        completeness: "local_exact",
        freshness: "exact_head",
        remediation: "Update the contract",
        deterministic: true,
      }]
    : [];
  const value = {
    schema: "compass.pr_intelligence.report/1",
    identity: {
      repository: { forge: "github", host: "github.com", owner: "crabbuild", name: "compass" },
      pull_request_number: PR,
      revisions: {
        merge_base: BASE,
        pull_request_head: HEAD,
        target_head: BASE,
        merge_result: { state: "clean", object_id: RESULT },
      },
      graph_schema: "networkx-node-link/v1",
      extractor_version: "extractor/1",
      configuration_digest: "4".repeat(64),
      policy_pack_digest: `sha256:${"5".repeat(64)}`,
      evidence_manifest_digest: `sha256:${"6".repeat(64)}`,
    },
    completeness,
    findings,
    risk_factors: gateState === "fail"
      ? [{ kind: "public_contract_change", points: 20, explanation: "Public contracts changed", finding_fingerprints: [fingerprint] }]
      : gateState === "indeterminate"
        ? [{ kind: "incomplete_evidence", points: 20, explanation: "Evidence is incomplete", finding_fingerprints: [] }]
        : [],
    advisory_risk: {
      rubric_version: 1,
      score: gateState === "pass" ? 0 : 20,
      band: gateState === "pass" ? "low" : "moderate",
      explanation: "advisory",
    },
    gates: [
      {
        id: "proven-contract-break",
        rule_version: 1,
        state: gateState,
        statement: "fixture",
        finding_fingerprints: gateState === "fail" ? [fingerprint] : [],
      },
    ],
    omissions: [],
  };
  value.report_digest = `sha256:${crypto.createHash("sha256").update(stable(value)).digest("hex")}`;
  return value;
}

function fixture(gateState = "pass", markdown = "## Review\n") {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "compass-action-"));
  const reportPath = path.join(directory, "report.json");
  const markdownPath = path.join(directory, "report.md");
  fs.writeFileSync(reportPath, JSON.stringify(report(gateState)));
  fs.writeFileSync(markdownPath, markdown);
  return { directory, reportPath, markdownPath };
}

function core() {
  return {
    outputs: new Map(),
    warnings: [],
    setOutput(name, value) {
      this.outputs.set(name, value);
    },
    warning(value) {
      this.warnings.push(value);
    },
  };
}

function argumentsFor(github, files, actionCore = core()) {
  return {
    github,
    core: actionCore,
    reportPath: files.reportPath,
    markdownPath: files.markdownPath,
    repository: REPOSITORY,
    host: "github.com",
    pullRequestNumber: PR,
    expectedBase: BASE,
    expectedHead: HEAD,
    actionIdentity: "crabbuild/compass/pr-review@1",
  };
}

test("creates one marker-owned comment", async () => {
  const files = fixture();
  const calls = [];
  const github = {
    rest: {
      issues: {
        listComments: async () => ({ data: [] }),
        createComment: async (input) => calls.push(["create", input]),
        updateComment: async () => assert.fail("unexpected update"),
        deleteComment: async () => assert.fail("unexpected delete"),
      },
    },
  };
  assert.equal(await deliver(argumentsFor(github, files)), "created");
  assert.equal(calls.length, 1);
  assert.match(calls[0][1].body, /compass-pr-review:v1/);
});

test("updates one comment and removes duplicate owned markers", async () => {
  const files = fixture();
  const marker = "<!-- compass-pr-review:v1 repo=crabbuild/compass pr=42 schema=compass.pr_intelligence.report/1 action=crabbuild/compass/pr-review@1 -->";
  const updates = [];
  const deletes = [];
  const github = {
    rest: {
      issues: {
        listComments: async () => ({ data: [{ id: 10, body: marker }, { id: 11, body: marker }] }),
        createComment: async () => assert.fail("unexpected create"),
        updateComment: async (input) => updates.push(input.comment_id),
        deleteComment: async (input) => deletes.push(input.comment_id),
      },
    },
  };
  assert.equal(await deliver(argumentsFor(github, files)), "updated");
  assert.deepEqual(updates, [10]);
  assert.deepEqual(deletes, [11]);
});

test("paginates comments within the hard page bound before updating", async () => {
  const files = fixture();
  const marker = "<!-- compass-pr-review:v1 repo=crabbuild/compass pr=42 schema=compass.pr_intelligence.report/1 action=crabbuild/compass/pr-review@1 -->";
  const pages = [];
  const updates = [];
  const github = {
    rest: {
      issues: {
        listComments: async ({ page }) => {
          pages.push(page);
          return {
            data: page === 1
              ? Array.from({ length: 100 }, (_, id) => ({ id, body: "unowned" }))
              : [{ id: 101, body: marker }],
          };
        },
        createComment: async () => assert.fail("unexpected create"),
        updateComment: async ({ comment_id }) => updates.push(comment_id),
        deleteComment: async () => assert.fail("unexpected delete"),
      },
    },
  };
  assert.equal(await deliver(argumentsFor(github, files)), "updated");
  assert.deepEqual(pages, [1, 2]);
  assert.deepEqual(updates, [101]);
});

test("permission denial is non-fatal after report publication", async () => {
  const files = fixture();
  const denied = Object.assign(new Error("denied"), { status: 403, response: { headers: {} } });
  const github = {
    rest: { issues: { listComments: async () => { throw denied; } } },
  };
  const actionCore = core();
  assert.equal(await deliver(argumentsFor(github, files, actionCore)), "permission-denied");
  assert.equal(actionCore.outputs.get("outcome"), "permission-denied");
});

test("rate limits retry and oversized or stale reports fail closed", async () => {
  const files = fixture();
  let attempts = 0;
  const github = {
    rest: {
      issues: {
        listComments: async () => {
          attempts += 1;
          if (attempts === 1) throw Object.assign(new Error("rate"), { status: 429 });
          return { data: [] };
        },
        createComment: async () => {},
      },
    },
  };
  assert.equal(await deliver(argumentsFor(github, files)), "created");
  assert.equal(attempts, 2);

  let exhaustedAttempts = 0;
  const exhausted = {
    rest: {
      issues: {
        listComments: async () => {
          exhaustedAttempts += 1;
          throw Object.assign(new Error("rate exhausted"), {
            status: 403,
            response: { headers: { "x-ratelimit-remaining": "0" } },
          });
        },
      },
    },
  };
  await assert.rejects(deliver(argumentsFor(exhausted, files)), /rate exhausted/);
  assert.equal(exhaustedAttempts, 3);

  const oversized = fixture("pass", "x".repeat(60_001));
  await assert.rejects(deliver(argumentsFor(github, oversized)), /comment exceeds/);
  await assert.rejects(
    deliver({ ...argumentsFor(github, files), expectedHead: "9".repeat(40) }),
    /stale report revision/,
  );
  await assert.rejects(
    deliver({ ...argumentsFor(github, files), host: "github.example.com" }),
    /stale report repository/,
  );
});

test("report adapter rejects nested schema and semantic contradictions", () => {
  const nested = report("fail");
  nested.findings[0].verification.unexpected = true;
  assert.throws(() => validateShape(nested), /unknown field/);

  const contradictory = report("fail");
  contradictory.advisory_risk = { rubric_version: 1, score: 0, band: "low", explanation: "wrong" };
  assert.throws(() => validateShape(contradictory), /contradicts risk factors/);
});

test("analysis-error summary uses a bounded diagnostic excerpt", () => {
  const files = fixture();
  const log = path.join(files.directory, "analysis.log");
  const summary = path.join(files.directory, "summary.md");
  fs.writeFileSync(log, "x".repeat(5_000));
  const result = spawnSync(process.execPath, [path.join(__dirname, "summary.mjs")], {
    encoding: "utf8",
    env: {
      ...process.env,
      GITHUB_STEP_SUMMARY: summary,
      COMPASS_ACTION_OK: "false",
      COMPASS_ACTION_MARKDOWN: files.markdownPath,
      COMPASS_ACTION_LOG: log,
    },
  });
  assert.equal(result.status, 0, result.stderr);
  const contents = fs.readFileSync(summary, "utf8");
  assert.match(contents, /Analysis error/);
  assert.ok(contents.includes("x".repeat(4_000)));
  assert.ok(!contents.includes("x".repeat(4_001)));
});

function runGate(files, failOn, analysisOk = true, overrides = {}) {
  const output = path.join(files.directory, `output-${failOn}-${analysisOk}`);
  const result = spawnSync(process.execPath, [path.join(__dirname, "gate.mjs")], {
    encoding: "utf8",
    env: {
      ...process.env,
      GITHUB_OUTPUT: output,
      COMPASS_ACTION_ANALYSIS_OK: String(analysisOk),
      COMPASS_ACTION_REPORT: files.reportPath,
      COMPASS_ACTION_FAIL_ON: failOn,
      COMPASS_ACTION_EXPECTED_REPOSITORY: REPOSITORY,
      COMPASS_ACTION_EXPECTED_HOST: "github.com",
      COMPASS_ACTION_EXPECTED_PR: String(PR),
      COMPASS_ACTION_EXPECTED_BASE: BASE,
      COMPASS_ACTION_EXPECTED_HEAD: HEAD,
      ...overrides,
    },
  });
  return { result, output: fs.readFileSync(output, "utf8") };
}

test("gate policy never converts advisory risk into a deterministic failure", () => {
  const failedGate = fixture("fail");
  const advisory = runGate(failedGate, "none");
  assert.equal(advisory.result.status, 0);
  assert.match(advisory.output, /outcome=advisory/);
  const deterministic = runGate(failedGate, "deterministic");
  assert.notEqual(deterministic.result.status, 0);
  assert.match(deterministic.output, /outcome=deterministic-fail/);

  const indeterminate = runGate(fixture("indeterminate"), "deterministic");
  assert.equal(indeterminate.result.status, 0);
  assert.match(indeterminate.output, /outcome=indeterminate/);

  const analysisError = runGate(failedGate, "none", false);
  assert.notEqual(analysisError.result.status, 0);
  assert.match(analysisError.output, /outcome=analysis-error/);
});

test("fork context suppresses comment delivery before any token-bearing step", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "compass-prepare-"));
  const run = (args) => {
    const result = spawnSync("git", ["-C", directory, ...args], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  run(["init", "--quiet"]);
  run(["config", "user.name", "Compass Test"]);
  run(["config", "user.email", "compass@example.invalid"]);
  fs.writeFileSync(path.join(directory, "a"), "a\n");
  run(["add", "a"]);
  run(["commit", "-m", "base", "--quiet"]);
  const base = run(["rev-parse", "HEAD"]);
  fs.writeFileSync(path.join(directory, "a"), "b\n");
  run(["commit", "-am", "head", "--quiet"]);
  const head = run(["rev-parse", "HEAD"]);
  const eventPath = path.join(directory, "event.json");
  const outputPath = path.join(directory, "output");
  fs.writeFileSync(
    eventPath,
    JSON.stringify({
      pull_request: {
        number: PR,
        base: { sha: base },
        head: { sha: head, repo: { fork: true } },
      },
    }),
  );
  const result = spawnSync(process.execPath, [path.join(__dirname, "prepare.mjs")], {
    cwd: directory,
    encoding: "utf8",
    env: {
      ...process.env,
      GITHUB_EVENT_PATH: eventPath,
      GITHUB_OUTPUT: outputPath,
      GITHUB_REPOSITORY: REPOSITORY,
      COMPASS_ACTION_COMMENT: "true",
      COMPASS_ACTION_HAS_TOKEN: "true",
    },
  });
  assert.equal(result.status, 0, result.stderr);
  const output = fs.readFileSync(outputPath, "utf8");
  assert.match(output, /allow-comment=false/);
  assert.match(output, /comment-reason=fork-read-only/);
  assert.match(output, /host=github\.com/);
});

test("installer pins release identity, checksum, and archive layout", () => {
  const installer = fs.readFileSync(path.join(__dirname, "install.sh"), "utf8");
  assert.match(installer, /releases\/download\/compass-v\$version/);
  assert.match(installer, /sha256sum -c|shasum -a 256 -c/);
  assert.match(installer, /release archive has an unsafe or unexpected layout/);
  assert.match(installer, /test ! -L/);
  assert.match(installer, /-type l/);
  assert.ok(installer.includes("$0 ~ /(^|\\/)\\.\\.(\\/|$)/"));
  assert.doesNotMatch(installer, /releases\/latest/);
});
