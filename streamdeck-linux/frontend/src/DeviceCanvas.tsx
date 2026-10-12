import type { CSSProperties, ReactNode } from "react";
import type { Controller, Snapshot } from "./model";
import {
  deviceName,
  geometry,
  hasKeyImages,
  hasEncoderDisplay,
} from "./device";
export function canvasStyle(snapshot: Snapshot): CSSProperties {
  const value = geometry(snapshot);
  return {
    "--model-columns": value.columns,
    "--model-rows": value.rows,
    "--model-dials": Math.max(1, value.dials),
    "--model-key-aspect":
      value.key_height > 0 ? value.key_width / value.key_height : 1,
    "--model-extra-height": `${value.dials > 0 ? Math.max(0, value.rows - 2) * 40 : 0}px`,
  } as CSSProperties;
}
export function DeviceCanvas({
  snapshot,
  renderSlot,
  onPage,
}: {
  snapshot: Snapshot;
  renderSlot: (controller: Controller, index: number) => ReactNode;
  onPage: (direction: number) => void;
}) {
  const value = geometry(snapshot);
  const neo = snapshot.device_model === "neo";
  return (
    <div
      className={`deck model-canvas${hasKeyImages(snapshot) ? "" : " pedal-canvas"}`}
      data-model={snapshot.device_model ?? "plus"}
      data-key-images={hasKeyImages(snapshot)}
      data-dials={value.dials > 0}
      data-encoder-display={hasEncoderDisplay(snapshot)}
      data-infobar={neo}
    >
      <div
        className="key-grid"
        style={{
          gridTemplateColumns: `repeat(${value.columns},minmax(0,1fr))`,
        }}
      >
        {Array.from({ length: value.keys }, (_, index) =>
          renderSlot("Keypad", index),
        )}
      </div>
      {value.dials > 0 && (
        <div
          className="dial-grid"
          style={{
            gridTemplateColumns: `repeat(${value.dials},minmax(0,1fr))`,
          }}
        >
          {Array.from({ length: value.dials }, (_, index) =>
            renderSlot("Encoder", index),
          )}
        </div>
      )}
      {neo && (
        <div className="neo-band">
          <button
            type="button"
            className="neo-sensor"
            aria-label="Previous page"
            disabled={snapshot.page_navigation?.previous === false}
            aria-disabled={snapshot.page_navigation?.previous === false}
            title="Previous page · left touch sensor"
            onClick={() => onPage(-1)}
          >
            ‹
          </button>
          {renderSlot("Neo", 1)}
          <button
            type="button"
            className="neo-sensor"
            aria-label="Next page"
            disabled={snapshot.page_navigation?.next === false}
            aria-disabled={snapshot.page_navigation?.next === false}
            title="Next page · right touch sensor"
            onClick={() => onPage(1)}
          >
            ›
          </button>
        </div>
      )}
      <span className="deck-wordmark">
        {deviceName(snapshot).toUpperCase()}
      </span>
    </div>
  );
}
