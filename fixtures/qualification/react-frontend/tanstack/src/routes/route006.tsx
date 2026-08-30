import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget006() { return <span data-tanstack-widget="006" />; }
function loader006() { return { id: 6 }; }
export const Route = createFileRoute("/fixture006")({ component: TanStackWidget006, loader: loader006 });
export function TanStackApp006() { return <TanStackWidget006 />; }
