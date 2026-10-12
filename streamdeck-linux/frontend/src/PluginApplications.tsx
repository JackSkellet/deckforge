import { useEffect, useState } from "react";
import { getFormDraft, setFormDraft } from "./transient";
import { Field } from "./Fields";
import type { RecordValue } from "./model";
export interface MonitoredApplication {
  application: string;
  platforms: string[];
  executable: string;
}
type Mappings = Record<string, string>;
interface Draft {
  baseline: Mappings;
  mappings: Mappings;
  defaults: Mappings;
  expanded: boolean;
  dirty: boolean;
}
const fingerprint = (value: Mappings) =>
  JSON.stringify(Object.entries(value).sort(([a], [b]) => a.localeCompare(b)));
export function PluginApplications({
  id,
  applications,
  mappings,
  run,
}: {
  id: string;
  applications: MonitoredApplication[];
  mappings: Mappings;
  run: (payload: RecordValue) => Promise<boolean>;
}) {
  const key = `plugin-applications:${id}`;
  function fresh(expanded = false): Draft {
    return {
      baseline: { ...mappings },
      mappings: { ...mappings },
      defaults: Object.fromEntries(
        applications.map((item) => [item.application, item.executable]),
      ),
      expanded,
      dirty: false,
    };
  }
  const [draft, setDraft] = useState<Draft>(() => getFormDraft(key) ?? fresh());
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const actual = fingerprint(mappings);
  const effective = JSON.stringify(applications);
  const stale = draft.dirty && fingerprint(draft.baseline) !== actual;
  useEffect(() => {
    setFormDraft(key, draft);
  }, [key, draft]);
  useEffect(() => {
    setDraft((current) => {
      if (current.dirty && fingerprint(current.mappings) !== actual)
        return current;
      const next = fresh(current.expanded);
      return JSON.stringify(current) === JSON.stringify(next) ? current : next;
    });
  }, [actual, effective]);
  function change(application: string, value: string) {
    setStatus("");
    setDraft((current) => {
      const next = { ...current.mappings, [application]: value };
      return {
        ...current,
        mappings: next,
        dirty: fingerprint(next) !== fingerprint(current.baseline),
      };
    });
  }
  async function apply() {
    if (!draft.dirty || stale || busy) return;
    const captured = draft;
    setBusy(true);
    setStatus("");
    try {
      const saved = await run({
        command: "plugin_applications",
        id,
        mappings: captured.mappings,
        baseline: captured.baseline,
      });
      if (saved) {
        setDraft((current) => ({
          ...current,
          baseline: { ...captured.mappings },
          dirty:
            fingerprint(current.mappings) !== fingerprint(captured.mappings),
        }));
        setStatus("Application mappings saved");
      } else
        setStatus(
          "Mappings were not saved. Review the error and reload or retry.",
        );
    } catch {
      setStatus("Mappings were not saved. Reload or retry.");
    } finally {
      setBusy(false);
    }
  }
  return (
    <details
      className="plugin-applications"
      open={draft.expanded}
      onToggle={(event) => {
        const expanded = event.currentTarget.open;
        setDraft((current) =>
          current.expanded === expanded ? current : { ...current, expanded },
        );
      }}
    >
      <summary>Monitored applications</summary>
      <p className="hint">
        Map the plugin’s declared application identifiers to Linux executables
        for launch and quit events. Use an executable basename or absolute path,
        without arguments. Leave blank to disable monitoring for that
        identifier.
      </p>
      {applications.map((item) => {
        const value = Object.hasOwn(draft.mappings, item.application)
          ? draft.mappings[item.application]
          : Object.hasOwn(draft.defaults, item.application)
            ? draft.defaults[item.application]
            : item.executable;
        return (
          <div className="plugin-application" key={item.application}>
            <div className="plugin-application-identity">
              <code>{item.application}</code>
              <small>{item.platforms.join(", ")}</small>
            </div>
            <Field label="Linux executable">
              <input
                aria-label={`Linux executable for ${item.application}`}
                disabled={busy}
                value={value}
                onChange={(event) =>
                  change(item.application, event.target.value)
                }
                placeholder="application or /usr/bin/application"
                spellCheck={false}
                autoComplete="off"
              />
            </Field>
            {!value && <small className="hint">Not mapped</small>}
          </div>
        );
      })}
      {stale && (
        <p className="warning" role="status">
          Saved mappings changed while you were editing. Your draft is kept.
          Reload mappings before applying new changes.
        </p>
      )}
      <div className="row">
        <button
          type="button"
          className="primary"
          disabled={!draft.dirty || stale || busy}
          onClick={() => void apply()}
        >
          {busy ? "Applying…" : "Apply mappings"}
        </button>
        <button
          type="button"
          disabled={!draft.dirty || busy}
          onClick={() => {
            setDraft(fresh(draft.expanded));
            setStatus("");
          }}
        >
          {stale ? "Reload mappings" : "Revert"}
        </button>
        <span role="status" className="hint">
          {status || (draft.dirty ? "Unsaved mappings" : "Saved")}
        </span>
      </div>
    </details>
  );
}
