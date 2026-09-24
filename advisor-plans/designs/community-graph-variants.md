# Community graph design variants

Question this answers: **what should the code-graph overview look like, and
which reading should be the default?**

Written 2026-09-23 against commit `3fd246dc` plus the uncommitted Plan 026 work.
Measured on two real repositories exported by the local binary:

- `pallets/flask` — 118 files, 4,459 symbols, 174 communities, 49 cross-community
  relationships (export did not aggregate; the viewer derived the overview).
- `colinhacks/zod` — 616 files, 58,670 symbols, 2,641 communities, 157
  cross-community relationships (export aggregated to one node per community).

All four designs read the same validated model through one projection
(`communityVariants.ts`); none of them changes `graph.json`, the viewer schema,
or any query result.

## The four designs

| Design | Encodes | Answers | Fails at |
| --- | --- | --- | --- |
| **Bubbles** (canvas, existing) | position = packed importance, area = members, colour = community, edge colour = dominant relation kind | "What does the shape of this repository look like?" | More than a few hundred communities: the map becomes a texture, so labels carry everything |
| **Matrix** | row × column = community pair, cell saturation = exact relationship count, cell colour = dominant kind | "Who couples to whom, and how strongly?" | Communities beyond the importance cut; pair kinds are only known when the export kept them |
| **Area** (strip treemap) | tile area = symbols, order = importance, one disclosed tail tile | "Where do the symbols actually live, including the long tail?" | Coupling is not visible (hover only); tiles below ~70×30 px cannot be labelled |
| **Tiers** (lanes + ribbons) | lane = importance tier, bar width = symbols, ribbon width = couplings, ribbon colour = relation kind | "How does the important layer couple down into the rest?" | Reading order is imposed by the ranking, so spatial memory across rebuilds is weaker than bubbles |

## What the measurements showed

- **Flask (174 communities)** is the case all four designs handle. Bubbles give
  the best first impression; the matrix shows the top-left block of
  tightly-coupled Flask/AppContext/cli communities; the area map makes
  `test_basic.py` (375) and `setupmethod` (283) comparable at a glance and shows
  how much of the graph lives in the long tail; tiers make the "tier 1 couples
  down into everything" structure explicit (43 couplings drawn).
- **Zod (2,641 communities)** is where bubbles stop working. The area map stays
  readable: `util (core/util.ts:L1)` at 5,523 symbols dominates honestly, and
  the remaining 2,401 communities are disclosed as one tail tile with exact
  counts rather than 2,401 unreadable dots. The matrix stays useful as a
  coupling heat grid; the tiers view keeps the top 60 readable with everything
  else in the tail.
- **Exporter-aggregated graphs carry counts without kinds** (`"3 cross-community
  edges"`). The matrix and tiers render those cells neutrally and say so in the
  footer instead of implying the grey means something. Typed kinds appear when
  the viewer derives the overview itself (Plan 026's relationship mix) — closing
  that gap is what Plan 027's typed hierarchy artifact does.

## Recommendation

Keep all four, with **bubbles as the default** and the switcher one click away,
because they answer different questions and no single encoding survives both a
174-community and a 2,641-community repository:

- Orientation on a new repository → bubbles.
- "Why are these two subsystems coupled?" → matrix.
- "Where does the code actually live / what is the long tail?" → area.
- "What depends on the core layer?" → tiers.

The area map is the strongest candidate to become the default when a repository
exceeds roughly 500 communities, where bubbles stop being legible. That change
should follow reader testing rather than a guess; the threshold is a one-line
policy in `CompassGraph` (`variant` initial state) once the numbers are in.

## What is still open

- Per-variant label budgets were tuned by eye on two repositories; a third and
  fourth repository (Go, Java) should confirm the matrix cut (32) and the area
  tail threshold (240 visible tiles).
- The tier ranking currently uses the Plan 026 importance score. When Plan 027
  lands, tiers should read the hierarchy levels instead of the flat score.
- No a11y review has been done beyond keyboard-reachable cells, tiles, and bars
  plus `role`/`aria-label` coverage; a screen-reader pass on the matrix is the
  obvious next check.

## Status

Implemented in the shared viewer (Plan 026 branch of work, uncommitted):
`communityVariants.ts` (projection + layout math), `CommunityMatrix.tsx`,
`CommunityTreemap.tsx`, `CommunityLanes.tsx`, the `Overview design` switcher in
the graph toolbar, unit tests for the projection and layouts, and browser specs
that switch designs and drill in. Screenshots: `compass-ux-demo/shots/70–83`.
Latest captures after the narrow-width label fix: `shots/90–99`.

## Follow-up: reference styles from the reader (2026-09-23)

Two reference screenshots (dark canvas, dense force hairball with a few large
coloured hubs, and a black canvas with a coloured core plus a pale ring of tail
nodes) prompted a second pass. Verdict per idea:

| Reference idea | Verdict |
| --- | --- |
| Dark canvas as a first-class look | **Adopted.** The dark palette was deepened toward near-black, and standalone exports gained an `Auto / Light / Dark` control so a reader can pin it on a light system (and the reverse) without an editor host's theme being overridden. |
| Colour only the signal, grey the context | **Adopted.** Community bubbles that the importance budget labels keep their hue plus a soft halo; the long tail renders neutral, and hovering or selecting a context bubble reveals its community colour. This is what stops a 174-community map from reading as confetti. |
| Visible edge mesh under the nodes | **Partially there.** The Symbols canvas already draws the full mesh; the aggregated overview only has community-pair edges, so it cannot show the symbol mesh without inventing endpoints. Kept as is. |
| Pure ring/halo layout of tail nodes | **Not adopted.** It spends canvas on decoration and pushes labels off the readable band. The Automatic arrangement now groups by coupling instead, with a separation pass that keeps labels legible. |
| No labels at all (both references) | **Not adopted.** Both images are pictures, not tools: without labels there is no way back to source, and Compass's value is the exact, sourced graph. Labels stay bounded and ranked instead. |

Still open from this pass: the graph toolbar is dense once theme, scope,
design, layout, and camera controls share one row — at the workbench width the
right-hand icon buttons scroll out of view. The fix is to move rarely used
toggles (label and relationship-label switches) into the existing graph
settings panel rather than adding another row.

Resolved in the follow-up pass: label and relationship-label toggles, fit
selection, and reset view now live in the graph settings panel with `L` / `⇧ L`
shortcuts. The rail uses a container query on the graph stage, so below 1,240 px
it wraps onto a second row with the scope switch reduced to icons and the
breadcrumb moved down; measured at a 984 px stage, the row's hidden width went
from 467 px to 0. Hiding the visible scope labels also exposed an accessibility
hole — the buttons lost their accessible names — so both now carry explicit
`aria-label`s regardless of what fits on screen.
