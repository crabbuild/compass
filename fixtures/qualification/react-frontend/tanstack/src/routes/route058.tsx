import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget058() { return <span data-tanstack-widget="058" />; }
function loader058() { return { id: 58 }; }
export const Route = createFileRoute("/fixture058")({ component: TanStackWidget058, loader: loader058 });
export function TanStackApp058() { return <TanStackWidget058 />; }
