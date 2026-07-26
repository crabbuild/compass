# VS Code Query Composer Footer Design

Date: 2026-07-25

## Goal

Improve the Query Codebase input so its primary action sits at the bottom of the
composer and CompassQL parameters begin at the left edge. The result must remain
compact, keyboard-friendly, and native to every VS Code theme.

## Approved layout

The textarea and its controls form one unified composer:

```text
┌──────────────────────────────────────────────────────────────────┐
│ MATCH (n) RETURN n LIMIT 20                                      │
│                                                                  │
├──────────────────────────────────────────────────────────────────┤
│ Parameters  [ kind=Function, module=api ]      ⌘ Enter    [ Run ]│
└──────────────────────────────────────────────────────────────────┘
```

Natural-language mode uses the same footer without the parameter control.
Example prompts remain immediately below the composer.

## Interaction

- Run becomes Cancel while a request is active.
- `Command+Enter` on macOS and `Control+Enter` elsewhere continue to execute.
- The footer shortcut is explanatory text, not a separate control.
- The existing query validation, execution, cancellation, parameters, and
  result behavior do not change.
- Focus on the textarea or parameter input gives the unified composer a visible
  VS Code focus border.

## Wide layout

- The composer owns the outer border and background.
- The textarea is borderless inside the composer.
- A one-pixel semantic divider separates the textarea from the footer.
- CompassQL parameters align left and use available width up to a readable
  maximum.
- Shortcut and Run or Cancel align right.

## Narrow layout

At editor widths at or below 760 CSS pixels:

- The footer wraps without horizontal document overflow.
- CompassQL parameters occupy a full first row.
- The shortcut and Run or Cancel remain right-aligned on the second row.
- The parameter label and input remain visible at 320 CSS pixels.
- No core action is hidden.

## Theme and accessibility

- Surfaces, input backgrounds, text, borders, buttons, disabled states, and
  focus rings use VS Code semantic custom properties.
- High-contrast themes use the existing contrast-border treatment.
- The textarea and parameter input retain their accessible names.
- Run and Cancel retain their current accessible labels.
- The layout does not depend on color, hover, or animation.

## Scope

This change modifies only the Query Codebase composer markup, its styles, and
focused browser coverage. It does not change query schemas, host messages,
result rendering, execution settings, or the query-mode tabs.

## Verification

Implementation precedes regression coverage; no TDD loop is used. Verification
includes:

- Query interaction Playwright coverage
- Wide and narrow composer geometry
- Natural-language and CompassQL modes
- Run and Cancel placement
- Theme and high-contrast checks
- Viewer and VS Code TypeScript checks
- Viewer and VS Code unit suites
- Production builds

