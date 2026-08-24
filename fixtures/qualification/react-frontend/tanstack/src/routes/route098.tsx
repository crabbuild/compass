import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget098() { return <span data-tanstack-widget="098" />; }
function loader098() { return { id: 98 }; }
export const Route = createFileRoute("/fixture098")({ component: TanStackWidget098, loader: loader098 });
export function TanStackApp098() { return <TanStackWidget098 />; }
