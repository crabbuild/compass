import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget034() { return <span data-tanstack-widget="034" />; }
function loader034() { return { id: 34 }; }
export const Route = createFileRoute("/fixture034")({ component: TanStackWidget034, loader: loader034 });
export function TanStackApp034() { return <TanStackWidget034 />; }
