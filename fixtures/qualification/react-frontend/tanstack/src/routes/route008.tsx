import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget008() { return <span data-tanstack-widget="008" />; }
function loader008() { return { id: 8 }; }
export const Route = createFileRoute("/fixture008")({ component: TanStackWidget008, loader: loader008 });
export function TanStackApp008() { return <TanStackWidget008 />; }
