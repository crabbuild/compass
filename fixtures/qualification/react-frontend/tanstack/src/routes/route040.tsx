import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget040() { return <span data-tanstack-widget="040" />; }
function loader040() { return { id: 40 }; }
export const Route = createFileRoute("/fixture040")({ component: TanStackWidget040, loader: loader040 });
export function TanStackApp040() { return <TanStackWidget040 />; }
