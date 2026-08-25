import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget077() { return <span data-tanstack-widget="077" />; }
function loader077() { return { id: 77 }; }
export const Route = createFileRoute("/fixture077")({ component: TanStackWidget077, loader: loader077 });
export function TanStackApp077() { return <TanStackWidget077 />; }
