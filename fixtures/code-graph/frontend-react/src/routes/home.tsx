import { createRoutesFromElements, Route } from 'react-router-dom';
import { Card } from '../components/Card';

export function HomeRoute() {
  return <Card title="route" />;
}

export const routes = createRoutesFromElements(
  <Route path="/home" element={<HomeRoute />} />,
);
