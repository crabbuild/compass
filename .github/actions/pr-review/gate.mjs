import fs from "node:fs";
import reportModule from "./report.cjs";

const output = (value) => fs.appendFileSync(process.env.GITHUB_OUTPUT, `outcome=${value}\n`);
if (process.env.COMPASS_ACTION_ANALYSIS_OK !== "true") {
  output("analysis-error");
  throw new Error("Compass analysis failed; see the uploaded diagnostic artifact and job summary");
}
const failOn = process.env.COMPASS_ACTION_FAIL_ON || "none";
if (!new Set(["none", "deterministic"]).has(failOn)) {
  throw new Error("fail-on must be none or deterministic");
}
const report = reportModule.readReport(process.env.COMPASS_ACTION_REPORT);
reportModule.validateIdentity(report, {
  repository: process.env.COMPASS_ACTION_EXPECTED_REPOSITORY,
  host: process.env.COMPASS_ACTION_EXPECTED_HOST,
  pullRequestNumber: Number(process.env.COMPASS_ACTION_EXPECTED_PR || 0),
  expectedBase: process.env.COMPASS_ACTION_EXPECTED_BASE,
  expectedHead: process.env.COMPASS_ACTION_EXPECTED_HEAD,
});
const states = new Set(report.gates.map((gate) => gate.state));
if (states.has("error")) {
  output("analysis-error");
  throw new Error("a deterministic gate returned error");
}
if (failOn === "deterministic" && states.has("fail")) {
  output("deterministic-fail");
  throw new Error("one or more deterministic gates failed");
}
if (states.has("indeterminate")) output("indeterminate");
else if (states.has("fail")) output("advisory");
else output("pass");
