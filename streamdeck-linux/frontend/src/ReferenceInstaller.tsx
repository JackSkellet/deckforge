import { useEffect, useState } from "react";
import { pick, request } from "./bridge";
import { Field } from "./Fields";
import { getFormDraft, setFormDraft } from "./transient";
import type { RecordValue } from "./model";
interface ReferenceInfo {
  path: string;
  platform: string;
  version: string;
}
const platformName = (value: string) =>
  ["mac", "macos", "darwin"].includes(value)
    ? "macOS"
    : value === "windows"
      ? "Windows"
      : value;
export function ReferenceInstaller({
  configuredPath,
  reference,
  onReferenceChange,
  run,
}: {
  configuredPath?: string | null;
  reference: string;
  onReferenceChange: (path: string) => void;
  run: (payload: RecordValue) => Promise<boolean>;
}) {
  const saved = getFormDraft("reference-installer") as
    ReferenceInfo | null | undefined;
  const [info, setInfo] = useState<ReferenceInfo | null>(() =>
    saved && saved.path === configuredPath ? saved : null,
  );
  const [busy, setBusy] = useState<"import" | "save" | "clear" | "pick" | null>(
    null,
  );
  const [failure, setFailure] = useState("");
  useEffect(() => {
    setFormDraft("reference-installer", info);
  }, [info]);
  useEffect(() => {
    setInfo((current) => (current?.path === configuredPath ? current : null));
  }, [configuredPath]);
  async function importInstaller() {
    if (busy) return;
    const expected = configuredPath ?? null;
    setBusy("import");
    setFailure("");
    try {
      const path = await pick("reference_installer");
      if (!path) return;
      const result = await request<ReferenceInfo>("reference_import", {
        path,
        expected,
      });
      if (!result || typeof result.path !== "string" || !result.path)
        throw new Error("The installer import returned no reference file.");
      onReferenceChange(result.path);
      setInfo(result);
    } catch (error) {
      setFailure(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  }
  async function save(value: string | null) {
    if (busy) return;
    setBusy(value === null ? "clear" : "save");
    setFailure("");
    try {
      if (await run({ command: "reference_app", path: value })) {
        onReferenceChange(value ?? "");
        setInfo(null);
      } else
        setFailure(
          "The reference was not changed. Review the reported error and retry.",
        );
    } catch (error) {
      setFailure(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  }
  const currentPath = info?.path ?? configuredPath;
  return (
    <section
      className="protected-reference"
      aria-label="Protected Elgato packages"
    >
      <h3>Protected Elgato packages</h3>
      <p className="hint">
        Import the supported Stream Deck 7.6.0.23012 macOS .pkg installer for
        local verification of protected plugin packages. Files are extracted
        without running vendor code. No account login is required.
      </p>
      <div className="row">
        <button
          type="button"
          className="primary"
          disabled={!!busy}
          onClick={() => void importInstaller()}
        >
          {busy === "import"
            ? "Importing installer…"
            : "Import Elgato installer"}
        </button>
        <button
          type="button"
          disabled={!!busy || !currentPath}
          onClick={() => void save(null)}
        >
          {busy === "clear" ? "Clearing…" : "Clear reference"}
        </button>
      </div>
      <p className="hint" role="status">
        {info
          ? `Ready · Stream Deck ${info.version || ""} · ${platformName(info.platform || "")}`
          : configuredPath
            ? "Configured · ready for local protected-package verification"
            : "No verification reference configured"}
      </p>
      {currentPath && (
        <p className="hint reference-path" title={currentPath}>
          Reference:{" "}
          {currentPath.split(/[\\/]/).filter(Boolean).slice(-2).join("/")}
        </p>
      )}
      {failure && (
        <p className="error" role="alert">
          {failure}
        </p>
      )}
      <details className="advanced">
        <summary>Advanced: use an extracted executable</summary>
        <fieldset className="reference-controls" disabled={!!busy}>
          <Field label="Original Stream Deck executable">
            <input
              value={reference}
              onChange={(event) => onReferenceChange(event.target.value)}
            />
          </Field>
          <div className="row">
            <button
              type="button"
              onClick={async () => {
                if (busy) return;
                setBusy("pick");
                setFailure("");
                try {
                  const path = await pick("file");
                  if (path) onReferenceChange(path);
                } catch (error) {
                  setFailure(
                    error instanceof Error ? error.message : String(error),
                  );
                } finally {
                  setBusy(null);
                }
              }}
            >
              Choose executable…
            </button>
            <button
              type="button"
              onClick={() => void save(reference.trim() || null)}
            >
              {busy === "save" ? "Saving…" : "Save reference"}
            </button>
          </div>
        </fieldset>
      </details>
    </section>
  );
}
