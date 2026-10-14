import { Check } from "./Fields";
import type { Slot, RecordValue } from "./model";
export function PinOptions({
  slot,
  pages,
  run,
}: {
  slot: Slot;
  pages: { id: string; name: string; folder?: boolean }[];
  run: (request: RecordValue) => unknown;
}) {
  const excluded = slot.pin_excluded_pages ?? [];
  return (
    <div className="pin-options">
      <Check
        label="Pin on every page"
        checked={slot.pinned}
        onChange={(pinned) =>
          run({ command: "slot_pin", context: slot.context, pinned })
        }
      />
      {slot.pinned && (
        <details>
          <summary>
            Exclude pages{excluded.length ? ` (${excluded.length})` : ""}
          </summary>
          <p className="hint">
            Checked pages hide this pinned control. You can always edit it under
            Pinned actions.
          </p>
          {pages.map((page, index) => (
            <Check
              key={page.id}
              label={`${page.folder ? "Folder" : `Page ${index + 1}`}: ${page.name}`}
              checked={excluded.includes(page.id)}
              onChange={(checked) =>
                run({
                  command: "slot_pin_exclusions",
                  context: slot.context,
                  expected: excluded,
                  excluded_pages: checked
                    ? [...excluded, page.id]
                    : excluded.filter((id) => id !== page.id),
                })
              }
            />
          ))}
        </details>
      )}
    </div>
  );
}
