import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget002() { return <span data-tanstack-widget="002" />; }
function loader002() { return { id: 2 }; }
export const Route = createFileRoute("/fixture002")({ component: TanStackWidget002, loader: loader002 });
export function TanStackApp002() { return <TanStackWidget002 />; }
