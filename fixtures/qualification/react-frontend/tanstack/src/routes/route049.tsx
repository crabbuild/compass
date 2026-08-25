import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget049() { return <span data-tanstack-widget="049" />; }
function loader049() { return { id: 49 }; }
export const Route = createFileRoute("/fixture049")({ component: TanStackWidget049, loader: loader049 });
export function TanStackApp049() { return <TanStackWidget049 />; }
