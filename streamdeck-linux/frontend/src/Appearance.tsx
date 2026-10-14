import { useEffect, useRef, useState, type ReactNode } from "react";
import { request, pick } from "./bridge";
import { IconLibrary } from "./IconLibrary";
import { Check, Field, NumberField, SelectField } from "./Fields";
import {
  appearancePatch,
  hasAppearanceEdits,
  type AppearanceEdit,
  type AppearanceInfo,
  type Draft,
  type Slot,
  type TitleParameters,
} from "./model";
interface Preview extends AppearanceInfo {
  preview: string;
  state: number;
  title: string;
  font_info?: {
    warning?: string;
    fallback?: boolean;
    reason?: string;
    resolved_family?: string;
    resolved_style?: string;
  };
}
const imageUrl = (value: string) =>
  value.startsWith("data:") ? value : `data:image/png;base64,${value}`;
export function AppearanceEditor({
  slot,
  device,
  draft,
  userTitleEnabled = true,
  onEdit,
  onSelectState,
  onRefreshState,
  onError,
  children,
  fallback,
}: {
  slot: Slot;
  device?: string;
  draft: Draft;
  userTitleEnabled?: boolean;
  onEdit: (state: number, patch: AppearanceEdit, base: AppearanceInfo) => void;
  onSelectState: (state: number) => void;
  onRefreshState: () => void;
  onError: (error: unknown) => void;
  children?: ReactNode;
  fallback?: ReactNode;
}) {
  const state = draft.appearanceState ?? slot.state;
  const patch = appearancePatch(draft, state, userTitleEnabled);
  const expected = draft.appearanceExpectedState ?? slot.state;
  const stale = hasAppearanceEdits(draft) && expected !== slot.state;
  const [expanded, setExpanded] = useState(false);
  const [library, setLibrary] = useState<{ choose: (path: string) => void } | null>(null);
  const [fonts, setFonts] = useState<string[]>([]);
  const [fontError, setFontError] = useState("");
  const [result, setResult] = useState<{ key: string; value: Preview } | null>(
    null,
  );
  const [failure, setFailure] = useState<{
    key: string;
    message: string;
  } | null>(null);
  const [bases, setBases] = useState<Record<number, AppearanceInfo>>({});
  const generation = useRef(0);
  const [retry, setRetry] = useState(0);
  const payload = {
    ...(device ? { device } : {}),
    context: slot.context,
    expected_state: expected,
    state,
    ...patch,
  };
  const key = JSON.stringify([
    payload,
    slot.state,
    slot.title,
    slot.appearance,
    retry,
  ]);
  const needsPreview =
    state !== slot.state || Object.keys(patch).length > 0 || expanded;
  useEffect(() => {
    const revision = ++generation.current;
    if (!needsPreview || stale) return;
    const timer = setTimeout(() => {
      request<Preview>("appearance_preview", payload)
        .then((value) => {
          if (
            !value ||
            typeof value.preview !== "string" ||
            value.state !== state ||
            !value.title_parameters
          )
            throw new Error("The native appearance preview is unavailable.");
          if (generation.current !== revision) return;
          setResult({ key, value });
          setFailure(null);
          setBases((previous) => ({
            ...previous,
            [state]: {
              title: value.title,
              title_parameters: value.title_parameters,
              title_custom: value.title_custom,
              image_custom: value.image_custom,
            },
          }));
        })
        .catch((error) => {
          if (generation.current === revision)
            setFailure({
              key,
              message: error instanceof Error ? error.message : String(error),
            });
        });
    }, 180);
    return () => {
      clearTimeout(timer);
      generation.current++;
    };
  }, [key, needsPreview, stale]);
  useEffect(() => {
    if (!expanded) return;
    let live = true;
    request<unknown>("fonts")
      .then((value) => {
        if (!Array.isArray(value) || !value.every((v) => typeof v === "string"))
          throw new Error("The native font list is unavailable.");
        if (live) {
          setFonts(value);
          setFontError("");
        }
      })
      .catch((error) => {
        if (live)
          setFontError(error instanceof Error ? error.message : String(error));
      });
    return () => {
      live = false;
    };
  }, [expanded]);
  const preview = result?.key === key ? result.value : null;
  const error = failure?.key === key ? failure.message : "";
  const saved =
    state === slot.state
      ? {
          title_parameters: slot.appearance?.title_parameters ?? {},
          title_custom: slot.appearance?.title_custom ?? false,
          image_custom: slot.appearance?.image_custom ?? false,
          title: slot.title,
        }
      : undefined;
  const base =
    preview ?? draft.appearanceBases?.[state] ?? saved ?? bases[state];
  const params: TitleParameters = {
    ...base?.title_parameters,
    ...patch.title_parameters,
  };
  const title = Object.hasOwn(patch, "title")
    ? (patch.title ?? "")
    : (preview?.title ??
      (state === slot.state && !patch.reset_title
        ? slot.title
        : (base?.title ?? "")));
  const previewImage =
    preview?.preview ?? (state === slot.state ? slot.preview : undefined);
  const update = (changes: AppearanceEdit) => {
    if (base) onEdit(state, changes, base);
  };
  const style = (changes: TitleParameters) =>
    update({ title_parameters: changes });
  const loading = needsPreview && !stale && !preview && !error;
  const families: [string, string][] = [
    ["", "Default"],
    ...fonts
      .filter((font) => font !== "")
      .map((font) => [font, font] as [string, string]),
  ];
  const colour = params.titleColor ?? "#ffffff";
  return (
    <>
      {library && <IconLibrary onChoose={library.choose} onClose={() => setLibrary(null)} />}
      <div className="appearance">
        <div className="appearance-preview">
          <div className="selected-art" aria-busy={loading}>
            {previewImage ? (
              <img
                alt={
                  preview && Object.keys(patch).length
                    ? "Draft appearance preview"
                    : "Selected control preview"
                }
                src={imageUrl(previewImage)}
              />
            ) : (
              fallback
            )}
          </div>
          {loading && <small>Previewing…</small>}
          {preview && Object.keys(patch).length > 0 && (
            <small>Draft preview</small>
          )}
        </div>
        <div className="appearance-fields">
          <Field
            label="Title"
            hint={userTitleEnabled ? undefined : "Updated live by this action."}
          >
            <textarea
              aria-label="Title"
              className="appearance-title"
              rows={Math.min(3, Math.max(1, title.split(/\r\n|\r|\n/).length))}
              disabled={!userTitleEnabled || !base}
              value={title}
              onChange={(event) => update({ title: event.target.value })}
            />
          </Field>
          {children}
        </div>
      </div>
      {stale && (
        <div className="warning appearance-warning">
          <p>
            The action changed to state {slot.state + 1}. Your appearance drafts
            are kept for their selected states.
          </p>
          <button type="button" onClick={onRefreshState}>
            Refresh state and preview
          </button>
        </div>
      )}
      {error && (
        <div className="appearance-error">
          <p role="status">Preview: {error}</p>
          <button type="button" onClick={() => setRetry((value) => value + 1)}>
            Retry preview
          </button>
        </div>
      )}
      <details
        className="appearance-options"
        onToggle={(event) => setExpanded(event.currentTarget.open)}
      >
        <summary>
          Appearance
          {Object.keys(draft.appearanceEdits ?? {}).length > 0
            ? " · edited"
            : ""}
        </summary>
        {Math.max(1, slot.state_count ?? 1) > 1 && (
          <SelectField
            label="Appearance state"
            value={state}
            options={Array.from(
              { length: slot.state_count ?? 1 },
              (_, index) => [
                index,
                `${slot.state_names?.[index] || `State ${index + 1}`}${index === slot.state ? " · live" : ""}`,
              ],
            )}
            onChange={(value) => onSelectState(Number(value))}
          />
        )}
        <p className="hint">
          Editing state {state + 1}. Previewing another state does not change
          the device.
        </p>
        <fieldset
          className="appearance-style"
          disabled={!base || (!slot.appearance && !preview && !bases[state])}
        >
          <div className="appearance-icon-actions">
            <button type="button" onClick={() => {
              const original = base, targetState = state, edit = onEdit;
              if (original) setLibrary({ choose: (path) => edit(targetState, { image_path: path }, original) });
            }}>Icon library…</button>
            <button
              type="button"
              onClick={() => {
                const targetState = state;
                const original = base;
                if (!original) return;
                pick("image")
                  .then((path) => {
                    if (path)
                      onEdit(targetState, { image_path: path }, original);
                  })
                  .catch(onError);
              }}
            >
              Choose icon…
            </button>
            <button
              type="button"
              disabled={
                !base?.image_custom && !Object.hasOwn(patch, "image_path")
              }
              onClick={() => update({ reset_image: true })}
            >
              Reset icon
            </button>
            <button
              type="button"
              disabled={
                !userTitleEnabled ||
                (!base?.title_custom && !Object.hasOwn(patch, "title"))
              }
              onClick={() => update({ reset_title: true })}
            >
              Reset title
            </button>
          </div>
          {Object.hasOwn(patch, "image_path") && (
            <p className="appearance-path">Icon draft: {patch.image_path}</p>
          )}
          {patch.reset_image && (
            <p className="hint">The action's icon will be restored on Save.</p>
          )}
          {patch.reset_title && (
            <p className="hint">The action's title will resume on Save.</p>
          )}
          <div className="appearance-grid">
            <SelectField
              label="Font family"
              value={params.fontFamily ?? ""}
              options={families}
              onChange={(fontFamily) => style({ fontFamily })}
            />
            <NumberField
              key={`font-size:${state}`}
              editorKey={`${slot.context}/appearance/${state}`}
              label="Font size"
              value={params.fontSize ?? 12}
              min={1}
              max={96}
              step="any"
              onChange={(fontSize) => style({ fontSize })}
            />
            <SelectField
              label="Font style"
              value={params.fontStyle ?? ""}
              options={[
                ["", "Default"],
                ["Regular", "Regular"],
                ["Bold", "Bold"],
                ["Italic", "Italic"],
                ["Bold Italic", "Bold Italic"],
              ]}
              onChange={(fontStyle) =>
                style({ fontStyle: fontStyle as TitleParameters["fontStyle"] })
              }
            />
            <SelectField
              label="Title alignment"
              value={params.titleAlignment ?? "middle"}
              options={[
                ["top", "Top"],
                ["middle", "Middle"],
                ["bottom", "Bottom"],
              ]}
              onChange={(titleAlignment) =>
                style({
                  titleAlignment:
                    titleAlignment as TitleParameters["titleAlignment"],
                })
              }
            />
          </div>
          {fontError && (
            <p className="hint">{fontError} Saved font identities are kept.</p>
          )}
          {(preview?.font_info?.warning ||
            (preview?.font_info?.fallback && preview.font_info.reason)) && (
            <p className="hint">
              {preview.font_info.warning || preview.font_info.reason}
            </p>
          )}
          <div className="appearance-toggles">
            <Check
              label="Show title"
              checked={params.showTitle ?? true}
              onChange={(showTitle) => style({ showTitle })}
            />
            <Check
              label="Underline"
              checked={params.fontUnderline ?? false}
              onChange={(fontUnderline) => style({ fontUnderline })}
            />
            <Field label="Title colour">
              <span className="colour-control">
                <input
                  aria-label="Choose title colour"
                  type="color"
                  value={
                    /^#?[0-9a-f]{6}$/i.test(colour)
                      ? `#${colour.replace(/^#/, "")}`
                      : "#ffffff"
                  }
                  onChange={(event) =>
                    style({ titleColor: event.target.value })
                  }
                />
                <input
                  aria-label="Title colour value"
                  pattern="#?[0-9a-fA-F]{6}"
                  required
                  value={colour}
                  onChange={(event) =>
                    style({ titleColor: event.target.value })
                  }
                />
              </span>
            </Field>
          </div>
        </fieldset>
      </details>
    </>
  );
}
