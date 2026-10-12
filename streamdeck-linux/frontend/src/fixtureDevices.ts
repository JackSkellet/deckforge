// Synthetic device descriptors for browser development. Production uses Rust's snapshot.
import type { DeviceGeometry, DeviceCapabilities } from "./device";
interface FixtureDevice {
  id: string;
  name: string;
  geometry: DeviceGeometry;
  capabilities: DeviceCapabilities;
}
const definitions: [
  string,
  string,
  number,
  number,
  number,
  number,
  number,
  number,
][] = [
  ["mini", "Stream Deck Mini", 3, 2, 0, 80, 0, 0],
  ["classic", "Stream Deck", 5, 3, 0, 72, 0, 0],
  ["classic-2017", "Stream Deck (2017)", 5, 3, 0, 72, 0, 0],
  ["xl", "Stream Deck XL", 8, 4, 0, 96, 0, 0],
  ["plus", "Stream Deck +", 4, 2, 4, 120, 800, 100],
  ["plus-xl", "Stream Deck + XL", 9, 4, 6, 112, 1200, 100],
  ["studio", "Stream Deck Studio", 16, 2, 2, 144, 0, 0],
  ["neo", "Stream Deck Neo", 4, 2, 0, 96, 248, 58],
  ["pedal", "Stream Deck Pedal", 3, 1, 0, 0, 0, 0],
];
export function fixtureDevice(id: string): FixtureDevice {
  const value =
    definitions.find(
      (row) =>
        row[0] === (id === "min" ? "mini" : id === "plusxl" ? "plus-xl" : id),
    ) ?? definitions.find((row) => row[0] === "plus")!;
  const [
    model,
    name,
    columns,
    rows,
    dials,
    key_width,
    strip_width,
    strip_height,
  ] = value;
  return {
    id: model,
    name,
    geometry: {
      columns,
      rows,
      keys: columns * rows,
      dials,
      key_width,
      key_height: model === "studio" ? 112 : key_width,
      strip_width,
      strip_height,
    },
    capabilities: {
      transport_supported: true,
      has_lcd: model !== "pedal",
      key_images: model !== "pedal",
      brightness: model !== "pedal",
      sleep: !["pedal", "classic-2017", "studio"].includes(model),
      full_lcd: !["mini", "pedal", "classic-2017", "studio"].includes(model),
      strip: strip_width > 0,
      touchscreen: dials > 0 && strip_width > 0,
      sensors: model === "neo" ? 2 : 0,
    },
  };
}
