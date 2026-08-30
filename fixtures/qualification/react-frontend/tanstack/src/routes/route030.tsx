import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget030() { return <span data-tanstack-widget="030" />; }
function loader030() { return { id: 30 }; }
export const Route = createFileRoute("/fixture030")({ component: TanStackWidget030, loader: loader030 });
export function TanStackApp030() { return <TanStackWidget030 />; }
