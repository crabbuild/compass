import os from "node:os";
import path from "node:path";
import { runTests } from "@vscode/test-electron";

async function main(): Promise<void> {
  const extensionDevelopmentPath = path.resolve(__dirname, "../..");
  const extensionTestsPath = path.resolve(__dirname, "suite/index.cjs");
  const testWorkspace = path.resolve(extensionDevelopmentPath, "src/test/fixture");
  const fakeBin = path.resolve(extensionDevelopmentPath, "src/test/fake-bin");
  const testProfile = path.join(os.tmpdir(), `compass-vscode-${process.pid}`);
  await runTests({
    extensionDevelopmentPath,
    extensionTestsPath,
    launchArgs: [
      testWorkspace,
      "--disable-extensions",
      `--user-data-dir=${path.join(testProfile, "user-data")}`,
      `--extensions-dir=${path.join(testProfile, "extensions")}`
    ],
    extensionTestsEnv: {
      ...process.env,
      PATH: `${fakeBin}${path.delimiter}${process.env.PATH ?? ""}`
    }
  });
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
