import { ImageResponse } from 'next/og';

import { siteUrl } from '@/lib/site';

export const alt = 'Compass — understand the codebase before changing it';
export const size = { width: 1200, height: 630 };
export const contentType = 'image/png';

export default function OpenGraphImage() {
  const displayUrl = siteUrl.replace(/^https?:\/\//, '');

  return new ImageResponse(
    (
      <div
        style={{
          background: '#f5f7ff',
          color: '#111827',
          display: 'flex',
          flexDirection: 'column',
          height: '100%',
          justifyContent: 'space-between',
          padding: '64px 72px',
          width: '100%',
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 18 }}>
          <div
            style={{
              alignItems: 'center',
              background: '#5865f2',
              borderRadius: 22,
              color: '#ffffff',
              display: 'flex',
              fontSize: 44,
              fontWeight: 700,
              height: 76,
              justifyContent: 'center',
              width: 76,
            }}
          >
            C
          </div>
          <div style={{ display: 'flex', fontSize: 34, fontWeight: 700 }}>Compass</div>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 20, maxWidth: 920 }}>
          <div style={{ color: '#5865f2', display: 'flex', fontSize: 24, fontWeight: 600, letterSpacing: 4, textTransform: 'uppercase' }}>
            Local-first code intelligence
          </div>
          <div style={{ display: 'flex', fontSize: 66, fontWeight: 700, letterSpacing: -3, lineHeight: 1.04 }}>
            Understand the codebase before changing it.
          </div>
          <div style={{ color: '#526079', display: 'flex', fontSize: 28, lineHeight: 1.3 }}>
            Explore source relationships, trace impact, and keep evidence attached.
          </div>
        </div>

        <div style={{ color: '#526079', display: 'flex', fontFamily: 'monospace', fontSize: 20 }}>
          {displayUrl} · native · inspectable
        </div>
      </div>
    ),
    size,
  );
}
