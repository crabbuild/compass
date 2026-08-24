import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget005() { return <span data-tanstack-widget="005" />; }
function loader005() { return { id: 5 }; }
export const Route = createFileRoute("/fixture005")({ component: TanStackWidget005, loader: loader005 });
export function TanStackApp005() { return <TanStackWidget005 />; }
