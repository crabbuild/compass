import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget029() { return <span data-tanstack-widget="029" />; }
function loader029() { return { id: 29 }; }
export const Route = createFileRoute("/fixture029")({ component: TanStackWidget029, loader: loader029 });
export function TanStackApp029() { return <TanStackWidget029 />; }
