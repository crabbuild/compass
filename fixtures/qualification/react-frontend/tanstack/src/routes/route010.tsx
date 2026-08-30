import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget010() { return <span data-tanstack-widget="010" />; }
function loader010() { return { id: 10 }; }
export const Route = createFileRoute("/fixture010")({ component: TanStackWidget010, loader: loader010 });
export function TanStackApp010() { return <TanStackWidget010 />; }
