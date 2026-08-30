import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget086() { return <span data-tanstack-widget="086" />; }
function loader086() { return { id: 86 }; }
export const Route = createFileRoute("/fixture086")({ component: TanStackWidget086, loader: loader086 });
export function TanStackApp086() { return <TanStackWidget086 />; }
