import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget022() { return <span data-tanstack-widget="022" />; }
function loader022() { return { id: 22 }; }
export const Route = createFileRoute("/fixture022")({ component: TanStackWidget022, loader: loader022 });
export function TanStackApp022() { return <TanStackWidget022 />; }
