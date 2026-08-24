import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget088() { return <span data-tanstack-widget="088" />; }
function loader088() { return { id: 88 }; }
export const Route = createFileRoute("/fixture088")({ component: TanStackWidget088, loader: loader088 });
export function TanStackApp088() { return <TanStackWidget088 />; }
