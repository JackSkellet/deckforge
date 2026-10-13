import { useCallback, useEffect, useRef, useState } from "react";
import { command, pick, request } from "./bridge";
import { mergeSettingsEdit } from "./settings";
import {
  allowed,
  filteredActions,
  reconcileDraft,
  editDraft,
  finishSave,
  saveRequests,
  editAppearance,
  hasAppearanceEdits,
  sourceOf,
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
import { FolderExit } from "./FolderExit";
import { Sequence } from "./Sequence";
import { AudioMixer } from "./Mixer";
import { ActionEdit, isEditingTarget } from "./ActionEdit";
import { AppearanceEditor } from "./Appearance";
import { DeviceCanvas, canvasStyle } from "./DeviceCanvas";
import {
  deviceName,
  geometry,
  hasKeyImages,
  hasEncoderDisplay,
  hasTouchscreen,
  supportedControllers,
  validControl,
  isAssignableSlot,
  outsideControls,
  acceptsInput,
  controlName,
  associatedDevices,
  deviceOptionLabel,
} from "./device";
import { Modal, Plugins, Preferences } from "./Panels";
import {
  ProfileCopyDialog,
  eligibleProfileTargets,
  type ProfileCopyDraft,
} from "./ProfileCopy";
import {
  exportTextDrafts,
  clearDeviceFormDrafts,
  importTextDrafts,
  hasTextDrafts,
  exportFormDrafts,
  importFormDrafts,
  getFormDraft,
  setFormDraft,
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
const defaultSelection = { controller: "Keypad" as Controller, index: 0 };
export default function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const currentDevice = snapshot?.device ?? "";
  const [selections, setSelections] = useState<
    Record<string, { controller: Controller; index: number }>
  >({});
  const selected = selections[currentDevice] ?? defaultSelection;
  function setSelected(value: { controller: Controller; index: number }) {
    const owner = currentDevice;
    setSelections((previous) => ({ ...previous, [owner]: value }));
  }
  const [selectingDevice, setSelectingDevice] = useState(false);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<Controller | "All">("All");
  const [panel, setPanel] = useState<
    "plugins" | "preferences" | "logs" | "mixer" | null
  >(null);
  const [libraryEdit, setLibraryEdit] = useState<{
    kind: "profile" | "page";
    device?: string;
    id?: string;
    name: string;
    operation: "create" | "rename" | "delete" | "duplicate";
  } | null>(null);
  const [copyProfile, setCopyProfile] = useState<ProfileCopyDraft | null>(null);
  const profileCopySerial = useRef(0);
  const [copyingProfile, setCopyingProfile] = useState(false);
  const [saving, setSaving] = useState(false);
  const [editBusy, setEditBusy] = useState(false);
  const [editMenuRequest, setEditMenuRequest] = useState<{
    id: number;
    anchor?: { x: number; y: number };
  }>({ id: 0 });
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
          if (state.selections) setSelections(state.selections);
          else if (state.selected)
            setSelections({ [s.device]: state.selected });
          if (typeof state.search === "string") setSearch(state.search);
          if (["All", "Keypad", "Encoder", "Neo"].includes(state.filter))
            setFilter(state.filter);
          importTextDrafts(state.textDrafts);
          importFormDrafts(state.formDrafts);
          if (["plugins", "preferences", "logs", "mixer"].includes(state.panel))
            setPanel(state.panel);
          if (state.copyProfile) setCopyProfile(state.copyProfile);
          if (state.libraryEdit)
            setLibraryEdit({
              ...state.libraryEdit,
              device: state.libraryEdit.device ?? s.device,
            });
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
    selections,
    search,
    filter,
    textDrafts: exportTextDrafts(),
    formDrafts: exportFormDrafts(),
    panel,
    libraryEdit,
    copyProfile,
  };
  useEffect(() => {
    if (booted) void request("ui_state_set", { state: uiState }).catch(onError);
  }, [
    drafts,
    selected,
    selections,
    search,
    filter,
    booted,
    textRevision,
    panel,
    libraryEdit,
    copyProfile,
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
    async (payload: RecordValue, capturedDevice = currentDevice) => {
      try {
        setError("");
        await command(payload, capturedDevice);
        if (payload.command === "device_forget") {
          const device = payload.device ?? capturedDevice;
          clearDeviceFormDrafts(device);
          setSelections((previous) => {
            const next = { ...previous };
            delete next[device];
            return next;
          });
        }
        accept(await request<Snapshot>("snapshot"));
        if (payload.command === "device_forget")
          document
            .querySelector<HTMLButtonElement>(
              '[aria-label="Close Preferences"]',
            )
            ?.focus();
        return true;
      } catch (e) {
        onError(e);
        return false;
      }
    },
    [accept, onError, currentDevice],
  );
  async function selectDevice(device: string) {
    if (device === currentDevice || selectingDevice) return;
    setSelectingDevice(true);
    try {
      await run({ command: "device_select", device });
    } finally {
      setSelectingDevice(false);
    }
  }
  const slot = snapshot?.slots.find(
    (s): s is Slot & { controller: Controller } =>
      isAssignableSlot(s) &&
      s.controller === selected.controller &&
      s.index === selected.index,
  );
  const selectedAvailable =
    !!snapshot && validControl(snapshot, selected.controller, selected.index);
  useEffect(() => {
    if (!snapshot) return;
    if (
      !validControl(snapshot, selected.controller, selected.index) &&
      !snapshot.slots.some(
        (s) =>
          s.controller === selected.controller && s.index === selected.index,
      )
    )
      setSelected({ controller: "Keypad", index: 0 });
    if (filter !== "All" && !supportedControllers(snapshot).includes(filter))
      setFilter("All");
  }, [
    snapshot?.device,
    snapshot?.device_model,
    snapshot?.device_geometry?.keys,
    snapshot?.device_geometry?.dials,
    selected.controller,
    selected.index,
    filter,
  ]);
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
  const folderPages: [string, string][] = (profile?.pages ?? [])
    .filter((candidate) => candidate.id !== profile?.pin_page)
    .map((candidate) => [candidate.id, candidate.name]);
  const readonly = profile?.readonly ?? false;
  function openProfileCopy() {
    if (!snapshot || !profile) return;
    const devices = associatedDevices(snapshot).map((device) => ({
      ...device,
    }));
    setError("");
    setCopyProfile({
      operation_id: String(++profileCopySerial.current),
      device: currentDevice,
      profile: profile.id,
      source_name: profile.name,
      devices,
      target_device:
        eligibleProfileTargets(devices, currentDevice)[0]?.id ?? "",
      name: `${profile.name} copy`,
      simulate: snapshot.simulate,
    });
  }
  async function submitProfileCopy() {
    if (
      !copyProfile ||
      copyingProfile ||
      !eligibleProfileTargets(copyProfile.devices, copyProfile.device).some(
        (device) => device.id === copyProfile.target_device,
      )
    )
      return;
    const captured = copyProfile;
    setCopyingProfile(true);
    try {
      if (
        await run(
          {
            command: "profile_copy_to_device",
            device: captured.device,
            profile: captured.profile,
            target_device: captured.target_device,
            ...(captured.name.trim() ? { name: captured.name.trim() } : {}),
          },
          captured.device,
        )
      ) {
        setCopyProfile((current) =>
          current?.operation_id === captured.operation_id ? null : current,
        );
        const target = captured.devices.find(
          (device) => device.id === captured.target_device,
        );
        setNotice(
          `Copied ${captured.source_name} to ${target?.name ?? captured.target_device}`,
        );
      }
    } finally {
      setCopyingProfile(false);
    }
  }
  function change(patch: Partial<Draft>) {
    if (!slot) return;
    const rendered = draft ?? reconcileDraft(undefined, slot);
    setDrafts((prev) => {
      const latest = prev[slot.context] ?? rendered;
      const edit = Object.hasOwn(patch, "settings")
        ? {
            ...patch,
            settings: mergeSettingsEdit(
              rendered.settings,
              patch.settings,
              latest.settings,
            ),
          }
        : Object.hasOwn(patch, "raw")
          ? {
              ...patch,
              raw: mergeSettingsEdit(rendered.raw, patch.raw, latest.raw),
            }
          : patch;
      return { ...prev, [slot.context]: editDraft(latest, edit) };
    });
  }
  const appearanceStale =
    !!slot &&
    !!draft &&
    hasAppearanceEdits(draft) &&
    draft.appearanceExpectedState !== undefined &&
    draft.appearanceExpectedState !== slot.state;
  async function save() {
    if (
      !slot ||
      !draft ||
      !draft.dirty ||
      readonly ||
      saving ||
      editBusy ||
      !form.current?.reportValidity()
    )
      return;
    if (appearanceStale) {
      onError(
        new Error(
          "The action state changed. Refresh the appearance state and preview before saving.",
        ),
      );
      return;
    }
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
      for (const payload of saveRequests(
        target,
        savingDraft,
        action?.user_title_enabled !== false,
      )) {
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
      if (
        event.defaultPrevented ||
        event.repeat ||
        event.altKey ||
        event.shiftKey ||
        isEditingTarget(event.target)
      )
        return;
      if (
        event.key === "Escape" &&
        snapshot?.clipboard?.mode === "cut" &&
        !document.querySelector(
          ".control-edit-menu[open],.page-menu[open],.sidebar-bottom details[open]",
        )
      ) {
        event.preventDefault();
        void cancelCut();
        return;
      }
      const shortcut = event.key.toLowerCase();
      if (
        (event.ctrlKey || event.metaKey) &&
        ["c", "x", "v"].includes(shortcut)
      ) {
        event.preventDefault();
        if (shortcut === "v") void pasteAction();
        else void clipboardAction(shortcut === "x" ? "cut" : "copy");
      } else if (event.key === "Delete" && !event.ctrlKey && !event.metaKey) {
        event.preventDefault();
        void removeAction();
      }
    }
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [
    snapshot,
    slot,
    draft,
    drafts,
    saving,
    editBusy,
    selected,
    readonly,
    textRevision,
  ]);
  function openControlMenu(
    target: typeof selected,
    anchor?: { x: number; y: number },
  ) {
    if (document.querySelector("dialog[open]")) return false;
    setSelected(target);
    setEditMenuRequest((previous) => ({ id: previous.id + 1, anchor }));
    return true;
  }
  function hasUnsaved(context: string | undefined) {
    return !!context && (!!drafts[context]?.dirty || hasTextDrafts(context));
  }
  function savedOnly(context: string | undefined, operation: string) {
    if (hasUnsaved(context)) {
      onError(new Error(`Save or revert changes before ${operation}.`));
      return false;
    }
    return true;
  }
  async function cancelCut() {
    if (editBusy || saving) return;
    if (await run({ command: "slot_clipboard_clear" }))
      setNotice("Cut cancelled; the source action is unchanged.");
  }
  async function clipboardAction(mode: "copy" | "cut") {
    if (!slot || editBusy || saving || (mode === "cut" && readonly)) return;
    if (
      !savedOnly(
        slot.context,
        `${mode === "copy" ? "copying" : "cutting"} this action`,
      )
    )
      return;
    setEditBusy(true);
    try {
      if (
        await run({
          command: mode === "copy" ? "slot_copy" : "slot_cut",
          context: slot.context,
        })
      )
        setNotice(
          `${mode === "copy" ? "Copied" : "Cut"} ${slot.name}. Select a destination and paste.`,
        );
    } finally {
      setEditBusy(false);
    }
  }
  async function pasteAction(target = selected) {
    const clipboard = snapshot?.clipboard;
    if (!clipboard || readonly || editBusy || saving) return;
    if (!snapshot || !validControl(snapshot, target.controller, target.index)) {
      onError(
        new Error("Select a control within this device layout before pasting."),
      );
      return;
    }
    const destination = snapshot?.slots.find(
      (s) => s.controller === target.controller && s.index === target.index,
    );
    if (
      !savedOnly(destination?.context, "replacing this action") ||
      (clipboard.mode === "cut" &&
        !savedOnly(clipboard.context, "moving the cut action"))
    )
      return;
    const metadata = snapshot?.actions.find((a) => a.uuid === clipboard.uuid);
    if (metadata && !metadata.controllers.includes(target.controller)) {
      onError(new Error("The copied action does not support that control."));
      return;
    }
    setEditBusy(true);
    try {
      if (
        await run({
          command: "slot_paste",
          controller: target.controller,
          index: target.index,
          expected_target: destination?.context ?? null,
        })
      ) {
        setSelected(target);
        setNotice(`Pasted ${clipboard.name}`);
      }
    } finally {
      setEditBusy(false);
    }
  }
  async function transferAction(
    context: string,
    target: typeof selected,
    copy: boolean,
  ) {
    if (readonly || editBusy || saving) return;
    const source = snapshot?.slots.find(
      (s): s is Slot & { controller: Controller } =>
        isAssignableSlot(s) && s.context === context,
    );
    const destination = snapshot?.slots.find(
      (s) => s.controller === target.controller && s.index === target.index,
    );
    if (destination?.context === context) return;
    if (
      !savedOnly(
        context,
        copy ? "copying this action" : "moving this action",
      ) ||
      !savedOnly(destination?.context, "replacing this action")
    )
      return;
    const metadata = snapshot?.actions.find((a) => a.uuid === source?.uuid);
    if (metadata && !metadata.controllers.includes(target.controller)) {
      onError(new Error("This action does not support that control."));
      return;
    }
    const reverse = snapshot?.actions.find((a) => a.uuid === destination?.uuid);
    if (
      !copy &&
      source &&
      reverse &&
      !reverse.controllers.includes(source.controller)
    ) {
      onError(
        new Error(
          "These actions cannot swap because their supported controls differ.",
        ),
      );
      return;
    }
    setEditBusy(true);
    try {
      if (
        await run({
          command: "slot_transfer",
          context,
          controller: target.controller,
          index: target.index,
          copy,
          expected_target: destination?.context ?? null,
        })
      ) {
        setSelected(target);
        setNotice(`${copy ? "Copied" : "Moved"} ${source?.name ?? "action"}`);
      }
    } finally {
      setEditBusy(false);
    }
  }
  async function removeAction() {
    if (
      !slot ||
      readonly ||
      saving ||
      editBusy ||
      !savedOnly(slot.context, "removing this action")
    )
      return;
    setEditBusy(true);
    try {
      if (await run({ command: "remove", context: slot.context }))
        setNotice("Action removed");
    } finally {
      setEditBusy(false);
    }
  }
  async function assign(uuid: string, target = selected) {
    if (readonly || editBusy || saving) return;
    if (!snapshot || !validControl(snapshot, target.controller, target.index)) {
      onError(new Error("Select a control within this device layout first."));
      return;
    }
    const destination = snapshot?.slots.find(
      (s) => s.controller === target.controller && s.index === target.index,
    );
    if (!savedOnly(destination?.context, "replacing this action")) return;

    const metadata = snapshot?.actions.find((a) => a.uuid === uuid);
    if (metadata?.enabled === false) {
      onError(
        new Error(
          "Enable this plugin in Plugins before assigning its actions.",
        ),
      );
      return;
    }
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
    if (
      !slot ||
      slot.plugin_disabled ||
      !snapshot ||
      !acceptsInput(snapshot, slot.controller, slot.index)
    )
      return;
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
    const context = event.dataTransfer.getData("application/x-deckforge-slot");
    if (context) {
      void transferAction(
        context,
        target,
        Boolean(event.ctrlKey || event.metaKey),
      );
      return;
    }
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
  const availableControllers = supportedControllers(snapshot);
  const visible = filteredActions(
    snapshot.actions,
    search,
    filter,
    availableControllers,
  );
  const overflow = outsideControls(snapshot);
  const archivedOverflow = snapshot.overflow_slots ?? [];
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
          (controller === "Neo"
            ? "deck-infobar"
            : controller === "Keypad"
              ? "deck-key"
              : "deck-dial") +
          (active ? " selected" : "") +
          (!item ? " unassigned" : "") +
          (item &&
          snapshot.clipboard?.mode === "cut" &&
          snapshot.clipboard.context === item.context
            ? " cut-control"
            : "")
        }
        aria-label={`${controlName(controller, snapshot)}${controller === "Neo" ? "" : ` ${index + 1}`}: ${item?.name ?? "Unassigned"}`}
        title={`${controlName(controller, snapshot)}${controller === "Neo" ? "" : ` ${index + 1}`}: ${item?.name ?? "Unassigned"}${item?.title ? `\n${item.title}` : ""}`}
        aria-pressed={active}
        onClick={() => setSelected(target)}
        onContextMenu={(event) => {
          if (openControlMenu(target, { x: event.clientX, y: event.clientY }))
            event.preventDefault();
        }}
        onKeyDown={(event) => {
          if (
            event.key === "ContextMenu" ||
            (event.shiftKey && event.key === "F10")
          ) {
            const rect = event.currentTarget.getBoundingClientRect();
            if (openControlMenu(target, { x: rect.left, y: rect.bottom })) {
              event.preventDefault();
              event.stopPropagation();
            }
          }
        }}
        draggable={!!item && !editBusy && !saving}
        onDragStart={(event) => {
          if (!item) return;
          if (
            (readonly && !event.ctrlKey && !event.metaKey) ||
            !savedOnly(item.context, "moving or copying this action")
          ) {
            event.preventDefault();
            return;
          }
          event.dataTransfer.setData(
            "application/x-deckforge-slot",
            item.context,
          );
          // WebKitGTK needs a standard format for native drag feedback. The
          // custom type remains the only payload accepted by our drop handler.
          event.dataTransfer.setData("text/plain", item.name || item.uuid);
          event.dataTransfer.effectAllowed = "copyMove";
          setSelected(target);
        }}
        onDragOver={(e) => {
          if (
            e.dataTransfer.types.includes("application/x-deckforge-action") ||
            e.dataTransfer.types.includes("application/x-deckforge-slot")
          ) {
            e.preventDefault();
            e.dataTransfer.dropEffect =
              !e.dataTransfer.types.includes("application/x-deckforge-slot") ||
              e.ctrlKey ||
              e.metaKey
                ? "copy"
                : "move";
          }
        }}
        onDrop={(e) => drop(e, target)}
      >
        {controller === "Encoder" && !hasEncoderDisplay(snapshot) ? (
          <span className="dial-assignment">
            <span>Dial {index + 1}</span>
            <strong>{item?.name ?? "Unassigned"}</strong>
          </span>
        ) : !hasKeyImages(snapshot) && controller === "Keypad" ? (
          <span className="pedal-assignment">
            <span>Pedal {index + 1}</span>
            <strong>{item?.name ?? "Unassigned"}</strong>
          </span>
        ) : item?.preview ? (
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
        {item &&
          snapshot.clipboard?.mode === "cut" &&
          snapshot.clipboard.context === item.context && (
            <span className="cut-badge">Cut</span>
          )}
        {item?.pinned && (
          <span className="pin" title="Pinned">
            ◆
          </span>
        )}
        <span className="sr-only">{item?.title}</span>
        {controller === "Encoder" && (
          <span className="dial-knob" aria-hidden="true">
            {item?.software_icon?.trim() ? (
              <img
                className="dial-software-icon"
                src={
                  item.software_icon.startsWith("data:image/png;base64,")
                    ? item.software_icon
                    : `data:image/png;base64,${item.software_icon}`
                }
                alt=""
                draggable={false}
              />
            ) : (
              <i />
            )}
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
        <div
          className="device-status"
          title={snapshot.connection_error || snapshot.device}
        >
          <span className={"dot " + (snapshot.connected ? "online" : "")} />
          <label className="device-picker">
            <span className="sr-only">Selected device</span>
            <select
              value={snapshot.device}
              disabled={selectingDevice}
              onChange={(event) => void selectDevice(event.target.value)}
              title={`${deviceName(snapshot)} · ${snapshot.device}`}
            >
              {associatedDevices(snapshot).map((device) => (
                <option
                  key={device.id}
                  value={device.id}
                  title={`${device.name} · ${device.id}`}
                >
                  {deviceOptionLabel(
                    device,
                    associatedDevices(snapshot),
                    snapshot.simulate,
                  )}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="header-tools">
          <button onClick={() => setPanel("mixer")}>Audio mixer</button>
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
      {snapshot.marketplace &&
        (snapshot.marketplace.busy ||
          snapshot.marketplace.pending ||
          snapshot.marketplace.review ||
          snapshot.marketplace.error) && (
          <div
            className={`banner ${snapshot.marketplace.error ? "error" : ""}`}
            role="status"
          >
            <span>
              {snapshot.marketplace.error || snapshot.marketplace.message}
            </span>
            <button
              onClick={() => {
                const key = `preferences:${snapshot.device}`;
                setFormDraft(key, { ...getFormDraft(key), tab: "Marketplace" });
                setPanel("preferences");
              }}
            >
              Marketplace
            </button>
          </div>
        )}
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
                  device: currentDevice,
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
                disabled={p.compatible === false}
                title={
                  p.compatible === false
                    ? `This profile requires ${p.device_name ?? p.device_model ?? "another device model"}. Its controls are preserved.`
                    : p.device_name
                }
                onClick={() => run({ command: "profile_select", id: p.id })}
              >
                <span className="profile-symbol" aria-hidden="true">
                  ▦
                </span>
                <span>
                  {p.name}
                  {p.device_name && <small>{p.device_name}</small>}
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
                <button
                  type="button"
                  disabled={!profile}
                  onClick={openProfileCopy}
                >
                  Copy to device…
                </button>
                {(["rename", "duplicate", "delete"] as const).map(
                  (operation) => (
                    <button
                      key={operation}
                      disabled={
                        !profile || (readonly && operation !== "duplicate")
                      }
                      onClick={() =>
                        setLibraryEdit({
                          device: currentDevice,
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
              {snapshot.slots.filter(isAssignableSlot).length - overflow.length}{" "}
              assigned controls
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
                    device: currentDevice,
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
                          device: currentDevice,
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
          {(overflow.length > 0 || archivedOverflow.length > 0) && (
            <details className="layout-overflow">
              <summary>
                {overflow.length + archivedOverflow.length} saved controls
                outside this device layout
              </summary>
              <p>
                These assignments are preserved. They are not sent to this
                device.
              </p>
              <ul>
                {overflow.map((item) => (
                  <li key={item.context}>
                    <span>
                      {item.name} · {item.controller} {item.index + 1}
                    </span>
                    <button
                      type="button"
                      onClick={() =>
                        setSelected({
                          controller: item.controller,
                          index: item.index,
                        })
                      }
                    >
                      Inspect / copy
                    </button>
                  </li>
                ))}
                {archivedOverflow.map((item, index) => (
                  <li key={item.context ?? index}>
                    <details>
                      <summary>
                        {item.name ?? item.uuid ?? "Saved action"} ·{" "}
                        {item.controller ?? "Unknown control"}{" "}
                        {item.position ?? item.index ?? ""}
                      </summary>
                      <p>
                        {item.reason ??
                          "Original profile data is preserved; export the profile to retain this assignment."}
                      </p>
                      <pre>{JSON.stringify(item.raw ?? item, null, 2)}</pre>
                    </details>
                  </li>
                ))}
              </ul>
            </details>
          )}
          <div
            className="device-stage model-stage"
            style={canvasStyle(snapshot)}
            data-dials={geometry(snapshot).dials > 0}
            data-infobar={snapshot.device_model === "neo"}
            data-key-images={hasKeyImages(snapshot)}
            data-rows={geometry(snapshot).rows}
            data-model={snapshot.device_model}
            data-encoder-display={hasEncoderDisplay(snapshot)}
          >
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
            <DeviceCanvas
              snapshot={snapshot}
              renderSlot={slotButton}
              onPage={(direction) =>
                void run({ command: "page_step", direction })
              }
            />
            <p className="stage-hint">
              {hasKeyImages(snapshot)
                ? `Select a control to edit · Drag an action onto a ${geometry(snapshot).dials > 0 ? "key or dial" : snapshot.device_model === "neo" ? "key or the infobar" : "key"}`
                : "Software assignments · This device has no displays"}
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
                  {controlName(selected.controller, snapshot).toUpperCase()}
                  {selected.controller === "Neo"
                    ? ""
                    : ` ${selected.index + 1}`}
                </span>
                <h2>{slot?.name ?? "Choose an action"}</h2>
              </div>
              <div className="row">
                <ActionEdit
                  openRequest={editMenuRequest}
                  destinationAvailable={selectedAvailable}
                  slot={slot}
                  clipboard={snapshot.clipboard}
                  readonly={readonly}
                  busy={editBusy || saving}
                  onCopy={() => void clipboardAction("copy")}
                  onCut={() => void clipboardAction("cut")}
                  onPaste={() => void pasteAction()}
                  onRemove={() => void removeAction()}
                  onCancelCut={() => void cancelCut()}
                />
                {slot && (
                  <>
                    {slot.controller !== "Neo" && (
                      <button
                        onClick={testPress}
                        disabled={!selectedAvailable || slot.plugin_disabled}
                      >
                        Test{" "}
                        {selected.controller === "Keypad" ? "press" : "push"}
                      </button>
                    )}
                    {slot.has_inspector && !slot.plugin_disabled && (
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
                  </>
                )}
              </div>
            </header>
            {!selectedAvailable && (
              <p className="warning outside-selected">
                This saved action lies outside the current device layout. You
                can inspect or copy it; hardware tests and assignment are
                unavailable here.
              </p>
            )}
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
                    disabled={readonly || saving || editBusy}
                    className="inspector-fields"
                  >
                    {slot.controller === "Encoder" &&
                    !hasEncoderDisplay(snapshot) ? (
                      <>
                        <p className="hint">
                          This dial has no display. Its action settings remain
                          editable.
                        </p>
                        {snapshot.device_model === "studio" && (
                          <p className="hint">
                            Studio LED and ring feedback is not supported.
                          </p>
                        )}
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
                      </>
                    ) : (
                      <AppearanceEditor
                        key={`appearance:${snapshot.device}:${snapshot.device_model}:${slot.context}`}
                        slot={slot}
                        device={currentDevice}
                        draft={draft}
                        userTitleEnabled={action?.user_title_enabled !== false}
                        fallback={<ActionIcon action={action} />}
                        onError={onError}
                        onEdit={(state, patch, base) =>
                          setDrafts((previous) => ({
                            ...previous,
                            [slot.context]: editAppearance(
                              previous[slot.context] ??
                                reconcileDraft(undefined, slot),
                              slot,
                              state,
                              patch,
                              base,
                            ),
                          }))
                        }
                        onSelectState={(state) =>
                          setDrafts((previous) => ({
                            ...previous,
                            [slot.context]: {
                              ...previous[slot.context],
                              appearanceState: state,
                            },
                          }))
                        }
                        onRefreshState={() =>
                          setDrafts((previous) => ({
                            ...previous,
                            [slot.context]: {
                              ...previous[slot.context],
                              appearanceExpectedState: slot.state,
                              revision: previous[slot.context].revision + 1,
                            },
                          }))
                        }
                      >
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
                      </AppearanceEditor>
                    )}
                    {slot.plugin_disabled ? (
                      <p className="warning">
                        This plugin is disabled. Its settings and draft are
                        preserved.{" "}
                        <button
                          type="button"
                          onClick={() => setPanel("plugins")}
                        >
                          Enable plugin in Plugins
                        </button>
                      </p>
                    ) : (
                      slot.available === false && (
                        <p className="warning">
                          This action's plugin is unavailable. Your assignment
                          and settings are preserved. Reinstall the plugin in
                          Plugins, or restart it if it is already installed.{" "}
                          <button
                            type="button"
                            onClick={() => setPanel("plugins")}
                          >
                            Open Plugins
                          </button>
                        </p>
                      )
                    )}
                    {draft.dirty && draft.source !== sourceOf(slot) && (
                      <p className="warning">
                        The plugin changed this action while you were editing.
                        Your draft is kept; saving applies your changes.
                      </p>
                    )}
                    {snapshot.library.folder_depth > 0 &&
                      ![
                        "com.elgato.streamdeck.profile.openchild",
                        "com.elgato.streamdeck.page.next",
                        "com.elgato.streamdeck.page.previous",
                        "com.elgato.streamdeck.page.goto",
                        "com.elgato.streamdeck.page.indicator",
                      ].includes(slot.uuid) && (
                        <FolderExit
                          key={`folder-exit:${slot.context}`}
                          resetCount={
                            snapshot.slots.filter(
                              (s) =>
                                !s.pinned &&
                                s.context !== slot.context &&
                                s.raw.AutoExit != null,
                            ).length
                          }
                          onReset={async () => {
                            if (
                              snapshot.slots.some((s) => hasUnsaved(s.context))
                            ) {
                              onError(
                                new Error(
                                  "Save or revert this folder's action drafts before resetting overrides.",
                                ),
                              );
                              return;
                            }
                            if (
                              await run({
                                command: "folder_exit_reset",
                                context: slot.context,
                                page: snapshot.library.active_page,
                                confirm: true,
                              })
                            )
                              setNotice(
                                "Action overrides reset to the folder default",
                              );
                          }}
                          raw={draft.raw}
                          parent={
                            snapshot.slots.find(
                              (s) =>
                                s.uuid ===
                                "com.elgato.streamdeck.profile.backtoparent",
                            )?.raw
                          }
                          back={
                            slot.uuid ===
                            "com.elgato.streamdeck.profile.backtoparent"
                          }
                          onChange={(update) =>
                            setDrafts((previous) => {
                              const latest =
                                previous[slot.context] ??
                                reconcileDraft(undefined, slot);
                              return {
                                ...previous,
                                [slot.context]: editDraft(latest, {
                                  raw: update(latest.raw),
                                }),
                              };
                            })
                          }
                        />
                      )}
                    {isContainer(slot.uuid) ? (
                      <Sequence
                        folderPages={folderPages}
                        encoderDisplay={hasEncoderDisplay(snapshot)}
                        device={currentDevice}
                        mixer={snapshot.mixer?.config}
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
                        folderPages={folderPages}
                        device={currentDevice}
                        mixer={snapshot.mixer?.config}
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
                          editBusy ||
                          appearanceStale ||
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
                      disabled={readonly || saving || editBusy}
                      onClick={() => void removeAction()}
                    >
                      Remove action
                    </button>
                  </footer>
                </form>
              )
            )}
            {slot?.controller === "Encoder" &&
              selectedAvailable &&
              !slot.plugin_disabled && (
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
                  {hasTouchscreen(snapshot) && (
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
                  )}
                  {hasTouchscreen(snapshot) && (
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
                  )}
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
            {(["All", ...availableControllers] as const).map((value) => (
              <button
                key={value}
                aria-pressed={filter === value}
                onClick={() => setFilter(value)}
              >
                {value === "Keypad"
                  ? "Keys"
                  : value === "Encoder"
                    ? "Dials"
                    : value === "Neo"
                      ? "Infobar"
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
                      className={`library-action${a.enabled === false ? " plugin-disabled" : ""}`}
                      title={
                        a.enabled === false
                          ? "Enable this plugin in Plugins to assign its actions."
                          : a.tooltip
                      }
                      aria-disabled={a.enabled === false}
                      key={a.uuid}
                      draggable={!readonly && a.enabled !== false}
                      onDragStart={(event) => {
                        if (readonly || a.enabled === false) {
                          event.preventDefault();
                          return;
                        }
                        event.dataTransfer.setData(
                          "application/x-deckforge-action",
                          a.uuid,
                        );
                        event.dataTransfer.setData(
                          "text/plain",
                          a.name || a.uuid,
                        );
                        event.dataTransfer.effectAllowed = "copy";
                      }}
                    >
                      <ActionIcon action={a} />
                      <div className="action-copy">
                        <strong>{a.name}</strong>
                        <small>
                          {a.enabled === false ? "Disabled · " : ""}
                          {a.controllers
                            .filter((c) => availableControllers.includes(c))
                            .map((c) => controlName(c, snapshot))
                            .join(" · ")}
                        </small>
                      </div>
                      <button
                        title={`Assign ${a.name} to selected ${selected.controller === "Keypad" ? "key" : "dial"}`}
                        aria-label={`Assign ${a.name}`}
                        disabled={
                          readonly ||
                          !selectedAvailable ||
                          !allowed(a, selected.controller)
                        }
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
          wide={panel === "plugins" || panel === "logs" || panel === "mixer"}
          className={panel === "mixer" ? "mixer-dialog" : undefined}
          title={
            panel === "mixer"
              ? "Audio mixer"
              : panel === "plugins"
                ? "Plugins"
                : panel === "preferences"
                  ? "Preferences"
                  : "Activity log"
          }
          onClose={() => setPanel(null)}
        >
          {panel === "mixer" ? (
            <AudioMixer snapshot={snapshot.mixer} run={run} error={error} />
          ) : panel === "plugins" ? (
            <Plugins
              snapshot={snapshot}
              run={run}
              onError={onError}
              error={error}
            />
          ) : panel === "preferences" ? (
            <Preferences
              key={`${snapshot.device}:${snapshot.device_model}`}
              snapshot={snapshot}
              run={run}
              onError={onError}
              error={error}
            />
          ) : (
            <pre className="logs">
              {snapshot.logs.join("\n") || "No activity yet."}
            </pre>
          )}
        </Modal>
      )}
      {copyProfile && (
        <ProfileCopyDialog
          draft={copyProfile}
          busy={copyingProfile}
          error={error}
          onChange={setCopyProfile}
          onSubmit={() => void submitProfileCopy()}
          onClose={() => setCopyProfile(null)}
        />
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
                await run(
                  {
                    command: `${libraryEdit.kind}_${libraryEdit.operation}`,
                    id: libraryEdit.id,
                    name: libraryEdit.name.trim(),
                  },
                  libraryEdit.device ?? currentDevice,
                )
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
