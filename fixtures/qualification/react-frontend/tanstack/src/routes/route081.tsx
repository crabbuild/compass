import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget081() { return <span data-tanstack-widget="081" />; }
function loader081() { return { id: 81 }; }
export const Route = createFileRoute("/fixture081")({ component: TanStackWidget081, loader: loader081 });
export function TanStackApp081() { return <TanStackWidget081 />; }
