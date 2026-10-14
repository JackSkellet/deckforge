import { useEffect, useRef, useState } from "react";
import type { Clipboard, Slot } from "./model";

/** Leave text editing shortcuts to the focused editor, including future rich inputs. */
export function isEditingTarget(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    !!target.closest(
      'input,textarea,select,[contenteditable="true"],[role="textbox"],[role="combobox"]',
    )
  );
}
export function ActionEdit({
  slot,
  clipboard,
  readonly,
  busy,
  onCopy,
  onCut,
  onPaste,
  onRemove,
  onCancelCut,
  onCreateLogic,
  onImport,
  onExport,
  openRequest,
  destinationAvailable = true,
}: {
  slot: Slot | undefined;
  clipboard: Clipboard | null | undefined;
  readonly: boolean;
  busy: boolean;
  onCopy: () => void;
  onCut: () => void;
  onPaste: () => void;
  onRemove: () => void;
  onCancelCut: () => void;
  onCreateLogic?: () => void;
  onImport?: () => void;
  onExport?: () => void;
  destinationAvailable?: boolean;
  openRequest?: { id: number; anchor?: { x: number; y: number } };
}) {
  const ref = useRef<HTMLDetailsElement>(null);
  const [position, setPosition] = useState<{ left: number; top: number }>({
    left: 0,
    top: 0,
  });
  const requestedAnchor = useRef<{ x: number; y: number } | undefined>(
    undefined,
  );
  function positionMenu() {
    if (!ref.current?.open) return;
    const rect = ref.current.querySelector("summary")!.getBoundingClientRect();
    const height = ref.current
      .querySelector(".control-edit-options")!
      .getBoundingClientRect().height;
    const anchor = requestedAnchor.current;
    setPosition({
      left: Math.max(
        8,
        Math.min(anchor?.x ?? rect.right - 230, window.innerWidth - 238),
      ),
      top: Math.max(
        8,
        Math.min(anchor?.y ?? rect.bottom + 5, window.innerHeight - height - 8),
      ),
    });
  }
  useEffect(() => {
    if (!openRequest?.id || !ref.current) return;
    requestedAnchor.current = openRequest.anchor;
    ref.current.open = true;
    positionMenu();
    (
      ref.current.querySelector<HTMLButtonElement>("button:not(:disabled)") ??
      ref.current.querySelector<HTMLElement>("summary")
    )?.focus();
  }, [openRequest]);
  function choose(callback: () => void) {
    if (ref.current) {
      ref.current.open = false;
      ref.current.querySelector("summary")?.focus();
    }
    callback();
  }
  return (
    <details
      className="control-edit-menu"
      ref={ref}
      onToggle={positionMenu}
      onBlur={(event) => {
        if (
          event.relatedTarget instanceof Node &&
          !event.currentTarget.contains(event.relatedTarget)
        )
          event.currentTarget.open = false;
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          event.currentTarget.open = false;
          event.currentTarget.querySelector("summary")?.focus();
        }
      }}
    >
      <summary
        aria-label="Edit selected control"
        onClick={() => {
          requestedAnchor.current = undefined;
        }}
      >
        Edit ▾
      </summary>
      <div
        style={{ position: "fixed", right: "auto", width: 230, ...position }}
        className="control-edit-options"
        aria-label="Action editing commands"
      >
        <button
          type="button"
          disabled={!slot || busy}
          onClick={() => choose(onCopy)}
        >
          Copy <kbd>Ctrl C</kbd>
        </button>
        <button
          type="button"
          disabled={!slot || readonly || busy}
          onClick={() => choose(onCut)}
        >
          Cut <kbd>Ctrl X</kbd>
        </button>
        <button
          type="button"
          disabled={!clipboard || readonly || busy || !destinationAvailable}
          onClick={() => choose(onPaste)}
        >
          Paste <kbd>Ctrl V</kbd>
        </button>
        {onImport && (
          <button
            type="button"
            disabled={readonly || busy || !destinationAvailable}
            onClick={() => choose(onImport)}
          >
            Import Action…
          </button>
        )}
        {onExport && (
          <button
            type="button"
            disabled={!slot || busy}
            onClick={() => choose(onExport)}
          >
            Export Action…
          </button>
        )}
        {onCreateLogic && (
          <button
            type="button"
            disabled={readonly || busy || !destinationAvailable}
            onClick={() => choose(onCreateLogic)}
          >
            Create Key Logic
          </button>
        )}
        <button
          type="button"
          className="danger quiet"
          disabled={!slot || readonly || busy}
          onClick={() => choose(onRemove)}
        >
          Remove <kbd>Delete</kbd>
        </button>
        {clipboard?.mode === "cut" && (
          <button
            type="button"
            disabled={busy}
            onClick={() => choose(onCancelCut)}
          >
            Cancel cut <kbd>Esc</kbd>
          </button>
        )}
        <p className="clipboard-caption">
          {clipboard
            ? `${clipboard.mode === "cut" ? "Cut" : "Copied"}: ${clipboard.name} · ${clipboard.controller === "Keypad" ? "Key" : clipboard.controller === "Neo" ? "Infobar" : "Dial"}`
            : "Action clipboard is empty"}
          {clipboard?.device && (
            <span className="clipboard-source">
              Source device: {clipboard.device}
            </span>
          )}
        </p>
      </div>
    </details>
  );
}
