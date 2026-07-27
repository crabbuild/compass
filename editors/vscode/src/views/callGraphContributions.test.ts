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
  it("contributes one editor submenu with caller, callee, and combined actions", () => {
    expect(manifest.contributes.commands).toContainEqual(expect.objectContaining({
      command: "compass.openCallGraphGuide",
      title: "Compass: Call Graph Guide"
    }));
    expect(manifest.contributes.submenus).toContainEqual({
      id: "compass.callGraph",
      label: "Compass Call Graph"
    });
    expect(manifest.contributes.menus["editor/context"]).toContainEqual({
      submenu: "compass.callGraph",
      when: "resourceScheme == file",
      group: "navigation@90"
    });
    expect(
      manifest.contributes.menus["compass.callGraph"]?.map((item) => item.command)
    ).toEqual([
      "compass.openCallers",
      "compass.openCallees",
      "compass.openCallersAndCallees"
    ]);
  });
});
