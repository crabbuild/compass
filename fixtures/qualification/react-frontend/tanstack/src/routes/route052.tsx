import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget052() { return <span data-tanstack-widget="052" />; }
function loader052() { return { id: 52 }; }
export const Route = createFileRoute("/fixture052")({ component: TanStackWidget052, loader: loader052 });
export function TanStackApp052() { return <TanStackWidget052 />; }
