import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget101() { return <span data-tanstack-widget="101" />; }
function loader101() { return { id: 101 }; }
export const Route = createFileRoute("/fixture101")({ component: TanStackWidget101, loader: loader101 });
export function TanStackApp101() { return <TanStackWidget101 />; }
