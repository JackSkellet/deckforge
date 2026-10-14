import { useEffect, useRef, useState } from "react";
import { pick, request } from "./bridge";
import { Modal } from "./Panels";
import { fit, pan, slider, zoom, type CropEdit, type CropRect } from "./crop";
interface Source {
  preview: string;
  width: number;
  height: number;
  frames: number;
  frame: number;
  source_hash: string;
}
export function ImageCropper({
  edit,
  onChange,
  onClose,
  onSaved,
}: {
  edit: CropEdit;
  onChange: (edit: CropEdit) => void;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [source, setSource] = useState<{
      key: string;
      path?: string;
      value: Source;
    } | null>(null),
    [error, setError] = useState(""),
    [loading, setLoading] = useState(false),
    [saving, setSaving] = useState(false);
  const change = useRef(onChange);
  change.current = onChange;
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const key = JSON.stringify([
    edit.context,
    edit.device,
    edit.path,
    edit.frame,
  ]);
  useEffect(() => {
    if (!edit.path) return;
    let live = true;
    setLoading(true);
    setError("");
    request<Source>("image_crop", {
      mode: "load",
      context: edit.context,
      device: edit.device,
      path: edit.path,
      frame: edit.frame,
      source_hash: edit.source_hash,
    })
      .then((value) => {
        if (!live) return;
        setSource({ key, path: edit.path, value });
        change.current({
          ...edit,
          source_hash: value.source_hash,
          rect: edit.rect ?? fit(value.width, value.height, "cover"),
        });
      })
      .catch((e) => {
        if (live) setError(String(e instanceof Error ? e.message : e));
      })
      .finally(() => {
        if (live) setLoading(false);
      });
    return () => {
      live = false;
    };
  }, [key]);
  const metadata = source?.path === edit.path ? source?.value : null;
  const image = source?.key === key ? source.value : null,
    rect = edit.rect;
  const drag = useRef<{
    id: number;
    x: number;
    y: number;
    rect: CropRect;
  } | null>(null);
  function update(next: CropRect) {
    onChange({ ...edit, rect: next });
  }
  async function choose() {
    try {
      const path = await pick("image");
      if (path && alive.current)
        onChange({
          ...edit,
          path,
          frame: 0,
          source_hash: undefined,
          rect: undefined,
        });
    } catch (e) {
      if (alive.current) setError(String(e));
    }
  }
  async function save() {
    if (!image || !rect || saving) return;
    setSaving(true);
    setError("");
    try {
      await request("image_crop", {
        ...edit,
        mode: "save",
        rect,
        source_hash: image.source_hash,
      });
      if (alive.current) {
        onSaved();
        onClose();
      }
    } catch (e) {
      if (alive.current) setError(String(e instanceof Error ? e.message : e));
    } finally {
      if (alive.current) setSaving(false);
    }
  }
  return (
    <Modal
      title="Customize Image"
      onClose={onClose}
      closeDisabled={saving}
      className="image-cropper"
      wide
    >
      <div className="crop-toolbar">
        <button onClick={() => void choose()} disabled={saving || loading}>
          Choose image…
        </button>
        <button
          disabled={!image || saving}
          onClick={() =>
            image && update(fit(image.width, image.height, "width"))
          }
        >
          Align to width
        </button>
        <button
          disabled={!image || saving}
          onClick={() =>
            image && update(fit(image.width, image.height, "height"))
          }
        >
          Align to height
        </button>
        <button
          disabled={!image || saving}
          onClick={() =>
            image && update(fit(image.width, image.height, "cover"))
          }
        >
          Reset image
        </button>
      </div>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <div aria-live="polite" className="crop-status">
        {loading
          ? "Loading image…"
          : !edit.path
            ? "Choose an image to position inside the frame."
            : null}
      </div>
      <svg
        className="crop-viewport"
        viewBox="-60 -60 320 220"
        role="img"
        tabIndex={image ? 0 : -1}
        aria-label="Crop preview. Arrow keys move the image; Shift moves ten pixels."
        onPointerDown={(e) => {
          if (!rect || !image || saving) return;
          e.preventDefault();
          e.currentTarget.focus();
          e.currentTarget.setPointerCapture(e.pointerId);
          drag.current = { id: e.pointerId, x: e.clientX, y: e.clientY, rect };
        }}
        onPointerMove={(e) => {
          const d = drag.current;
          if (!d || d.id !== e.pointerId) return;
          const scale = 320 / e.currentTarget.getBoundingClientRect().width;
          update(
            pan(
              {
                ...d.rect,
                x: d.rect.x + (e.clientX - d.x) * scale,
                y: d.rect.y + (e.clientY - d.y) * scale,
              },
              edit.snap !== false,
            ),
          );
        }}
        onPointerUp={() => {
          drag.current = null;
        }}
        onPointerCancel={() => {
          drag.current = null;
        }}
        onLostPointerCapture={() => {
          drag.current = null;
        }}
        onKeyDown={(e) => {
          if (!rect || !image || saving) return;
          const vector: { [k: string]: [number, number] } = {
            ArrowLeft: [-1, 0],
            ArrowRight: [1, 0],
            ArrowUp: [0, -1],
            ArrowDown: [0, 1],
          };
          const v = vector[e.key];
          if (v) {
            e.preventDefault();
            const step = e.shiftKey ? 10 : 1;
            update(
              pan(
                { ...rect, x: rect.x + v[0] * step, y: rect.y + v[1] * step },
                false,
              ),
            );
          }
        }}
      >
        <defs>
          <pattern
            id="crop-checker"
            width="10"
            height="10"
            patternUnits="userSpaceOnUse"
          >
            <rect width="10" height="10" fill="#24323b" />
            <path d="M0 0H5V5H0ZM5 5H10V10H5Z" fill="#34434c" />
          </pattern>
          <clipPath id="crop-window">
            <rect width="200" height="100" />
          </clipPath>
        </defs>
        <rect x="-60" y="-60" width="320" height="220" fill="#10191f" />
        <rect width="200" height="100" fill="url(#crop-checker)" />
        {image && rect && (
          <>
            <image
              href={`data:image/png;base64,${image.preview}`}
              {...rect}
              preserveAspectRatio="none"
              opacity="0.25"
            />
            <g clipPath="url(#crop-window)">
              <image
                href={`data:image/png;base64,${image.preview}`}
                {...rect}
                preserveAspectRatio="none"
              />
            </g>
          </>
        )}
        <rect
          width="200"
          height="100"
          fill="none"
          stroke="#99e4c8"
          strokeWidth="1"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      <p className="hint">
        Drag to position. Arrow keys move one pixel; Shift moves ten.
      </p>
      <div className="crop-controls">
        <label>
          Zoom
          <input
            aria-label="Zoom"
            type="range"
            min="40"
            max="4000"
            value={rect ? slider(rect) : 40}
            disabled={!image || !rect || saving}
            onChange={(e) => {
              if (rect && image)
                update(
                  zoom(
                    rect,
                    image.width / image.height,
                    Number(e.target.value),
                    edit.snap !== false,
                  ),
                );
            }}
            onKeyDown={(e) => {
              if (
                !rect ||
                !image ||
                !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(
                  e.key,
                )
              )
                return;
              e.preventDefault();
              const factor =
                e.key === "ArrowLeft" || e.key === "ArrowDown" ? 0.9 : 1.1;
              const next = {
                ...rect,
                width: rect.width * factor,
                height: rect.height * factor,
              };
              update(
                zoom(rect, image.width / image.height, slider(next), false),
              );
            }}
          />
        </label>
        <label>
          <input
            type="checkbox"
            checked={edit.snap !== false}
            disabled={saving}
            onChange={(e) => onChange({ ...edit, snap: e.target.checked })}
          />{" "}
          Snap to edges
        </label>
        {metadata && metadata.frames > 1 && (
          <label>
            Frame
            <input
              aria-label="Frame"
              type="number"
              min="1"
              max={metadata.frames}
              value={edit.frame + 1}
              disabled={saving || loading}
              onChange={(e) => {
                const frame = Math.max(
                  0,
                  Math.min(metadata.frames - 1, Number(e.target.value) - 1),
                );
                if (Number.isInteger(frame)) onChange({ ...edit, frame });
              }}
            />{" "}
            of {metadata.frames}
          </label>
        )}
      </div>
      <div className="row crop-footer">
        <button
          className="primary"
          disabled={!image || !rect || loading || saving}
          onClick={() => void save()}
        >
          {saving ? "Saving…" : "Set Image"}
        </button>
        <button disabled={saving} onClick={onClose}>
          Cancel
        </button>
      </div>
    </Modal>
  );
}
