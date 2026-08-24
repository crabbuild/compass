import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget091() { return <span data-tanstack-widget="091" />; }
function loader091() { return { id: 91 }; }
export const Route = createFileRoute("/fixture091")({ component: TanStackWidget091, loader: loader091 });
export function TanStackApp091() { return <TanStackWidget091 />; }
