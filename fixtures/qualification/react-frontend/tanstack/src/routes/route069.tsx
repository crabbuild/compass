import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget069() { return <span data-tanstack-widget="069" />; }
function loader069() { return { id: 69 }; }
export const Route = createFileRoute("/fixture069")({ component: TanStackWidget069, loader: loader069 });
export function TanStackApp069() { return <TanStackWidget069 />; }
