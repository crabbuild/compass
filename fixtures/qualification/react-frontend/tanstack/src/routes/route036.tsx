import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget036() { return <span data-tanstack-widget="036" />; }
function loader036() { return { id: 36 }; }
export const Route = createFileRoute("/fixture036")({ component: TanStackWidget036, loader: loader036 });
export function TanStackApp036() { return <TanStackWidget036 />; }
