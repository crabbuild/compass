import { useEffect, useRef, useState, type RefObject } from "react";

/**
 * Track an element's layout box so a variant can lay itself out in the pane the
 * reader actually has, instead of being letterboxed inside a fixed viewBox.
 * Falls back to the supplied size when layout measurement is unavailable.
 */
export function useElementSize<T extends HTMLElement>(fallback: {
  width: number;
  height: number;
}): { ref: RefObject<T | null>; size: { width: number; height: number } } {
  const ref = useRef<T | null>(null);
  const [size, setSize] = useState(fallback);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => {
      const width = element.clientWidth;
      const height = element.clientHeight;
      if (width <= 0 || height <= 0) return;
      setSize((current) => current.width === width && current.height === height
        ? current
        : { width, height });
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return { ref, size };
}
