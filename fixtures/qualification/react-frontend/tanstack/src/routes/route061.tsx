import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget061() { return <span data-tanstack-widget="061" />; }
function loader061() { return { id: 61 }; }
export const Route = createFileRoute("/fixture061")({ component: TanStackWidget061, loader: loader061 });
export function TanStackApp061() { return <TanStackWidget061 />; }
