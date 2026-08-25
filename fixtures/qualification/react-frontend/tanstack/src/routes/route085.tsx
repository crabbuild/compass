import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget085() { return <span data-tanstack-widget="085" />; }
function loader085() { return { id: 85 }; }
export const Route = createFileRoute("/fixture085")({ component: TanStackWidget085, loader: loader085 });
export function TanStackApp085() { return <TanStackWidget085 />; }
