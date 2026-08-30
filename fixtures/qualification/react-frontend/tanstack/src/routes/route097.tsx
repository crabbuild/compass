import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget097() { return <span data-tanstack-widget="097" />; }
function loader097() { return { id: 97 }; }
export const Route = createFileRoute("/fixture097")({ component: TanStackWidget097, loader: loader097 });
export function TanStackApp097() { return <TanStackWidget097 />; }
