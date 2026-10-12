import { useCallback, useEffect, useRef, useState } from "react";
import { command, pick, request } from "./bridge";
import {
  allowed,
  filteredActions,
  reconcileDraft,
  editDraft,
  finishSave,
  saveRequests,
  isContainer,
  STACK,
  type Action,
  type Controller,
  type Draft,
  type RecordValue,
  type Slot,
  type Snapshot,
} from "./model";
import { Field, Check, SettingsFields } from "./Fields";
import { Sequence } from "./Sequence";
import { Modal, Plugins, Preferences } from "./Panels";
import {
  exportTextDrafts,
  importTextDrafts,
  hasTextDrafts,
  exportFormDrafts,
  importFormDrafts,
} from "./transient";
import "./style.css";
const iconCache = new Map<string, string | null>();
function ActionIcon({ action }: { action: Action | undefined }) {
  const key = `${action?.uuid}:${action?.icon_generation ?? 0}`;
  const [src, setSrc] = useState<string | null>(iconCache.get(key) ?? null);
  useEffect(() => {
    let live = true;
    setSrc(iconCache.get(key) ?? null);
    if (action && !iconCache.has(key))
      request<string | null>("action_icon", {
        uuid: action.uuid,
        generation: action.icon_generation,
      })
        .then((value) => {
          iconCache.set(key, value);
          if (live) setSrc(value);
        })
        .catch(() => {});
    return () => {
      live = false;
    };
  }, [key]);
  return src ? (
    <img className="action-icon" src={src} alt="" />
  ) : (
    <span className="action-icon fallback" aria-hidden="true">
      {action?.name.slice(0, 1) ?? "+"}
    </span>
  );
}
export default function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [selected, setSelected] = useState<{
    controller: Controller;
    index: number;
  }>({ controller: "Keypad", index: 0 });
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<Controller | "All">("All");
  const [panel, setPanel] = useState<"plugins" | "preferences" | "logs" | null>(
    null,
  );
  const [libraryEdit, setLibraryEdit] = useState<{
    kind: "profile" | "page";
    id?: string;
    name: string;
    operation: "create" | "rename" | "delete" | "duplicate";
  } | null>(null);
  const [saving, setSaving] = useState(false);
  const form = useRef<HTMLFormElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const [booted, setBooted] = useState(false);
  const [textRevision, setTextRevision] = useState(0);
  const onError = useCallback((e: unknown) => {
    setError(e instanceof Error ? e.message : String(e));
  }, []);
  const accept = useCallback((next: Snapshot) => {
    setSnapshot(next);
    setDrafts((prev) => {
      let changed = false;
      const result = { ...prev };
      for (const slot of next.slots) {
        const draft = reconcileDraft(prev[slot.context], slot);
        if (draft !== prev[slot.context]) {
          changed = true;
          result[slot.context] = draft;
        }
      }
      return changed ? result : prev;
    });
  }, []);
  useEffect(() => {
    let live = true;
    let latest: Snapshot | null = null;
    let ready = false;
    const stop = window.deckforge?.onSnapshot((s) => {
      latest = s;
      if (live && ready) accept(s);
    });
    Promise.all([
      request<Snapshot>("snapshot"),
      request<RecordValue | null>("ui_state_get"),
    ])
      .then(([s, state]) => {
        if (!live) return;
        if (state) {
          if (state.drafts) setDrafts(state.drafts);
          if (state.selected) setSelected(state.selected);
          if (typeof state.search === "string") setSearch(state.search);
          if (["All", "Keypad", "Encoder"].includes(state.filter))
            setFilter(state.filter);
          importTextDrafts(state.textDrafts);
          importFormDrafts(state.formDrafts);
          if (["plugins", "preferences", "logs"].includes(state.panel))
            setPanel(state.panel);
          if (state.libraryEdit) setLibraryEdit(state.libraryEdit);
        }
        ready = true;
        accept(latest ?? s);
        setBooted(true);
        requestAnimationFrame(() =>
          window.dispatchEvent(new CustomEvent("deckforge-ready")),
        );
      })
      .catch(onError);
    return () => {
      live = false;
      stop?.();
    };
  }, [accept, onError]);
  useEffect(() => {
    const changed = () => setTextRevision((v) => v + 1);
    window.addEventListener("deckforge-draft-change", changed);
    return () => window.removeEventListener("deckforge-draft-change", changed);
  }, []);
  const uiState = {
    drafts,
    selected,
    search,
    filter,
    textDrafts: exportTextDrafts(),
    formDrafts: exportFormDrafts(),
    panel,
    libraryEdit,
  };
  useEffect(() => {
    if (booted) void request("ui_state_set", { state: uiState }).catch(onError);
  }, [
    drafts,
    selected,
    search,
    filter,
    booted,
    textRevision,
    panel,
    libraryEdit,
    onError,
  ]);
  async function hide() {
    try {
      await request("ui_state_set", { state: uiState });
      await request("window", { action: "hide" });
    } catch (e) {
      onError(e);
    }
  }
  const run = useCallback(
    async (payload: RecordValue) => {
      try {
        setError("");
        await command(payload);
        accept(await request<Snapshot>("snapshot"));
        return true;
      } catch (e) {
        onError(e);
        return false;
      }
    },
    [accept, onError],
  );
  const slot = snapshot?.slots.find(
    (s) => s.controller === selected.controller && s.index === selected.index,
  );
  const draft = slot
    ? (drafts[slot.context] ?? reconcileDraft(undefined, slot))
    : undefined;
  const action = snapshot?.actions.find((a) => a.uuid === slot?.uuid);
  const profile = snapshot?.library.profiles.find(
    (p) => p.id === snapshot.library.active_profile,
  );
  const page = profile?.pages.find(
    (p) => p.id === snapshot?.library.active_page,
  );
  const readonly = profile?.readonly ?? false;
  function change(patch: Partial<Draft>) {
    if (!slot) return;
    setDrafts((prev) => ({
      ...prev,
      [slot.context]: editDraft(
        prev[slot.context] ?? reconcileDraft(undefined, slot),
        patch,
      ),
    }));
  }
  async function save() {
    if (!slot || !draft || !draft.dirty || readonly || saving || !form.current?.reportValidity()) return;
    if (hasTextDrafts(slot.context)) {
      onError(
        new Error(
          "Apply or discard the pending advanced settings text before saving.",
        ),
      );
      return;
    }
    const target = slot;
    const savingDraft = draft;
    setSaving(true);
    try {
      for (const payload of saveRequests(target, savingDraft)) {
        if (!(await run(payload))) return;
      }
      setDrafts((prev) => ({
        ...prev,
        [target.context]: finishSave(prev[target.context], savingDraft),
      }));
      setNotice("Action saved");
    } finally {
      setSaving(false);
    }
  }
  useEffect(() => {
    function key(event: KeyboardEvent) {
      if (document.querySelector("dialog[open]")) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        searchRef.current?.focus();
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void save();
      }
    }
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [slot, draft, saving]);
  async function assign(uuid: string, target = selected) {
    const metadata = snapshot?.actions.find((a) => a.uuid === uuid);
    if (!metadata || !allowed(metadata, target.controller)) {
      onError(new Error("This action does not support that control."));
      return;
    }
    if (
      await run({
        command: "assign",
        controller: target.controller,
        index: target.index,
        uuid,
      })
    ) {
      setSelected(target);
      setNotice(`${metadata.name} assigned`);
    }
  }
  async function testPress() {
    if (!slot) return;
    const prefix = slot.controller === "Encoder" ? "dial" : "key";
    if (
      await run({ command: "input", event: `${prefix}Down`, index: slot.index })
    )
      await run({ command: "input", event: `${prefix}Up`, index: slot.index });
  }
  async function chooseImport() {
    try {
      const path = await pick("profile");
      if (path && (await run({ command: "import_profile", path })))
        setNotice("Profile imported");
    } catch (e) {
      onError(e);
    }
  }
  function drop(
    event: React.DragEvent,
    target: { controller: Controller; index: number },
  ) {
    event.preventDefault();
    const uuid = event.dataTransfer.getData("application/x-deckforge-action");
    if (uuid) void assign(uuid, target);
  }
  if (!snapshot)
    return (
      <main className="loading">
        <div className="brand-mark">D</div>
        <h1>Deckforge</h1>
        <p role={error ? "alert" : "status"}>
          {error || "Connecting to your device and plugins…"}
        </p>
        {error && (
          <button
            onClick={() =>
              request<Snapshot>("snapshot").then(accept).catch(onError)
            }
          >
            Retry connection
          </button>
        )}
      </main>
    );
  const visible = filteredActions(snapshot.actions, search, filter);
  const categories = [...new Set(visible.map((a) => a.category))];
  const slotButton = (controller: Controller, index: number) => {
    const target = { controller, index };
    const item = snapshot.slots.find(
      (s) => s.controller === controller && s.index === index,
    );
    const active =
      selected.controller === controller && selected.index === index;
    return (
      <button
        key={index}
        type="button"
        className={
          (controller === "Keypad" ? "deck-key" : "deck-dial") +
          (active ? " selected" : "") +
          (!item ? " unassigned" : "")
        }
        aria-label={`${controller === "Keypad" ? "Key" : "Dial"} ${index + 1}: ${item?.name ?? "Unassigned"}`}
        aria-pressed={active}
        onClick={() => setSelected(target)}
        onDragOver={(e) => {
          if (e.dataTransfer.types.includes("application/x-deckforge-action"))
            e.preventDefault();
        }}
        onDrop={(e) => drop(e, target)}
      >
        {item?.preview ? (
          <img
            src={
              item.preview.startsWith("data:")
                ? item.preview
                : `data:image/png;base64,${item.preview}`
            }
            alt=""
          />
        ) : (
          <span className="empty-control">+</span>
        )}
        {item?.pinned && (
          <span className="pin" title="Pinned">
            ◆
          </span>
        )}
        <span className="sr-only">{item?.title}</span>
        {controller === "Encoder" && (
          <span className="dial-knob" aria-hidden="true">
            <i />
          </span>
        )}
      </button>
    );
  };
  return (
    <div className="application">
      <header className="app-header">
        <div className="brand">
          <span className="brand-mark">D</span>
          <strong>Deckforge</strong>
          <span className="platform-label">LINUX</span>
        </div>
        <div className="device-status" title={snapshot.connection_error || snapshot.device}>
          <span className={"dot " + (snapshot.connected ? "online" : "")} />
          <span>
            {snapshot.simulate
              ? "Simulated Stream Deck +"
              : snapshot.connected
                ? "Stream Deck + connected"
                : "Waiting for Stream Deck +"}
          </span>
        </div>
        <div className="header-tools">
          <button onClick={() => setPanel("plugins")}>
            Plugins <span className="count">{snapshot.plugins.length}</span>
          </button>
          <button onClick={() => setPanel("preferences")}>Preferences</button>
          <button
            title="Keep running in background"
            aria-label="Hide Deckforge"
            onClick={hide}
          >
            −
          </button>
        </div>
      </header>
      {error && (
        <div className="banner error" role="alert">
          <span>{error}</span>
          <button aria-label="Dismiss error" onClick={() => setError("")}>
            ×
          </button>
        </div>
      )}
      <div className="workspace">
        <aside className="profile-sidebar">
          <div className="section-heading">
            <h2>Profiles</h2>
            <button
              aria-label="New profile"
              title="New profile"
              onClick={() =>
                setLibraryEdit({
                  kind: "profile",
                  name: "New profile",
                  operation: "create",
                })
              }
            >
              +
            </button>
          </div>
          <nav aria-label="Profiles">
            {snapshot.library.profiles.map((p) => (
              <button
                key={p.id}
                className={
                  "profile-item " + (p.id === profile?.id ? "active" : "")
                }
                aria-current={p.id === profile?.id ? "page" : undefined}
                onClick={() => run({ command: "profile_select", id: p.id })}
              >
                <span className="profile-symbol" aria-hidden="true">
                  ▦
                </span>
                <span>
                  {p.name}
                  {p.readonly && <small>Plugin managed</small>}
                </span>
              </button>
            ))}
          </nav>
          <div className="sidebar-bottom">
            <button onClick={chooseImport}>↓ Import profile</button>
            <button
              disabled={!profile}
              onClick={async () => {
                try {
                  const path = await pick("export_profile");
                  if (path && (await run({ command: "export_profile", path })))
                    setNotice("Profile exported");
                } catch (e) {
                  onError(e);
                }
              }}
            >
              ↑ Export profile
            </button>
            <details>
              <summary>Profile options</summary>
              <div className="menu-stack">
                {(["rename", "duplicate", "delete"] as const).map(
                  (operation) => (
                    <button
                      key={operation}
                      disabled={
                        !profile || (readonly && operation !== "duplicate")
                      }
                      onClick={() =>
                        setLibraryEdit({
                          kind: "profile",
                          id: profile!.id,
                          name:
                            profile!.name +
                            (operation === "duplicate" ? " copy" : ""),
                          operation,
                        })
                      }
                    >
                      {operation[0].toUpperCase() + operation.slice(1)} profile
                    </button>
                  ),
                )}
              </div>
            </details>
          </div>
        </aside>
        <main className="editor">
          <div className="editor-heading">
            <div>
              <p className="eyebrow">YOUR WORKSPACE</p>
              <h1>{profile?.name ?? "No profile"}</h1>
            </div>
            <span className="muted">
              {snapshot.slots.length} assigned controls
            </span>
          </div>
          <div className="page-toolbar">
            <nav className="page-tabs" aria-label="Pages">
              {profile?.pages
                .filter((p) => !p.folder)
                .map((p, i) => (
                  <button
                    key={p.id}
                    className={
                      p.id === snapshot.library.active_page ? "active" : ""
                    }
                    aria-current={
                      p.id === snapshot.library.active_page ? "page" : undefined
                    }
                    onClick={() => run({ command: "page_select", id: p.id })}
                  >
                    <span>{i + 1}</span>
                    {p.name}
                  </button>
                ))}
              <button
                title="New page"
                aria-label="New page"
                disabled={readonly}
                onClick={() =>
                  setLibraryEdit({
                    kind: "page",
                    name: "New page",
                    operation: "create",
                  })
                }
              >
                +
              </button>
            </nav>
            <details className="page-menu">
              <summary aria-label="Page options">•••</summary>
              <div className="floating-menu">
                <button
                  onClick={() =>
                    run({ command: "page_select", id: profile?.pin_page })
                  }
                >
                  Pinned actions
                </button>
                {(["rename", "duplicate", "delete"] as const).map(
                  (operation) => (
                    <button
                      key={operation}
                      disabled={!page || readonly}
                      onClick={() =>
                        setLibraryEdit({
                          kind: "page",
                          id: page!.id,
                          name:
                            page!.name +
                            (operation === "duplicate" ? " copy" : ""),
                          operation,
                        })
                      }
                    >
                      {operation[0].toUpperCase() + operation.slice(1)} page
                    </button>
                  ),
                )}
                <button
                  disabled={!page || readonly}
                  onClick={() =>
                    run({ command: "page_move", id: page?.id, direction: -1 })
                  }
                >
                  Move page left
                </button>
                <button
                  disabled={!page || readonly}
                  onClick={() =>
                    run({ command: "page_move", id: page?.id, direction: 1 })
                  }
                >
                  Move page right
                </button>
              </div>
            </details>
          </div>
          <div className="device-stage">
            {snapshot.library.folder_depth > 0 && (
              <button
                className="folder-back"
                onClick={() => run({ command: "folder_back" })}
              >
                ← Back from {page?.name ?? "folder"}
              </button>
            )}
            {snapshot.library.active_page === profile?.pin_page && (
              <div className="stage-label">
                Pinned actions · appear on every page
              </div>
            )}
            <div className="deck">
              <div className="key-grid">
                {Array.from({ length: 8 }, (_, i) => slotButton("Keypad", i))}
              </div>
              <div className="dial-grid">
                {Array.from({ length: 4 }, (_, i) => slotButton("Encoder", i))}
              </div>
              <span className="deck-wordmark">STREAM DECK +</span>
            </div>
            <p className="stage-hint">
              Select a control to edit · Drag an action onto a key or dial
            </p>
            {snapshot.screensaver?.active && (
              <div className="row">
                <span>Device sleeping</span>
                <button onClick={() => run({ command: "wake_device" })}>
                  Wake device
                </button>
              </div>
            )}
          </div>
          <section className="inspector" aria-label="Selected control">
            <header className="inspector-heading">
              <div>
                <span className="eyebrow">
                  {selected.controller === "Keypad" ? "KEY" : "DIAL"}{" "}
                  {selected.index + 1}
                </span>
                <h2>{slot?.name ?? "Choose an action"}</h2>
              </div>
              {slot && (
                <div className="row">
                  <button onClick={testPress}>
                    Test {selected.controller === "Keypad" ? "press" : "push"}
                  </button>
                  {slot.has_inspector && (
                    <button
                      className="primary"
                      onClick={() =>
                        run({
                          command: "open_inspector",
                          context: slot.context,
                        })
                      }
                    >
                      Plugin settings ↗
                    </button>
                  )}
                </div>
              )}
            </header>
            {!slot ? (
              <div className="empty-control-editor">
                <p>
                  Drag an action here, or select one from the library and choose
                  Assign.
                </p>
                {selected.controller === "Keypad" && (
                  <button
                    disabled={readonly}
                    onClick={() =>
                      run({
                        command: "folder_create",
                        index: selected.index,
                        name: "New folder",
                      })
                    }
                  >
                    Create folder
                  </button>
                )}
              </div>
            ) : (
              draft && (
                <form
                  ref={form}
                  onSubmit={(e) => {
                    e.preventDefault();
                    void save();
                  }}
                >
                  <fieldset
                    disabled={readonly || saving}
                    className="inspector-fields"
                  >
                    <div className="appearance">
                      <div className="selected-art">
                        {slot.preview ? (
                          <img
                            alt="Selected control preview"
                            src={
                              slot.preview.startsWith("data:")
                                ? slot.preview
                                : `data:image/png;base64,${slot.preview}`
                            }
                          />
                        ) : (
                          <ActionIcon action={action} />
                        )}
                      </div>
                      <div className="appearance-fields">
                        <Field label="Title">
                          <input
                            value={draft.title}
                            onChange={(e) => change({ title: e.target.value })}
                          />
                        </Field>
                        <Check
                          label="Pin on every page"
                          checked={slot.pinned}
                          onChange={(pinned) =>
                            run({
                              command: "slot_pin",
                              context: slot.context,
                              pinned,
                            })
                          }
                        />
                      </div>
                    </div>
                    {slot.available === false && (
                      <p className="warning">
                        This action's plugin is unavailable. Its settings are
                        preserved; install or restart the plugin to use it.
                      </p>
                    )}
                    {draft.dirty &&
                      draft.source !==
                        JSON.stringify([
                          slot.settings,
                          slot.title,
                          slot.raw,
                        ]) && (
                        <p className="warning">
                          The plugin changed this action while you were editing.
                          Your draft is kept; saving applies your changes.
                        </p>
                      )}
                    {isContainer(slot.uuid) ? (
                      <Sequence
                        scope={slot.context}
                        runtime={slot}
                        run={run}
                        key={slot.context}
                        raw={draft.raw}
                        uuid={slot.uuid}
                        onChange={(raw) => change({ raw })}
                        actions={snapshot.actions}
                        outputs={snapshot.audio_outputs}
                        onError={onError}
                      />
                    ) : (
                      <SettingsFields
                        editorKey={slot.context}
                        key={slot.context}
                        uuid={slot.uuid}
                        controller={slot.controller}
                        settings={draft.settings}
                        onChange={(settings) => change({ settings })}
                        outputs={snapshot.audio_outputs}
                        onError={onError}
                      />
                    )}
                  </fieldset>
                  <footer className="inspector-footer">
                    <div className="row">
                      <button
                        type="submit"
                        className="primary"
                        disabled={
                          !draft.dirty ||
                          readonly ||
                          saving ||
                          hasTextDrafts(slot.context)
                        }
                      >
                        {saving ? "Saving…" : "Save changes"}
                      </button>
                      <button
                        type="button"
                        disabled={!draft.dirty || saving}
                        onClick={() =>
                          setDrafts((prev) => ({
                            ...prev,
                            [slot.context]: reconcileDraft(undefined, slot),
                          }))
                        }
                      >
                        Revert
                      </button>
                      <span className="hint">
                        {hasTextDrafts(slot.context)
                          ? "Unapplied text in editor"
                          : draft.dirty
                            ? "Unsaved draft"
                            : "Saved"}
                      </span>
                    </div>
                    <button
                      type="button"
                      className="danger quiet"
                      disabled={readonly || saving}
                      onClick={() =>
                        run({ command: "remove", context: slot.context })
                      }
                    >
                      Remove action
                    </button>
                  </footer>
                </form>
              )
            )}
            {slot?.controller === "Encoder" && (
              <div className="test-controls">
                <span>Test dial</span>
                {[-1, 1].map((ticks) => (
                  <button
                    key={ticks}
                    onClick={() =>
                      run({
                        command: "input",
                        event: "dialRotate",
                        index: slot.index,
                        ticks,
                      })
                    }
                  >
                    {ticks < 0 ? "↶ Turn left" : "Turn right ↷"}
                  </button>
                ))}
                <button
                  onClick={() =>
                    run({
                      command: "input",
                      event: "touchTap",
                      index: slot.index,
                    })
                  }
                >
                  Tap screen
                </button>
                <button
                  onClick={() =>
                    run({
                      command: "input",
                      event: "touchTap",
                      index: slot.index,
                      hold: true,
                    })
                  }
                >
                  Hold screen
                </button>
                {slot.uuid === STACK &&
                  slot.stack_children?.map((child, i) => (
                    <button
                      key={child.context}
                      onClick={() =>
                        run({
                          command: "stack_select",
                          context: slot.context,
                          index: i,
                        })
                      }
                    >
                      {child.name}
                    </button>
                  ))}
              </div>
            )}
          </section>
        </main>
        <aside className="action-sidebar">
          <div className="section-heading">
            <h2>Action library</h2>
            <span className="count">{visible.length}</span>
          </div>
          <label className="search">
            <span className="sr-only">Search actions</span>
            <input
              ref={searchRef}
              type="search"
              placeholder="Search actions…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
            <kbd>Ctrl F</kbd>
          </label>
          <div className="segmented controller-filter">
            {(["All", "Keypad", "Encoder"] as const).map((value) => (
              <button
                key={value}
                aria-pressed={filter === value}
                onClick={() => setFilter(value)}
              >
                {value === "Keypad"
                  ? "Keys"
                  : value === "Encoder"
                    ? "Dials"
                    : value}
              </button>
            ))}
          </div>
          <div className="action-list">
            {categories.map((category) => (
              <details key={category} open>
                <summary>
                  {category}
                  <span>
                    {visible.filter((a) => a.category === category).length}
                  </span>
                </summary>
                {visible
                  .filter((a) => a.category === category)
                  .map((a) => (
                    <div
                      className="library-action"
                      key={a.uuid}
                      draggable={!readonly}
                      onDragStart={(event) => {
                        event.dataTransfer.setData(
                          "application/x-deckforge-action",
                          a.uuid,
                        );
                        event.dataTransfer.effectAllowed = "copy";
                      }}
                    >
                      <ActionIcon action={a} />
                      <div className="action-copy">
                        <strong>{a.name}</strong>
                        <small>
                          {a.controllers
                            .map((c) => (c === "Encoder" ? "Dial" : "Key"))
                            .join(" · ")}
                        </small>
                      </div>
                      <button
                        title={`Assign ${a.name} to selected ${selected.controller === "Keypad" ? "key" : "dial"}`}
                        aria-label={`Assign ${a.name}`}
                        disabled={readonly || !allowed(a, selected.controller)}
                        onClick={() => assign(a.uuid)}
                      >
                        +
                      </button>
                    </div>
                  ))}
              </details>
            ))}
            {!visible.length && (
              <div className="empty">
                <h3>No matching actions</h3>
                <p>Try another search or install a plugin.</p>
                <button
                  onClick={() => {
                    setSearch("");
                    setFilter("All");
                  }}
                >
                  Clear filters
                </button>
              </div>
            )}
          </div>
          <button
            className="install-footer"
            onClick={() => setPanel("plugins")}
          >
            ＋ Install a plugin
          </button>
        </aside>
      </div>
      <footer className="statusbar">
        <span role="status">{notice || snapshot.system_status || "Ready"}</span>
        <div className="row">
          {Array.isArray(snapshot.multi_runs) &&
            snapshot.multi_runs.length > 0 && (
              <button onClick={() => run({ command: "multi_cancel" })}>
                Stop sequences
              </button>
            )}
          {Object.keys(snapshot.audio_active ?? {}).length > 0 && (
            <button onClick={() => run({ command: "soundboard_stop" })}>
              Stop audio
            </button>
          )}
          <button onClick={() => setPanel("logs")}>Activity log</button>
        </div>
      </footer>
      {panel && (
        <Modal
          wide={panel === "plugins" || panel === "logs"}
          title={
            panel === "plugins"
              ? "Plugins"
              : panel === "preferences"
                ? "Preferences"
                : "Activity log"
          }
          onClose={() => setPanel(null)}
        >
          {panel === "plugins" ? (
            <Plugins snapshot={snapshot} run={run} onError={onError} />
          ) : panel === "preferences" ? (
            <Preferences snapshot={snapshot} run={run} onError={onError} />
          ) : (
            <pre className="logs">
              {snapshot.logs.join("\n") || "No activity yet."}
            </pre>
          )}
        </Modal>
      )}
      {libraryEdit && (
        <Modal
          title={`${libraryEdit.operation[0].toUpperCase() + libraryEdit.operation.slice(1)} ${libraryEdit.kind}`}
          onClose={() => setLibraryEdit(null)}
        >
          <form
            onSubmit={async (e) => {
              e.preventDefault();
              if (
                await run({
                  command: `${libraryEdit.kind}_${libraryEdit.operation}`,
                  id: libraryEdit.id,
                  name: libraryEdit.name.trim(),
                })
              )
                setLibraryEdit(null);
            }}
          >
            {libraryEdit.operation === "delete" ? (
              <p>Delete “{libraryEdit.name}” and its assignments?</p>
            ) : (
              <Field label="Name">
                <input
                  required
                  autoFocus
                  value={libraryEdit.name}
                  onChange={(e) =>
                    setLibraryEdit({ ...libraryEdit, name: e.target.value })
                  }
                />
              </Field>
            )}
            <div className="row">
              <button
                type="submit"
                className={
                  libraryEdit.operation === "delete" ? "danger" : "primary"
                }
              >
                {libraryEdit.operation === "delete" ? "Delete" : "Save"}
              </button>
              <button type="button" onClick={() => setLibraryEdit(null)}>
                Cancel
              </button>
            </div>
          </form>
        </Modal>
      )}
    </div>
  );
}
