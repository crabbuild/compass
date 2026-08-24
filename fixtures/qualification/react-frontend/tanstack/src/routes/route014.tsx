import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget014() { return <span data-tanstack-widget="014" />; }
function loader014() { return { id: 14 }; }
export const Route = createFileRoute("/fixture014")({ component: TanStackWidget014, loader: loader014 });
export function TanStackApp014() { return <TanStackWidget014 />; }
