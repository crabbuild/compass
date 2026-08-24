import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget028() { return <span data-tanstack-widget="028" />; }
function loader028() { return { id: 28 }; }
export const Route = createFileRoute("/fixture028")({ component: TanStackWidget028, loader: loader028 });
export function TanStackApp028() { return <TanStackWidget028 />; }
