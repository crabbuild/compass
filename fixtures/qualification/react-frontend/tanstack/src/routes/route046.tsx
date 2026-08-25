import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget046() { return <span data-tanstack-widget="046" />; }
function loader046() { return { id: 46 }; }
export const Route = createFileRoute("/fixture046")({ component: TanStackWidget046, loader: loader046 });
export function TanStackApp046() { return <TanStackWidget046 />; }
