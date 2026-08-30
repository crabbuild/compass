import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget099() { return <span data-tanstack-widget="099" />; }
function loader099() { return { id: 99 }; }
export const Route = createFileRoute("/fixture099")({ component: TanStackWidget099, loader: loader099 });
export function TanStackApp099() { return <TanStackWidget099 />; }
