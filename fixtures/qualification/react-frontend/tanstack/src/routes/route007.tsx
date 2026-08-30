import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget007() { return <span data-tanstack-widget="007" />; }
function loader007() { return { id: 7 }; }
export const Route = createFileRoute("/fixture007")({ component: TanStackWidget007, loader: loader007 });
export function TanStackApp007() { return <TanStackWidget007 />; }
