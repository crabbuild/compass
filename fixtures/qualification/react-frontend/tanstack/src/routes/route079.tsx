import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget079() { return <span data-tanstack-widget="079" />; }
function loader079() { return { id: 79 }; }
export const Route = createFileRoute("/fixture079")({ component: TanStackWidget079, loader: loader079 });
export function TanStackApp079() { return <TanStackWidget079 />; }
