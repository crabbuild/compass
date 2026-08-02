import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

type Contribution = {
  command?: string;
  submenu?: string;
  when?: string;
};

const manifest = JSON.parse(
  readFileSync(new URL("../../package.json", import.meta.url), "utf8")
) as {
  contributes: {
    commands: Contribution[];
    submenus?: Array<{ id: string; label: string }>;
    menus: Record<string, Contribution[]>;
  };
};

describe("call graph editor contributions", () => {
  it("contributes one Compass editor submenu with cursor-aware graph actions", () => {
    expect(manifest.contributes.commands).toContainEqual(expect.objectContaining({
      command: "compass.openCallGraphGuide",
      title: "Compass: Call Graph Guide"
    }));
    expect(manifest.contributes.submenus).toContainEqual({
      id: "compass.codeGraph",
      label: "Compass"
    });
    expect(manifest.contributes.menus["editor/context"]).toEqual([{
      submenu: "compass.codeGraph",
      when: "resourceScheme == file",
      group: "navigation@90"
    }]);
    expect(
      manifest.contributes.menus["compass.codeGraph"]?.map((item) => item.command)
    ).toEqual([
      "compass.openCallers",
      "compass.openCallees",
      "compass.openCallersAndCallees",
      "compass.showCodeImpact",
      "compass.exploreCode",
      "compass.showNodeTrail"
    ]);
    expect(manifest.contributes.submenus).toHaveLength(1);
  });
});
