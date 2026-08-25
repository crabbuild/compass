import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget060() { return <span data-tanstack-widget="060" />; }
function loader060() { return { id: 60 }; }
export const Route = createFileRoute("/fixture060")({ component: TanStackWidget060, loader: loader060 });
export function TanStackApp060() { return <TanStackWidget060 />; }
