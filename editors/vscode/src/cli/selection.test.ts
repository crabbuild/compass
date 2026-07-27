import { describe, expect, it } from "vitest";
import type { CompassDiscovery } from "./discovery";
import { compassSelectionItems } from "./selection";

describe("compassSelectionItems", () => {
  it("shows detected versions, paths, and the active installation", () => {
    const discovery: CompassDiscovery = {
      kind: "found",
      executable: "/home/dev/.local/bin/compass",
      version: "0.1.5",
      installations: [
        {
          executable: "/home/dev/.local/bin/compass",
          version: "0.1.5",
          source: "common"
        },
        {
          executable: "/home/dev/.cargo/bin/compass",
          version: "0.2.0-beta.1",
          source: "path"
        }
      ],
      searched: []
    };

    expect(compassSelectionItems(discovery)).toEqual([
      expect.objectContaining({
        label: "$(terminal) Compass 0.1.5",
        description: "Current selection",
        detail: "/home/dev/.local/bin/compass"
      }),
      expect.objectContaining({
        label: "$(terminal) Compass 0.2.0-beta.1",
        description: "Detected on PATH",
        detail: "/home/dev/.cargo/bin/compass"
      }),
      expect.objectContaining({
        label: "$(folder-opened) Browse for another Compass CLI…",
        browse: true
      }),
      expect.objectContaining({
        label: "$(edit) Enter Compass CLI path manually…",
        manual: true
      })
    ]);
  });

  it("offers browsing when no installation was detected", () => {
    const items = compassSelectionItems({
      kind: "missing",
      installations: [],
      searched: []
    });

    expect(items).toHaveLength(2);
    expect(items[0]?.browse).toBe(true);
    expect(items[1]?.manual).toBe(true);
  });
});
