import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget042() { return <span data-tanstack-widget="042" />; }
function loader042() { return { id: 42 }; }
export const Route = createFileRoute("/fixture042")({ component: TanStackWidget042, loader: loader042 });
export function TanStackApp042() { return <TanStackWidget042 />; }
