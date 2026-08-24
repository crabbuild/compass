import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget035() { return <span data-tanstack-widget="035" />; }
function loader035() { return { id: 35 }; }
export const Route = createFileRoute("/fixture035")({ component: TanStackWidget035, loader: loader035 });
export function TanStackApp035() { return <TanStackWidget035 />; }
