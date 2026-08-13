import { ImageResponse } from 'next/og';

import { siteUrl } from '@/lib/site';

export const alt = 'Compass — understand the codebase before changing it';
export const size = { width: 1200, height: 630 };
export const contentType = 'image/png';

export default function OpenGraphImage() {
  const displayUrl = siteUrl.replace(/^https?:\/\//, '');

  return new ImageResponse(
    <div
      style={{
        background:
          'linear-gradient(135deg, #0b0d14 0%, #121625 58%, #1b2040 100%)',
        color: '#f8f9ff',
        display: 'flex',
        height: '100%',
        overflow: 'hidden',
        padding: '58px 64px',
        position: 'relative',
        width: '100%',
      }}
    >
      <div
        style={{
          background:
            'radial-gradient(circle, rgba(123, 132, 255, 0.34) 0%, rgba(88, 101, 242, 0) 70%)',
          display: 'flex',
          height: 660,
          position: 'absolute',
          right: -210,
          top: -260,
          width: 660,
        }}
      />

      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          justifyContent: 'space-between',
          width: 720,
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 18 }}>
          <div
            style={{
              alignItems: 'center',
              background: '#5865f2',
              border: '1px solid rgba(255, 255, 255, 0.24)',
              borderRadius: 20,
              color: '#ffffff',
              display: 'flex',
              height: 70,
              justifyContent: 'center',
              width: 70,
            }}
          >
            <svg width="44" height="44" viewBox="0 0 24 24" fill="none">
              <path
                fill="currentColor"
                fillRule="evenodd"
                clipRule="evenodd"
                d="M3.554 21.529c1.797 1.221 4.943-.038 11.236-2.554 1.342-.537 2.013-.806 2.54-1.267q.201-.177.378-.378c.461-.527.73-1.198 1.267-2.54 2.515-6.293 3.775-9.44 2.554-11.236a4.1 4.1 0 0 0-1.083-1.083c-1.797-1.221-4.944.037-11.236 2.554-1.342.537-2.013.806-2.54 1.267q-.201.177-.378.378c-.461.527-.73 1.198-1.267 2.54-2.517 6.292-3.775 9.439-2.554 11.236.29.426.657.793 1.083 1.083M8.25 12a3.75 3.75 0 1 1 7.5 0 3.75 3.75 0 0 1-7.5 0m1.5 0a2.25 2.25 0 1 1 4.5 0 2.25 2.25 0 0 1-4.5 0"
              />
            </svg>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            <div
              style={{
                display: 'flex',
                fontSize: 32,
                fontWeight: 700,
                letterSpacing: -1,
              }}
            >
              Compass
            </div>
            <div
              style={{
                color: '#9aa6c4',
                display: 'flex',
                fontSize: 16,
                letterSpacing: 2.5,
                textTransform: 'uppercase',
              }}
            >
              Local-first code intelligence
            </div>
          </div>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 22 }}>
          <div
            style={{
              display: 'flex',
              fontSize: 60,
              fontWeight: 700,
              letterSpacing: -3,
              lineHeight: 1.02,
            }}
          >
            Understand the codebase before changing it.
          </div>
          <div
            style={{
              color: '#aeb8d1',
              display: 'flex',
              fontSize: 25,
              lineHeight: 1.35,
              maxWidth: 680,
            }}
          >
            Trace relationships, inspect impact, and keep every answer attached
            to evidence.
          </div>
        </div>

        <div
          style={{
            alignItems: 'center',
            color: '#aeb8d1',
            display: 'flex',
            fontFamily: 'monospace',
            fontSize: 18,
            gap: 12,
          }}
        >
          <div
            style={{
              background: '#55d6a4',
              borderRadius: 999,
              display: 'flex',
              height: 9,
              width: 9,
            }}
          />
          {displayUrl} · native · inspectable
        </div>
      </div>

      <div
        style={{
          alignItems: 'center',
          background: 'rgba(15, 18, 31, 0.82)',
          border: '1px solid rgba(158, 168, 255, 0.24)',
          borderRadius: 28,
          display: 'flex',
          height: 470,
          justifyContent: 'center',
          marginLeft: 44,
          marginTop: 26,
          position: 'relative',
          width: 326,
        }}
      >
        <div
          style={{
            color: '#7784a9',
            display: 'flex',
            fontFamily: 'monospace',
            fontSize: 13,
            left: 22,
            position: 'absolute',
            top: 20,
          }}
        >
          graph / overview
        </div>
        <svg width="290" height="360" viewBox="0 0 290 360" fill="none">
          <g stroke="#5865f2" strokeOpacity="0.48" strokeWidth="2">
            <path d="M38 177L98 105L151 164L221 80" />
            <path d="M38 177L109 251L151 164L235 238" />
            <path d="M98 105L151 164L235 238" />
          </g>
          <g fill="#1a2037" stroke="#7b84ff" strokeWidth="3">
            <circle cx="38" cy="177" r="13" />
            <circle cx="98" cy="105" r="16" />
            <circle cx="109" cy="251" r="14" />
            <circle cx="151" cy="164" r="22" fill="#5865f2" />
            <circle cx="221" cy="80" r="14" />
            <circle cx="235" cy="238" r="17" />
          </g>
        </svg>
        {[
          { label: 'files', left: 22, top: 232 },
          { label: 'extract', left: 78, top: 98 },
          { label: 'resolve', left: 78, top: 304 },
          { label: 'graph', left: 139, top: 188, color: '#ffffff' },
          { label: 'query', left: 211, top: 73 },
          { label: 'impact', left: 221, top: 292 },
        ].map((node) => (
          <div
            key={node.label}
            style={{
              color: node.color ?? '#c9d0e5',
              display: 'flex',
              fontFamily: 'monospace',
              fontSize: 12,
              left: node.left,
              position: 'absolute',
              top: node.top,
            }}
          >
            {node.label}
          </div>
        ))}
        <div
          style={{
            bottom: 20,
            color: '#7784a9',
            display: 'flex',
            fontFamily: 'monospace',
            fontSize: 12,
            position: 'absolute',
            right: 22,
          }}
        >
          12 nodes · 12 edges
        </div>
      </div>
    </div>,
    {
      ...size,
      headers: {
        'Cache-Control': 'public, max-age=3600, s-maxage=31536000, immutable',
      },
    },
  );
}
