import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget096() { return <span data-tanstack-widget="096" />; }
function loader096() { return { id: 96 }; }
export const Route = createFileRoute("/fixture096")({ component: TanStackWidget096, loader: loader096 });
export function TanStackApp096() { return <TanStackWidget096 />; }
