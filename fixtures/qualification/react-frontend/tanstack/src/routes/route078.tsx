import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget078() { return <span data-tanstack-widget="078" />; }
function loader078() { return { id: 78 }; }
export const Route = createFileRoute("/fixture078")({ component: TanStackWidget078, loader: loader078 });
export function TanStackApp078() { return <TanStackWidget078 />; }
