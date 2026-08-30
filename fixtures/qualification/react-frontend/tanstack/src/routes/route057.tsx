import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget057() { return <span data-tanstack-widget="057" />; }
function loader057() { return { id: 57 }; }
export const Route = createFileRoute("/fixture057")({ component: TanStackWidget057, loader: loader057 });
export function TanStackApp057() { return <TanStackWidget057 />; }
