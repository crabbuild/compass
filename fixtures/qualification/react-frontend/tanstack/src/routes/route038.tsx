import { createFileRoute } from "@tanstack/react-router";

function TanStackWidget038() { return <span data-tanstack-widget="038" />; }
function loader038() { return { id: 38 }; }
export const Route = createFileRoute("/fixture038")({ component: TanStackWidget038, loader: loader038 });
export function TanStackApp038() { return <TanStackWidget038 />; }
