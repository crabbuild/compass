import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget003() { return <span data-tanstack-widget="003" />; }
function loader003() { return { id: 3 }; }
export const Route = createFileRoute("/fixture003")({ component: TanStackWidget003, loader: loader003 });
export function TanStackApp003() { return <TanStackWidget003 />; }
