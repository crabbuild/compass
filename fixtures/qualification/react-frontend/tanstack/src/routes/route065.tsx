import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget065() { return <span data-tanstack-widget="065" />; }
function loader065() { return { id: 65 }; }
export const Route = createFileRoute("/fixture065")({ component: TanStackWidget065, loader: loader065 });
export function TanStackApp065() { return <TanStackWidget065 />; }
