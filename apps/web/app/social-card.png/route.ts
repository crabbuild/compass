import OpenGraphImage from '../opengraph-image';

export const runtime = 'edge';

export function GET() {
  return OpenGraphImage();
}
