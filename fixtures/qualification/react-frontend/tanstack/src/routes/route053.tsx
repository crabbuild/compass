import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget053() { return <span data-tanstack-widget="053" />; }
function loader053() { return { id: 53 }; }
export const Route = createFileRoute("/fixture053")({ component: TanStackWidget053, loader: loader053 });
export function TanStackApp053() { return <TanStackWidget053 />; }
