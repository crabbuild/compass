import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget047() { return <span data-tanstack-widget="047" />; }
function loader047() { return { id: 47 }; }
export const Route = createFileRoute("/fixture047")({ component: TanStackWidget047, loader: loader047 });
export function TanStackApp047() { return <TanStackWidget047 />; }
