import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { request, pick } from "./bridge";
import { Field, Check, SelectField } from "./Fields";
import type { Snapshot, RecordValue } from "./model";
import { getFormDraft, setFormDraft } from "./transient";
import { deviceName } from "./device";
export type Run = (payload: RecordValue) => Promise<boolean>;
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
}: {
  snapshot: Snapshot;
  run: Run;
  onError: (e: unknown) => void;
}) {
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
              <span className={"status " + (p.connected ? "good" : "")}>
                {p.connected ? "Connected" : p.status}
              </span>
              <button
                onClick={() => run({ command: "restart_plugin", id: p.id })}
              >
                Restart
              </button>
            </div>
            {p.error && (
              <p role="status" className="error">
                {p.error}
              </p>
            )}
            <details>
              <summary>Runtime details</summary>
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
      <details className="advanced">
        <summary>Protected Elgato packages</summary>
        <p className="hint">
          Some downloaded packages require the original Stream Deck executable
          to read their manifest. This does not make Windows executables run on
          Linux.
        </p>
        <Field label="Original Stream Deck executable">
          <input
            value={reference}
            onChange={(e) => setReference(e.target.value)}
          />
        </Field>
        <div className="row">
          <button
            onClick={() =>
              pick("file")
                .then((value) => {
                  if (value) setReference(value);
                })
                .catch(onError)
            }
          >
            Choose executable…
          </button>
          <button
            onClick={() =>
              run({ command: "reference_app", path: reference.trim() || null })
            }
          >
            Save reference
          </button>
        </div>
      </details>
    </>
  );
}
export function Preferences({
  snapshot,
  run,
  onError,
}: {
  snapshot: Snapshot;
  run: Run;
  onError: (e: unknown) => void;
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
      {tab === "Device" && (
        <section className="preference-section">
          <h3>{deviceName(snapshot)}</h3>
          <p className="hint">
            {snapshot.device} · Firmware {snapshot.firmware || "unavailable"}
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
