import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget017() { return <span data-tanstack-widget="017" />; }
function loader017() { return { id: 17 }; }
export const Route = createFileRoute("/fixture017")({ component: TanStackWidget017, loader: loader017 });
export function TanStackApp017() { return <TanStackWidget017 />; }
