export const INSPECTOR_MIN_WIDTH = 280;
export const INSPECTOR_MAX_WIDTH = 560;
export const INSPECTOR_COLLAPSED_WIDTH = 48;
export const INSPECTOR_KEYBOARD_STEP = 24;

export type InspectorLayout = {
  width: number;
  collapsed: boolean;
};

export const DEFAULT_INSPECTOR_LAYOUT: InspectorLayout = {
  width: 340,
  collapsed: false
};

export function clampInspectorWidth(width: number): number {
  if (!Number.isFinite(width)) return DEFAULT_INSPECTOR_LAYOUT.width;
  return Math.min(INSPECTOR_MAX_WIDTH, Math.max(INSPECTOR_MIN_WIDTH, Math.round(width)));
}

export function normalizeInspectorLayout(
  value: Partial<InspectorLayout> | undefined
): InspectorLayout {
  return {
    width: clampInspectorWidth(value?.width ?? DEFAULT_INSPECTOR_LAYOUT.width),
    collapsed: value?.collapsed ?? DEFAULT_INSPECTOR_LAYOUT.collapsed
  };
}

export function resizeInspectorFromPointer(containerRight: number, clientX: number): number {
  return clampInspectorWidth(containerRight - clientX);
}

export function resizeInspectorByKeyboard(width: number, key: string): number {
  if (key === "ArrowLeft") {
    return clampInspectorWidth(width + INSPECTOR_KEYBOARD_STEP);
  }
  if (key === "ArrowRight") {
    return clampInspectorWidth(width - INSPECTOR_KEYBOARD_STEP);
  }
  return clampInspectorWidth(width);
}
