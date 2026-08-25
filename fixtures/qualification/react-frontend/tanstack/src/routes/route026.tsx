import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget026() { return <span data-tanstack-widget="026" />; }
function loader026() { return { id: 26 }; }
export const Route = createFileRoute("/fixture026")({ component: TanStackWidget026, loader: loader026 });
export function TanStackApp026() { return <TanStackWidget026 />; }
