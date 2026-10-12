import type { AppSelector } from "./Mixer";
export type AudioEndpoint =
  | { kind: "source" | "output"; name: string }
  | { kind: "channel" | "mix"; id: string }
  | { kind: "recording"; mix: string }
  | { kind: "app"; selector: AppSelector }
  | { kind: "all_audio" };
export interface AudioConnection {
  id: string;
  from: AudioEndpoint;
  to: AudioEndpoint;
  source_channel: string | null;
  target_channel: string | null;
  volume: number;
  muted: boolean;
}
export type GraphPort = (
  | { kind: "source" | "output" | "channel" | "mix" | "recording"; id: string }
  | { kind: "app"; id: string; selector: AppSelector }
  | { kind: "all"; id: string }
) & { signal?: string };
export function endpoint(port: GraphPort): AudioEndpoint {
  switch (port.kind) {
    case "source":
    case "output":
      return { kind: port.kind, name: port.id };
    case "channel":
    case "mix":
      return { kind: port.kind, id: port.id };
    case "recording":
      return { kind: "recording", mix: port.id };
    case "app":
      return { kind: "app", selector: port.selector };
    case "all":
      return { kind: "all_audio" };
  }
}
export function graphPort(
  value: AudioEndpoint,
  signal?: string | null,
): GraphPort {
  const channel = signal ? { signal } : {};
  switch (value.kind) {
    case "source":
    case "output":
      return { kind: value.kind, id: value.name, ...channel };
    case "channel":
    case "mix":
      return { ...value, ...channel };
    case "recording":
      return { kind: "recording", id: value.mix, ...channel };
    case "app":
      return {
        kind: "app",
        id: JSON.stringify([value.selector.field, value.selector.value]),
        selector: value.selector,
        ...channel,
      };
    case "all_audio":
      return { kind: "all", id: "all", ...channel };
  }
}
export const signalLabel = (value?: string) =>
  value
    ? ({ "front-left": "Left", "front-right": "Right", mono: "Mono" }[value] ??
      value)
    : "Stereo";
