import assert from "node:assert/strict";
import test from "node:test";
import { sourceRouteParent } from "../react_route_hierarchy_oracle.mjs";

// Expected parentage is written from the framework examples and source-file
// roles. These fixtures do not invoke the production resolver or read a graph.
const cases = [
  ["next-app", ["app/page.tsx", "app/admin/page.tsx", "app/admin/settings/page.tsx"], []],
  ["next-app", ["app/layout.tsx", "app/page.tsx", "app/blog/layout.tsx", "app/blog/page.tsx", "app/blog/post/page.tsx", "app/api/route.ts", "app/global-error.tsx"], [
    ["app/layout.tsx", "app/page.tsx"], ["app/layout.tsx", "app/blog/layout.tsx"], ["app/blog/layout.tsx", "app/blog/page.tsx"], ["app/blog/layout.tsx", "app/blog/post/page.tsx"],
  ]],
  ["next-app", ["app/layout.tsx", "app/layout.js", "app/page.tsx"], []],
  ["next-app", ["one/app/layout.tsx", "two/app/page.tsx"], []],
  ["next-pages", ["pages/index.tsx", "pages/blog.tsx", "pages/blog/post.tsx"], []],
  ["react-router", ["src/routes/home.tsx", "src/routes/tanstack.tsx"], []],
  ["react-router", ["app/routes/concerts.tsx", "app/routes/concerts.$city.tsx", "app/routes/concerts._index.tsx", "app/routes/concerts_.mine.tsx"], [
    ["app/routes/concerts.tsx", "app/routes/concerts.$city.tsx"], ["app/routes/concerts.tsx", "app/routes/concerts._index.tsx"],
  ]],
  ["react-router", ["app/routes/_auth.tsx", "app/routes/_auth.login.tsx", "app/routes/_index.tsx", "app/routes/about.tsx"], [["app/routes/_auth.tsx", "app/routes/_auth.login.tsx"]]],
  ["remix", ["app/routes/concerts/route.tsx", "app/routes/concerts.$city.tsx", "app/routes/concerts/helper.tsx"], [["app/routes/concerts/route.tsx", "app/routes/concerts.$city.tsx"]]],
  ["remix", ["app/routes/concerts.tsx", "app/routes/concerts/route.tsx", "app/routes/concerts.$city.tsx"], []],
  ["react-router", ["app/routes/a.tsx", "app/routes/a.b.tsx", "app/routes/a.b_.c.tsx"], [["app/routes/a.tsx", "app/routes/a.b.tsx"], ["app/routes/a.tsx", "app/routes/a.b_.c.tsx"]]],
  ["react-router", ["app/routes/foo[.]bar.tsx", "app/routes/foo[.]bar.child.tsx", "app/routes/foo.tsx"], [["app/routes/foo[.]bar.tsx", "app/routes/foo[.]bar.child.tsx"]]],
  ["tanstack-router", ["src/routes/__root.tsx", "src/routes/index.tsx", "src/routes/posts.tsx", "src/routes/posts.index.tsx", "src/routes/posts.$id.tsx", "src/routes/posts_.$id.edit.tsx"], [
    ["src/routes/__root.tsx", "src/routes/index.tsx"], ["src/routes/__root.tsx", "src/routes/posts.tsx"], ["src/routes/posts.tsx", "src/routes/posts.index.tsx"], ["src/routes/posts.tsx", "src/routes/posts.$id.tsx"], ["src/routes/__root.tsx", "src/routes/posts_.$id.edit.tsx"],
  ]],
  ["tanstack-router", ["src/routes/account/route.tsx", "src/routes/account/overview.tsx", "src/routes/account/index.tsx", "src/routes/account/-helper.tsx", "src/routes/account/route.lazy.tsx"], [
    ["src/routes/account/route.tsx", "src/routes/account/overview.tsx"], ["src/routes/account/route.tsx", "src/routes/account/index.tsx"],
  ]],
  ["tanstack-router", ["src/routes/index.tsx", "src/routes/about.tsx"], []],
];

for (const [index, [framework, files, expected]] of cases.entries()) {
  test(`source parent case ${index + 1}: ${framework}`, () => {
    for (const candidates of [files, [...files].reverse()]) {
      const actual = candidates.flatMap((file) => {
        const parent = sourceRouteParent(file, candidates, framework);
        return parent ? [[parent, file]] : [];
      });
      assert.deepEqual(actual.sort(), [...expected].sort());
    }
  });
}
