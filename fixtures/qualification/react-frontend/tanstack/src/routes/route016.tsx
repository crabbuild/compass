import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget016() { return <span data-tanstack-widget="016" />; }
function loader016() { return { id: 16 }; }
export const Route = createFileRoute("/fixture016")({ component: TanStackWidget016, loader: loader016 });
export function TanStackApp016() { return <TanStackWidget016 />; }
