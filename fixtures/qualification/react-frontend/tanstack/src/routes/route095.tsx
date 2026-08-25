import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget095() { return <span data-tanstack-widget="095" />; }
function loader095() { return { id: 95 }; }
export const Route = createFileRoute("/fixture095")({ component: TanStackWidget095, loader: loader095 });
export function TanStackApp095() { return <TanStackWidget095 />; }
