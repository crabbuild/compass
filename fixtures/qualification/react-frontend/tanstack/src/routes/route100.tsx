import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget100() { return <span data-tanstack-widget="100" />; }
function loader100() { return { id: 100 }; }
export const Route = createFileRoute("/fixture100")({ component: TanStackWidget100, loader: loader100 });
export function TanStackApp100() { return <TanStackWidget100 />; }
