import assert from "node:assert/strict";
import * as vscode from "vscode";

suite("Compass extension", () => {
  test("activates and registers every primary workflow", async () => {
    const extension = vscode.extensions.getExtension(
      "crabbuild.crabbuild-compass-vscode"
    );
    assert.ok(extension, "Compass extension is installed in the test host");
    await extension.activate();
    const commands = new Set(await vscode.commands.getCommands(true));
    for (const command of [
      "compass.initialize",
      "compass.update",
      "compass.refreshWorkspace",
      "compass.toggleWatch",
      "compass.startWatch",
      "compass.stopWatch",
      "compass.openSettings",
      "compass.openGraph",
      "compass.openCallGraphGuide",
      "compass.openCallGraph",
      "compass.openCallers",
      "compass.openCallees",
      "compass.openCallersAndCallees",
      "compass.openArchitecture",
      "compass.openQuery",
      "compass.searchSymbols",
      "compass.showCodeCallers",
      "compass.showCodeCallees",
      "compass.showCodeImpact",
      "compass.exploreCode",
      "compass.showNodeTrail",
      "compass.openHistory",
      "compass.installCli",
      "compass.selectCli"
    ]) {
      assert.ok(commands.has(command), `${command} is registered`);
    }
    const viewTitle = extension.packageJSON.contributes.menus["view/title"] as Array<{
      command: string;
      when: string;
    }>;
    assert.deepEqual(
      extension.packageJSON.contributes.views.compass.map((view: { id: string }) => view.id),
      ["compass.status"],
      "Compass contributes one Workspace view"
    );
    assert.deepEqual(
      viewTitle.filter((item) => item.when === "view == compass.status")
        .map((item) => item.command),
      ["compass.openSettings", "compass.refreshWorkspace"],
      "Workspace exposes settings and refresh title actions"
    );
    await vscode.commands.executeCommand("compass.refreshWorkspace");
  });
});
