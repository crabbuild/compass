import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget009() { return <span data-tanstack-widget="009" />; }
function loader009() { return { id: 9 }; }
export const Route = createFileRoute("/fixture009")({ component: TanStackWidget009, loader: loader009 });
export function TanStackApp009() { return <TanStackWidget009 />; }
