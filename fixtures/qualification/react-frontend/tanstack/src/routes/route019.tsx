import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget019() { return <span data-tanstack-widget="019" />; }
function loader019() { return { id: 19 }; }
export const Route = createFileRoute("/fixture019")({ component: TanStackWidget019, loader: loader019 });
export function TanStackApp019() { return <TanStackWidget019 />; }
