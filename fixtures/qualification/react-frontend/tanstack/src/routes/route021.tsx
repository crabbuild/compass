import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget021() { return <span data-tanstack-widget="021" />; }
function loader021() { return { id: 21 }; }
export const Route = createFileRoute("/fixture021")({ component: TanStackWidget021, loader: loader021 });
export function TanStackApp021() { return <TanStackWidget021 />; }
