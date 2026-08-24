import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget090() { return <span data-tanstack-widget="090" />; }
function loader090() { return { id: 90 }; }
export const Route = createFileRoute("/fixture090")({ component: TanStackWidget090, loader: loader090 });
export function TanStackApp090() { return <TanStackWidget090 />; }
