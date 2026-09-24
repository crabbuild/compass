import { useMemo, useState } from "react";
import {
  layoutCommunityTreemap,
  TREEMAP_COMMUNITY_LIMIT,
  TREEMAP_VIEWBOX,
  type CommunityVariantData
} from "./communityVariants";
import { useElementSize } from "./useElementSize";
import { readableInk } from "../lib/color";
import { canvasColor, useThemeRevision } from "../lib/theme";

const TILE_OPACITY = 0.82;

/**
 * Area treemap: every tile's area is proportional to its symbol count, ordered
 * by importance, with one disclosed tail tile for the communities below the
 * render bound. This is the variant that stays honest when a repository has
 * thousands of communities — the long tail is visible as small area, not as a
 * dot field.
 */
export function CommunityTreemap({
  data,
  onOpenCommunity
}: {
  data: CommunityVariantData;
  onOpenCommunity(communityId: number): void;
}) {
  const { ref, size } = useElementSize<HTMLDivElement>(TREEMAP_VIEWBOX);
  const themeRevision = useThemeRevision();
  // Label ink is chosen against the tile as it actually appears on this canvas.
  const inkBackdrop = useMemo(() => canvasColor(), [themeRevision]);
  const layout = useMemo(
    () => layoutCommunityTreemap(data, TREEMAP_COMMUNITY_LIMIT, size),
    [data, size]
  );
  const [active, setActive] = useState<number | null>(null);
  const activeCommunity = active === null ? undefined : data.byId.get(active);
  const activeCoupling = useMemo(() => {
    if (active === null) return [];
    return data.links
      .filter((link) => link.source === active || link.target === active)
      .sort((left, right) =>
        right.weight - left.weight
        || left.id.localeCompare(right.id))
      .slice(0, 3);
  }, [active, data.links]);

  return (
    <div className="compass-variant compass-treemap" data-variant="treemap" ref={ref}>
      <svg
        className="compass-treemap-canvas"
        viewBox={`0 0 ${size.width} ${size.height}`}
        role="group"
        aria-label="Community area map"
        preserveAspectRatio="none"
      >
        {layout.tiles.map((tile) => {
          const tail = tile.community.id < 0;
          const labelFits = tile.width > 96 && tile.height > 46;
          const countFits = tile.width > 70 && tile.height > 30;
          return (
            <g
              key={tile.community.nodeId}
              transform={`translate(${tile.x} ${tile.y})`}
              data-tail={tail ? "true" : undefined}
              data-active={active === tile.community.id ? "true" : undefined}
            >
              <rect
                width={Math.max(0, tile.width - 1.5)}
                height={Math.max(0, tile.height - 1.5)}
                rx={3}
                fill={tile.community.color}
                fillOpacity={tail ? 0.32 : 0.82}
              />
              {tail ? null : (
                <rect
                  className="compass-treemap-hit"
                  width={tile.width}
                  height={tile.height}
                  tabIndex={0}
                  role="button"
                  aria-label={`${tile.community.label}, ${tile.community.memberCount.toLocaleString()} symbols`}
                  onMouseEnter={() => setActive(tile.community.id)}
                  onMouseLeave={() => setActive(null)}
                  onFocus={() => setActive(tile.community.id)}
                  onBlur={() => setActive(null)}
                  onClick={() => onOpenCommunity(tile.community.id)}
                  onKeyDown={(event) => {
                    if (event.key !== "Enter" && event.key !== " ") return;
                    event.preventDefault();
                    onOpenCommunity(tile.community.id);
                  }}
                />
              )}
              {labelFits ? (
                <text
                  className="compass-treemap-label"
                  x={7}
                  y={20}
                  style={tail
                    ? undefined
                    : { fill: readableInk(tile.community.color, inkBackdrop, TILE_OPACITY) }}
                >
                  {tile.community.label}
                </text>
              ) : null}
              {countFits ? (
                <text
                  className="compass-treemap-count"
                  x={7}
                  y={labelFits ? 36 : 22}
                  style={tail
                    ? undefined
                    : { fill: readableInk(tile.community.color, inkBackdrop, TILE_OPACITY) }}
                >
                  {tile.community.memberCount.toLocaleString()} symbols
                </text>
              ) : null}
            </g>
          );
        })}
      </svg>
      <div className="compass-variant-footer" role="status">
        <span>
          {data.communities.length.toLocaleString()} communities ·{" "}
          {data.symbols.toLocaleString()} symbols · area is symbol count
          {layout.omitted.communities > 0
            ? ` · ${layout.omitted.communities.toLocaleString()} smallest disclosed as one tile`
            : ""}
        </span>
        {activeCommunity ? (
          <span className="compass-variant-detail">
            <strong>{activeCommunity.label}</strong>
            <span>{activeCommunity.memberCount.toLocaleString()} symbols</span>
            {activeCoupling.map((link) => (
              <span key={link.id}>
                ↔ {data.byId.get(link.source === activeCommunity.id ? link.target : link.source)?.label}
                {" "}{link.relation}
              </span>
            ))}
          </span>
        ) : (
          <span className="compass-variant-hint">
            Hover a tile for its couplings · select a tile to open that community
          </span>
        )}
      </div>
    </div>
  );
}
