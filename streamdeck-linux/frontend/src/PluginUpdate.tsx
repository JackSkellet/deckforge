import { useState } from "react";
import { pick, request } from "./bridge";
import { getFormDraft, setFormDraft } from "./transient";
import type { Action, RecordValue } from "./model";
export interface PluginUpdateReview {
  ticket: string;
  id: string;
  name: string;
  current_version: string | null;
  new_version: string | null;
  comparison: "upgrade" | "downgrade" | "same" | "unknown";
  removed_actions: string[];
  added_actions: string[];
  was_enabled: boolean;
  kind: "update" | "rollback";
}
export function PluginUpdate({
  plugin,
  actions,
  busy,
  onBusy,
  run,
}: {
  plugin: RecordValue;
  actions: Action[];
  busy: boolean;
  onBusy: (busy: boolean) => void;
  run: (payload: RecordValue) => Promise<boolean>;
}) {
  const key = `plugin-update:${plugin.id}`;
  const [review, setReview] = useState<PluginUpdateReview | null>(
    () => getFormDraft(key) ?? null,
  );
  const [operation, setOperation] = useState<
    "prepare" | "apply" | "cancel" | null
  >(null);
  const [error, setError] = useState("");
  function retain(value: PluginUpdateReview | null) {
    setReview(value);
    setFormDraft(key, value);
  }
  async function prepare(kind: "update" | "rollback") {
    if (busy || operation || review) return;
    const captured = {
      id: plugin.id,
      expected_path: plugin.path,
      expected_version: plugin.version ?? null,
    };
    setOperation("prepare");
    onBusy(true);
    setError("");
    try {
      let value: PluginUpdateReview;
      if (kind === "update") {
        const path = await pick("plugin");
        if (!path) return;
        value = await request("plugin_update_prepare", { ...captured, path });
      } else value = await request("plugin_rollback_prepare", captured);
      if (!value?.ticket || value.id !== captured.id)
        throw Error("Prepared update does not match this plugin.");
      retain(value);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setOperation(null);
      onBusy(false);
    }
  }
  async function complete(cancel: boolean) {
    if (!review || busy || operation) return;
    const ticket = review.ticket;
    setOperation(cancel ? "cancel" : "apply");
    onBusy(true);
    setError("");
    try {
      const success = await run(
        cancel
          ? { command: "plugin_update_cancel", ticket }
          : { command: "plugin_update_apply", ticket, confirm: true },
      );
      if (success) retain(null);
      else
        setError(
          cancel
            ? "The review could not be cancelled. Retry or reopen Plugins."
            : "The update did not complete. Review the reported error. You can retry, or cancel and prepare it again.",
        );
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setOperation(null);
      onBusy(false);
    }
  }
  const locked = busy || !!operation;
  const actionLabel = (uuid: string) => {
    const known = actions.find((action) => action.uuid === uuid);
    return known ? `${known.name} · ${uuid}` : uuid;
  };
  return (
    <section className="plugin-update" aria-label={`Update ${plugin.name}`}>
      {plugin.removal_mode === "archive" ? (
        <div className="row">
          <button
            type="button"
            disabled={locked || !!review}
            onClick={() => void prepare("update")}
          >
            {operation === "prepare" ? "Preparing…" : "Update from package…"}
          </button>
          {plugin.rollback_available === true && (
            <button
              type="button"
              disabled={locked || !!review}
              onClick={() => void prepare("rollback")}
            >
              Restore version
              {plugin.rollback_version ? ` ${plugin.rollback_version}` : ""}…
            </button>
          )}
        </div>
      ) : (
        <p className="hint">
          Update unavailable for this folder. Update the external files and
          restart the plugin, or install a package managed by Deckforge.
        </p>
      )}
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      {review && (
        <div
          className="plugin-update-review"
          role="region"
          aria-label={`Review ${review.name} ${review.kind}`}
        >
          <h4>
            {review.kind === "rollback" ? "Restore" : "Update"} {review.name}
          </h4>
          <p className="version-change">
            {review.current_version || "Unknown version"} →{" "}
            {review.new_version || "Unknown version"}
          </p>
          <p className={review.comparison === "downgrade" ? "warning" : "hint"}>
            {review.comparison === "upgrade"
              ? "A newer version is ready."
              : review.comparison === "downgrade"
                ? "This is an older version."
                : review.comparison === "same"
                  ? "Same version: this replaces the current package contents."
                  : "These version numbers could not be compared."}
          </p>
          <p className="hint">
            Existing assignments and settings are retained. Actions removed by
            this version stay assigned but unavailable.
          </p>
          {!review.was_enabled && (
            <p className="hint">
              This plugin is disabled and will remain disabled.
            </p>
          )}
          {review.removed_actions.length > 0 && (
            <div>
              <strong>Removed actions</strong>
              <ul>
                {review.removed_actions.map((uuid) => (
                  <li key={uuid}>{actionLabel(uuid)}</li>
                ))}
              </ul>
            </div>
          )}
          {review.added_actions.length > 0 && (
            <div>
              <strong>Added actions</strong>
              <ul>
                {review.added_actions.map((uuid) => (
                  <li key={uuid}>{actionLabel(uuid)}</li>
                ))}
              </ul>
            </div>
          )}
          {review.removed_actions.length === 0 &&
            review.added_actions.length === 0 && (
              <p className="hint">
                No action identifiers are added or removed.
              </p>
            )}
          <div className="row">
            <button
              type="button"
              className="primary"
              disabled={locked}
              onClick={() => void complete(false)}
            >
              {operation === "apply"
                ? "Applying…"
                : review.kind === "rollback"
                  ? "Confirm restore"
                  : "Confirm update"}
            </button>
            <button
              type="button"
              disabled={locked}
              onClick={() => void complete(true)}
            >
              {operation === "cancel" ? "Cancelling…" : "Cancel review"}
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
