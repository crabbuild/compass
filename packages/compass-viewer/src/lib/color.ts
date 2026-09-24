/**
 * Small, dependency-free color math shared by the canvas and the SVG variants.
 * Everything here is per-pixel arithmetic on a color the viewer already owns;
 * no color is ever inferred for data the model did not provide.
 */

/** Parse `#rgb`, `#rrggbb`, or `rgb()/rgba()` into channels; otherwise `undefined`. */
export function parseColor(color: string): [number, number, number] | undefined {
  const hex = color.match(/^#([\da-f]{3}|[\da-f]{6})$/i)?.[1];
  if (hex) {
    const normalized = hex.length === 3
      ? [...hex].map((value) => `${value}${value}`).join("")
      : hex;
    return [
      Number.parseInt(normalized.slice(0, 2), 16),
      Number.parseInt(normalized.slice(2, 4), 16),
      Number.parseInt(normalized.slice(4, 6), 16)
    ];
  }
  const rgb = color.match(/^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)/i);
  if (!rgb) return undefined;
  return [
    Number.parseFloat(rgb[1] ?? "0"),
    Number.parseFloat(rgb[2] ?? "0"),
    Number.parseFloat(rgb[3] ?? "0")
  ];
}

/** Mix `foreground` into `background` by ratio 0–1, returning `rgb()`. */
export function blendColor(
  background: string,
  foreground: string,
  foregroundRatio: number
): string {
  const backgroundRgb = parseColor(background);
  const foregroundRgb = parseColor(foreground);
  if (!backgroundRgb || !foregroundRgb) return foreground;
  const values = backgroundRgb.map((value, index) =>
    Math.round(value * (1 - foregroundRatio) + foregroundRgb[index]! * foregroundRatio));
  return `rgb(${values[0]}, ${values[1]}, ${values[2]})`;
}

function channelToLinear(value: number): number {
  const channel = value / 255;
  return channel <= 0.04045
    ? channel / 12.92
    : ((channel + 0.055) / 1.055) ** 2.4;
}

/** WCAG relative luminance, 0 (black) to 1 (white). Unknown colors read as light. */
export function relativeLuminance(color: string): number {
  const rgb = parseColor(color);
  if (!rgb) return 1;
  return 0.2126 * channelToLinear(rgb[0])
    + 0.7152 * channelToLinear(rgb[1])
    + 0.0722 * channelToLinear(rgb[2]);
}

/** True when a surface is dark enough that light ink reads better on it. */
export function isDarkColor(color: string): boolean {
  const rgb = parseColor(color);
  if (!rgb) return true;
  return (rgb[0] * 299 + rgb[1] * 587 + rgb[2] * 114) / 1000 < 145;
}

export const INK_ON_LIGHT = "#0E1319";
export const INK_ON_DARK = "#F5F8FC";

/** WCAG contrast ratio between two opaque colors, 1 (none) to 21 (maximum). */
export function contrastRatio(first: string, second: string): number {
  const a = relativeLuminance(first);
  const b = relativeLuminance(second);
  const lighter = Math.max(a, b);
  const darker = Math.min(a, b);
  return (lighter + 0.05) / (darker + 0.05);
}

/**
 * Ink that maximizes contrast on a filled surface. `opacity` accounts for a
 * tile painted at partial opacity over the canvas, because the effective color
 * is what the text actually sits on. Choosing by measured contrast (not a
 * brightness threshold) keeps labels legible across the whole palette band,
 * where mid-tone fills often favor ink over white.
 */
export function readableInk(
  surface: string,
  canvas: string,
  opacity = 1
): string {
  const effective = opacity >= 1 || opacity <= 0
    ? surface
    : blendColor(canvas, surface, opacity);
  return contrastRatio(effective, INK_ON_LIGHT) >= contrastRatio(effective, INK_ON_DARK)
    ? INK_ON_LIGHT
    : INK_ON_DARK;
}
