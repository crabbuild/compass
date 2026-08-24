import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget076() { return <span data-tanstack-widget="076" />; }
function loader076() { return { id: 76 }; }
export const Route = createFileRoute("/fixture076")({ component: TanStackWidget076, loader: loader076 });
export function TanStackApp076() { return <TanStackWidget076 />; }
