export interface CropRect {
  x: number;
  y: number;
  width: number;
  height: number;
}
export interface CropEdit {
  context: string;
  device: string;
  expected: unknown;
  path?: string;
  frame: number;
  source_hash?: string;
  rect?: CropRect;
  snap?: boolean;
}
export function fit(
  width: number,
  height: number,
  mode: "cover" | "width" | "height",
): CropRect {
  const scale =
    mode === "width"
      ? 200 / width
      : mode === "height"
        ? 100 / height
        : Math.max(200 / width, 100 / height);
  const cap = Math.min(scale, 4000 / Math.max(width, height));
  const w = Math.max(1, Math.round(width * cap)),
    h = Math.max(1, Math.round(height * cap));
  return {
    x: Math.floor((200 - w) / 2),
    y: Math.floor((100 - h) / 2),
    width: w,
    height: h,
  };
}
export function zoom(
  rect: CropRect,
  aspect: number,
  value: number,
  snap = true,
): CropRect {
  const side = 40 + (Math.max(40, Math.min(4000, value)) - 40) ** 2 / 3960;
  let w = Math.max(1, Math.round(aspect >= 1 ? side : side * aspect));
  let h = Math.max(1, Math.round(aspect >= 1 ? side / aspect : side));
  if (snap && Math.abs(w - 200) < 13) {
    w = 200;
    h = Math.max(1, Math.round(w / aspect));
  } else if (snap && Math.abs(h - 100) < 6) {
    h = 100;
    w = Math.max(1, Math.round(h * aspect));
  }
  const cap = Math.min(1, 4000 / Math.max(w, h));
  w = Math.max(1, Math.round(w * cap));
  h = Math.max(1, Math.round(h * cap));
  return pan(
    {
      ...rect,
      x: rect.x + Math.floor((rect.width - 1) / 2) - Math.floor((w - 1) / 2),
      y: rect.y + Math.floor((rect.height - 1) / 2) - Math.floor((h - 1) / 2),
      width: w,
      height: h,
    },
    false,
  );
}
export function slider(rect: CropRect) {
  return Math.max(
    40,
    Math.min(
      4000,
      40 +
        Math.sqrt(Math.max(0, Math.max(rect.width, rect.height) - 40) * 3960),
    ),
  );
}
export function pan(rect: CropRect, snap: boolean): CropRect {
  let { x, y } = rect;
  if (snap) {
    const xs = [-x, 200 - rect.width - x].sort(
      (a, b) => Math.abs(a) - Math.abs(b),
    );
    const ys = [-y, 100 - rect.height - y].sort(
      (a, b) => Math.abs(a) - Math.abs(b),
    );
    if (Math.abs(xs[0]) < 13) x += xs[0];
    if (Math.abs(ys[0]) < 6) y += ys[0];
  }
  return {
    ...rect,
    x: Math.round(Math.max(-40 - rect.width, Math.min(240, x))),
    y: Math.round(Math.max(-40 - rect.height, Math.min(140, y))),
  };
}
