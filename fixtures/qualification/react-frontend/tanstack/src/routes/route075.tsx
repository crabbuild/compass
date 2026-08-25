import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget075() { return <span data-tanstack-widget="075" />; }
function loader075() { return { id: 75 }; }
export const Route = createFileRoute("/fixture075")({ component: TanStackWidget075, loader: loader075 });
export function TanStackApp075() { return <TanStackWidget075 />; }
