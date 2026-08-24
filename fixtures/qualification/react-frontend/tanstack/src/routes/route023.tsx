import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget023() { return <span data-tanstack-widget="023" />; }
function loader023() { return { id: 23 }; }
export const Route = createFileRoute("/fixture023")({ component: TanStackWidget023, loader: loader023 });
export function TanStackApp023() { return <TanStackWidget023 />; }
