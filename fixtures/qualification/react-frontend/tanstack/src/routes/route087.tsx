import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget087() { return <span data-tanstack-widget="087" />; }
function loader087() { return { id: 87 }; }
export const Route = createFileRoute("/fixture087")({ component: TanStackWidget087, loader: loader087 });
export function TanStackApp087() { return <TanStackWidget087 />; }
