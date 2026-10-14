import { useEffect, useState } from "react";
import { command, request } from "./bridge";
import { Modal } from "./Panels";
import { Check, Field, SelectField } from "./Fields";
import { ApplicationPicker, type Application } from "./ApplicationPicker";
import type {
  Profile,
  SmartBinding,
  SmartConfig,
  SmartProfileStatus,
} from "./model";

export interface SmartDraft {
  device: string;
  deviceName: string;
  profile: string;
  profiles: Profile[];
  expected: SmartConfig;
  settings: SmartConfig;
}
export function SmartProfiles({
  draft,
  status,
  busy,
  error,
  onChange,
  onSave,
  onClose,
  onError,
}: {
  draft: SmartDraft;
  status?: SmartProfileStatus;
  busy: boolean;
  error: string;
  onChange: (value: SmartDraft) => void;
  onSave: () => void;
  onClose: () => void;
  onError: (e: unknown) => void;
}) {
  const [catalog, setCatalog] = useState<Application[]>([]);
  const [search, setSearch] = useState("");
  useEffect(() => {
    let live = true;
    void request<Application[]>("application_catalog")
      .then((items) => {
        if (live) setCatalog(items);
      })
      .catch(onError);
    void command({ command: "smart_preview", enabled: true }).catch(onError);
    return () => {
      live = false;
      void command({ command: "smart_preview", enabled: false }).catch(
        () => {},
      );
    };
  }, [onError]);
  const profiles = draft.profiles.filter((p) => p.compatible !== false);
  const profile = profiles.find((p) => p.id === draft.profile);
  const binding = draft.settings.bindings[draft.profile];
  const matchingApplications = catalog.filter((app) => binding && (
    binding.kind === "desktop_id" ? app.id === binding.value
      : binding.kind === "window_class" ? app.window_class === binding.value
        : app.executable === binding.value
  ));
  const selectedApplication = matchingApplications.length === 1 ? matchingApplications[0]
    : matchingApplications.find((app) => app.name === binding?.name);
  const update = (settings: SmartConfig) => onChange({ ...draft, settings });
  const bind = (value: SmartBinding | null) => {
    const bindings = { ...draft.settings.bindings };
    if (value) bindings[draft.profile] = value;
    else delete bindings[draft.profile];
    update({ ...draft.settings, bindings });
  };
  const recent = new Map<string, SmartBinding>();
  for (const app of status?.recent ?? []) {
    const name =
      app.window_class ||
      app.desktop_id ||
      app.executable?.split("/").pop() ||
      "Application";
    const candidates: SmartBinding[] = [];
    if (app.desktop_id)
      candidates.push({ kind: "desktop_id", value: app.desktop_id, name });
    if (app.executable)
      candidates.push({ kind: "executable", value: app.executable, name });
    if (app.window_class)
      candidates.push({ kind: "window_class", value: app.window_class, name });
    for (const item of candidates)
      recent.set(`${item.kind}:${item.value}`, item);
  }
  const duplicates = binding
    ? profiles.filter(
        (p) =>
          p.id !== draft.profile &&
          draft.settings.bindings[p.id]?.kind === binding.kind &&
          draft.settings.bindings[p.id]?.value === binding.value,
      )
    : [];
  return (
    <Modal
      title="Smart Profiles"
      className="smart-profile-dialog"
      onClose={onClose}
      closeDisabled={busy}
      wide
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSave();
        }}
      >
        <fieldset disabled={busy} className="smart-profile-fields">
          <p className="hint smart-device-name">{draft.deviceName}</p>
          <Check
            label="Switch profiles automatically"
            checked={draft.settings.enabled}
            onChange={(enabled) => update({ ...draft.settings, enabled })}
          />
          <p className="hint">
            Switching pauses while the editor or a plugin inspector is open.
            Hide or minimize the editor to use your rules. Focus detection
            currently requires KDE Plasma 6.
          </p>
          <SelectField
            label="Default profile"
            value={draft.settings.fallback ?? ""}
            options={[
              ["", "Keep current profile when no application matches"],
              ...draft.profiles.map(
                (p) =>
                  [
                    p.id,
                    p.name +
                      (p.compatible === false ? " (different layout)" : ""),
                  ] as [string, string],
              ),
            ]}
            onChange={(value) =>
              update({ ...draft.settings, fallback: String(value) || null })
            }
          />
          <hr />
          <SelectField
            label="Profile to configure"
            value={draft.profile}
            options={profiles.map((p) => [p.id, p.name])}
            onChange={(value) => onChange({ ...draft, profile: String(value) })}
          />
          {profile?.app_identifier && (
            <p className="hint">
              {profile.app_identifier === "*"
                ? "This profile has an imported default-profile marker."
                : `Imported application: ${profile.app_identifier}. Choose its Linux application below.`}
            </p>
          )}
          <Field label="Find installed application">
            <input value={search} onChange={(e) => setSearch(e.target.value)} />
          </Field>
          <ApplicationPicker applications={catalog.filter((app) => app.id)} search={search}
            value={selectedApplication?.desktop || ""}
            onSelect={(app) => bind({
              kind: app.window_class ? "window_class" : "desktop_id",
              value: app.window_class || app.id!, name: app.name.slice(0, 128),
            })} />
          <Field label="Recently focused application">
            <select
              value=""
              onChange={(e) => {
                const item = recent.get(e.target.value);
                if (item) bind({ ...item, name: item.name.slice(0, 128) });
              }}
            >
              <option value="">Choose an observed identity…</option>
              {[...recent].map(([key, item]) => (
                <option key={key} value={key}>
                  {item.value} · {item.kind.replaceAll("_", " ")}
                </option>
              ))}
            </select>
          </Field>
          <p className="hint">
            Focus your application, then return here to choose its observed
            identity. Some apps do not report a desktop ID; use their executable
            or window class instead.
          </p>
          {binding && (
            <p className="hint smart-binding-summary">
              Matches {binding.kind.replaceAll("_", " ")}:{" "}
              {binding.value || "choose an identity"}
            </p>
          )}
          <details data-smart-advanced>
            <summary>Advanced application identity</summary>
            <div className="smart-profile-fields">
              <SelectField
                label="Match by"
                value={binding?.kind ?? ""}
                options={[
                  ["", "No application binding"],
                  ["desktop_id", "Desktop ID"],
                  ["executable", "Executable path"],
                  ["window_class", "Window class"],
                ]}
                onChange={(value) =>
                  bind(
                    value
                      ? {
                          kind: value as SmartBinding["kind"],
                          value: binding?.value ?? "",
                          name: binding?.name ?? "",
                        }
                      : null,
                  )
                }
              />
              {binding && (
                <>
                  <Field label="Application identity">
                    <input
                      required
                      maxLength={binding.kind === "executable" ? 4096 : 1024}
                      value={binding.value}
                      onChange={(e) =>
                        bind({ ...binding, value: e.target.value })
                      }
                    />
                  </Field>
                  <Field label="Application label">
                    <input
                      maxLength={128}
                      value={binding.name}
                      onChange={(e) =>
                        bind({ ...binding, name: e.target.value })
                      }
                    />
                  </Field>
                </>
              )}
            </div>
          </details>
          {duplicates.length > 0 && (
            <p className="warning">
              Also assigned to {duplicates.map((p) => p.name).join(", ")}. The
              first matching profile in sidebar order wins.
            </p>
          )}
          <p role="status" className="hint">
            {status?.error ??
              (status?.observing
                ? "Focus observer connected. Switching is paused while editing."
                : "Starting focus observer…")}
          </p>
          {error && (
            <p role="alert" className="warning">
              {error}
            </p>
          )}
        </fieldset>
        <div className="modal-actions">
          <button type="button" disabled={busy} onClick={onClose}>
            Cancel
          </button>
          <button
            type="submit"
            className="primary"
            disabled={busy || !profile || (!!binding && !binding.value)}
          >
            {busy ? "Saving…" : "Save Smart Profiles"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
