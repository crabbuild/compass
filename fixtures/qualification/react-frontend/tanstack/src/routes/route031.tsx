import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget031() { return <span data-tanstack-widget="031" />; }
function loader031() { return { id: 31 }; }
export const Route = createFileRoute("/fixture031")({ component: TanStackWidget031, loader: loader031 });
export function TanStackApp031() { return <TanStackWidget031 />; }
