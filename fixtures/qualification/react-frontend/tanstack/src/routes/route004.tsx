import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget004() { return <span data-tanstack-widget="004" />; }
function loader004() { return { id: 4 }; }
export const Route = createFileRoute("/fixture004")({ component: TanStackWidget004, loader: loader004 });
export function TanStackApp004() { return <TanStackWidget004 />; }
