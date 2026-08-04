import { ImageResponse } from 'next/og';

export const size = { width: 180, height: 180 };
export const contentType = 'image/png';

export default function AppleIcon() {
  return new ImageResponse(
    (
      <div
        style={{
          alignItems: 'center',
          background: '#5865f2',
          borderRadius: 42,
          color: '#ffffff',
          display: 'flex',
          fontSize: 106,
          fontWeight: 700,
          height: '100%',
          justifyContent: 'center',
          width: '100%',
        }}
      >
        C
      </div>
    ),
    size,
  );
}
