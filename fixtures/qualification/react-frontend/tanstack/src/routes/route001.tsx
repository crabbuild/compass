import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget001() { return <span data-tanstack-widget="001" />; }
function loader001() { return { id: 1 }; }
export const Route = createFileRoute("/fixture001")({ component: TanStackWidget001, loader: loader001 });
export function TanStackApp001() { return <TanStackWidget001 />; }
