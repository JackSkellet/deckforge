import { useEffect, useState } from "react";
import { Field } from "./Fields";
import { getFormDraft, setFormDraft } from "./transient";
import type { PluginLanguageState, RecordValue } from "./model";
const formKey = "plugin-language";
interface Draft {
  value: string;
  baseline: string;
  dirty: boolean;
}
export function PluginLanguage({
  language,
  run,
  error = "",
}: {
  language: PluginLanguageState;
  run: (payload: RecordValue) => Promise<boolean>;
  error?: string;
}) {
  const fresh = (): Draft => ({
    value: language.preference,
    baseline: language.preference,
    dirty: false,
  });
  const [draft, setDraft] = useState<Draft>(
    () => getFormDraft(formKey) ?? fresh(),
  );
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const stale = draft.dirty && draft.baseline !== language.preference;
  useEffect(() => {
    setFormDraft(formKey, draft);
  }, [draft]);
  useEffect(() => {
    setDraft((current) =>
      current.dirty && current.value !== language.preference
        ? current
        : fresh(),
    );
  }, [language.preference]);
  const label = (code: string) =>
    language.options.find((option) => option.value === code)?.label ?? code;
  async function save() {
    if (busy || stale || !draft.dirty) return;
    const captured = draft;
    setBusy(true);
    setStatus("");
    try {
      if (
        await run({
          command: "plugin_language",
          value: captured.value,
          baseline: captured.baseline,
        })
      ) {
        setDraft((current) => ({
          ...current,
          baseline: captured.value,
          dirty: current.value !== captured.value,
        }));
        setStatus("Plugin language preference saved");
      } else setStatus("Language preference was not saved. Reload or retry.");
    } catch {
      setStatus("Language preference was not saved. Reload or retry.");
    } finally {
      setBusy(false);
    }
  }
  return (
    <section
      className="plugin-language"
      aria-label="Plugin language preference"
    >
      <Field label="Plugin language">
        <select
          aria-label="Plugin language"
          disabled={busy}
          value={draft.value}
          onChange={(event) => {
            const value = event.target.value;
            setDraft((current) => ({
              ...current,
              value,
              dirty: value !== current.baseline,
            }));
            setStatus("");
          }}
        >
          {!language.options.some((option) => option.value === draft.value) && (
            <option value={draft.value}>{draft.value || "System"}</option>
          )}
          {language.options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </Field>
      <p className="hint">
        Affects plugin and action labels and property inspectors. Deckforge
        menus remain English.
      </p>
      <p className="hint">
        Changes take effect after you quit and reopen Deckforge. Restarting an
        individual plugin is not enough.
      </p>
      <p className="hint">
        Running plugins: {label(language.active)}. Saved selection resolves to:{" "}
        {label(language.resolved)}.
      </p>
      {language.restart_required && (
        <p className="warning" role="status">
          Restart Deckforge to apply the saved plugin language.
        </p>
      )}
      {stale && (
        <p className="warning" role="status">
          The saved language changed while you were editing. Your draft is kept.
          Reload the language preference before saving.
        </p>
      )}
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      <div className="row">
        <button
          type="button"
          className="primary"
          disabled={!draft.dirty || stale || busy}
          onClick={() => void save()}
        >
          {busy ? "Saving…" : "Save plugin language"}
        </button>
        <button
          type="button"
          disabled={!draft.dirty || busy}
          onClick={() => {
            setDraft(fresh());
            setStatus("");
          }}
        >
          {stale ? "Reload language" : "Revert language"}
        </button>
      </div>
      <p className="hint" role="status">
        {status || (draft.dirty ? "Unsaved language preference" : "Saved")}
      </p>
    </section>
  );
}
