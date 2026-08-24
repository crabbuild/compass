import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget032() { return <span data-tanstack-widget="032" />; }
function loader032() { return { id: 32 }; }
export const Route = createFileRoute("/fixture032")({ component: TanStackWidget032, loader: loader032 });
export function TanStackApp032() { return <TanStackWidget032 />; }
