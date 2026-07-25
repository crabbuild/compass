# Make Compass views feel native in VS Code

Date: 2026-07-25

## Goal

This design makes every Compass webview inherit the active Visual Studio Code (VS Code) theme and interaction model. It repairs Architecture Flow, Ask Codebase, and Codebase Evolution while preserving existing Compass command-line interface (CLI) contracts.

The work targets developers who explore large repositories inside VS Code. They need readable data, predictable controls, responsive layouts, and recoverable failure states without leaving the editor.

## Success criteria

The implementation must meet these outcomes:

- Every ordinary surface, control, border, state, and focus ring follows VS Code semantic theme tokens
- Light, dark, and high-contrast themes remain readable without a separate Compass theme
- Compass branding appears only in the product mark and graph data colors
- Graph and Architecture Flow loading states show balanced, informative progress
- Architecture Flow supports global search, usable symbol cards, and searchable paginated calls
- Ask Codebase presents readable inputs, progress, errors, and structured results
- Codebase Evolution loads available revisions on selection and guides unavailable revisions through graph creation
- Narrow editor columns preserve every core action
- Keyboard navigation, reduced motion, and assistive technology receive equivalent behavior

## Scope

This change covers the shared viewer theme and the Graph, Call Graph, Architecture Flow, Ask Codebase, and Codebase Evolution webviews. It includes small shared components and pure state utilities when they remove duplicated behavior.

The implementation preserves the current graph, call-flow, query, and history schemas. Additive webview host messages are allowed for retry, cancellation, progress, or output-channel actions.

The work does not redesign native VS Code tree views, change Compass graph generation, add remote services, or replace the graph rendering engine.

## Visual system

VS Code semantic variables define the complete application chrome. Compass aliases may supply fallbacks for offline fixtures, but they must not create an independent light or dark palette.

The token layer maps these roles:

- **Surfaces**: editor, sidebar, panel, menu, hover widget, and input backgrounds
- **Text**: editor foreground, sidebar foreground, descriptions, disabled text, links, and errors
- **Interaction**: buttons, list selection, hover states, focus borders, input borders, and progress indicators
- **Structure**: panel borders, contrast borders, table separators, and sticky headers
- **Status**: success, warning, error, information, and pending states

Ordinary interface surfaces use flat fills, VS Code density, and restrained radii from 3 to 6 px. Decorative gradients, glass effects, heavy shadows, and fixed foreground colors do not appear in application chrome.

Compass community and graph colors remain visible as data. Theme-aware outlines, labels, and selection halos preserve their contrast. High-contrast themes use `--vscode-contrastBorder` and `--vscode-contrastActiveBorder` where available.

## Shared interface components

The viewer package owns reusable presentation primitives:

- A workspace header with title, context, actions, and optional search
- A VS Code-native search field with result count and clear action
- A segmented control for compact mutually exclusive modes
- Loading, empty, error, and unavailable states
- A data table shell with sticky headers, sorting, and keyboard focus
- Pagination with previous, next, page count, and visible row range
- Status and evidence badges that do not rely on color alone

Pure utilities own filtering, grouping, sorting, and pagination. Components receive already-derived view data where practical.

## Loading and failure states

Graph loading uses a centered 48 px Compass mark, a native progress treatment, one primary status, and three short processing stages. The mark remains visually balanced inside its container. Reduced-motion mode removes ambient animation.

Architecture Flow uses the same product mark and status language. Its loader also shows a low-detail skeleton of the section rail, flow summary, and content rows. This preview communicates the destination layout without presenting fake progress.

Loading copy names the real operation. Examples include `Reading graph`, `Deriving subsystem flows`, and `Preparing symbol index`.

Recoverable errors keep the same shell and expose the next relevant action:

- **Retry** repeats the failed request in the current tab
- **Show Compass output** opens diagnostic output
- **Build graph** starts a missing revision build
- **Revise query** returns focus to the query editor

## Architecture Flow

Architecture Flow uses a two-column workspace on wide editor columns. The left rail contains subsystem navigation, counts, and repository statistics. The main area contains global search, system flows, and selected-subsystem details.

Global search scans:

- Subsystem names
- Symbol names and kinds
- Source paths
- Caller and callee names
- Relationship and evidence labels

Results group by subsystem. Selecting a result activates its subsystem and the matching Symbols or Calls tab. Search remains global even after a subsystem selection.

The system-flow overview presents readable source, direction, target, and call count rows. It uses graph accent colors as data markers while text and borders follow VS Code tokens. Large result sets use progressive disclosure.

The Symbols tab presents cards with symbol name, kind, subsystem, and source path. The source path acts as a clear navigation control. Search and pagination prevent thousands of cards from mounting at once. The default page size is 24.

