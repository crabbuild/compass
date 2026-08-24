import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget054() { return <span data-tanstack-widget="054" />; }
function loader054() { return { id: 54 }; }
export const Route = createFileRoute("/fixture054")({ component: TanStackWidget054, loader: loader054 });
export function TanStackApp054() { return <TanStackWidget054 />; }
