import type { CSSProperties, ReactNode } from "react";
import type { Controller, Snapshot } from "./model";
import {
  deviceName,
  geometry,
  hasKeyImages,
  hasEncoderDisplay,
} from "./device";
function PhysicalDial({
  name,
  disabled,
  input,
}: {
  name: string;
  disabled: boolean;
  input: (event: string, ticks?: number) => void;
}) {
  return (
    <span>
      <span className="physical-dial-buttons">
        <button
          type="button"
          disabled={disabled}
          aria-label={`Turn ${name.toLowerCase()} left`}
          onClick={() => input("dialRotate", -1)}
        >
          ‹
        </button>
        <button
          type="button"
          disabled={disabled}
          className="physical-dial-push"
          aria-label={`Press ${name.toLowerCase()}`}
          onClick={() => input("click")}
        >
          <i />
        </button>
        <button
          type="button"
          disabled={disabled}
          aria-label={`Turn ${name.toLowerCase()} right`}
          onClick={() => input("dialRotate", 1)}
        >
          ›
        </button>
      </span>
      <button
        type="button"
        className="physical-dial-route"
        disabled={disabled}
        aria-label={`Switch ${name.toLowerCase()} row`}
        onClick={() => input("next")}
      >
        {name} ↕
      </button>
    </span>
  );
}
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
  onDialInput,
}: {
  snapshot: Snapshot;
  renderSlot: (controller: Controller, index: number) => ReactNode;
  onPage: (direction: number) => void;
  onDialInput?: (index: number, event: string, ticks?: number) => void;
}) {
  const value = geometry(snapshot);
  const neo = snapshot.device_model === "neo";
  const galleon = snapshot.device_model === "galleon";
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
          data-background={!!snapshot.encoder_background}
          style={{
            backgroundImage:
              galleon && snapshot.encoder_background
                ? `url(data:image/png;base64,${snapshot.encoder_background})`
                : undefined,
            backgroundSize: "100% 100%",
            gridTemplateColumns: `repeat(${value.encoder_columns ?? value.dials},minmax(0,1fr))`,
          }}
        >
          {Array.from({ length: value.dials }, (_, index) =>
            renderSlot("Encoder", index),
          )}
          {galleon && snapshot.encoder_markers && (
            <img
              className="encoder-route-markers"
              src={`data:image/png;base64,${snapshot.encoder_markers}`}
              alt=""
              aria-hidden="true"
            />
          )}
        </div>
      )}
      {galleon && (
        <div
          className="galleon-dials"
          role="group"
          aria-label="Physical dial controls"
        >
          {["Left dial", "Right dial"].map((name, index) =>
            onDialInput ? (
              <PhysicalDial
                key={`${snapshot.device}:${index}`}
                name={name}
                disabled={snapshot.encoder_routes?.[index] == null}
                input={(event, ticks) => onDialInput(index, event, ticks)}
              />
            ) : (
              <span key={name}>
                <i />
                {name}
              </span>
            ),
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