The Calls tab presents a sticky-header table with caller, relation, callee, and evidence columns. Global and local filters match caller, callee, relation, source, subsystem, and evidence. Users can sort text columns and paginate through 25 rows per page. The footer reports the visible row range and total matches.

Each tab preserves its search, sort, and page state while the user switches tabs or subsystems. Changing a filter resets only the affected page to the first page.

## Ask Codebase

Ask Codebase follows a compact VS Code workbench pattern. The header identifies the working tree or historical revision. A segmented control selects Natural Language or CompassQL mode.

The query editor uses editor and input tokens, readable placeholder text, a visible `⌘ Enter` or `Ctrl Enter` shortcut, and a Run or Cancel action. Natural-language mode shows focused example prompts. CompassQL mode exposes parameters without competing with the primary editor.

The workspace displays one clear state at a time:

- An instructional empty state with example queries
- A cancellable running state with the current operation
- A nearby error state with recovery
- A text answer with readable wrapping
- A structured result table when the response contains consistent records
- A formatted raw JSON fallback for irregular data

Session query history remains local to the webview. Reusing a prior query fills the editor without executing it.

## Codebase Evolution

Codebase Evolution uses a two-pane revision browser. The left timeline shows commit subject, short hash, author, date, and graph state. Search matches these fields and graph-state labels.

Selecting a commit changes the active revision immediately:

- If its graph exists, the webview loads it automatically
- If its graph is missing, the graph area shows the build action and available profiles
- If a build runs, the graph area shows progress and cancellation
- If a build fails, the same area explains the failure and offers retry

The selected commit's metadata and structural change counts stay visible above the graph. Parent comparison is an explicit mode with added, removed, and changed counts plus semantic findings.

The host tags revision, comparison, count, and community responses with the requested commit. The webview ignores responses that no longer match the selected commit. Selection changes clear stale graph and comparison presentation immediately.

If the history command fails before hydration, the panel still renders a recoverable error. Disabled history, an empty repository, a missing parent graph, and an unavailable CLI each receive a specific explanation and action.

At narrow widths, the timeline becomes a compact revision selector above the details. The graph and every action remain available.

## Data flow and ownership

The VS Code extension host owns CLI execution, cancellation, output logging, request generations, and source navigation. React webviews own presentation state such as selected tabs, filters, sorting, pagination, and active revision.

The shared `@compass/viewer` package owns theme aliases, common presentation components, and pure view-state utilities. VS Code entry points hydrate these components and translate host messages.

Architecture search indexes the hydrated call-flow model in memory. It does not add a CLI request per keystroke. Codebase Evolution loads revision graphs through the existing revision store and rejects stale responses through commit identity checks.

## Accessibility and responsive behavior

Every interactive element has a visible focus state and an accessible name. Tables expose semantic headers. Search results, tabs, listboxes, and pagination support keyboard operation.

Status icons include text labels or accessible names. Evidence and graph states never rely on color alone. Loading and result counts use polite live regions without announcing decorative motion.

Layouts adapt at content-driven breakpoints. Wide columns use side-by-side navigation and content. Narrow columns stack controls and use horizontal overflow only for data tables. Reduced-motion preferences disable nonessential animation.

## Testing strategy

Behavior changes follow a red, green, refactor loop. Pure utilities receive unit coverage before component integration.

Automated coverage includes:

- Theme token aliases and removal of undefined custom variables
- Architecture global search, grouped navigation, symbol pagination, call filtering, sorting, and pagination
- Ask Codebase keyboard execution, cancellation, error recovery, and structured result normalization
- Evolution automatic loading, missing-graph actions, build progress, cancellation, retry, comparison, and stale-response rejection
- Loading balance, error recovery, reduced motion, and high-contrast borders
- Keyboard access and automated accessibility checks
- Representative light, dark, high-contrast, narrow, and large fixture layouts

Verification runs viewer and extension unit tests, TypeScript checks, production builds, relevant Playwright suites, accessibility tests, and Visual Studio Extension package smoke checks. The final implementation runs `graphify update .` after code changes.

## Integration constraints

The working tree may contain unfinished Graph, Call Graph, loading, inspector, and viewer-theme changes. Implementation must inspect and preserve those changes. It may refactor them only when the approved design requires it.

No implementation step may overwrite unrelated user changes or generated repository artifacts. Build outputs update only through existing project scripts.

## Non-goals

- Replacing VS Code-native Repository or Operations trees with webviews
- Introducing a custom Compass theme selector
- Automatically building every historical revision
- Changing Compass CLI query languages or graph semantics
- Adding server-side search for hydrated Architecture data
- Replacing the current graph visualization library
