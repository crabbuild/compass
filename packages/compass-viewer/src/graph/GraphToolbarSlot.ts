import { createContext, useContext } from "react";

/**
 * Element a host wants the graph control rail rendered into.
 *
 * The rail floats over the canvas by default, which suits a webview that is
 * only a graph. A host that owns a header row — the workbench, with its view
 * title and coverage — mounts the rail there instead, so the controls share the
 * top area and the canvas keeps the whole stage.
 */
export const GraphToolbarSlotContext = createContext<HTMLElement | null>(null);

export function useGraphToolbarSlot(): HTMLElement | null {
  return useContext(GraphToolbarSlotContext);
}
