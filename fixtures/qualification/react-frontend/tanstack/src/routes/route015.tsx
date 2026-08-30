import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget015() { return <span data-tanstack-widget="015" />; }
function loader015() { return { id: 15 }; }
export const Route = createFileRoute("/fixture015")({ component: TanStackWidget015, loader: loader015 });
export function TanStackApp015() { return <TanStackWidget015 />; }
