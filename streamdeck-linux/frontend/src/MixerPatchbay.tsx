import { useState } from "react";
import { newId } from "./ids";
import { appChoices, sameApplication } from "./Mixer";
import type { AppSelector, MixerConfig, MixerSnapshot } from "./Mixer";
import "./MixerPatchbay.css";

type Port =
  | { kind: "source" | "output" | "channel" | "mix" | "recording"; id: string }
  | { kind: "app"; id: string; selector: AppSelector }
  | { kind: "all"; id: string };
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
}
const key = (port: Port) => `${port.kind}:${port.id}`;
const selectorKey = (s: AppSelector) => JSON.stringify([s.field, s.value]);
const compatible = (from: Port, to: Port) =>
  (["source", "app", "all"].includes(from.kind) && to.kind === "channel") ||
  (from.kind === "channel" && to.kind === "mix") ||
  (from.kind === "mix" && to.kind === "output");

/** Edits the existing mixer draft. No direct audio-server commands or hidden routes. */
export function connectPorts(
  config: MixerConfig,
  from: Port,
  to: Port,
  snapshot: MixerSnapshot,
  remove = false,
): MixerConfig {
  if (!compatible(from, to)) return config;
  const next = structuredClone(config);
  if (to.kind === "channel") {
    const channel = next.channels.find((c) => c.id === to.id);
    if (!channel) return config;
    if (from.kind === "source") {
      for (const c of next.channels) {
        if (c.id === to.id || !remove)
          c.sources = c.sources.filter((s) => s !== from.id);
      }
      if (!remove) channel.sources.push(from.id);
    }
    if (from.kind === "all") {
      for (const c of next.channels)
        if (c.id === to.id || !remove)
          c.capture_unassigned = !remove && c.id === to.id;
    }
    if (from.kind === "app") {
      for (const c of next.channels) {
        if (c.id === to.id || !remove)
          c.apps = c.apps.filter(
            (s) => !sameApplication(s, from.selector, snapshot.apps),
          );
      }
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
    // The chosen listening device has one selected mix. Other outputs are fixed.
    if (next.monitoring?.output === to.id) {
      if (!remove || next.monitoring.mix === from.id)
        next.monitoring.mix = remove ? null : from.id;
    } else {
      mix.outputs = remove
        ? mix.outputs.filter((o) => o !== to.id)
        : [...new Set([...mix.outputs, to.id])];
      if (remove && mix.output_controls) delete mix.output_controls[to.id];
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
  if (config.monitoring?.output && !config.monitoring.mix)
    problems.push(
      "A listening device is selected, but no mix is connected to it.",
    );
  const audible = (mix: MixerConfig["mixes"][number]) =>
    !mix.muted &&
    mix.volume > 0 &&
    [
      ...mix.outputs.filter(
        (output) =>
          !mix.output_controls?.[output]?.muted &&
          (mix.output_controls?.[output]?.volume ?? 100) > 0,
      ),
      ...(config.monitoring?.mix === mix.id && config.monitoring.output
        ? [config.monitoring.output]
        : []),
    ].some((output) =>
      snapshot.sinks.some((s) => s.name === output && !s.muted),
    );
  for (const channel of config.channels) {
    const routes = config.routes.filter((r) => r.channel === channel.id);
    if (!routes.length)
      problems.push(`${channel.name} has no connection to a mix.`);
    else if (
      !routes.some(
        (r) =>
          !r.muted &&
          r.volume > 0 &&
          config.mixes.some((m) => m.id === r.mix && audible(m)),
      )
    )
      problems.push(
        `${channel.name} has no audible path to a device. Recording outputs may still receive audio.`,
      );
    for (const source of channel.sources)
      if (!snapshot.sources.some((s) => s.name === source))
        problems.push(`${channel.name}: input ${source} is unavailable.`);
  }
  for (const channel of config.channels)
    for (const source of channel.sources) {
      if (snapshot.sources.some((s) => s.name === source && s.muted))
        problems.push(
          `${channel.name}: input ${source} is muted in system settings.`,
        );
    }
  for (const app of snapshot.apps) {
    if (
      snapshot.sinks.some((sink) => sink.name === app.sink && sink.owned) &&
      app.sink.startsWith("deckforge.") &&
      app.sink.includes(".channel.") &&
      !config.channels.some((c) => app.sink.endsWith(`.channel.${c.id}`))
    )
      problems.push(
        `${app.name} is attached to a removed channel. Select an available output for this app in your system sound settings.`,
      );
  }
  return problems;
}

export function MixerPatchbay({
  config,
  snapshot,
  onChange,
  onSetup,
}: {
  config: MixerConfig;
  snapshot: MixerSnapshot;
  onChange: (config: MixerConfig) => void;
  onSetup: () => void;
}) {
  const [start, setStart] = useState<Port | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [pinned, setPinned] = useState<AppSelector[]>([]);
  const [zoom, setZoom] = useState(1);
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
    if (config.monitoring.mix)
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
      label: `${route.volume}%${route.muted ? " · Muted" : ""}`,
      muted: route.muted || route.volume === 0,
    });
  const columns = [0, 1, 2, 3].map((column) =>
    nodes.filter((n) => n.column === column),
  );
  const positions = new Map(
    columns.flatMap((column, x) =>
      column.map(
        (node, y) =>
          [key(node.port), { x: 20 + x * 278, y: 54 + y * 112 }] as const,
      ),
    ),
  );
  const height = Math.max(
    320,
    70 + Math.max(...columns.map((c) => c.length)) * 112,
  );
  const linkId = (link: Link) => JSON.stringify([key(link.from), key(link.to)]);
  const selectedLink = links.find((l) => linkId(l) === selected);
  const name = (port: Port) =>
    nodes.find((n) => key(n.port) === key(port))?.name ?? port.id;
  const problems = routingProblems(config, snapshot);
  const connect = (to: Port) => {
    if (!start) return;
    if (!compatible(start, to)) {
      setNotice(
        "Connect a source to a channel, a channel to a mix, or a mix to an output.",
      );
      return;
    }
    onChange(connectPorts(config, start, to, snapshot));
    setNotice(
      `${name(start)} connected to ${name(to)} in the draft. Apply routes to hear the change.`,
    );
    setStart(null);
  };
  return (
    <section className="mixer-patchbay" aria-label="Advanced routing">
      <div className="patchbay-tools">
        <div>
          <h3>Audio connections</h3>
          <p className="hint">
            Choose an output port, then an input port. Changes stay in your
            draft until Apply routes.
          </p>
        </div>
        <button
          type="button"
          disabled={config.channels.length >= 12}
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
          disabled={config.mixes.length >= 5}
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
          Manage channels &amp; mixes
        </button>
        <label>
          Zoom{" "}
          <select
            aria-label="Routing graph zoom"
            value={zoom}
            onChange={(e) => setZoom(Number(e.target.value))}
          >
            {[0.65, 0.8, 1, 1.2].map((value) => (
              <option key={value} value={value}>
                {Math.round(value * 100)}%
              </option>
            ))}
          </select>
        </label>
      </div>
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
      <p className="patchbay-instruction" role="status">
        {start
          ? `Connect ${name(start)} to a highlighted input port. Escape cancels.`
          : notice ||
            "Connections run from left to right. Select a cable to inspect or remove it."}
      </p>
      <div
        className="patchbay-viewport"
        tabIndex={0}
        aria-label="Audio connection canvas"
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            setStart(null);
            setSelected(null);
          }
        }}
      >
        <div style={{ width: 1100 * zoom, height: height * zoom }}>
          <div
            className="patchbay-canvas"
            style={{ width: 1100, height, transform: `scale(${zoom})` }}
          >
            {["Applications & inputs", "Channels", "Mixes", "Outputs"].map(
              (label, column) => (
                <h4 key={label} style={{ left: 20 + column * 278 }}>
                  {label}
                </h4>
              ),
            )}
            <svg
              width="1100"
              height={height}
              className="patchbay-cables"
              aria-label="Routing connections"
            >
              {links.map((link) => {
                const a = positions.get(key(link.from)),
                  b = positions.get(key(link.to));
                if (!a || !b) return null;
                const id = linkId(link);
                const d = `M ${a.x + 220} ${a.y + 46} C ${a.x + 252} ${a.y + 46}, ${b.x - 32} ${b.y + 46}, ${b.x} ${b.y + 46}`;
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
                output = ["source", "app", "all", "channel", "mix"].includes(
                  node.port.kind,
                );
              return (
                <article
                  key={key(node.port)}
                  className={`patchbay-node ${node.port.kind}${node.missing ? " missing" : ""}`}
                  style={{ left: pos.x, top: pos.y }}
                >
                  {input && (
                    <button
                      type="button"
                      className={`patchbay-port input${start && compatible(start, node.port) ? " compatible" : ""}`}
                      aria-label={`Connect to ${node.name}`}
                      disabled={!start || !compatible(start, node.port)}
                      onClick={() => connect(node.port)}
                      onDragOver={(event) => {
                        if (start && compatible(start, node.port)) {
                          event.preventDefault();
                          event.dataTransfer.dropEffect = "link";
                        }
                      }}
                      onDrop={(event) => {
                        event.preventDefault();
                        const from = nodes.find(
                          (n) =>
                            key(n.port) ===
                            event.dataTransfer.getData(
                              "application/x-deckforge-audio-port",
                            ),
                        )?.port;
                        if (from && compatible(from, node.port)) {
                          onChange(
                            connectPorts(config, from, node.port, snapshot),
                          );
                          setStart(null);
                          setNotice(
                            "Connection added to the draft. Apply routes to update audio.",
                          );
                        }
                      }}
                    >
                      ●
                    </button>
                  )}
                  <strong title={node.name}>{node.name}</strong>
                  <small>{node.detail}</small>
                  {output && (
                    <button
                      type="button"
                      className="patchbay-port output"
                      aria-label={`Connect from ${node.name}`}
                      draggable
                      onDragStart={(event) => {
                        setStart(node.port);
                        event.dataTransfer.effectAllowed = "link";
                        event.dataTransfer.setData(
                          "application/x-deckforge-audio-port",
                          key(node.port),
                        );
                      }}
                      onDragEnd={() => setStart(null)}
                      aria-pressed={!!start && key(start) === key(node.port)}
                      onClick={() => {
                        setStart(node.port);
                        setSelected(null);
                      }}
                    >
                      ●
                    </button>
                  )}
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
            <button
              type="button"
              onClick={() => {
                onChange(
                  connectPorts(
                    config,
                    selectedLink.from,
                    selectedLink.to,
                    snapshot,
                    true,
                  ),
                );
                setSelected(null);
                setNotice(
                  "Connection removed from the draft. Apply routes to update audio.",
                );
              }}
            >
              Disconnect
            </button>
          )}
        </div>
      )}
    </section>
  );
}
