import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget043() { return <span data-tanstack-widget="043" />; }
function loader043() { return { id: 43 }; }
export const Route = createFileRoute("/fixture043")({ component: TanStackWidget043, loader: loader043 });
export function TanStackApp043() { return <TanStackWidget043 />; }
