import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget064() { return <span data-tanstack-widget="064" />; }
function loader064() { return { id: 64 }; }
export const Route = createFileRoute("/fixture064")({ component: TanStackWidget064, loader: loader064 });
export function TanStackApp064() { return <TanStackWidget064 />; }
