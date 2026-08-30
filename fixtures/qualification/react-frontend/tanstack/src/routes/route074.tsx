import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget074() { return <span data-tanstack-widget="074" />; }
function loader074() { return { id: 74 }; }
export const Route = createFileRoute("/fixture074")({ component: TanStackWidget074, loader: loader074 });
export function TanStackApp074() { return <TanStackWidget074 />; }
