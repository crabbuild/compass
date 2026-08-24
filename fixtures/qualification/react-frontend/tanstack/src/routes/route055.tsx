import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget055() { return <span data-tanstack-widget="055" />; }
function loader055() { return { id: 55 }; }
export const Route = createFileRoute("/fixture055")({ component: TanStackWidget055, loader: loader055 });
export function TanStackApp055() { return <TanStackWidget055 />; }
