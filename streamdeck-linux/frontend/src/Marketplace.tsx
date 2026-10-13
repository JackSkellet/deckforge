import { useEffect, useState } from "react";
import { request } from "./bridge";
import type { MarketplaceState } from "./model";

export function Marketplace({ state }: { state: MarketplaceState }) {
  const [error, setError] = useState("");
  const [sending, setSending] = useState(false);
  async function act(action: string) {
    setSending(true);
    setError("");
    try {
      await request("marketplace", { action });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  }
  useEffect(() => {
    if (
      state.authenticated === null &&
      !state.busy &&
      !state.review &&
      !state.pending
    )
      void act("check");
  }, []);
  const busy = state.busy || sending;
  const review = state.review;
  return (
    <section className="preference-section" aria-label="Elgato Marketplace">
      <h3>Elgato Marketplace</h3>
      <p>
        {state.authenticated === true
          ? `Signed in · ${state.account || "Elgato account"}`
          : state.authenticated === false
            ? "Not signed in"
            : "Session not checked yet"}
      </p>
      <p className="hint">
        Sign in on Elgato's page. Deckforge remembers this session for browser
        installs and checks it before downloading. If it expires, sign in again.
      </p>
      <div className="row">
        <button
          className="primary"
          disabled={busy || !!review}
          onClick={() => void act("login")}
        >
          {state.authenticated
            ? "Open Marketplace sign-in"
            : "Sign in to Marketplace"}
        </button>
        <button disabled={busy || !!review} onClick={() => void act("check")}>
          Check session
        </button>
        <button disabled={busy || !!review} onClick={() => void act("logout")}>
          Sign out on this device
        </button>
      </div>
      {state.message && <p role="status">{state.message}</p>}
      {(error || state.error) && (
        <p className="error" role="alert">
          {error || state.error}
        </p>
      )}
      {state.pending && !review && !busy && (
        <button onClick={() => void act("retry")}>Retry plugin install</button>
      )}
      {review && (
        <section aria-label={`Review ${review.name} update`}>
          <h4>Update {review.name}</h4>
          <p>
            {review.current_version || "Unknown version"} →{" "}
            {review.new_version || "Unknown version"}
          </p>
          {review.comparison === "downgrade" && (
            <p className="warning">
              This replaces the installed plugin with an older version.
            </p>
          )}
          {review.comparison === "same" && (
            <p className="warning">
              This replaces the installed package with the same version.
            </p>
          )}
          <p className="hint">
            Assignments and settings are retained. The previous version is
            available in Plugins after updating.
          </p>
          {!review.was_enabled && (
            <p className="hint">This plugin will remain disabled.</p>
          )}
          {review.removed_actions.length > 0 && (
            <>
              <p>These actions will become unavailable:</p>
              <ul>
                {review.removed_actions.map((id) => (
                  <li key={id}>{id}</li>
                ))}
              </ul>
            </>
          )}
          <div className="row">
            <button
              className="primary"
              disabled={busy}
              onClick={() => void act("apply")}
            >
              Install update
            </button>
            <button disabled={busy} onClick={() => void act("cancel")}>
              Cancel update
            </button>
          </div>
        </section>
      )}
    </section>
  );
}
