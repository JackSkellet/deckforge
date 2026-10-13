import { useEffect, useState } from "react";
import { request } from "./bridge";
import type { Run } from "./Panels";
interface Bundle {
  id: string;
  name: string;
  version: string;
  path: string;
}
export function BundledPlugins({
  installed,
  run,
}: {
  installed: string[];
  run: Run;
}) {
  const [bundles, setBundles] = useState<Bundle[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let live = true;
    void request<Bundle[]>("bundled_plugins")
      .then((value) => {
        if (live) setBundles(Array.isArray(value) ? value : []);
      })
      .catch((error) => {
        if (live) setError(String(error));
      });
    return () => {
      live = false;
    };
  }, []);
  if (!bundles.length && !error) return null;
  return (
    <section aria-label="Included with Deckforge">
      <h3>Included with Deckforge</h3>
      <p className="hint">
        Ready to install. Connect your own account in the plugin’s settings.
      </p>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {bundles.map((bundle) => {
        const exists = installed.includes(bundle.id);
        return (
          <article className="plugin-row" key={bundle.id}>
            <div className="plugin-heading">
              <div>
                <strong>{bundle.name}</strong>
                <p className="hint">v{bundle.version} · Native Linux plugin</p>
              </div>
              <button
                type="button"
                disabled={exists || busy !== null}
                onClick={async () => {
                  setBusy(bundle.id);
                  setError("");
                  try {
                    await run({ command: "install_plugin", path: bundle.path });
                  } catch (error) {
                    setError(String(error));
                  } finally {
                    setBusy(null);
                  }
                }}
              >
                {exists
                  ? "Already installed"
                  : busy === bundle.id
                    ? "Installing…"
                    : `Install ${bundle.name}`}
              </button>
            </div>
          </article>
        );
      })}
    </section>
  );
}
