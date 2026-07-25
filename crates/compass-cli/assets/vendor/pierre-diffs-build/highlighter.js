import { createHighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";

const compassTheme = {
  name: "compass-dark",
  type: "dark",
  colors: {
    "editor.background": "#0b0e13",
    "editor.foreground": "#c5cad3"
  },
  settings: [{ "settings": { "foreground": "#c5cad3" } }]
};

let highlighter;

export function getHighlighterIfLoaded() {
  return highlighter && !("then" in highlighter) ? highlighter : undefined;
}

export async function getSharedHighlighter() {
  highlighter ??= createHighlighterCore({
    themes: [compassTheme],
    langs: [],
    engine: createJavaScriptRegexEngine()
  });
  highlighter = await highlighter;
  return highlighter;
}

export async function disposeHighlighter() {
  if (highlighter) (await highlighter).dispose();
  highlighter = undefined;
}

export function isHighlighterLoaded() {
  return Boolean(getHighlighterIfLoaded());
}

export function isHighlighterLoading() {
  return Boolean(highlighter && "then" in highlighter);
}

export function isHighlighterNull() {
  return !highlighter;
}

export async function preloadHighlighter() {
  await getSharedHighlighter();
}
