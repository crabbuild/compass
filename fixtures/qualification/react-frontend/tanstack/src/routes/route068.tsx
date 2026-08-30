import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget068() { return <span data-tanstack-widget="068" />; }
function loader068() { return { id: 68 }; }
export const Route = createFileRoute("/fixture068")({ component: TanStackWidget068, loader: loader068 });
export function TanStackApp068() { return <TanStackWidget068 />; }
