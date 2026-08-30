import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget051() { return <span data-tanstack-widget="051" />; }
function loader051() { return { id: 51 }; }
export const Route = createFileRoute("/fixture051")({ component: TanStackWidget051, loader: loader051 });
export function TanStackApp051() { return <TanStackWidget051 />; }
