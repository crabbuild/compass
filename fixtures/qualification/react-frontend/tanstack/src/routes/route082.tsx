import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget082() { return <span data-tanstack-widget="082" />; }
function loader082() { return { id: 82 }; }
export const Route = createFileRoute("/fixture082")({ component: TanStackWidget082, loader: loader082 });
export function TanStackApp082() { return <TanStackWidget082 />; }
