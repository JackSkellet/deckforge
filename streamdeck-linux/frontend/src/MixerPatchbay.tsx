import { endpoint, graphPort, signalLabel, type GraphPort } from "./MixerGraph";
import { usePatchbayCanvas } from "./PatchbayCanvas";
import { useState } from "react";
import { newId } from "./ids";
import { appChoices, sameApplication, sameMixerValue } from "./Mixer";
import type { AppSelector, MixerConfig, MixerSnapshot } from "./Mixer";
import "./MixerPatchbay.css";

type Port = GraphPort;
interface Node {
  port: Port;
  name: string;
  detail: string;
  column: number;
  missing?: boolean;
}
interface Link {
  from: Port;
  to: Port;
  label: string;
  fixed?: boolean;
  muted?: boolean;
  original?: Link;
  gain?: { volume: number; muted: boolean };
}
const key = (port: Port) => `${port.kind}:${port.id}`;
const selectorKey = (s: AppSelector) => JSON.stringify([s.field, s.value]);
const compatible = (from: Port, to: Port) =>
  ["channel", "mix", "output"].includes(to.kind) &&
  !!from.signal === !!to.signal;
const legacyConnection = (from: Port, to: Port) =>
  !from.signal &&
  !to.signal &&
  ((["source", "app", "all"].includes(from.kind) && to.kind === "channel") ||
    (from.kind === "channel" && to.kind === "mix") ||
    (from.kind === "mix" && to.kind === "output"));

/** Edits the existing mixer draft. No direct audio-server commands or hidden routes. */
export function connectPorts(
  config: MixerConfig,
  from: Port,
  to: Port,
  snapshot: MixerSnapshot,
  remove = false,
): MixerConfig {
  if (!compatible(from, to)) return config;
  if (!remove && from.signal && to.signal) {
    const whole =
      (config.connections ?? []).some(
        (c) =>
          !c.source_channel &&
          !c.target_channel &&
          sameMixerValue(c.from, endpoint(from)) &&
          sameMixerValue(c.to, endpoint(to)),
      ) ||
      (to.kind === "channel" &&
        config.channels.some(
          (c) =>
            c.id === to.id &&
            ((from.kind === "source" && c.sources.includes(from.id)) ||
              (from.kind === "app" &&
                c.apps.some((a) =>
                  sameApplication(a, from.selector, snapshot.apps),
                )) ||
              (from.kind === "all" && c.capture_unassigned)),
        )) ||
      (from.kind === "channel" &&
        to.kind === "mix" &&
        config.routes.some((r) => r.channel === from.id && r.mix === to.id)) ||
      (from.kind === "mix" &&
        to.kind === "output" &&
        (config.mixes.some(
          (m) => m.id === from.id && m.outputs.includes(to.id),
        ) ||
          (config.monitoring?.mix === from.id &&
            config.monitoring.output === to.id)));
    const sourceChannels =
      from.kind === "source"
        ? snapshot.sources.find((d) => d.name === from.id)?.channels
        : from.kind === "output"
          ? snapshot.sinks.find((d) => d.name === from.id)?.channels
          : undefined;
    const targetChannels =
      to.kind === "output"
        ? snapshot.sinks.find((d) => d.name === to.id)?.channels
        : undefined;
    if (
      whole &&
      (from.signal === to.signal ||
        sourceChannels?.length === 1 ||
        targetChannels?.length === 1)
    )
      return config;
  }
  const next = structuredClone(config);
  if (remove)
    next.connections = (next.connections ?? []).filter(
      (c) =>
        !(
          sameMixerValue(c.from, endpoint(from)) &&
          sameMixerValue(c.to, endpoint(to)) &&
          (c.source_channel ?? undefined) === from.signal &&
          (c.target_channel ?? undefined) === to.signal
        ),
    );
  if (!legacyConnection(from, to)) {
    const matches = (c: NonNullable<MixerConfig["connections"]>[number]) =>
      sameMixerValue(c.from, endpoint(from)) &&
      sameMixerValue(c.to, endpoint(to)) &&
      (c.source_channel ?? undefined) === from.signal &&
      (c.target_channel ?? undefined) === to.signal;
    const connections = next.connections ?? [];
    next.connections = remove
      ? connections.filter((c) => !matches(c))
      : connections.some(matches)
        ? connections
        : [
            ...connections,
            {
              id: newId(),
              from: endpoint(from),
              to: endpoint(to),
              source_channel: from.signal ?? null,
              target_channel: to.signal ?? null,
              volume: 100,
              muted: false,
            },
          ];
    return next;
  }
  if (to.kind === "channel") {
    const channel = next.channels.find((c) => c.id === to.id);
    if (!channel) return config;
    if (from.kind === "source")
      channel.sources = remove
        ? channel.sources.filter((source) => source !== from.id)
        : [...new Set([...channel.sources, from.id])];
    if (from.kind === "all") channel.capture_unassigned = !remove;
    if (from.kind === "app") {
      channel.apps = channel.apps.filter(
        (selector) => !sameApplication(selector, from.selector, snapshot.apps),
      );
      if (!remove) channel.apps.push(from.selector);
    }
  } else if (from.kind === "channel" && to.kind === "mix") {
    if (
      !next.channels.some((c) => c.id === from.id) ||
      !next.mixes.some((m) => m.id === to.id)
    )
      return config;
    const exists = next.routes.some(
      (r) => r.channel === from.id && r.mix === to.id,
    );
    if (remove)
      next.routes = next.routes.filter(
        (r) => r.channel !== from.id || r.mix !== to.id,
      );
    else if (!exists)
      next.routes.push({
        channel: from.id,
        mix: to.id,
        volume: 100,
        muted: false,
      });
  } else if (from.kind === "mix" && to.kind === "output") {
    const mix = next.mixes.find((m) => m.id === from.id);
    if (!mix) return config;
    mix.outputs = remove
      ? mix.outputs.filter((o) => o !== to.id)
      : [...new Set([...mix.outputs, to.id])];
    if (remove) {
      if (mix.output_controls) delete mix.output_controls[to.id];
      if (next.monitoring?.output === to.id && next.monitoring.mix === from.id)
        next.monitoring.mix = null;
    }
  }
  return next;
}

