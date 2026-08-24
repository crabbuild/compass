import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget080() { return <span data-tanstack-widget="080" />; }
function loader080() { return { id: 80 }; }
export const Route = createFileRoute("/fixture080")({ component: TanStackWidget080, loader: loader080 });
export function TanStackApp080() { return <TanStackWidget080 />; }
