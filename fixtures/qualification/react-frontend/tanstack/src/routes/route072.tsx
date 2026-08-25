import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget072() { return <span data-tanstack-widget="072" />; }
function loader072() { return { id: 72 }; }
export const Route = createFileRoute("/fixture072")({ component: TanStackWidget072, loader: loader072 });
export function TanStackApp072() { return <TanStackWidget072 />; }
