import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget011() { return <span data-tanstack-widget="011" />; }
function loader011() { return { id: 11 }; }
export const Route = createFileRoute("/fixture011")({ component: TanStackWidget011, loader: loader011 });
export function TanStackApp011() { return <TanStackWidget011 />; }
