import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget013() { return <span data-tanstack-widget="013" />; }
function loader013() { return { id: 13 }; }
export const Route = createFileRoute("/fixture013")({ component: TanStackWidget013, loader: loader013 });
export function TanStackApp013() { return <TanStackWidget013 />; }
