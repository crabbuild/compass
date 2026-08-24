import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget073() { return <span data-tanstack-widget="073" />; }
function loader073() { return { id: 73 }; }
export const Route = createFileRoute("/fixture073")({ component: TanStackWidget073, loader: loader073 });
export function TanStackApp073() { return <TanStackWidget073 />; }
