import { useEffect, useState } from "react";
import { request } from "./bridge";
import type { RecordValue } from "./model";
import type { Run } from "./Panels";

export function PluginSetup({
  plugin,
  run,
  busy,
}: {
  plugin: RecordValue;
  run: Run;
  busy: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const [choice, setChoice] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [opened, setOpened] = useState(false);
  const [bridge, setBridge] = useState<RecordValue | null>(null);
  const actions: { uuid: string; name: string }[] = plugin.setup_actions ?? [];
  const selected = actions.some((a) => a.uuid === choice)
    ? choice
    : actions[0]?.uuid;
  const obs = plugin.id === "com.elgato.obsstudio";
  async function checkBridge(operation = "status") {
    setPending(true);
    setError("");
    try {
      setBridge(await request("obs_bridge", { operation }));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }
  useEffect(() => {
    if (expanded && obs) void checkBridge();
  }, [expanded, obs]);
  async function open() {
    setPending(true);
    setError("");
    setOpened(false);
    try {
      if (await run({ command: "plugin_setup", id: plugin.id, uuid: selected }))
        setOpened(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }
  return (
    <section className="plugin-setup">
      <button
        type="button"
        aria-label={`Configure ${plugin.name}`}
        aria-expanded={expanded}
        onClick={() => setExpanded(!expanded)}
      >
        Configure…
      </button>
      {expanded && (
        <div className="plugin-setup-content">
          {obs && (
            <>
              <h4>OBS connection</h4>
              <p className="hint">
                OBS needs the Stream Deck bridge. This connection is separate
                from the plugin's connection to Deckforge.
              </p>
              {bridge && (
                <p role="status">
                  {bridge.connection
                    ? `OBS ${bridge.connection.obs_version} connected · bridge ${bridge.connection.bridge_version}`
                    : bridge.installed
                      ? "Bridge installed. Fully quit and reopen OBS when you are ready, then check the connection."
                      : "OBS bridge not detected. Install the bridge for native Linux OBS."}
                </p>
              )}
              <div className="row">
                {bridge?.available && !bridge?.installed && (
                  <button
                    type="button"
                    disabled={pending || busy}
                    onClick={() => void checkBridge("install")}
                  >
                    Install OBS bridge
                  </button>
                )}
                <button
                  type="button"
                  disabled={pending || busy}
                  onClick={() => void checkBridge()}
                >
                  {pending ? "Checking…" : "Check OBS connection"}
                </button>
              </div>
              {bridge && !bridge.available && !bridge.installed && (
                <p className="hint">
                  This build has no OBS bridge package. Build it with
                  scripts/build-obs-bridge.py and run the desktop installer
                  again.
                </p>
              )}
              <p className="hint">
                For Flatpak OBS, install a compatible Stream Deck OBS bridge
                through Flatpak. The native bridge installer does not modify
                Flatpak.
              </p>
            </>
          )}
          <h4>Plugin settings</h4>
          {actions.length ? (
            <>
              <p className="hint">
                Open the plugin's own setup page to sign in or change shared
                settings. Shared settings are saved. Action-specific selections
                in this setup window are temporary; edit an assigned key to save
                those.
              </p>
              <div className="row">
                {actions.length > 1 && (
                  <label>
                    Settings page{" "}
                    <select
                      aria-label={`${plugin.name} settings page`}
                      value={selected}
                      onChange={(e) => {
                        setChoice(e.target.value);
                        setOpened(false);
                      }}
                    >
                      {actions.map((a) => (
                        <option key={a.uuid} value={a.uuid}>
                          {a.name}
                        </option>
                      ))}
                    </select>
                  </label>
                )}
                <button
                  type="button"
                  disabled={
                    pending ||
                    busy ||
                    plugin.enabled === false ||
                    !plugin.connected
                  }
                  onClick={() => void open()}
                >
                  Open plugin settings
                </button>
              </div>
              {plugin.enabled === false ? (
                <p className="hint">Enable this plugin to open its settings.</p>
              ) : (
                !plugin.connected && (
                  <p className="hint">
                    Wait for the plugin to connect, or restart it if it has
                    stopped.
                  </p>
                )
              )}
              {opened && (
                <p role="status">
                  Plugin settings opened in a separate window.
                </p>
              )}
            </>
          ) : (
            <p className="hint">
              This plugin does not provide a settings page.
            </p>
          )}
          {error && (
            <p role="alert" className="error">
              {error}
            </p>
          )}
        </div>
      )}
    </section>
  );
}
