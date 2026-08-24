import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget039() { return <span data-tanstack-widget="039" />; }
function loader039() { return { id: 39 }; }
export const Route = createFileRoute("/fixture039")({ component: TanStackWidget039, loader: loader039 });
export function TanStackApp039() { return <TanStackWidget039 />; }
