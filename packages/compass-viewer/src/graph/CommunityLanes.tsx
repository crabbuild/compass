import { useMemo, useState } from "react";
import {
  laneRibbons,
  layoutCommunityLanes,
  type CommunityVariantData
} from "./communityVariants";
import { edgeSemanticCssColor } from "./semanticAppearance";
import { useElementSize } from "./useElementSize";
import { readableInk } from "../lib/color";
import { canvasColor, useThemeRevision } from "../lib/theme";

export const LANE_VIEWBOX = { width: 1_000, height: 620 } as const;
const LANE_GAP = 26;
const MINIMUM_LANE_HEIGHT = 54;
const MAXIMUM_LANE_HEIGHT = 96;

/**
 * Tiered ribbon map: communities are stacked into importance tiers, each tier a
 * proportional bar, and couplings cross between tiers as ribbons coloured by
 * relationship kind. It answers "how does the important layer couple down into
 * the rest", which a force layout cannot show because position carries no rank.
 */
export function CommunityLanes({
  data,
  onOpenCommunity
}: {
  data: CommunityVariantData;
  onOpenCommunity(communityId: number): void;
}) {
  const { ref, size } = useElementSize<HTMLDivElement>(LANE_VIEWBOX);
  const themeRevision = useThemeRevision();
  const inkBackdrop = useMemo(() => canvasColor(), [themeRevision]);
  const layout = useMemo(() => layoutCommunityLanes(data), [data]);
  const ribbons = useMemo(() => laneRibbons(data, layout), [data, layout]);
  const laneCount = Math.max(1, layout.lanes.length);
  const laneHeight = Math.max(
    MINIMUM_LANE_HEIGHT,
    Math.min(
      MAXIMUM_LANE_HEIGHT,
      (size.height - 40) / laneCount - LANE_GAP
    )
  );
  const [active, setActive] = useState<number | null>(null);
  const strongest = ribbons.reduce((largest, ribbon) => Math.max(largest, ribbon.link.weight), 1);
  const kindsKnown = data.links.some((link) => link.category !== "other");
  const laneY = (tier: number) => 20 + tier * (laneHeight + LANE_GAP);
  const bandHeight = laneY(laneCount - 1) + laneHeight + 12;
  const height = Math.max(size.height, bandHeight);
  const barX = (x: number) => 8 + x * (size.width - 16);
  const activeCommunity = active === null ? undefined : data.byId.get(active);

  return (
    <div className="compass-variant compass-lanes" data-variant="lanes" ref={ref}>
      <svg
        className="compass-lanes-canvas"
        viewBox={`0 0 ${size.width} ${height}`}
        role="group"
        aria-label="Community tier map"
        preserveAspectRatio="none"
      >
        {ribbons.map((ribbon) => {
          const sourceX = barX(ribbon.source.x + ribbon.source.width / 2);
          const targetX = barX(ribbon.target.x + ribbon.target.width / 2);
          const sourceY = laneY(ribbon.sourceTier) + laneHeight;
          const targetY = laneY(ribbon.targetTier);
          const top = ribbon.sourceTier <= ribbon.targetTier;
          const fromY = top ? sourceY : targetY + laneHeight;
          const toY = top ? targetY : sourceY + laneHeight;
          const midY = (fromY + toY) / 2;
          const highlighted = active !== null
            && (ribbon.link.source === active || ribbon.link.target === active);
          return (
            <path
              key={ribbon.link.id}
              className="compass-lane-ribbon"
              data-active={highlighted ? "true" : undefined}
              data-dim={active !== null && !highlighted ? "true" : undefined}
              d={`M ${sourceX} ${sourceY} C ${sourceX} ${midY}, ${targetX} ${midY}, ${targetX} ${targetY}`}
              stroke={edgeSemanticCssColor(ribbon.link.category)}
              strokeWidth={Math.min(
                12,
                1.4 + 3.4 * Math.sqrt(ribbon.link.weight / strongest)
              )}
            >
              <title>{`${data.byId.get(ribbon.link.source)?.label} ↔ ${data.byId.get(ribbon.link.target)?.label}: ${ribbon.link.relation}`}</title>
            </path>
          );
        })}
        {layout.lanes.map((lane) => (
          <g key={lane.tier} transform={`translate(0 ${laneY(lane.tier)})`}>
            <text className="compass-lane-tier" x={8} y={-6}>
              {lane.tier === layout.lanes.length - 1 && layout.omitted.communities > 0
                ? "tail"
                : `tier ${lane.tier + 1}`}
            </text>
            {lane.entries.map((entry) => {
              const tail = entry.community.id < 0;
              const width = Math.max(2, entry.width * (size.width - 16));
              const showLabel = width > 78;
              return (
                <g
                  key={entry.community.nodeId}
                  transform={`translate(${barX(entry.x)} 0)`}
                  data-tail={tail ? "true" : undefined}
                >
                  <rect
                    className="compass-lane-bar"
                    width={Math.max(0, width - 2)}
                    height={laneHeight}
                    rx={4}
                    fill={entry.community.color}
                    fillOpacity={tail ? 0.3 : active === entry.community.id ? 1 : 0.86}
                  />
                  {tail ? null : (
                    <rect
                      className="compass-lane-hit"
                      width={Math.max(2, width - 2)}
                      height={laneHeight}
                      tabIndex={0}
                      role="button"
                      aria-label={`${entry.community.label}, ${entry.community.memberCount.toLocaleString()} symbols`}
                      onMouseEnter={() => setActive(entry.community.id)}
                      onMouseLeave={() => setActive(null)}
                      onFocus={() => setActive(entry.community.id)}
                      onBlur={() => setActive(null)}
                      onClick={() => onOpenCommunity(entry.community.id)}
                      onKeyDown={(event) => {
                        if (event.key !== "Enter" && event.key !== " ") return;
                        event.preventDefault();
                        onOpenCommunity(entry.community.id);
                      }}
                    />
                  )}
                  {showLabel ? (
                    <>
                      <text
                        className="compass-lane-label"
                        x={8}
                        y={26}
                        style={tail
                          ? undefined
                          : { fill: readableInk(entry.community.color, inkBackdrop, 0.86) }}
                      >
                        {entry.community.label.length > 18
                          ? `${entry.community.label.slice(0, 17).trimEnd()}…`
                          : entry.community.label}
                      </text>
                      <text
                        className="compass-lane-count"
                        x={8}
                        y={laneHeight > 64 ? 44 : 40}
                        style={tail
                          ? undefined
                          : { fill: readableInk(entry.community.color, inkBackdrop, 0.86) }}
                      >
                        {tail
                          ? `${entry.community.memberCount.toLocaleString()} symbols`
                          : entry.community.memberCount.toLocaleString()}
                      </text>
                    </>
                  ) : null}
                </g>
              );
            })}
          </g>
        ))}
      </svg>
      <div className="compass-variant-footer" role="status">
        <span>
          {data.communities.length.toLocaleString()} communities ·{" "}
          {ribbons.length.toLocaleString()} couplings drawn · tier 1 is the most
          coupled and boundary-rich layer
          {layout.omitted.communities > 0
            ? ` · ${layout.omitted.communities.toLocaleString()} more in the tail`
            : ""}
        </span>
        {activeCommunity ? (
          <span className="compass-variant-detail">
            <strong>{activeCommunity.label}</strong>
            <span>
              {activeCommunity.memberCount.toLocaleString()} symbols ·{" "}
              {activeCommunity.degree.toLocaleString()} coupling
              {activeCommunity.degree === 1 ? "" : "s"}
            </span>
          </span>
        ) : (
          <span className="compass-variant-hint">
            {kindsKnown
              ? "Ribbon colour is the relationship kind · select a tier bar to open that community"
              : "This export records relationship counts without kinds · select a tier bar to open that community"}
          </span>
        )}
      </div>
    </div>
  );
}
