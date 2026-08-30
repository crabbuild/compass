import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget037() { return <span data-tanstack-widget="037" />; }
function loader037() { return { id: 37 }; }
export const Route = createFileRoute("/fixture037")({ component: TanStackWidget037, loader: loader037 });
export function TanStackApp037() { return <TanStackWidget037 />; }
