import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget059() { return <span data-tanstack-widget="059" />; }
function loader059() { return { id: 59 }; }
export const Route = createFileRoute("/fixture059")({ component: TanStackWidget059, loader: loader059 });
export function TanStackApp059() { return <TanStackWidget059 />; }
