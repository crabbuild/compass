import fs from "node:fs";
import reportModule from "./report.cjs";

const destination = process.env.GITHUB_STEP_SUMMARY;
if (!destination) throw new Error("GITHUB_STEP_SUMMARY is unavailable");
if (process.env.COMPASS_ACTION_OK === "true") {
  const markdown = reportModule.decodeUtf8(
    reportModule.readBounded(process.env.COMPASS_ACTION_MARKDOWN, 60_000, "summary Markdown"),
    "summary Markdown",
  );
  fs.appendFileSync(destination, markdown);
} else {
  const log = fs.existsSync(process.env.COMPASS_ACTION_LOG)
    ? reportModule
        .decodeUtf8(
          reportModule.readBounded(
            process.env.COMPASS_ACTION_LOG,
            reportModule.MAX_REPORT_BYTES,
            "analysis log",
          ),
          "analysis log",
        )
        .slice(0, 4_000)
    : "Compass produced no diagnostic log.";
  fs.appendFileSync(
    destination,
    `## Compass PR review\n\n**Analysis error.** No risk or gate conclusion was produced.\n\n\`\`\`text\n${log.replaceAll("```", "` ` `")}\n\`\`\`\n`,
  );
}
