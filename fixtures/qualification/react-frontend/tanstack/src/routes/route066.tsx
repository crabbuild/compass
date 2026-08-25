import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget066() { return <span data-tanstack-widget="066" />; }
function loader066() { return { id: 66 }; }
export const Route = createFileRoute("/fixture066")({ component: TanStackWidget066, loader: loader066 });
export function TanStackApp066() { return <TanStackWidget066 />; }
