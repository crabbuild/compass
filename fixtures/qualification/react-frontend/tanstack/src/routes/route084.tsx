import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget084() { return <span data-tanstack-widget="084" />; }
function loader084() { return { id: 84 }; }
export const Route = createFileRoute("/fixture084")({ component: TanStackWidget084, loader: loader084 });
export function TanStackApp084() { return <TanStackWidget084 />; }
