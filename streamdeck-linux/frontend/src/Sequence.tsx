import { useId, useRef, useState, type DragEvent } from "react";
import { isEditingTarget } from "./ActionEdit";
import { CommandOutput } from "./CommandOutput";
import { TypingAccess } from "./TypingAccess";
import type { MixerConfig } from "./Mixer";
import {
  children,
  replaceChildren,
  newChild,
  childAllowed,
  isMulti,
  isContainer,
  STACK,
  WHEEL,
  TRIGGER,
  LOGIC,
  clone,
  type RecordValue,
  type Action,
  type Slot,
  type Clipboard,
  type TypingStatus,
} from "./model";
import {
  SettingsFields,
  NumberField,
  Check,
  SelectField,
  JsonEditor,
  Field,
} from "./Fields";
export function Sequence({
  raw,
  uuid,
  onChange,
  actions,
  outputs,
  onError,
  depth = 0,
  scope = uuid,
  runtime,
  run,
  mixer,
  device,
  encoderDisplay = true,
  folderPages,
  clipboard,
  clipboardRun,
  clipboardReady = true,
  readOnly = false,
  typingStatus,
}: {
  scope?: string;
  folderPages?: [string, string][];
  device?: string;
  encoderDisplay?: boolean;
  mixer?: MixerConfig;
  runtime?: Slot;
  run?: (p: RecordValue) => Promise<boolean>;
  raw: RecordValue;
  uuid: string;
  onChange: (v: RecordValue) => void;
  actions: Action[];
  outputs: [string, string][];
  onError: (e: unknown) => void;
  depth?: number;
  clipboard?: Clipboard | null;
  clipboardRun?: (p: RecordValue) => Promise<boolean>;
  clipboardReady?: boolean;
  readOnly?: boolean;
  typingStatus?: TypingStatus;
}) {
  const owner = useId();
  const dragging = useRef<{index:number;branch:number;signature:string} | null>(null);
  const [dropIndex,setDropIndex] = useState<number | null>(null);
  const [branch, setBranch] = useState(0);
  const [selected, setSelected] = useState<number | null>(null);
  const [add, setAdd] = useState("");
  const entries = children(raw, uuid, branch);
  const actionIdCounts = new Map<string, number>();
  for (const entry of entries) if (entry.ActionID !== undefined) {
    const id = String(entry.ActionID);
    actionIdCounts.set(id, (actionIdCounts.get(id) ?? 0) + 1);
  }
  const logic = uuid === LOGIC;
  const trigger = uuid === TRIGGER || logic;
  const stack = uuid === STACK;
  const wheel = uuid === WHEEL;
  const selectedEntry = selected === null ? null : entries[selected];
  const runtimeEntries = runtime
    ? runtime.uuid === TRIGGER || runtime.uuid === LOGIC
      ? runtime.trigger_children
      : runtime.uuid === STACK
        ? runtime.stack_children
        : (runtime.multi_children[branch] ?? [])
    : [];
  const runtimeChild = (entry: RecordValue) => {
    if (entry.ActionID === undefined) {
      const child = runtimeEntries[entries.indexOf(entry)];
      return child &&
        child.uuid === entry.UUID &&
        child.raw?.ActionID === undefined
        ? child
        : undefined;
    }
    return runtimeEntries.find(
      (child) =>
        child &&
        child.uuid === entry.UUID &&
        child.raw?.ActionID === entry.ActionID,
    );
  };
  const liveChild = selectedEntry ? runtimeChild(selectedEntry) : undefined;
  // Saved legacy children can be matched by position. While editing/reordering,
  // require a unique stable ID so Stop/Copy cannot target a same-UUID sibling.
  const commandChild = (() => {
    if (selectedEntry?.UUID !== "com.deckforge.system.command" || selected === null) return undefined;
    if (clipboardReady) {
      const child = runtimeEntries[selected];
      return child && child.uuid === selectedEntry.UUID && child.raw?.ActionID === selectedEntry.ActionID ? child : undefined;
    }
    if (selectedEntry.ActionID === undefined) return undefined;
    const matches = runtimeEntries.filter((child) => child && child.uuid === selectedEntry.UUID && child.raw?.ActionID === selectedEntry.ActionID);
    return matches.length === 1 && entries.filter((entry) => entry.UUID === selectedEntry.UUID && entry.ActionID === selectedEntry.ActionID).length === 1 ? matches[0] : undefined;
  })();
  const copiedAction = actions.find((a) => a.uuid === clipboard?.uuid);
  const canPaste =
    !!clipboard &&
    (copiedAction
      ? childAllowed(copiedAction, uuid)
      : clipboard.controller === (stack ? "Encoder" : "Keypad"));
  function childLabel(entry: RecordValue, fallback = "Unassigned") {
    const action = actions.find((candidate) => candidate.uuid === entry.UUID);
    if (entry.Name && entry.Name !== action?.source_name) return entry.Name;
    const child = runtimeChild(entry);
    return child?.name || action?.name || entry.Name || fallback;
  }
  const compatible = actions.filter(
    (a) => childAllowed(a, uuid) && a.visible !== false,
  );
  function update(values: RecordValue[]) {
    if (readOnly) return false;
    try {
      onChange(replaceChildren(raw, uuid, values, branch));
      return true;
    } catch (e) {
      onError(e);
      return false;
    }
  }
  function replace(index: number, entry: RecordValue) {
    const values = clone(entries);
    while (values.length <= index) values.push({ Actions: [] });
    values[index] = entry;
    update(values);
  }
  function move(index: number, delta: number) {
    const values = clone(entries);
    const next = index + delta;
    if (next < 0 || next >= values.length) return;
    [values[index], values[next]] = [values[next], values[index]];
    update(values);
    setSelected(next);
  }
  const actionType = "application/x-deckforge-action";
  const stepType = "application/x-deckforge-sequence-step";
  function dragOver(event: DragEvent<HTMLElement>, index: number) {
    const step = event.dataTransfer.types.includes(stepType);
    if (!step && !event.dataTransfer.types.includes(actionType)) return;
    event.stopPropagation();
    if (readOnly || event.currentTarget.closest("fieldset:disabled") || (step && (trigger || !dragging.current))) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = step ? "move" : "copy";
    setDropIndex(index);
    const panel=event.currentTarget.closest<HTMLElement>(".inspector-fields");
    if(panel && panel.scrollHeight>panel.clientHeight){
      const box=panel.getBoundingClientRect();
      if(event.clientY>=box.top && event.clientY<box.top+28) panel.scrollTop-=16;
      else if(event.clientY<=box.bottom && event.clientY>box.bottom-28) panel.scrollTop+=16;
    }
  }
  function drop(event: DragEvent<HTMLElement>, index: number) {
    const step = event.dataTransfer.types.includes(stepType);
    if (!step && !event.dataTransfer.types.includes(actionType)) return;
    event.preventDefault();event.stopPropagation();setDropIndex(null);
    if (readOnly || event.currentTarget.closest("fieldset:disabled")) return;
    if (step) {
      const source=dragging.current;dragging.current=null;
      try {
        const data=JSON.parse(event.dataTransfer.getData(stepType));
        if (trigger || !source || data.owner!==owner || data.branch!==branch || data.index!==source.index || source.branch!==branch || source.signature!==JSON.stringify(entries))
          throw new Error("The sequence changed during the drag. Drag the step again within the same sequence.");
        const values=clone(entries);const [entry]=values.splice(source.index,1);
        const destination=index>source.index ? index-1 : index;
        values.splice(destination,0,entry);
        if (destination===source.index || update(values)) setSelected(destination);
      } catch(error) {onError(error);}
      return;
    }
    const action=compatible.find(a=>a.uuid===event.dataTransfer.getData(actionType)&&a.enabled!==false);
    if (!action) {onError(new Error(trigger ? "This action cannot be assigned to this gesture." : stack ? "Dial stacks accept compatible dial actions." : "This action cannot be added to this sequence."));return;}
    if (trigger) {replace(index,newChild(action));setSelected(index);}
    else {const values=clone(entries);values.splice(index,0,newChild(action));if(update(values))setSelected(index);}
  }
  function insertion(event: DragEvent<HTMLElement>, index:number) {
    if (trigger) return index;
    const box=event.currentTarget.getBoundingClientRect();
    return index+(event.clientY>=box.top+box.height/2 ? 1 : 0);
  }
  function copySelected(cut = false) {
    if ((!cut || !readOnly) && clipboardReady && liveChild && clipboardRun)
      void clipboardRun({
        command: cut ? "slot_cut" : "slot_copy",
        context: liveChild.context,
      });
  }
  async function pasteSelected() {
    if (
      readOnly ||
      !clipboardReady ||
      !runtime ||
      !canPaste ||
      !clipboardRun ||
      (trigger && selected === null)
    )
      return;
    const index =
      selected === null ? entries.length : trigger ? selected : selected + 1;
    if (
      await clipboardRun({
        command: "child_paste",
        context: runtime.context,
        branch,
        index,
        expected: runtime.raw,
      })
    )
      setSelected(
        !trigger &&
          clipboard?.mode === "cut" &&
          runtimeEntries.some(
            (c, i) => c?.context === clipboard.context && i < index,
          )
          ? index - 1
          : index,
      );
  }
  return (
    <section
      className="sequence"
      onKeyDown={(event) => {
        if (isEditingTarget(event.target)) return;
        const key = event.key.toLowerCase();
        if ((event.ctrlKey || event.metaKey) && ["c", "x", "v"].includes(key)) {
          event.preventDefault();
          event.stopPropagation();
          if (key === "c") copySelected();
          else if (key === "v") void pasteSelected();
          else copySelected(true);
        } else if (event.key === "Delete" && selected !== null) {
          event.preventDefault();
          event.stopPropagation();
          if (readOnly) return;
          if (trigger) replace(selected, { Actions: [] });
          else update(entries.filter((_, i) => i !== selected));
          setSelected(null);
        }
      }}
      onDragOver={(event) => {
        if(!trigger)dragOver(event,entries.length);
        else if(event.dataTransfer.types.includes(actionType)||event.dataTransfer.types.includes(stepType))event.stopPropagation();
      }}
      onDrop={(event) => {
        if(!trigger)drop(event,entries.length);
        else if(event.dataTransfer.types.includes(actionType)||event.dataTransfer.types.includes(stepType)){
          event.preventDefault();event.stopPropagation();
          if(!readOnly&&!event.currentTarget.closest("fieldset:disabled"))onError(new Error("Drop the action onto a gesture slot."));
        }
      }}
      onDragLeave={(event) => {if(!(event.relatedTarget instanceof Node)||!event.currentTarget.contains(event.relatedTarget))setDropIndex(null);}}
      onDragEnd={() => {dragging.current=null;setDropIndex(null);}}
      aria-label={
        stack
          ? "Dial stack"
          : wheel
            ? "Action wheel"
            : trigger
              ? logic
                ? "Key logic"
                : "Action trigger"
              : "Action sequence"
      }
    >
      {readOnly && depth === 0 && (
        <p className="hint">
          Read-only profile. Select a child to inspect or copy it.
        </p>
      )}
      {uuid.endsWith(".routine2") && (
        <div className="segmented" aria-label="Switch sequence">
          {[0, 1].map((n) => (
            <button
              type="button"
              key={n}
              aria-pressed={branch === n}
              onClick={() => {
                setBranch(n);
                setSelected(null);
              }}
            >
              Sequence {n + 1}
            </button>
          ))}
        </div>
      )}
      {stack && (
        <fieldset
          disabled={readOnly}
          className="sequence-edit-fields stack-options"
        >
          {[
            ["ShowStackIcon", "Show stack icon", true],
            ["FastNavigation", "Hold and turn to choose", false],
          ]
            .filter(([field]) => encoderDisplay || field !== "ShowStackIcon")
            .map(([field, label, fallback]) => (
              <Check
                key={String(field)}
                label={String(label)}
                checked={raw.Settings?.[String(field)] ?? fallback}
                onChange={(value) =>
                  onChange({
                    ...raw,
                    Settings: { ...raw.Settings, [String(field)]: value },
                  })
                }
              />
            ))}
        </fieldset>
      )}
      <div className="sequence-list">
        {(trigger
          ? Array.from({ length: 3 }, (_, i) => entries[i] ?? { Actions: [] })
          : entries
        ).map((entry, index) => (
          <div
            className={
              "sequence-row " +
              (selected === index ? "active" : "") +
              (!trigger && dropIndex === index ? " drop-before" : "") +
              (clipboard?.mode === "cut" &&
              runtimeChild(entry)?.context === clipboard.context
                ? " cut-control"
                : "")
            }
            key={entry.ActionID === undefined || (actionIdCounts.get(String(entry.ActionID)) ?? 0) > 1 ? `position:${index}` : `id:${entry.ActionID}`}
            onDragOver={(event) => dragOver(event,insertion(event,index))}
            onDrop={(event) => drop(event,insertion(event,index))}
          >
            {!trigger && <button type="button" className="sequence-drag" disabled={readOnly}
              aria-label={`Drag action ${index+1} to reorder`} title="Drag to reorder. You can also use the Move up and Move down buttons."
              draggable={!readOnly} onClick={()=>setSelected(index)}
              onDragStart={(event)=>{
                event.stopPropagation();
                if(readOnly || event.currentTarget.matches(":disabled")){event.preventDefault();return;}
                dragging.current={index,branch,signature:JSON.stringify(entries)};
                event.dataTransfer.setData(stepType,JSON.stringify({owner,branch,index}));
                event.dataTransfer.setData("text/plain",childLabel(entry));event.dataTransfer.effectAllowed="move";
              }}>⠿</button>}
            <button
              type="button"
              className="sequence-select"
              onClick={() => setSelected(index)}
            >
              <span className="step-number">
                {trigger
                  ? (logic ? ["1×", "2×", "Hold"] : ["↶", "↓", "↷"])[index]
                  : index + 1}
              </span>
              {runtimeChild(entry)?.preview && (
                <img
                  className="sequence-preview"
                  src={`data:image/png;base64,${runtimeChild(entry)!.preview}`}
                  alt=""
                  aria-hidden="true"
                />
              )}
              <span>
                <strong>{childLabel(entry)}</strong>
                {trigger && (
                  <small>
                    {
                      (logic
                        ? ["Single press", "Double press", "Press and hold"]
                        : ["Turn left", "Press / tap", "Turn right"])[index]
                    }
                  </small>
                )}
              </span>
            </button>
            <fieldset
              disabled={readOnly}
              className="sequence-edit-fields row compact"
            >
              {wheel && runtime && run && runtimeChild(entry) && (
                <button
                  type="button"
                  aria-label={`Show action ${index + 1} on dial`}
                  onClick={() =>
                    run({
                      command: "wheel_select",
                      context: runtime.context,
                      index: runtimeEntries.findIndex(
                        (child) =>
                          child?.context === runtimeChild(entry)?.context,
                      ),
                    })
                  }
                >
                  Show
                </button>
              )}

              {!trigger && (
                <>
                  <button
                    type="button"
                    title="Move up"
                    aria-label={`Move action ${index + 1} up`}
                    disabled={index === 0}
                    onClick={() => move(index, -1)}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    title="Move down"
                    aria-label={`Move action ${index + 1} down`}
                    disabled={index === entries.length - 1}
                    onClick={() => move(index, 1)}
                  >
                    ↓
                  </button>
                </>
              )}
              <button
                type="button"
                aria-label={`Remove action ${index + 1}`}
                title="Remove"
                onClick={() => {
                  if (trigger) replace(index, { Actions: [] });
                  else update(entries.filter((_, i) => i !== index));
                  setSelected(null);
                }}
              >
                ×
              </button>
            </fieldset>
          </div>
        ))}
        {!trigger && !readOnly && <div className={`sequence-drop-target${entries.length ? " compact" : ""}${dropIndex===entries.length ? " dropping" : ""}`}
          onDragOver={(event)=>dragOver(event,entries.length)} onDrop={(event)=>drop(event,entries.length)}>
          <strong>{entries.length ? "Drop an action to add a step" : "Drag actions here"}</strong>
          {!entries.length && <span>From the action library on the right, in the order you want them to run.</span>}
        </div>}
      </div>
      {clipboardRun && (
        <div
          className="row sequence-clipboard"
          aria-label="Nested action clipboard"
        >
          <button
            type="button"
            disabled={!clipboardReady || !liveChild}
            onClick={() => copySelected()}
          >
            Copy selected
          </button>
          <button
            type="button"
            disabled={readOnly || !clipboardReady || !liveChild}
            onClick={() => copySelected(true)}
          >
            Cut selected
          </button>
          <button
            type="button"
            disabled={
              readOnly ||
              !clipboardReady ||
              !runtime ||
              !canPaste ||
              (trigger && selected === null)
            }
            onClick={() => void pasteSelected()}
          >
            {trigger
              ? "Paste into selected"
              : selected === null
                ? "Paste at end"
                : "Paste after selected"}
          </button>
          {!clipboardReady && (
            <span className="hint">
              Save or revert changes before copying or pasting.
            </span>
          )}
        </div>
      )}
      <fieldset
        disabled={readOnly}
        className="sequence-edit-fields row add-action"
      >
        <select
          aria-label="Action to add"
          value={add}
          onChange={(e) => setAdd(e.target.value)}
        >
          <option value="">Choose an action…</option>
          {compatible.map((a) => (
            <option key={a.uuid} value={a.uuid}>
              {a.name} · {a.plugin_name}
            </option>
          ))}
        </select>
        <button
          type="button"
          disabled={
            !compatible.some((action) => action.uuid === add) ||
            (trigger && selected === null)
          }
          onClick={() => {
            const action = compatible.find((a) => a.uuid === add);
            if (!action) return;
            const child = newChild(action);
            if (trigger) replace(selected!, child);
            else {
              update([...entries, child]);
              setSelected(entries.length);
            }
            setAdd("");
          }}
        >
          {trigger ? "Assign" : "Add"}
        </button>
      </fieldset>
      {wheel && (
        <details className="wheel-options">
          <summary>Wheel options</summary>
          <fieldset
            disabled={readOnly}
            className="sequence-edit-fields stack-options"
          >
            <Check
              label="Reverse rotation"
              checked={raw.Settings?.InvertRotation ?? false}
              onChange={(value) =>
                onChange({
                  ...raw,
                  Settings: { ...raw.Settings, InvertRotation: value },
                })
              }
            />
            {encoderDisplay && (
              <Check
                label="Show selected title across the dial display"
                checked={raw.Settings?.ExtTitles ?? false}
                onChange={(value) =>
                  onChange({
                    ...raw,
                    Settings: { ...raw.Settings, ExtTitles: value },
                  })
                }
              />
            )}
            <p className="hint">
              Turn to choose an action. Press the dial or tap its centre to run
              it.
            </p>
            <p className="hint">Drop key actions here, or choose one below.</p>
          </fieldset>
        </details>
      )}
      {trigger && selected === null && (
        <p className="hint">
          {readOnly
            ? "Select a gesture to inspect its action."
            : "Select a gesture, then choose its action."}
        </p>
      )}
      {selectedEntry?.UUID && (
        <div className="child-editor">
          <div className="section-label">
            {childLabel(selectedEntry, "Selected action")}
          </div>
          <fieldset disabled={readOnly} className="sequence-edit-fields">
            {liveChild?.has_inspector && !liveChild.plugin_disabled && run && (
              <button
                type="button"
                onClick={() =>
                  run({ command: "open_inspector", context: liveChild.context })
                }
              >
                Plugin settings ↗
              </button>
            )}
            <Field label="Action label">
              <input
                value={selectedEntry.Name ?? ""}
                onChange={(e) =>
                  replace(selected!, { ...selectedEntry, Name: e.target.value })
                }
              />
            </Field>
            {!isContainer(selectedEntry.UUID) && (
              <SettingsFields
                folderPages={folderPages}
                device={device}
                mixer={mixer}
                key={`settings:${selectedEntry.ActionID ?? selected}`}
                editorKey={`${scope}/${selectedEntry.ActionID ?? selected}`}
                uuid={selectedEntry.UUID}
                controller={stack ? "Encoder" : "Keypad"}
                settings={selectedEntry.Settings ?? {}}
                outputs={outputs}
                onError={onError}
                onChange={(settings) =>
                  replace(selected!, { ...selectedEntry, Settings: settings })
                }
              />
            )}
          </fieldset>
          {selectedEntry.UUID === "com.elgato.streamdeck.system.text" && selectedEntry.Settings?.isTypingMode === true && <TypingAccess status={typingStatus} onError={onError} />}
          {selectedEntry.UUID === "com.deckforge.system.command" && device && (
            commandChild ? <>
              {!clipboardReady && commandChild.command_run && <p className="hint">Results belong to the last saved version of this action.</p>}
              <CommandOutput key={commandChild.context} slot={commandChild} device={device} />
            </> : !clipboardReady && <p className="hint">Save or revert the container to match this command to its running instance.</p>
          )}
          {isContainer(selectedEntry.UUID) && depth < 8 && (
            <Sequence
              typingStatus={typingStatus}
              folderPages={folderPages}
              encoderDisplay={encoderDisplay}
              device={device}
              mixer={mixer}
              key={`sequence:${selectedEntry.ActionID ?? selected}`}
              scope={`${scope}/${selectedEntry.ActionID ?? selected}`}
              runtime={liveChild ?? undefined}
              run={run}
              clipboard={clipboard}
              clipboardRun={clipboardRun}
              clipboardReady={clipboardReady}
              readOnly={readOnly}
              raw={selectedEntry}
              uuid={selectedEntry.UUID}
              actions={actions}
              outputs={outputs}
              depth={depth + 1}
              onError={onError}
              onChange={(next) => replace(selected!, next)}
            />
          )}
          {isContainer(selectedEntry.UUID) && depth >= 8 && (
            <p className="hint">
              Use the advanced action record for deeper nesting.
            </p>
          )}
          <fieldset disabled={readOnly} className="sequence-edit-fields">
            {isMulti(uuid) &&
              selectedEntry.UUID !==
                "com.elgato.streamdeck.multiactions.delay" && (
                <>
                  <SelectField
                    label="State"
                    value={selectedEntry.OverrideState ?? ""}
                    options={[
                      ["", "Default (state 1)"],
                      ...(selectedEntry.States ?? []).map(
                        (state: RecordValue, i: number) =>
                          [
                            i,
                            `State ${i + 1}${state.Title ? ` · ${state.Title}` : ""}`,
                          ] as [number, string],
                      ),
                    ]}
                    onChange={(value) => {
                      const next = { ...selectedEntry };
                      if (value === "") delete next.OverrideState;
                      else next.OverrideState = Number(value);
                      replace(selected!, next);
                    }}
                  />
                  <Check
                    label="Custom action timing"
                    checked={selectedEntry.MultiActionDelays !== undefined}
                    onChange={(enabled) => {
                      const next = { ...selectedEntry };
                      if (enabled)
                        next.MultiActionDelays = { Delay1: 100, Delay2: 100 };
                      else delete next.MultiActionDelays;
                      replace(selected!, next);
                    }}
                  />
                  {selectedEntry.MultiActionDelays !== undefined ? (
                    <>
                      {(
                        [
                          ["Delay1", "Hold duration (milliseconds)"],
                          ["Delay2", "Wait after action (milliseconds)"],
                        ] as const
                      ).map(([field, label]) => (
                        <NumberField
                          key={field}
                          editorKey={`${scope}/${selectedEntry.ActionID ?? selected}`}
                          label={label}
                          value={
                            selectedEntry.MultiActionDelays?.[field] ?? 100
                          }
                          min={1}
                          max={100}
                          onChange={(value) =>
                            replace(selected!, {
                              ...selectedEntry,
                              MultiActionDelays: {
                                ...selectedEntry.MultiActionDelays,
                                Delay1:
                                  selectedEntry.MultiActionDelays?.Delay1 ??
                                  100,
                                Delay2:
                                  selectedEntry.MultiActionDelays?.Delay2 ??
                                  100,
                                [field]: value,
                              },
                            })
                          }
                        />
                      ))}
                    </>
                  ) : (
                    <p className="hint">
                      Uses the plugin's timing when supplied, otherwise 100 ms
                      held and 100 ms before the next action.
                    </p>
                  )}
                </>
              )}
            <JsonEditor
              key={`raw:${selectedEntry.ActionID ?? selected}`}
              editorKey={`${scope}/${selectedEntry.ActionID ?? selected}`}
              label="Advanced action record"
              value={selectedEntry}
              onChange={(next) => replace(selected!, next)}
            />
          </fieldset>
        </div>
      )}
      <fieldset disabled={readOnly} className="sequence-edit-fields">
        <JsonEditor
          editorKey={scope}
          label="Advanced container record"
          value={raw}
          onChange={onChange}
        />
      </fieldset>
    </section>
  );
}
