import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget094() { return <span data-tanstack-widget="094" />; }
function loader094() { return { id: 94 }; }
export const Route = createFileRoute("/fixture094")({ component: TanStackWidget094, loader: loader094 });
export function TanStackApp094() { return <TanStackWidget094 />; }
