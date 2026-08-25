import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget048() { return <span data-tanstack-widget="048" />; }
function loader048() { return { id: 48 }; }
export const Route = createFileRoute("/fixture048")({ component: TanStackWidget048, loader: loader048 });
export function TanStackApp048() { return <TanStackWidget048 />; }
