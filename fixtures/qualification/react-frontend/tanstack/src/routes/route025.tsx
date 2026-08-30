import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget025() { return <span data-tanstack-widget="025" />; }
function loader025() { return { id: 25 }; }
export const Route = createFileRoute("/fixture025")({ component: TanStackWidget025, loader: loader025 });
export function TanStackApp025() { return <TanStackWidget025 />; }
