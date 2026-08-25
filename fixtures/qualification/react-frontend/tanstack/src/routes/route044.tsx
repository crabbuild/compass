import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget044() { return <span data-tanstack-widget="044" />; }
function loader044() { return { id: 44 }; }
export const Route = createFileRoute("/fixture044")({ component: TanStackWidget044, loader: loader044 });
export function TanStackApp044() { return <TanStackWidget044 />; }
