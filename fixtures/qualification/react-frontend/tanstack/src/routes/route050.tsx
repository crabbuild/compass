import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget050() { return <span data-tanstack-widget="050" />; }
function loader050() { return { id: 50 }; }
export const Route = createFileRoute("/fixture050")({ component: TanStackWidget050, loader: loader050 });
export function TanStackApp050() { return <TanStackWidget050 />; }
