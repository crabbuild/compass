import fs from "node:fs";
import { spawnSync } from "node:child_process";
import reportModule from "./report.cjs";

const readEvent = () => {
  const path = process.env.GITHUB_EVENT_PATH;
  if (!path) return {};
  return JSON.parse(
    reportModule.decodeUtf8(
      reportModule.readBounded(path, reportModule.MAX_REPORT_BYTES, "GitHub event"),
      "GitHub event",
    ),
  );
};

const event = readEvent();
const pull = event.pull_request;
const mergeGroup = event.merge_group;
const base = process.env.COMPASS_ACTION_BASE || pull?.base?.sha || mergeGroup?.base_sha || "";
const head = process.env.COMPASS_ACTION_HEAD || pull?.head?.sha || mergeGroup?.head_sha || "";
const repository = process.env.COMPASS_ACTION_REPOSITORY || process.env.GITHUB_REPOSITORY || "";
const pullRequestNumber = process.env.COMPASS_ACTION_PR || pull?.number?.toString() || "";
const serverUrl = process.env.GITHUB_SERVER_URL || "https://github.com";
const host = new URL(serverUrl).hostname;
const objectId = /^[0-9a-f]{40}([0-9a-f]{24})?$/;

if (!objectId.test(base) || !objectId.test(head)) {
  throw new Error("base and head must resolve to full lowercase Git object IDs from inputs or the event");
}
if (!/^[A-Za-z0-9_.-]{1,255}\/[A-Za-z0-9_.-]{1,255}$/.test(repository)) {
  throw new Error("repository must be OWNER/REPO");
}
if (pullRequestNumber && !/^[1-9][0-9]*$/.test(pullRequestNumber)) {
  throw new Error("pull-request-number must be a positive integer");
}

const git = (args, allowFailure = false) => {
  const result = spawnSync("git", args, {
    encoding: "utf8",
    timeout: 60_000,
    maxBuffer: 16 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.error) throw result.error;
  if (result.status !== 0 && !allowFailure) {
    throw new Error(`git ${args[0]} failed: ${result.stderr.trim()}`);
  }
  return result;
};

git(["rev-parse", "--is-inside-work-tree"]);
const ensureCommit = (oid, fallbackRef) => {
  if (git(["cat-file", "-e", `${oid}^{commit}`], true).status !== 0) {
    const refspec = fallbackRef || oid;
    git(["-c", "protocol.version=2", "fetch", "--no-tags", "--depth=1", "origin", refspec]);
  }
  const resolved = git(["rev-parse", "--verify", `${oid}^{commit}`]).stdout.trim();
  if (resolved !== oid) throw new Error(`fetched object identity drifted: expected ${oid}, got ${resolved}`);
};

ensureCommit(base, base);
ensureCommit(head, pullRequestNumber ? `refs/pull/${pullRequestNumber}/head` : head);

const fork = Boolean(pull?.head?.repo?.fork);
const commentInput = (process.env.COMPASS_ACTION_COMMENT || "true").toLowerCase();
if (!new Set(["true", "false"]).has(commentInput)) {
  throw new Error("comment must be true or false");
}
const wantsComment = commentInput === "true";
const hasToken = process.env.COMPASS_ACTION_HAS_TOKEN === "true";
const allowComment = wantsComment && hasToken && Boolean(pullRequestNumber) && !fork;
let commentReason = "enabled";
if (!wantsComment) commentReason = "disabled";
else if (!hasToken) commentReason = "no-token";
else if (!pullRequestNumber) commentReason = "no-pull-request";
else if (fork) commentReason = "fork-read-only";

const outputs = {
  base,
  head,
  repository,
  host,
  "pull-request-number": pullRequestNumber,
  fork: String(fork),
  "allow-comment": String(allowComment),
  "comment-reason": commentReason,
};
const outputPath = process.env.GITHUB_OUTPUT;
if (!outputPath) throw new Error("GITHUB_OUTPUT is unavailable");
for (const [name, value] of Object.entries(outputs)) {
  fs.appendFileSync(outputPath, `${name}=${value}\n`);
}
