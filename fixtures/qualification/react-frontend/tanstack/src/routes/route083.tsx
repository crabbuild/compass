import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget083() { return <span data-tanstack-widget="083" />; }
function loader083() { return { id: 83 }; }
export const Route = createFileRoute("/fixture083")({ component: TanStackWidget083, loader: loader083 });
export function TanStackApp083() { return <TanStackWidget083 />; }
