import { useEffect, useState } from "react";

/**
 * Read a themed CSS custom property with a fallback, so canvas and SVG
 * rendering can share the exact color the chrome is using.
 */
export function cssColor(name: string, fallback: string): string {
  if (typeof window === "undefined") return fallback;
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
    || fallback;
}

/**
 * Revision counter that changes whenever the host theme changes: a class or
 * inline-style change on the document, or an operating-system color-scheme
 * change. Memoized colors should depend on it so they are recomputed rather
 * than baked in at first paint.
 */
export function useThemeRevision(): number {
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const refresh = () => setRevision((current) => current + 1);
    const observer = new MutationObserver(refresh);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "style"]
    });
    observer.observe(document.body, {
      attributes: true,
      attributeFilter: ["class", "style"]
    });
    // jsdom and some embedded hosts have no matchMedia; the observer above
    // still covers every host-driven theme change.
    const colorScheme = typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: dark)")
      : undefined;
    colorScheme?.addEventListener("change", refresh);
    return () => {
      observer.disconnect();
      colorScheme?.removeEventListener("change", refresh);
    };
  }, []);
  return revision;
}

/** The canvas color variants paint onto, resolving the host theme first. */
export function canvasColor(): string {
  return cssColor(
    "--vscode-editor-background",
    cssColor("--compass-canvas", cssColor("--background", "#FBFCFD"))
  );
}

/**
 * Reader-facing theme choice for standalone documents. `auto` follows the
 * operating system; `light` and `dark` pin the palette regardless of it, which
 * is what a graph picture or a shared screenshot needs. Editors that inject
 * their own theme tokens keep winning, because their variables take precedence
 * inside every rule that uses them.
 */
export type ThemePreference = "auto" | "light" | "dark";

export const THEME_PREFERENCES: ReadonlyArray<{
  value: ThemePreference;
  label: string;
  hint: string;
}> = [
  { value: "auto", label: "Auto", hint: "Follow the operating system theme" },
  { value: "light", label: "Light", hint: "Always use the light palette" },
  { value: "dark", label: "Dark", hint: "Always use the dark palette" }
];

/** Apply a preference to the document root; `auto` clears the override. */
export function applyThemePreference(preference: ThemePreference): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (preference === "auto") delete root.dataset.compassTheme;
  else root.dataset.compassTheme = preference;
}
