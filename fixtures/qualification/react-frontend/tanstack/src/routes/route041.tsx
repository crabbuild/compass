import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget041() { return <span data-tanstack-widget="041" />; }
function loader041() { return { id: 41 }; }
export const Route = createFileRoute("/fixture041")({ component: TanStackWidget041, loader: loader041 });
export function TanStackApp041() { return <TanStackWidget041 />; }
