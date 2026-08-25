import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget089() { return <span data-tanstack-widget="089" />; }
function loader089() { return { id: 89 }; }
export const Route = createFileRoute("/fixture089")({ component: TanStackWidget089, loader: loader089 });
export function TanStackApp089() { return <TanStackWidget089 />; }
