import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  GraphTransitionScreen,
  SLOW_LAYOUT_DELAY_MS
} from "./GraphTransitionScreen";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("GraphTransitionScreen", () => {
  it("describes community loading without exposing an early escape action", () => {
    render(
      <GraphTransitionScreen kind="community" communityLabel="Request handling" />
    );

    expect(screen.getByRole("status").textContent).toContain(
      "Opening Request handling"
    );
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("offers the current graph when initial layout takes too long", () => {
    vi.useFakeTimers();
    const onShowGraph = vi.fn();
    render(<GraphTransitionScreen kind="layout" onShowGraph={onShowGraph} />);

    expect(screen.getByRole("heading").textContent).toBe(
      "Arranging graph layout"
    );
    expect(screen.queryByRole("button")).toBeNull();

    act(() => {
      vi.advanceTimersByTime(SLOW_LAYOUT_DELAY_MS);
    });

    expect(screen.getByRole("heading").textContent).toBe(
      "Still arranging this graph"
    );
    fireEvent.click(screen.getByRole("button", { name: "Show graph now" }));
    expect(onShowGraph).toHaveBeenCalledOnce();
  });
});
