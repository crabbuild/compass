import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget024() { return <span data-tanstack-widget="024" />; }
function loader024() { return { id: 24 }; }
export const Route = createFileRoute("/fixture024")({ component: TanStackWidget024, loader: loader024 });
export function TanStackApp024() { return <TanStackWidget024 />; }
