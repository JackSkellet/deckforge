import { useLayoutEffect, useRef, useState } from "react";
import type { Controller } from "./model";

export interface DroppedFile {
  path: string;
  kind: "action" | "profile" | "plugin" | "icon_pack";
}
export interface DropControl {
  controller: Controller;
  index: number;
}

function controlAt(x: unknown, y: unknown): DropControl | null {
  if (
    typeof x !== "number" ||
    typeof y !== "number" ||
    !Number.isFinite(x) ||
    !Number.isFinite(y)
  )
    return null;
  const element = document
    .elementFromPoint(x, y)
    ?.closest<HTMLElement>("[data-drop-controller]");
  const controller = element?.dataset.dropController;
  const index = Number(element?.dataset.dropIndex);
  return element &&
    ["Keypad", "Encoder", "Neo"].includes(controller ?? "") &&
    Number.isInteger(index) &&
    index >= 0
    ? { controller: controller as Controller, index }
    : null;
}
function file(value: any): DroppedFile | null {
  return typeof value?.path === "string" &&
    value.path.startsWith("/") &&
    ["action", "profile", "plugin", "icon_pack"].includes(value.kind)
    ? { path: value.path, kind: value.kind }
    : null;
}

/** Native paths stay outside browser File objects; ordinary action-library HTML
 * drag/drop continues to use its existing handlers. */
export function useNativeFileDrop(
  onDrop: (file: DroppedFile, target: DropControl | null) => Promise<void>,
  onError: (error: unknown) => void,
) {
  const drag = useRef<DroppedFile | null>(null);
  const [target, setTarget] = useState<DropControl | null>(null);
  useLayoutEffect(() => {
    const receive = (event: Event) => {
      const value = (event as CustomEvent).detail;
      if (!value || typeof value !== "object") return;
      if (value.phase === "leave") {
        drag.current = null;
        setTarget(null);
        return;
      }
      if (value.phase === "enter") drag.current = file(value);
      if (value.phase === "enter" || value.phase === "over") {
        const next =
          drag.current?.kind === "action" ? controlAt(value.x, value.y) : null;
        setTarget((current) =>
          current?.controller === next?.controller &&
          current?.index === next?.index
            ? current
            : next,
        );
        return;
      }
      if (value.phase !== "drop") return;
      drag.current = null;
      setTarget(null);
      if (typeof value.error === "string") {
        onError(new Error(value.error));
        return;
      }
      const dropped = file(value);
      if (!dropped) {
        onError(new Error("The dropped file is not supported."));
        return;
      }
      void onDrop(dropped, controlAt(value.x, value.y)).catch(onError);
    };
    window.addEventListener("deckforge-file-drag", receive);
    return () => window.removeEventListener("deckforge-file-drag", receive);
  }, [onDrop, onError]);
  return target;
}
