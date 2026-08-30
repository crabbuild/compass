import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget093() { return <span data-tanstack-widget="093" />; }
function loader093() { return { id: 93 }; }
export const Route = createFileRoute("/fixture093")({ component: TanStackWidget093, loader: loader093 });
export function TanStackApp093() { return <TanStackWidget093 />; }
