import type { Controller, Slot, Snapshot, DeviceRecord } from "./model";
export interface DeviceGeometry {
  columns: number;
  rows: number;
  keys: number;
  dials: number;
  key_width: number;
  key_height: number;
  strip_width: number;
  strip_height: number;
}
export interface DeviceCapabilities {
  transport_supported?: boolean;
  has_lcd?: boolean;
  key_images?: boolean;
  brightness?: boolean;
  sleep?: boolean;
  full_lcd?: boolean;
  strip?: boolean;
  touchscreen?: boolean;
  sensors?: number;
}
// Older saved UI fixtures predate model descriptors and represent Stream Deck +.
const legacyPlus: DeviceGeometry = {
  columns: 4,
  rows: 2,
  keys: 8,
  dials: 4,
  key_width: 120,
  key_height: 120,
  strip_width: 800,
  strip_height: 100,
};
export function geometry(snapshot: Snapshot): DeviceGeometry {
  return snapshot.device_geometry ?? legacyPlus;
}
export function deviceName(snapshot: Snapshot) {
  return (
    snapshot.device_name ??
    (snapshot.device_model
      ? `Stream Deck (${snapshot.device_model})`
      : "Stream Deck +")
  );
}
export function hasKeyImages(snapshot: Snapshot) {
  return (
    snapshot.device_capabilities?.key_images ?? geometry(snapshot).key_width > 0
  );
}
export function supportedControllers(snapshot: Snapshot): Controller[] {
  const value = geometry(snapshot);
  return [
    ...(value.keys > 0 ? ["Keypad" as const] : []),
    ...(value.dials > 0 ? ["Encoder" as const] : []),
    ...(snapshot.device_model === "neo" ? ["Neo" as const] : []),
  ];
}
export function validControl(
  snapshot: Snapshot,
  controller: Controller,
  index: number,
) {
  const value = geometry(snapshot);
  return controller === "Neo"
    ? snapshot.device_model === "neo" && index === 1
    : Number.isInteger(index) &&
        index >= 0 &&
        index < (controller === "Keypad" ? value.keys : value.dials);
}
export function isAssignableSlot(
  slot: Slot,
): slot is Slot & { controller: Controller } {
  return (
    slot.controller === "Keypad" ||
    slot.controller === "Encoder" ||
    slot.controller === "Neo"
  );
}
export function outsideControls(snapshot: Snapshot) {
  return snapshot.slots
    .filter(isAssignableSlot)
    .filter((slot) => !validControl(snapshot, slot.controller, slot.index));
}

export function acceptsInput(
  snapshot: Snapshot,
  controller: Controller,
  index: number,
) {
  return controller !== "Neo" && validControl(snapshot, controller, index);
}
export function controlName(controller: Controller, snapshot?: Snapshot) {
  return controller === "Neo"
    ? "Infobar"
    : controller === "Encoder"
      ? "Dial"
      : snapshot && !hasKeyImages(snapshot)
        ? "Pedal"
        : "Key";
}

export function associatedDevices(snapshot: Snapshot) {
  return snapshot.devices?.length
    ? [...snapshot.devices].sort(
        (a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id),
      )
    : [
        {
          id: snapshot.device,
          model: snapshot.device_model ?? "plus",
          name: deviceName(snapshot),
          connected: snapshot.connected,
          online: snapshot.device_online,
          firmware: snapshot.firmware,
          selected: true,
        },
      ];
}

export function deviceOptionLabel(
  device: DeviceRecord,
  peers: DeviceRecord[],
  simulate = false,
) {
  const duplicates =
    peers.filter((peer) => peer.name === device.name).length > 1;
  const identity =
    duplicates || device.id.length <= 12
      ? device.id
      : `…${device.id.slice(-8)}`;
  const status =
    simulate && device.online !== false
      ? " · Simulated"
      : device.connected
        ? ""
        : " · Offline";
  return `${device.name} · ${identity}${status}`;
}
