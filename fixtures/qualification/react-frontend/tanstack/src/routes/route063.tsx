import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget063() { return <span data-tanstack-widget="063" />; }
function loader063() { return { id: 63 }; }
export const Route = createFileRoute("/fixture063")({ component: TanStackWidget063, loader: loader063 });
export function TanStackApp063() { return <TanStackWidget063 />; }
