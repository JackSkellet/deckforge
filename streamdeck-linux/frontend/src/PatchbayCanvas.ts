import { useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";
import { getFormDraft, setFormDraft } from "./transient";
export interface CanvasPoint {
  x: number;
  y: number;
}
interface Layout {
  positions: Record<string, CanvasPoint>;
  zoom: number;
  split: Record<string, boolean>;
}
const initial = (): Layout => {
  const saved = getFormDraft("audio-mixer-canvas");
  return {
    positions: saved?.positions ?? {},
    split: saved?.split ?? {},
    zoom: Math.max(0.25, Math.min(2, saved?.zoom ?? 1)),
  };
};
/** Canvas gestures change editor layout only. Routing is owned by the mixer draft. */
export function usePatchbayCanvas() {
  const [layout, setLayout] = useState(initial);
  const latest = useRef(layout);
  const viewport = useRef<HTMLDivElement>(null);
  const move = useRef<{
    id: string;
    start: CanvasPoint;
    point: CanvasPoint;
  } | null>(null);
  const pan = useRef<{ start: CanvasPoint; scroll: CanvasPoint } | null>(null);
  const update = (next: Layout, save = false) => {
    latest.current = next;
    setLayout(next);
    if (save) setFormDraft("audio-mixer-canvas", next);
  };
  const stop = () => {
    if (move.current) setFormDraft("audio-mixer-canvas", latest.current);
    move.current = null;
    pan.current = null;
  };
  return {
    layout,
    viewport,
    zoom: (value: number) =>
      update(
        { ...latest.current, zoom: Math.max(0.25, Math.min(2, value)) },
        true,
      ),
    split: (id: string) =>
      update(
        {
          ...latest.current,
          split: { ...latest.current.split, [id]: !latest.current.split[id] },
        },
        true,
      ),
    arrange: () => update({ ...latest.current, positions: {} }, true),
    nodeDown: (event: PointerEvent, id: string, point: CanvasPoint) => {
      if (event.button !== 0) return;
      event.stopPropagation();
      event.preventDefault();
      event.currentTarget.setPointerCapture?.(event.pointerId);
      move.current = {
        id,
        point,
        start: { x: event.clientX, y: event.clientY },
      };
    },
    nodeMove: (event: PointerEvent) => {
      const drag = move.current;
      if (!drag) return;
      const point = {
        x: Math.max(
          16,
          drag.point.x + (event.clientX - drag.start.x) / latest.current.zoom,
        ),
        y: Math.max(
          16,
          drag.point.y + (event.clientY - drag.start.y) / latest.current.zoom,
        ),
      };
      update({
        ...latest.current,
        positions: { ...latest.current.positions, [drag.id]: point },
      });
    },
    nodeKey: (event: KeyboardEvent, id: string, point: CanvasPoint) => {
      const direction: Record<string, CanvasPoint> = {
        ArrowLeft: { x: -1, y: 0 },
        ArrowRight: { x: 1, y: 0 },
        ArrowUp: { x: 0, y: -1 },
        ArrowDown: { x: 0, y: 1 },
      };
      const offset = direction[event.key];
      if (!offset) return;
      event.preventDefault();
      const step = event.shiftKey ? 50 : 10;
      update(
        {
          ...latest.current,
          positions: {
            ...latest.current.positions,
            [id]: {
              x: Math.max(16, point.x + offset.x * step),
              y: Math.max(16, point.y + offset.y * step),
            },
          },
        },
        true,
      );
    },
    panDown: (event: PointerEvent) => {
      if (
        ![0, 1].includes(event.button) ||
        (event.target as Element).closest(".patchbay-node,.patchbay-cable-hit")
      )
        return;
      const view = viewport.current;
      if (!view) return;
      event.preventDefault();
      event.currentTarget.setPointerCapture?.(event.pointerId);
      pan.current = {
        start: { x: event.clientX, y: event.clientY },
        scroll: { x: view.scrollLeft, y: view.scrollTop },
      };
    },
    panMove: (event: PointerEvent) => {
      const drag = pan.current,
        view = viewport.current;
      if (!drag || !view) return;
      view.scrollLeft = drag.scroll.x - event.clientX + drag.start.x;
      view.scrollTop = drag.scroll.y - event.clientY + drag.start.y;
    },
    stop,
  };
}
