import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget018() { return <span data-tanstack-widget="018" />; }
function loader018() { return { id: 18 }; }
export const Route = createFileRoute("/fixture018")({ component: TanStackWidget018, loader: loader018 });
export function TanStackApp018() { return <TanStackWidget018 />; }
