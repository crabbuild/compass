import { useRef, type PointerEvent as ReactPointerEvent } from "react";
import {
  INSPECTOR_MAX_WIDTH,
  INSPECTOR_MIN_WIDTH,
  resizeInspectorByKeyboard,
  resizeInspectorFromPointer
} from "./inspectorLayout";

export function InspectorResizeHandle({
  width,
  onResize
}: {
  width: number;
  onResize(width: number): void;
}) {
  const dragging = useRef(false);

  const stopDragging = (event: ReactPointerEvent<HTMLDivElement>) => {
    dragging.current = false;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  return (
    <div
      className="compass-inspector-resizer"
      role="separator"
      aria-label="Resize graph inspector"
      aria-orientation="vertical"
      aria-valuemin={INSPECTOR_MIN_WIDTH}
      aria-valuemax={INSPECTOR_MAX_WIDTH}
      aria-valuenow={width}
      tabIndex={0}
      onPointerDown={(event) => {
        dragging.current = true;
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (!dragging.current) return;
        const workspace = event.currentTarget.parentElement;
        if (!workspace) return;
        onResize(resizeInspectorFromPointer(
          workspace.getBoundingClientRect().right,
          event.clientX
        ));
      }}
      onPointerUp={stopDragging}
      onPointerCancel={stopDragging}
      onKeyDown={(event) => {
        if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
        event.preventDefault();
        onResize(resizeInspectorByKeyboard(width, event.key));
      }}
    >
      <span aria-hidden="true" />
    </div>
  );
}
