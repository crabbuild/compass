import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget020() { return <span data-tanstack-widget="020" />; }
function loader020() { return { id: 20 }; }
export const Route = createFileRoute("/fixture020")({ component: TanStackWidget020, loader: loader020 });
export function TanStackApp020() { return <TanStackWidget020 />; }
