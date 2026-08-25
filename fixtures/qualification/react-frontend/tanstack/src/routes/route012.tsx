import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget012() { return <span data-tanstack-widget="012" />; }
function loader012() { return { id: 12 }; }
export const Route = createFileRoute("/fixture012")({ component: TanStackWidget012, loader: loader012 });
export function TanStackApp012() { return <TanStackWidget012 />; }