export function routingProblems(
  config: MixerConfig,
  snapshot: MixerSnapshot,
): string[] {
  const problems: string[] = [];
  if (!config.enabled)
    problems.push("Routing is off. Enable it to hear these connections.");
  if (
    config.monitoring?.output &&
    !config.monitoring.mix &&
    !config.mixes.some((m) => m.outputs.includes(config.monitoring!.output!))
  )
    problems.push(
      "A listening device is selected, but no mix is connected to it.",
    );
  const node = (port: Port) =>
    port.kind === "recording" ? `mix:${port.id}` : key(port);
  const outgoing = new Map<string, Set<string>>();
  const connected = new Set<string>();
  const edge = (from: Port, to: Port, enabled = true) => {
    connected.add(node(from));
    if (enabled) {
      const fromKey = node(from);
      if (!outgoing.has(fromKey)) outgoing.set(fromKey, new Set());
      outgoing.get(fromKey)!.add(node(to));
    }
  };
  for (const route of config.routes)
    edge(
      { kind: "channel", id: route.channel },
      { kind: "mix", id: route.mix },
      !route.muted && route.volume > 0,
    );
  for (const mix of config.mixes) {
    for (const output of mix.outputs) {
      const gain = mix.output_controls?.[output];
      edge(
        { kind: "mix", id: mix.id },
        { kind: "output", id: output },
        !gain?.muted && (gain?.volume ?? 100) > 0,
      );
    }
  }
  if (config.monitoring?.mix && config.monitoring.output)
    edge(
      { kind: "mix", id: config.monitoring.mix },
      { kind: "output", id: config.monitoring.output },
    );
  for (const connection of config.connections ?? [])
    edge(
      graphPort(connection.from),
      graphPort(connection.to),
      !connection.muted && connection.volume > 0,
    );
  const audible = (start: string) => {
    const pending = [start],
      seen = new Set<string>();
    while (pending.length) {
      const current = pending.pop()!;
      if (seen.has(current)) continue;
      seen.add(current);
      if (
        config.mixes.some(
          (m) => `mix:${m.id}` === current && (m.muted || m.volume === 0),
        )
      )
        continue;
      if (
        snapshot.sinks.some((s) => `output:${s.name}` === current && !s.muted)
      )
        return true;
      pending.push(...(outgoing.get(current) ?? []));
    }
    return false;
  };
  for (const channel of config.channels) {
    if (!connected.has(`channel:${channel.id}`))
      problems.push(`${channel.name} has no outgoing audio connection.`);
    else if (!audible(`channel:${channel.id}`))
      problems.push(
        `${channel.name} has no audible path to a device. Recording outputs may still receive audio.`,
      );
  }
  const sources = new Set(config.channels.flatMap((c) => c.sources));
  const outputs = new Set(config.mixes.flatMap((m) => m.outputs));
  for (const connection of config.connections ?? [])
    for (const port of [connection.from, connection.to]) {
      if (port.kind === "source") sources.add(port.name);
      if (port.kind === "output") outputs.add(port.name);
    }
  for (const source of sources) {
    const device = snapshot.sources.find((s) => s.name === source);
    if (!device) problems.push(`Input ${source} is unavailable.`);
    else if (device.muted)
      problems.push(
        `Input ${device.description || source} is muted in system settings.`,
      );
  }
  for (const output of outputs) {
    const device = snapshot.sinks.find((s) => s.name === output);
    if (!device) problems.push(`Output ${output} is unavailable.`);
    else if (device.muted)
      problems.push(
        `Output ${device.description || output} is muted in system settings.`,
      );
  }
  for (const app of snapshot.apps) {
    if (
      snapshot.sinks.some((s) => s.name === app.sink && s.owned) &&
      app.sink.includes(".channel.") &&
      !app.sink.includes(".channel.patch_") &&
      !config.channels.some((c) => app.sink.endsWith(`.channel.${c.id}`))
    )
      problems.push(
        `${app.name} is attached to a removed channel. Select an available output for this app in your system sound settings.`,
      );
  }
  return [...new Set(problems)];
}

