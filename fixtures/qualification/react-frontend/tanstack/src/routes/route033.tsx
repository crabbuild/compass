import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget033() { return <span data-tanstack-widget="033" />; }
function loader033() { return { id: 33 }; }
export const Route = createFileRoute("/fixture033")({ component: TanStackWidget033, loader: loader033 });
export function TanStackApp033() { return <TanStackWidget033 />; }
