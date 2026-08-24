import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget092() { return <span data-tanstack-widget="092" />; }
function loader092() { return { id: 92 }; }
export const Route = createFileRoute("/fixture092")({ component: TanStackWidget092, loader: loader092 });
export function TanStackApp092() { return <TanStackWidget092 />; }
