import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget070() { return <span data-tanstack-widget="070" />; }
function loader070() { return { id: 70 }; }
export const Route = createFileRoute("/fixture070")({ component: TanStackWidget070, loader: loader070 });
export function TanStackApp070() { return <TanStackWidget070 />; }
