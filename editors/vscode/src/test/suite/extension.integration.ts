import assert from "node:assert/strict";
import * as vscode from "vscode";

suite("Compass extension", () => {
  test("activates and registers every primary workflow", async () => {
    const extension = vscode.extensions.getExtension("crabbuild.compass-vscode");
    assert.ok(extension, "Compass extension is installed in the test host");
    await extension.activate();
    const commands = new Set(await vscode.commands.getCommands(true));
    for (const command of [
      "compass.initialize",
      "compass.update",
      "compass.toggleWatch",
      "compass.openGraph",
      "compass.openCallGraph",
      "compass.openArchitecture",
      "compass.openQuery",
      "compass.openHistory"
    ]) {
      assert.ok(commands.has(command), `${command} is registered`);
    }
  });
});
