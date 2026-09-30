/**
 * Source-only route-parent oracle. Compare candidate/child relationships from
 * documented framework conventions; no Compass records or resolver indexes.
 *
 * Next: https://nextjs.org/docs/app/getting-started/layouts-and-pages
 * Flat routes: https://reactrouter.com/how-to/file-route-conventions
 * TanStack: https://tanstack.com/router/latest/docs/routing/file-based-routing
 */

export const HIERARCHY_SEMANTICS = 2;

export function sourceRouteScope(file, framework) {
  file = file.replaceAll("\\", "/");
  const markers = framework === "next-app"
    ? ["src/app/", "app/"]
    : framework === "tanstack-router"
      ? ["src/routes/", "routes/"]
      : ["app/routes/", "src/routes/", "routes/"];
  for (const marker of markers) {
    let offset = 0;
    while (offset < file.length) {
      const index = file.indexOf(marker, offset);
      if (index < 0) break;
      if (index === 0 || file[index - 1] === "/") return file.slice(0, index + marker.length);
      offset = index + 1;
    }
  }
  return null;
}

function description(file, framework) {
  const portable = file.replaceAll("\\", "/");
  const root = sourceRouteScope(portable, framework);
  if (!root) return null;
  const relative = portable.slice(root.length).replace(/\.[cm]?[jt]sx?$/u, "");
  if (relative === portable.slice(root.length)) return null;
  if (framework === "next-app") {
    const components = relative.split("/");
    const role = components.pop();
    if (!["layout", "page", "template", "error", "loading", "not-found", "default"].includes(role)) return null;
    return { root, parts: components, parent: role === "layout", layout: true };
  }
  let identifier = relative;
  if (framework !== "tanstack-router" && identifier.includes("/")) {
    const components = identifier.split("/");
    if (components.length !== 2 || components[1] !== "route") return null;
    identifier = components[0];
  }
  // Bracket escapes remain opaque, so a literal dot cannot create nesting.
  if (!/^(?:\[[^\[\]]*\]|[^\[\]])+$/u.test(identifier)) return null;
  let parts = identifier.match(/(?:\[[^\[\]]*\]|[^./])+/gu) ?? [];
  if (framework === "tanstack-router") {
    if (parts.some((part) => part.startsWith("-")) || parts.at(-1) === "lazy") return null;
    parts = parts.filter((part) => !/^\(.*\)$/u.test(part));
    if (parts.at(-1) === "route") parts.pop();
    if (parts.length === 1 && parts[0] === "__root") return { root, parts: [], parent: true, layout: false };
  }
  if (!parts.length) return null;
  const index = framework === "tanstack-router" ? "index" : "_index";
  return { root, parts, parent: parts.at(-1) !== index && !parts.at(-1).endsWith("_"), layout: false };
}

export function sourceRouteParent(file, candidates, framework) {
  if (!["next-app", "react-router", "remix", "tanstack-router"].includes(framework)) return null;
  const child = description(file, framework);
  if (!child) return null;
  const matches = [];
  for (const candidate of new Set(candidates)) {
    if (candidate.replaceAll("\\", "/") === file.replaceAll("\\", "/")) continue;
    const parent = description(candidate, framework);
    if (!parent?.parent || parent.root !== child.root) continue;
    const strict = !parent.layout || child.parent;
    if (parent.parts.length > child.parts.length || (strict && parent.parts.length === child.parts.length)) continue;
    if (parent.parts.every((part, index) => part === child.parts[index])) matches.push({ file: candidate, depth: parent.parts.length });
  }
  const deepest = matches.reduce((depth, match) => Math.max(depth, match.depth), -1);
  const closest = matches.filter((match) => match.depth === deepest);
  return closest.length === 1 ? closest[0].file : null;
}
