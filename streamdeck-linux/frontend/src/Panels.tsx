import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { request, pick } from "./bridge";
import { Field, Check, SelectField } from "./Fields";
import type { Snapshot, RecordValue } from "./model";
import { getFormDraft, setFormDraft } from "./transient";
import { PluginApplications } from "./PluginApplications";
import { PluginLanguage } from "./PluginLanguage";
import { ReferenceInstaller } from "./ReferenceInstaller";
import { PluginUpdate } from "./PluginUpdate";
import { BundledPlugins } from "./BundledPlugins";
import { Marketplace } from "./Marketplace";
import { deviceName } from "./device";
export type Run = (payload: RecordValue) => Promise<boolean>;
type PluginRemoval = {
  id: string;
  name: string;
  path: string;
  enabled: boolean;
  mode: "archive" | "detach" | "missing";
  reason?: string;
  error?: string;
};
export function Modal({
  title,
  children,
  onClose,
  wide = false,
  className,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  wide?: boolean;
  className?: string;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = ref.current;
    const active = document.activeElement as HTMLElement;
    dialog?.showModal();
    return () => {
      dialog?.close();
      active?.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      className={`${wide ? "wide" : ""} ${className ?? ""}`}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      <header className="dialog-header">
        <h2 id={titleId}>{title}</h2>
        <button aria-label={`Close ${title}`} onClick={onClose}>
          ×
        </button>
      </header>
      <div className="dialog-content">{children}</div>
    </dialog>
  );
}
export function Plugins({
  snapshot,
  run,
  onError,
  error = "",
}: {
  snapshot: Snapshot;
  run: Run;
  onError: (e: unknown) => void;
  error?: string;
}) {
  const [pluginBusy, setPluginBusy] = useState<
    Record<
      string,
      "enable" | "disable" | "restart" | "remove" | "update" | undefined
    >
  >({});
  const [removal, setRemoval] = useState<PluginRemoval | null>(null);
  async function removePlugin() {
    if (!removal || pluginBusy[removal.id]) return;
    const captured = removal;
    setPluginBusy((previous) => ({ ...previous, [captured.id]: "remove" }));
    setRemoval((current) => current && { ...current, error: undefined });
    try {
      if (
        await run({
          command: "uninstall_plugin",
          id: captured.id,
          expected_path: captured.path,
          expected_enabled: captured.enabled,
          confirm: true,
        })
      )
        setRemoval((current) => (current?.id === captured.id ? null : current));
      else
        setRemoval((current) =>
          current?.id === captured.id
            ? {
                ...current,
                error:
                  "Removal did not complete. Review the reported error, or cancel and review its current state.",
              }
            : current,
        );
    } catch (error) {
      onError(error);
      setRemoval((current) =>
        current?.id === captured.id
          ? {
              ...current,
              error: error instanceof Error ? error.message : String(error),
            }
          : current,
      );
    } finally {
      setPluginBusy((previous) => ({ ...previous, [captured.id]: undefined }));
    }
  }
  async function pluginCommand(
    id: string,
    operation: "enable" | "disable" | "restart",
    expectedEnabled: boolean,
  ) {
    if (pluginBusy[id]) return;
    setPluginBusy((previous) => ({ ...previous, [id]: operation }));
    try {
      await run(
        operation === "restart"
          ? { command: "restart_plugin", id }
          : {
              command: "plugin_enabled",
              id,
              enabled: operation === "enable",
              expected_enabled: expectedEnabled,
            },
      );
    } catch (error) {
      onError(error);
    } finally {
      setPluginBusy((previous) => ({ ...previous, [id]: undefined }));
    }
  }
  const saved = getFormDraft("plugins") ?? {};
  const [path, setPath] = useState(saved.path ?? "");
  const [reference, setReference] = useState(
    saved.reference ?? snapshot.reference_app ?? "",
  );
  useEffect(
    () => setFormDraft("plugins", { path, reference }),
    [path, reference],
  );
  const [busy, setBusy] = useState(false);
  const browse = async (kind: string) => {
    try {
      const result = await pick(kind);
      if (result) setPath(result);
    } catch (e) {
      onError(e);
    }
  };
  return (
    <>
      <p className="hint">
        Install a Stream Deck package or a plugin folder. Actions appear in the
        library when the plugin loads.
      </p>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      {snapshot.application_monitor?.error && (
        <p className="warning" role="status">
          Application monitoring: {snapshot.application_monitor.error}
        </p>
      )}
      <BundledPlugins
        installed={snapshot.plugins.map((plugin) => plugin.id)}
        run={run}
      />
      <div className="plugin-install">
        <Field label="Plugin package or folder">
          <input
            value={path}
            onChange={(e) => setPath(e.target.value)}
            placeholder="/path/to/plugin.streamDeckPlugin"
          />
        </Field>
        <div className="row">
          <button onClick={() => browse("plugin")}>Choose package…</button>
          <button onClick={() => browse("directory")}>Choose folder…</button>
          <button
            className="primary"
            disabled={!path.trim() || busy}
            onClick={async () => {
              setBusy(true);
              try {
                if (await run({ command: "install_plugin", path: path.trim() }))
                  setPath("");
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? "Installing…" : "Install plugin"}
          </button>
        </div>
      </div>
      <div className="plugin-list">
        {snapshot.plugins.map((p) => (
          <article className="plugin-row" key={p.id}>
            <div className="plugin-heading">
              <div>
                <h3>{p.name}</h3>
                <small>
                  {p.version ? `v${p.version} · ` : ""}
                  {p.id}
                </small>
              </div>
              <span
                className={
                  "status " + (p.enabled !== false && p.connected ? "good" : "")
                }
              >
                {p.enabled === false
                  ? "Disabled"
                  : p.connected
                    ? "Connected"
                    : p.status}
              </span>
              <button
                type="button"
                disabled={!!pluginBusy[p.id]}
                aria-label={`${p.enabled === false ? "Enable" : "Disable"} ${p.name}`}
                onClick={() =>
                  void pluginCommand(
                    p.id,
                    p.enabled === false ? "enable" : "disable",
                    p.enabled !== false,
                  )
                }
              >
                {pluginBusy[p.id] === "enable"
                  ? "Enabling…"
                  : pluginBusy[p.id] === "disable"
                    ? "Disabling…"
                    : p.enabled === false
                      ? "Enable"
                      : "Disable"}
              </button>
              <button
                type="button"
                disabled={p.enabled === false || !!pluginBusy[p.id]}
                aria-label={`Restart ${p.name}`}
                onClick={() =>
                  void pluginCommand(p.id, "restart", p.enabled !== false)
                }
              >
                {pluginBusy[p.id] === "restart" ? "Restarting…" : "Restart"}
              </button>
              <button
                type="button"
                className="danger quiet"
                aria-label={`Remove ${p.name}`}
                disabled={
                  !!pluginBusy[p.id] ||
                  typeof p.path !== "string" ||
                  !["archive", "detach", "missing"].includes(p.removal_mode)
                }
                title={
                  p.removal_reason ||
                  (!["archive", "detach", "missing"].includes(p.removal_mode)
                    ? "Removal is unavailable in this host."
                    : undefined)
                }
                onClick={() =>
                  setRemoval({
                    id: p.id,
                    name: p.name,
                    path: p.path,
                    enabled: p.enabled !== false,
                    mode: p.removal_mode,
                    reason: p.removal_reason,
                  })
                }
              >
                {pluginBusy[p.id] === "remove" ? "Removing…" : "Remove"}
              </button>
            </div>
            {removal && removal.id === p.id && (
              <section
                className="plugin-removal"
                aria-label={`Remove ${removal.name} confirmation`}
                onKeyDown={(event) => {
                  if (event.key === "Escape" && !pluginBusy[p.id]) {
                    event.preventDefault();
                    event.stopPropagation();
                    setRemoval(null);
                  }
                }}
              >
                <h4>Remove {removal.name}?</h4>
                <code>{removal.path}</code>
                <p>
                  {removal.mode === "archive"
                    ? "The managed plugin folder will be moved to a private removed-plugins backup."
                    : removal.mode === "detach"
                      ? "The external plugin folder will be left untouched. Deckforge will stop loading it."
                      : "The unavailable plugin entry will be removed from Deckforge."}
                </p>
                <p className="hint">
                  Profile assignments, action settings, and private plugin data
                  are retained. Assigned controls will be unavailable until you
                  reinstall the plugin.
                </p>
                {removal.reason && <p className="hint">{removal.reason}</p>}
                {removal.error && (
                  <p className="error" role="alert">
                    {removal.error}
                  </p>
                )}
                <div className="row">
                  <button
                    type="button"
                    className="danger"
                    disabled={!!pluginBusy[p.id]}
                    aria-label={`Confirm removal of ${removal.name}`}
                    onClick={() => void removePlugin()}
                  >
                    {pluginBusy[p.id] === "remove"
                      ? "Removing…"
                      : "Remove plugin"}
                  </button>
                  <button
                    type="button"
                    autoFocus
                    disabled={!!pluginBusy[p.id]}
                    onClick={() => setRemoval(null)}
                  >
                    Cancel
                  </button>
                </div>
              </section>
            )}
            <PluginUpdate
              plugin={p}
              actions={snapshot.actions ?? []}
              busy={!!pluginBusy[p.id]}
              onBusy={(busy) =>
                setPluginBusy((previous) => ({
                  ...previous,
                  [p.id]: busy ? "update" : undefined,
                }))
              }
              run={run}
            />
            {p.error && (
              <p role="status" className="error">
                {p.error}
              </p>
            )}
            {Array.isArray(p.applications) && p.applications.length > 0 && (
              <PluginApplications
                id={p.id}
                applications={p.applications}
                mappings={p.application_mappings ?? {}}
                run={run}
              />
            )}
            <details>
              <summary>Runtime details</summary>
              {p.localization_warning && (
                <p className="warning" role="status">
                  Localization: {p.localization_warning}
                </p>
              )}
              <dl>
                {[
                  ["Plugin folder", p.path],
                  ["Executable", p.entry_path],
                  ["Runtime", p.runtime_path],
                  ["Log", p.log_path],
                ]
                  .filter(([, v]) => v)
                  .map(([name, value]) => (
                    <div key={name}>
                      <dt>{name}</dt>
                      <dd>{value}</dd>
                    </div>
                  ))}
              </dl>
            </details>
          </article>
        ))}
        {snapshot.plugins.length === 0 && (
          <div className="empty">
            <h3>No plugins installed</h3>
            <p>
              Choose a package above. Built-in actions are ready in the action
              library.
            </p>
          </div>
        )}
      </div>
      <ReferenceInstaller
        configuredPath={snapshot.reference_app}
        reference={reference}
        onReferenceChange={setReference}
        run={run}
      />
    </>
  );
}
export function Preferences({
  snapshot,
  run,
  onError,
  error = "",
}: {
  snapshot: Snapshot;
  run: Run;
  onError: (e: unknown) => void;
  error?: string;
}) {
  const formKey = `preferences:${snapshot.device}`;
  const saved = getFormDraft(formKey) ?? {};
  const [tab, setTab] = useState(saved.tab ?? "Device");
  const [brightness, setBrightness] = useState(
    saved.brightness ?? snapshot.device_settings?.desired?.brightness ?? 80,
  );
  const [startup, setStartup] = useState<{
    enabled: boolean;
    this_session: boolean;
  } | null>(null);
  const prefs = snapshot.screensaver?.preferences ?? {};
  const [minutes, setMinutes] = useState(
    saved.minutes ?? prefs.after_minutes ?? 0,
  );
  const [lock, setLock] = useState(saved.lock ?? prefs.disable_on_lock ?? true);
  const [image, setImage] = useState<string | null>(
    saved.image !== undefined ? saved.image : (prefs.image ?? null),
  );
  useEffect(
    () => setFormDraft(formKey, { tab, brightness, minutes, lock, image }),
    [formKey, tab, brightness, minutes, lock, image],
  );
  useEffect(() => {
    request("startup_status").then(setStartup).catch(onError);
  }, []);
  return (
    <>
      <div className="segmented">
        {[
          "Device",
          "Startup & about",
          ...(snapshot.marketplace ? ["Marketplace"] : []),
          ...(snapshot.device_capabilities?.sleep === false ||
          snapshot.screensaver?.supported === false
            ? []
            : ["Screensaver"]),
        ].map((name) => (
          <button
            key={name}
            aria-pressed={tab === name}
            onClick={() => setTab(name)}
          >
            {name}
          </button>
        ))}
      </div>
      {tab === "Marketplace" && snapshot.marketplace && (
        <Marketplace state={snapshot.marketplace} />
      )}
      {tab === "Device" && (
        <section className="preference-section">
          <h3>{deviceName(snapshot)}</h3>
          <p className="hint">
            {snapshot.device.startsWith("offline-streamdeck-plus-")
              ? "Empty workspace. Connect a matching device to use these profiles."
              : `${snapshot.device} · Firmware ${snapshot.firmware || "unavailable"}`}
          </p>
          <DeviceNameForm snapshot={snapshot} run={run} />
          {snapshot.device_capabilities?.brightness !== false ? (
            <>
              <Field label={`Brightness · ${brightness}%`}>
                <input
                  type="range"
                  min="0"
                  max="100"
                  value={brightness}
                  onChange={(e) => setBrightness(Number(e.target.value))}
                />
              </Field>
              <button
                className="primary"
                onClick={() =>
                  run({
                    command: "device_controls",
                    settings: {
                      ...snapshot.device_settings?.desired,
                      brightness,
                    },
                  })
                }
              >
                Apply brightness
              </button>
            </>
          ) : (
            <p className="hint">
              This device has no display brightness control.
            </p>
          )}
          {snapshot.device_capabilities?.transport_supported === false && (
            <p className="warning">
              Physical transport for this model is not supported yet. The
              current layout is available for profile editing and simulation.
            </p>
          )}
          {!snapshot.connected && (
            <p className="hint">
              Saved settings apply when your device reconnects.
            </p>
          )}
          {snapshot.device_settings?.error && (
            <p className="error">{snapshot.device_settings.error}</p>
          )}
          {(!snapshot.device.startsWith("offline-streamdeck-plus-") ||
            (snapshot.devices?.length ?? 1) > 1) && (
            <ForgetDevice snapshot={snapshot} run={run} />
          )}
        </section>
      )}
      {tab === "Startup & about" && (
        <section className="preference-section">
          <h3>Ready when you sign in</h3>
          {startup ? (
            <Check
              label="Start Deckforge in the background at login"
              checked={startup.enabled && startup.this_session}
              onChange={(enabled) =>
                request("startup_set", { enabled })
                  .then(setStartup)
                  .catch(onError)
              }
            />
          ) : (
            <p>Reading startup preference…</p>
          )}
          <hr />
          <h3>Action feedback</h3>
          <Check
            label="Show Multi Action progress and completion check marks"
            checked={snapshot.multi_action_feedback ?? true}
            onChange={(enabled) =>
              run({
                command: "multi_action_feedback",
                enabled,
                baseline: snapshot.multi_action_feedback ?? true,
              })
            }
          />
          <p className="hint">Applies to Multi Actions on all devices.</p>
          {snapshot.plugin_language && (
            <>
              <hr />
              <PluginLanguage
                language={snapshot.plugin_language}
                run={run}
                error={error}
              />
            </>
          )}
          <hr />
          <h3>Deckforge</h3>
          <p>
            A native Linux application with a Rust device and plugin host, and a
            TypeScript editor.
          </p>
          <p className="hint">
            Built with AI because an expensive Stream Deck should keep working
            on Linux.
          </p>
        </section>
      )}
      {tab === "Screensaver" &&
        snapshot.device_capabilities?.sleep !== false &&
        snapshot.screensaver?.supported !== false && (
          <section className="preference-section">
            <p className="hint">
              Existing screensaver controls. Further screensaver work is
              deferred.
            </p>
            <SelectField
              label="Sleep after"
              value={minutes}
              options={[
                [0, "Never"],
                [5, "5 minutes"],
                [15, "15 minutes"],
                [30, "30 minutes"],
                [60, "1 hour"],
                [120, "2 hours"],
              ]}
              onChange={(value) => setMinutes(Number(value))}
            />
            <Check
              label="Disable controls while the computer is locked"
              checked={lock}
              onChange={setLock}
            />
            <Field label="Image">
              <input readOnly value={image ?? "Deckforge logo"} />
            </Field>
            <div className="row">
              <button
                onClick={() =>
                  pick("image")
                    .then((value) => {
                      if (value) setImage(value);
                    })
                    .catch(onError)
                }
              >
                Choose image…
              </button>
              <button onClick={() => setImage(null)}>Use logo</button>
            </div>
            <div className="row">
              <button
                className="primary"
                onClick={() =>
                  run({
                    command: "screensaver",
                    device: snapshot.device,
                    after_minutes: minutes,
                    disable_on_lock: lock,
                    image,
                  })
                }
              >
                Apply
              </button>
              <button onClick={() => run({ command: "sleep_device" })}>
                Preview on device
              </button>
              <button onClick={() => run({ command: "wake_device" })}>
                Wake
              </button>
            </div>
            {snapshot.screensaver?.error && (
              <p className="error">{snapshot.screensaver.error}</p>
            )}
          </section>
        )}
    </>
  );
}

export function DeviceNameForm({
  snapshot,
  run,
}: {
  snapshot: Snapshot;
  run: Run;
}) {
  const key = `device-name:${snapshot.device}`;
  const current = snapshot.device_custom_name ?? "";
  const [draft, setDraft] = useState(() => {
    const saved = getFormDraft(key);
    return saved && saved.value !== saved.source
      ? (saved as { value: string; source: string })
      : { value: current, source: current };
  });
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const hintId = useId();
  const invalid =
    Array.from(draft.value).length > 64 ||
    /[\u0000-\u001f\u007f-\u009f]/u.test(draft.value);
  useEffect(() => {
    setDraft((previous) =>
      previous.value === previous.source
        ? { value: current, source: current }
        : previous,
    );
  }, [current]);
  useEffect(() => setFormDraft(key, draft), [key, draft]);
  const save = async (name: string) => {
    setBusy(true);
    setStatus("");
    try {
      if (
        await run({ command: "device_rename", device: snapshot.device, name })
      ) {
        setDraft({ value: name.trim(), source: name.trim() });
        setStatus(
          name.trim() ? "Device name saved." : "Default name restored.",
        );
      } else
        setStatus("Could not save the device name. Your draft is preserved.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      className="device-name-form"
      onSubmit={(event) => {
        event.preventDefault();
        if (!invalid && !busy) void save(draft.value);
      }}
    >
      <Field label="Device name">
        <input
          value={draft.value}
          placeholder={snapshot.device_model_name ?? deviceName(snapshot)}
          disabled={busy}
          aria-invalid={invalid}
          aria-describedby={hintId}
          onChange={(event) => {
            setDraft({ ...draft, value: event.target.value });
            setStatus("");
          }}
        />
      </Field>
      <p id={hintId} className={invalid ? "error" : "hint"}>
        {invalid
          ? "Use at most 64 characters without control characters."
          : "Choose a name for the device picker and plugins. Leave blank to use the model name."}
      </p>
      <div className="row">
        <button
          className="primary"
          type="submit"
          disabled={busy || invalid || draft.value === current}
        >
          {busy ? "Saving…" : "Save name"}
        </button>
        <button
          type="button"
          disabled={busy || (!current && !draft.value)}
          onClick={() => void save("")}
        >
          Use default name
        </button>
      </div>
      {status && <p role="status">{status}</p>}
    </form>
  );
}

export function ForgetDevice({
  snapshot,
  run,
}: {
  snapshot: Snapshot;
  run: Run;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const launch = useRef<HTMLButtonElement>(null);
  const wasConfirming = useRef(false);
  useEffect(() => {
    if (!confirming && wasConfirming.current) launch.current?.focus();
    wasConfirming.current = confirming;
  }, [confirming]);
  const online = snapshot.device_online ?? snapshot.connected;
  const profileCount = snapshot.library.profiles.length;
  const reasonId = useId();
  return (
    <section className="forget-device" aria-label="Forget device">
      <h3>Forget this device</h3>
      <p id={reasonId} className="hint">
        {online
          ? "Disconnect this device before forgetting it."
          : "Remove this remembered device and its saved profiles. A recovery copy of the session is kept locally."}
      </p>
      {confirming ? (
        <>
          <p role="alert">
            Forget {deviceName(snapshot)} and remove {profileCount} saved{" "}
            {profileCount === 1 ? "profile" : "profiles"}? Other devices keep
            their profiles.
          </p>
          <div className="row">
            <button
              autoFocus
              disabled={busy}
              onClick={() => {
                setConfirming(false);
                setFailed(false);
              }}
            >
              Cancel
            </button>
            <button
              className="danger"
              disabled={busy || online}
              aria-describedby={reasonId}
              onClick={async () => {
                setBusy(true);
                setFailed(false);
                try {
                  if (
                    await run({
                      command: "device_forget",
                      device: snapshot.device,
                      confirm: true,
                    })
                  )
                    setConfirming(false);
                  else setFailed(true);
                } finally {
                  setBusy(false);
                }
              }}
            >
              {busy ? "Forgetting…" : "Forget device and profiles"}
            </button>
          </div>
          {failed && (
            <p role="status" className="error">
              Could not forget this device. Check that it is disconnected and
              try again.
            </p>
          )}
        </>
      ) : (
        <button
          ref={launch}
          className="danger"
          disabled={online}
          aria-describedby={reasonId}
          onClick={() => setConfirming(true)}
        >
          Forget device…
        </button>
      )}
    </section>
  );
}
