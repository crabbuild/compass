import { useMemo, useState } from "react";
import {
  communityMatrixSelection,
  type CommunityVariantCommunity,
  type CommunityVariantData,
  type CommunityVariantLink
} from "./communityVariants";
import { edgeSemanticCssColor } from "./semanticAppearance";

export const MATRIX_COMMUNITY_LIMIT = 32;

/**
 * Coupling matrix: rows are "who this community talks to", columns are "who
 * talks to it". The same relationship fills both (the overview pairs are
 * undirected), so a reader can scan either axis. Cell colour is the dominant
 * relationship category and saturation is the exact count.
 */
export function CommunityMatrix({
  data,
  limit = MATRIX_COMMUNITY_LIMIT,
  onOpenCommunity
}: {
  data: CommunityVariantData;
  limit?: number;
  onOpenCommunity(communityId: number): void;
}) {
  const { shown, omitted } = useMemo(
    () => communityMatrixSelection(data, limit),
    [data, limit]
  );
  const [active, setActive] = useState<string | null>(null);
  const links = useMemo(() => {
    const byPair = new Map<string, CommunityVariantLink>();
    for (const link of data.links) {
      byPair.set(`${link.source}:${link.target}`, link);
      byPair.set(`${link.target}:${link.source}`, link);
    }
    return byPair;
  }, [data.links]);
  const strongest = useMemo(
    () => data.links.reduce((largest, link) => Math.max(largest, link.weight), 1),
    [data.links]
  );
  // Exporter-aggregated overviews record counts without relationship kinds.
  // Say so rather than implying the neutral colour means something.
  const kindsKnown = useMemo(
    () => data.links.some((link) => link.category !== "other"),
    [data.links]
  );
  const activeLink = active ? links.get(active) : undefined;
  const activePair = active?.split(":").map(Number) ?? [];

  return (
    <div className="compass-variant compass-matrix" data-variant="matrix">
      <div className="compass-variant-scroll">
        <table role="grid" aria-label="Community coupling matrix">
          <caption className="sr-only">
            Community coupling. Rows are relationships from a community, columns
            are relationships to a community.
          </caption>
          <thead>
            <tr>
              <th scope="col" aria-label="Community" />
              {shown.map((community, column) => (
                <th
                  key={community.id}
                  scope="col"
                  data-active={activePair[1] === community.id ? "true" : undefined}
                  title={`${community.label} · ${community.memberCount.toLocaleString()} symbols`}
                >
                  <span className="compass-matrix-column-label">
                    {column + 1}
                  </span>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {shown.map((row) => (
              <tr key={row.id} data-active={activePair[0] === row.id ? "true" : undefined}>
                <th scope="row">
                  <span
                    className="compass-matrix-dot"
                    aria-hidden="true"
                    style={{ background: row.color }}
                  />
                  <span className="compass-matrix-row-label">{row.label}</span>
                  <small>{row.memberCount.toLocaleString()}</small>
                </th>
                {shown.map((column) => {
                  const self = row.id === column.id;
                  const link = self ? undefined : links.get(`${row.id}:${column.id}`);
                  const intensity = link
                    ? Math.min(0.94, 0.22 + 0.72 * Math.sqrt(link.weight / strongest))
                    : 0;
                  const key = `${row.id}:${column.id}`;
                  return (
                    <td
                      key={key}
                      data-self={self ? "true" : undefined}
                      data-filled={link ? "true" : undefined}
                    >
                      <button
                        type="button"
                        className="compass-matrix-cell"
                        aria-label={self
                          ? `${row.label}, ${row.memberCount.toLocaleString()} symbols`
                          : link
                            ? `${row.label} to ${column.label}: ${link.relation}`
                            : `${row.label} to ${column.label}: no direct relationship`}
                        disabled={self || link === undefined}
                        style={link
                          ? {
                            "--compass-matrix-fill": edgeSemanticCssColor(link.category),
                            "--compass-matrix-intensity": intensity
                          } as React.CSSProperties
                          : undefined}
                        onMouseEnter={() => setActive(link ? key : null)}
                        onMouseLeave={() => setActive(null)}
                        onFocus={() => setActive(link ? key : null)}
                        onBlur={() => setActive(null)}
                        onClick={() => onOpenCommunity(link ? column.id : row.id)}
                      >
                        <span className="sr-only">
                          {link ? `${link.weight} relationships` : "No relationship"}
                        </span>
                        {link ? (
                          <i aria-hidden="true" style={{ opacity: intensity }} />
                        ) : null}
                      </button>
                      {self ? <span className="compass-matrix-self" aria-hidden="true" /> : null}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="compass-variant-footer" role="status">
        <span>
          {shown.length.toLocaleString()} of {data.communities.length.toLocaleString()} communities
          {omitted > 0 ? ` · ${omitted.toLocaleString()} more below the importance cut` : ""}
        </span>
        {activeLink && activePair.length === 2 ? (
          <span className="compass-variant-detail">
            <strong>
              {data.byId.get(activePair[0]!)?.label} → {data.byId.get(activePair[1]!)?.label}
            </strong>
            <span>{activeLink.relation}</span>
          </span>
        ) : (
          <span className="compass-variant-hint">
            {kindsKnown
              ? "Cell colour is the dominant relationship kind · select a cell to open that community"
              : "This export records relationship counts without kinds · select a cell to open that community"}
          </span>
        )}
      </div>
    </div>
  );
}

export type { CommunityVariantCommunity };
