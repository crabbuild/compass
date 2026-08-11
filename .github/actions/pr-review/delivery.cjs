const { decodeUtf8, readBounded, readReport, validateIdentity } = require("./report.cjs");

const MAX_COMMENT_BYTES = 60_000;
const MAX_COMMENT_PAGES = 10;

const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

async function retry(operation) {
  let last;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      return await operation();
    } catch (error) {
      last = error;
      const status = Number(error.status || error.response?.status || 0);
      const remaining = error.response?.headers?.["x-ratelimit-remaining"];
      if (!(status === 429 || status >= 500 || (status === 403 && remaining === "0"))) throw error;
      if (attempt < 2) await sleep(25 * (attempt + 1));
    }
  }
  throw last;
}

module.exports = async function deliver({
  github,
  core,
  reportPath,
  markdownPath,
  repository,
  host,
  pullRequestNumber,
  expectedBase,
  expectedHead,
  actionIdentity,
}) {
  const report = readReport(reportPath);
  validateIdentity(report, { repository, host, pullRequestNumber, expectedBase, expectedHead });
  const [owner, repo] = repository.split("/");
  const marker = `<!-- compass-pr-review:v1 repo=${repository} pr=${pullRequestNumber} schema=${report.schema} action=${actionIdentity} -->`;
  const markdown = decodeUtf8(
    readBounded(markdownPath, MAX_COMMENT_BYTES, "comment"),
    "comment Markdown",
  );
  const body = `${marker}\n${markdown}`;
  if (Buffer.byteLength(body, "utf8") > MAX_COMMENT_BYTES) {
    throw new Error(`comment exceeds ${MAX_COMMENT_BYTES} bytes`);
  }
  try {
    const comments = [];
    for (let page = 1; page <= MAX_COMMENT_PAGES; page += 1) {
      const response = await retry(() =>
        github.rest.issues.listComments({ owner, repo, issue_number: pullRequestNumber, per_page: 100, page }),
      );
      comments.push(...response.data);
      if (response.data.length < 100) break;
      if (page === MAX_COMMENT_PAGES) throw new Error("comment pagination exceeds bounded limit");
    }
    const owned = comments.filter((comment) => typeof comment.body === "string" && comment.body.includes(marker));
    if (owned.length === 0) {
      await retry(() => github.rest.issues.createComment({ owner, repo, issue_number: pullRequestNumber, body }));
      core.setOutput("outcome", "created");
      return "created";
    }
    await retry(() => github.rest.issues.updateComment({ owner, repo, comment_id: owned[0].id, body }));
    for (const duplicate of owned.slice(1)) {
      await retry(() => github.rest.issues.deleteComment({ owner, repo, comment_id: duplicate.id }));
    }
    core.setOutput("outcome", "updated");
    return "updated";
  } catch (error) {
    if (
      Number(error.status || error.response?.status || 0) === 403 &&
      error.response?.headers?.["x-ratelimit-remaining"] !== "0"
    ) {
      core.warning("Compass report was published, but comment permission is unavailable.");
      core.setOutput("outcome", "permission-denied");
      return "permission-denied";
    }
    throw error;
  }
};
