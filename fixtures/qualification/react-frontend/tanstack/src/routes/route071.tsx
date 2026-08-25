import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget071() { return <span data-tanstack-widget="071" />; }
function loader071() { return { id: 71 }; }
export const Route = createFileRoute("/fixture071")({ component: TanStackWidget071, loader: loader071 });
export function TanStackApp071() { return <TanStackWidget071 />; }
