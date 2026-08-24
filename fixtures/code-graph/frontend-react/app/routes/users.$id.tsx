import { Card } from '../../src/components/Card';

export async function loader() {
  return { ready: true };
}

export default function UserRoute() {
  return <Card title="user" />;
}
