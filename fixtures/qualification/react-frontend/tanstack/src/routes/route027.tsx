import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget027() { return <span data-tanstack-widget="027" />; }
function loader027() { return { id: 27 }; }
export const Route = createFileRoute("/fixture027")({ component: TanStackWidget027, loader: loader027 });
export function TanStackApp027() { return <TanStackWidget027 />; }
