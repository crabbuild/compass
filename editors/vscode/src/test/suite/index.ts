import path from "node:path";
import Mocha from "mocha";

export function run(): Promise<void> {
  const mocha = new Mocha({ ui: "tdd", color: true, timeout: 20_000 });
  mocha.addFile(path.resolve(__dirname, "extension.integration.cjs"));
  return new Promise((resolve, reject) => {
    mocha.run((failures) => {
      if (failures > 0) reject(new Error(`${failures} VS Code integration test(s) failed`));
      else resolve();
    });
  });
}
