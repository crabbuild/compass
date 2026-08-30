import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget067() { return <span data-tanstack-widget="067" />; }
function loader067() { return { id: 67 }; }
export const Route = createFileRoute("/fixture067")({ component: TanStackWidget067, loader: loader067 });
export function TanStackApp067() { return <TanStackWidget067 />; }
