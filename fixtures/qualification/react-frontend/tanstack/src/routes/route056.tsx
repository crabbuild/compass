import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget056() { return <span data-tanstack-widget="056" />; }
function loader056() { return { id: 56 }; }
export const Route = createFileRoute("/fixture056")({ component: TanStackWidget056, loader: loader056 });
export function TanStackApp056() { return <TanStackWidget056 />; }
