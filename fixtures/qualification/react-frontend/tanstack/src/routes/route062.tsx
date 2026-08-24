import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget062() { return <span data-tanstack-widget="062" />; }
function loader062() { return { id: 62 }; }
export const Route = createFileRoute("/fixture062")({ component: TanStackWidget062, loader: loader062 });
export function TanStackApp062() { return <TanStackWidget062 />; }
