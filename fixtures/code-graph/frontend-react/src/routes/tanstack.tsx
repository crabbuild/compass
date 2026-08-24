import { createFileRoute } from '@tanstack/react-router';
import { Card } from '../components/Card';

function TanStackPage() {
  return <Card title="tanstack" />;
}

export const Route = createFileRoute('/tanstack')({
  component: TanStackPage,
  loader: async () => ({ ready: true }),
});
