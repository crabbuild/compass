import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget045() { return <span data-tanstack-widget="045" />; }
function loader045() { return { id: 45 }; }
export const Route = createFileRoute("/fixture045")({ component: TanStackWidget045, loader: loader045 });
export function TanStackApp045() { return <TanStackWidget045 />; }