export function MixerPatchbay({
  config,
  snapshot,
  onChange,
  onSetup,
  onApply,
  canApply = false,
  error = "",
}: {
  config: MixerConfig;
  snapshot: MixerSnapshot;
  onChange: (config: MixerConfig) => void;
  onSetup: () => void;
  onApply?: () => void;
  canApply?: boolean;
  error?: string;
}) {
  const [start, setStart] = useState<Port | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [pinned, setPinned] = useState<AppSelector[]>([]);
  const canvas = usePatchbayCanvas();
  const zoom = canvas.layout.zoom;
  const setZoom = canvas.zoom;
  const [expanded, setExpanded] = useState(false);
  const [notice, setNotice] = useState("");
  const nodes: Node[] = [],
    links: Link[] = [];
  const add = (node: Node) => {
    if (!nodes.some((n) => key(n.port) === key(node.port))) nodes.push(node);
  };
  const deviceName = (id: string, source = false) =>
    (source ? snapshot.sources : snapshot.sinks).find((d) => d.name === id)
      ?.description || id;
  add({
    port: { kind: "all", id: "all" },
    name: "All other audio",
    detail: "Apps without an explicit assignment",
    column: 0,
  });
  const apps = new Map<string, { selector: AppSelector; name: string }>();
  for (const app of snapshot.applications ?? [])
    apps.set(selectorKey({ field: "desktop_application", value: app.id }), {
      selector: { field: "desktop_application", value: app.id },
      name: app.name,
    });
  for (const app of appChoices(snapshot.apps))
    if (
      !snapshot.apps.some(
        (a) =>
          a[app.selector.field] === app.selector.value && a.desktop_application,
      )
    )
      apps.set(selectorKey(app.selector), {
        selector: app.selector,
        name: app.label,
      });
  const addApp = (selector: AppSelector) =>
    add({
      port: { kind: "app", id: selectorKey(selector), selector },
      name: apps.get(selectorKey(selector))?.name ?? selector.value,
      detail: "Application",
      column: 0,
    });
  pinned.forEach(addApp);
  for (const source of snapshot.sources.filter(
    (s) => !s.owned && !s.monitor && !s.name.startsWith("deckforge."),
  ))
    add({
      port: { kind: "source", id: source.name },
      name: source.description || source.name,
      detail: source.muted
        ? "Input muted in system settings"
        : "Microphone / audio input",
      column: 0,
    });
  for (const channel of config.channels) {
    const to: Port = { kind: "channel", id: channel.id };
    add({ port: to, name: channel.name, detail: "Input channel", column: 1 });
    for (const selector of channel.apps) {
      addApp(selector);
      links.push({
        from: { kind: "app", id: selectorKey(selector), selector },
        to,
        label: "App assignment",
      });
    }
    for (const source of channel.sources) {
      const from: Port = { kind: "source", id: source };
      add({
        port: from,
        name: deviceName(source, true),
        detail: snapshot.sources.some((s) => s.name === source)
          ? "Audio input"
          : "Input unavailable",
        column: 0,
        missing: !snapshot.sources.some((s) => s.name === source),
      });
      links.push({ from, to, label: "Audio input" });
    }
    if (channel.capture_unassigned)
      links.push({
        from: { kind: "all", id: "all" },
        to,
        label: "Default channel",
      });
  }
  for (const mix of config.mixes) {
    const from: Port = { kind: "mix", id: mix.id };
    add({
      port: from,
      name: mix.name,
      detail: `${mix.muted ? "Muted · " : ""}${mix.volume}% master`,
      column: 2,
    });
    const recording: Port = { kind: "recording", id: mix.id };
    add({
      port: recording,
      name: `${mix.name} recording`,
      detail: "Virtual input for OBS and other apps",
      column: 3,
    });
    links.push({
      from,
      to: recording,
      label: "Recording output",
      fixed: true,
      muted: mix.muted || mix.volume === 0,
    });
    for (const output of mix.outputs) {
      const to: Port = { kind: "output", id: output };
      const device = snapshot.sinks.find((s) => s.name === output);
      add({
        port: to,
        name: deviceName(output),
        detail: device ? "Output device" : "Output unavailable",
        missing: !device,
        column: 3,
      });
      const gain = mix.output_controls?.[output];
      links.push({
        from,
        to,
        gain: { volume: gain?.volume ?? 100, muted: gain?.muted ?? false },
        label: `Fixed output · ${gain?.volume ?? 100}%`,
        muted:
          mix.muted ||
          mix.volume === 0 ||
          gain?.muted ||
          gain?.volume === 0 ||
          device?.muted,
      });
    }
  }
  for (const output of snapshot.sinks.filter(
    (s) => !s.owned && !s.monitor && !s.name.startsWith("deckforge."),
  ))
    add({
      port: { kind: "output", id: output.name },
      name: output.description || output.name,
      detail: output.muted
        ? "Output muted in system settings"
        : "Output device",
      column: 3,
    });
  if (config.monitoring?.output) {
    const output = config.monitoring.output;
    const to: Port = { kind: "output", id: output };
    add({
      port: to,
      name: deviceName(output),
      detail: "Listening output unavailable",
      missing: !snapshot.sinks.some((s) => s.name === output),
      column: 3,
    });
    if (
      config.monitoring.mix &&
      !config.mixes.some(
        (m) => m.id === config.monitoring?.mix && m.outputs.includes(output),
      )
    )
      links.push({
        from: { kind: "mix", id: config.monitoring.mix },
        to,
        label: "Listening output",
      });
  }
  for (const route of config.routes)
    links.push({
      from: { kind: "channel", id: route.channel },
      to: { kind: "mix", id: route.mix },
      gain: { volume: route.volume, muted: route.muted },
      label: `${route.volume}%${route.muted ? " · Muted" : ""}`,
      muted: route.muted || route.volume === 0,
    });
  for (const connection of config.connections ?? []) {
    const from = graphPort(connection.from, connection.source_channel),
      to = graphPort(connection.to, connection.target_channel);
    if (from.kind === "app") addApp(from.selector);
    if (from.kind === "source")
      add({
        port: from,
        name: deviceName(from.id, true),
        detail: "Audio input",
        column: 0,
        missing: !snapshot.sources.some((s) => s.name === from.id),
      });
    for (const port of [from, to])
      if (port.kind === "output")
        add({
          port,
          name: deviceName(port.id),
          detail: "Output device",
          column: 3,
          missing: !snapshot.sinks.some((s) => s.name === port.id),
        });
    links.push({
      from,
      to,
      gain: { volume: connection.volume, muted: connection.muted },
      label: `${connection.volume}% · ${signalLabel(from.signal)} → ${signalLabel(to.signal)}`,
      muted: connection.muted || connection.volume === 0,
    });
  }
  const channels = (port: Port) => {
    const device =
      port.kind === "source"
        ? snapshot.sources.find((d) => d.name === port.id)
        : port.kind === "output"
          ? snapshot.sinks.find((d) => d.name === port.id)
          : undefined;
    return device?.channels?.length
      ? device.channels
      : ["front-left", "front-right"];
  };
  const signals = (port: Port): (string | undefined)[] =>
    canvas.layout.split[key(port)] ? channels(port) : [undefined];
  const nodeHeight = (port: Port) => 72 + signals(port).length * 28;
  const columns = [0, 1, 2, 3].map((column) =>
    nodes.filter((node) => node.column === column),
  );
  const positions = new Map<string, { x: number; y: number }>();
  columns.forEach((column, x) => {
    let y = 24;
    for (const node of column) {
      positions.set(
        key(node.port),
        canvas.layout.positions[key(node.port)] ?? { x: 20 + x * 278, y },
      );
      y += nodeHeight(node.port) + 28;
    }
  });
  const width = Math.max(
    1100,
    ...nodes.map((node) => positions.get(key(node.port))!.x + 320),
  );
  const height = Math.max(
    480,
    ...nodes.map(
      (node) => positions.get(key(node.port))!.y + nodeHeight(node.port) + 100,
    ),
  );
  const displayLinks = links.flatMap((link) => {
    if (
      link.from.signal ||
      link.to.signal ||
      (!canvas.layout.split[key(link.from)] &&
        !canvas.layout.split[key(link.to)])
    )
      return [link];
    const from = channels(link.from),
      to = channels(link.to);
    const pairs =
      from.length === 1
        ? to.map((t) => [from[0], t])
        : to.length === 1
          ? from.map((f) => [f, to[0]])
          : from.filter((f) => to.includes(f)).map((f) => [f, f]);
    return pairs.map(([f, t]) => ({
      ...link,
      from: { ...link.from, signal: f },
      to: { ...link.to, signal: t },
      label: `${link.label} · ${signalLabel(f)} → ${signalLabel(t)}`,
      original: link,
    }));
  });
  const disconnect = (link: Link) => {
    if (!link.original)
      return connectPorts(config, link.from, link.to, snapshot, true);
    const next = connectPorts(
      config,
      link.original.from,
      link.original.to,
      snapshot,
      true,
    );
    for (const sibling of displayLinks.filter(
      (l) => l.original === link.original && l !== link,
    )) {
      next.connections = [
        ...(next.connections ?? []),
        {
          id: newId(),
          from: endpoint(sibling.from),
          to: endpoint(sibling.to),
          source_channel: sibling.from.signal ?? null,
          target_channel: sibling.to.signal ?? null,
          volume: sibling.gain?.volume ?? 100,
          muted: sibling.gain?.muted ?? false,
        },
      ];
    }
    return next;
  };
  const setGain = (link: Link, patch: { volume?: number; muted?: boolean }) => {
    const gain = {
      volume: link.gain?.volume ?? 100,
      muted: link.gain?.muted ?? false,
      ...patch,
    };
    if (
      !link.original &&
      !link.from.signal &&
      link.from.kind === "channel" &&
      link.to.kind === "mix" &&
      config.routes.some(
        (r) => r.channel === link.from.id && r.mix === link.to.id,
      )
    ) {
      onChange({
        ...config,
        routes: config.routes.map((r) =>
          r.channel === link.from.id && r.mix === link.to.id
            ? { ...r, ...gain }
            : r,
        ),
      });
      return;
    }
    if (
      !link.original &&
      !link.from.signal &&
      link.from.kind === "mix" &&
      link.to.kind === "output" &&
      config.mixes.some(
        (m) => m.id === link.from.id && m.outputs.includes(link.to.id),
      )
    ) {
      onChange({
        ...config,
        mixes: config.mixes.map((m) =>
          m.id === link.from.id
            ? {
                ...m,
                output_controls: { ...m.output_controls, [link.to.id]: gain },
              }
            : m,
        ),
      });
      return;
    }
    const next = disconnect(link);
    next.connections = [
      ...(next.connections ?? []),
      {
        id: newId(),
        from: endpoint(link.from),
        to: endpoint(link.to),
        source_channel: link.from.signal ?? null,
        target_channel: link.to.signal ?? null,
        ...gain,
      },
    ];
    onChange(next);
  };
  const portY = (port: Port) =>
    68 + Math.max(0, signals(port).indexOf(port.signal)) * 28;
  const linkId = (link: Link) =>
    JSON.stringify([
      key(link.from),
      key(link.to),
      link.from.signal,
      link.to.signal,
    ]);
  const selectedLink = displayLinks.find((l) => linkId(l) === selected);
  const name = (port: Port) =>
    nodes.find((n) => key(n.port) === key(port))?.name ?? port.id;
  const problems = [
    ...new Set([
      ...routingProblems(config, snapshot),
      ...(expanded ? snapshot.warnings : []),
    ]),
  ];
  const connect = (to: Port) => {
    if (!start) return;
    if (!compatible(start, to)) {
      setNotice(
        "Choose an audio input port. Use L/R on both nodes for individual channels.",
      );
      return;
    }
    const next = connectPorts(config, start, to, snapshot);
    if (next === config) {
      setNotice("Those ports are already connected.");
      setStart(null);
      return;
    }
    onChange(next);
    setNotice(`${name(start)} connected to ${name(to)}.`);
    setStart(null);
  };
  return (
    <section
      className={`mixer-patchbay${expanded ? " expanded" : ""}`}
      aria-label="Advanced routing"
      role={expanded ? "dialog" : undefined}
      aria-modal={expanded || undefined}
      onKeyDown={(event) => {
        if (event.key === "Escape" && (expanded || start || selected)) {
          event.preventDefault();
          event.stopPropagation();
          if (start || selected) {
            setStart(null);
            setSelected(null);
          } else setExpanded(false);
        }
        if (event.key === "Tab" && expanded) {
          const controls = [
            ...event.currentTarget.querySelectorAll<HTMLElement>(
              'button:not(:disabled),input:not(:disabled),select:not(:disabled),[tabindex="0"]',
            ),
          ].filter((node) => node.getClientRects().length > 0);
          const first = controls[0],
            last = controls.at(-1);
          if (event.shiftKey && document.activeElement === first) {
            event.preventDefault();
            last?.focus();
          } else if (!event.shiftKey && document.activeElement === last) {
            event.preventDefault();
            first?.focus();
          }
        }
      }}
    >
      <div className="patchbay-tools">
        <div>
          <h3>Audio connections</h3>
          <p className="hint">
            Drag node headers to arrange the canvas. Connect output ports to
            input ports. Changes stay in your draft until Apply routes.
          </p>
        </div>
        <button
          type="button"
          disabled={config.channels.length >= 128}
          onClick={() =>
            onChange({
              ...config,
              channels: [
                ...config.channels,
                {
                  id: newId(),
                  name: `Channel ${config.channels.length + 1}`,
                  apps: [],
                  sources: [],
                },
              ],
            })
          }
        >
          Add channel
        </button>
        <button
          type="button"
          disabled={config.mixes.length >= 64}
          onClick={() =>
            onChange({
              ...config,
              mixes: [
                ...config.mixes,
                {
                  id: newId(),
                  name: `Mix ${config.mixes.length + 1}`,
                  outputs: [],
                  volume: 100,
                  muted: false,
                },
              ],
            })
          }
        >
          Add mix
        </button>
        <button type="button" onClick={onSetup}>
          Edit nodes
        </button>
        <label>
          Zoom{" "}
          <select
            aria-label="Routing graph zoom"
            value={zoom}
            onChange={(e) => setZoom(Number(e.target.value))}
          >
            {![0.25, 0.5, 0.65, 0.8, 1, 1.2, 1.5, 2].includes(zoom) && (
              <option value={zoom}>{Math.round(zoom * 100)}%</option>
            )}
            {[0.25, 0.5, 0.65, 0.8, 1, 1.2, 1.5, 2].map((value) => (
              <option key={value} value={value}>
                {Math.round(value * 100)}%
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          onClick={() => {
            const view = canvas.viewport.current;
            if (view)
              canvas.zoom(
                Math.min(view.clientWidth / width, view.clientHeight / height),
              );
          }}
        >
          Fit canvas
        </button>
        <button type="button" onClick={canvas.arrange}>
          Arrange nodes
        </button>
        <button type="button" onClick={() => setExpanded(!expanded)}>
          {expanded ? "Return to mixer" : "Expand canvas"}
        </button>
        {expanded && onApply && (
          <button
            type="button"
            className="primary"
            disabled={!canApply}
            onClick={onApply}
          >
            Apply routes
          </button>
        )}
      </div>
      {expanded && (error || snapshot.error) && (
        <p role="alert" className="error">
          {error || snapshot.error}
        </p>
      )}
      {problems.length > 0 && (
        <details className="patchbay-problems">
          <summary>Check these audio paths ({problems.length})</summary>
          <ul>
            {problems.map((problem, i) => (
              <li key={i}>{problem}</li>
            ))}
          </ul>
        </details>
      )}
      <div className="patchbay-source-toolbar">
        <label className="check">
          <input
            type="checkbox"
            checked={config.enabled}
            onChange={(e) => onChange({ ...config, enabled: e.target.checked })}
          />
          Enable audio routing
        </label>
        <div className="patchbay-app-picker">
          <label>
            Find an application
            <input
              type="search"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search installed and playing apps"
            />
          </label>
          {search.trim() && (
            <div className="patchbay-app-results">
              {[...apps.values()]
                .filter((a) =>
                  `${a.name} ${a.selector.value}`
                    .toLowerCase()
                    .includes(search.trim().toLowerCase()),
                )
                .slice(0, 12)
                .map((app) => (
                  <button
                    type="button"
                    key={selectorKey(app.selector)}
                    onClick={() => {
                      setPinned((p) => [...p, app.selector]);
                      setSearch("");
                      setStart({
                        kind: "app",
                        id: selectorKey(app.selector),
                        selector: app.selector,
                      });
                    }}
                  >
                    {app.name}
                  </button>
                ))}
            </div>
          )}
        </div>
      </div>
      <p className="patchbay-instruction" role="status">
        {snapshot.pending
          ? "Applying routing… "
          : canApply
            ? "Unapplied changes. Choose Apply routes to hear them. "
            : ""}
        {start
          ? `Connect ${name(start)} to a highlighted input port. Escape cancels.`
          : notice ||
            "Drag the background to pan. Select a cable to inspect or remove it."}
      </p>
      <div
        className="patchbay-viewport"
        ref={canvas.viewport}
        onPointerDown={canvas.panDown}
        onPointerMove={canvas.panMove}
        onPointerUp={canvas.stop}
        onLostPointerCapture={canvas.stop}
        tabIndex={0}
        aria-label="Audio connection canvas"
      >
        <div style={{ width: width * zoom, height: height * zoom }}>
          <div
            className="patchbay-canvas"
            style={{ width, height, transform: `scale(${zoom})` }}
          >
            <svg
              width={width}
              height={height}
              className="patchbay-cables"
              aria-label="Routing connections"
            >
              {displayLinks.map((link) => {
                const a = positions.get(key(link.from)),
                  b = positions.get(key(link.to));
                if (!a || !b) return null;
                const id = linkId(link);
                const d = `M ${a.x + 220} ${a.y + portY(link.from)} C ${a.x + 252} ${a.y + portY(link.from)}, ${b.x - 32} ${b.y + portY(link.to)}, ${b.x} ${b.y + portY(link.to)}`;
                return (
                  <g
                    key={id}
                    className={`${link.muted ? "muted " : ""}${selected === id ? "selected " : ""}${link.fixed ? "recording" : ""}`}
                  >
                    <path d={d} className="patchbay-cable" />
                    <path
                      d={d}
                      className="patchbay-cable-hit"
                      role="button"
                      tabIndex={0}
                      aria-label={`${name(link.from)} to ${name(link.to)}: ${link.label}`}
                      onClick={() => setSelected(id)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          setSelected(id);
                        }
                      }}
                    />
                  </g>
                );
              })}
            </svg>
            {nodes.map((node) => {
              const pos = positions.get(key(node.port))!;
              const input = ["channel", "mix", "output"].includes(
                  node.port.kind,
                ),
                output = true;
              const portList = signals(node.port);

              return (
                <article
                  key={key(node.port)}
                  className={`patchbay-node ${node.port.kind}${node.missing ? " missing" : ""}`}
                  style={{
                    left: pos.x,
                    top: pos.y,
                    height: nodeHeight(node.port),
                  }}
                >
                  {input &&
                    portList.map((signal) => {
                      const port = { ...node.port, signal };
                      return (
                        <button
                          key={signal ?? "stereo"}
                          type="button"
                          className={`patchbay-port input${start && compatible(start, port) ? " compatible" : ""}`}
                          style={{ top: portY(port) - 12 }}
                          aria-label={`Connect to ${node.name}${signal ? ` ${signalLabel(signal)}` : ""}`}
                          disabled={!start || !compatible(start, port)}
                          onClick={() => connect(port)}
                          onDragOver={(event) => {
                            if (start && compatible(start, port)) {
                              event.preventDefault();
                              event.dataTransfer.dropEffect = "link";
                            }
                          }}
                          onDrop={(event) => {
                            event.preventDefault();
                            if (
                              start &&
                              compatible(start, port) &&
                              event.dataTransfer.getData(
                                "application/x-deckforge-audio-port",
                              ) === JSON.stringify(start)
                            ) {
                              onChange(
                                connectPorts(config, start, port, snapshot),
                              );
                              setStart(null);
                            }
                          }}
                        >
                          {signalLabel(
                            signal ??
                              (channels(node.port).length === 1
                                ? channels(node.port)[0]
                                : undefined),
                          )}
                        </button>
                      );
                    })}
                  <button
                    type="button"
                    className="patchbay-node-header"
                    title={`Drag to move ${node.name}; arrow keys move the focused node`}
                    aria-label={`Move ${node.name}`}
                    onPointerDown={(event) =>
                      canvas.nodeDown(event, key(node.port), pos)
                    }
                    onPointerMove={canvas.nodeMove}
                    onPointerUp={canvas.stop}
                    onLostPointerCapture={canvas.stop}
                    onKeyDown={(event) =>
                      canvas.nodeKey(event, key(node.port), pos)
                    }
                  >
                    <strong>{node.name}</strong>
                  </button>
                  <small>{node.detail}</small>
                  <button
                    type="button"
                    className="patchbay-split"
                    aria-label={`Show individual channels for ${node.name}`}
                    aria-pressed={!!canvas.layout.split[key(node.port)]}
                    onClick={() => {
                      canvas.split(key(node.port));
                      setStart(null);
                    }}
                  >
                    L/R
                  </button>
                  {output &&
                    portList.map((signal) => {
                      const port = { ...node.port, signal };
                      return (
                        <button
                          key={signal ?? "stereo"}
                          type="button"
                          className="patchbay-port output"
                          style={{ top: portY(port) - 12 }}
                          aria-label={`Connect from ${node.name}${signal ? ` ${signalLabel(signal)}` : ""}`}
                          aria-pressed={
                            !!start &&
                            key(start) === key(port) &&
                            start.signal === signal
                          }
                          draggable
                          onDragStart={(event) => {
                            setStart(port);
                            event.dataTransfer.effectAllowed = "link";
                            event.dataTransfer.setData(
                              "application/x-deckforge-audio-port",
                              JSON.stringify(port),
                            );
                          }}
                          onDragEnd={() => setStart(null)}
                          onClick={() => {
                            setStart(port);
                            setSelected(null);
                          }}
                        >
                          {signalLabel(
                            signal ??
                              (channels(node.port).length === 1
                                ? channels(node.port)[0]
                                : undefined),
                          )}
                        </button>
                      );
                    })}
                </article>
              );
            })}
          </div>
        </div>
      </div>
      {selectedLink && (
        <div className="patchbay-selection">
          <span>
            <strong>
              {name(selectedLink.from)} → {name(selectedLink.to)}
            </strong>
            <small>{selectedLink.label}</small>
          </span>
          {selectedLink.fixed ? (
            <p className="hint">
              Each mix provides a recording input automatically.
            </p>
          ) : (
            <>
              <label>
                Level{" "}
                <input
                  aria-label="Connection level"
                  type="range"
                  min="0"
                  max="100"
                  step="1"
                  value={selectedLink.gain?.volume ?? 100}
                  onChange={(event) =>
                    setGain(selectedLink, {
                      volume: Number(event.target.value),
                    })
                  }
                />
                <span>{selectedLink.gain?.volume ?? 100}%</span>
              </label>
              <label className="check">
                <input
                  type="checkbox"
                  checked={selectedLink.gain?.muted ?? false}
                  onChange={(event) =>
                    setGain(selectedLink, { muted: event.target.checked })
                  }
                />
                Mute connection
              </label>
              <button
                type="button"
                onClick={() => {
                  onChange(disconnect(selectedLink));
                  setSelected(null);
                  setNotice(
                    "Connection removed from the draft. Apply routes to update audio.",
                  );
                }}
              >
                Disconnect
              </button>
            </>
          )}
        </div>
      )}
    </section>
  );
}
